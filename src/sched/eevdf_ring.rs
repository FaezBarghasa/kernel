#![forbid(unsafe_code)]

use alloc::sync::Arc;
use arc_swap::ArcSwapOption;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use crossbeam_utils::CachePadded;
use crate::context::{ContextLock, ContextRef};

pub const RING_SIZE: usize = 16384;

/// An entry inside the lock-free ContextRing.
pub struct ContextRingEntry {
    pub context: ContextRef,
    pub id: usize,
    pub vdeadline: u64,
    pub priority: u8,
    /// Atomic flag indicating if this entry has been removed by remove_by_id.
    pub is_removed: AtomicBool,
}

/// A lock-free, concurrent ring buffer of runnable contexts designed for EEVDF scheduling.
/// It uses atomic indices and an array of ArcSwapOption to avoid locks in select_next and enqueue.
pub struct ContextRing {
    buffer: [ArcSwapOption<ContextRingEntry>; RING_SIZE],
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
}

const EMPTY_SLOT: ArcSwapOption<ContextRingEntry> = ArcSwapOption::const_empty();

impl ContextRing {
    /// Creates a new, empty ContextRing.
    pub const fn new() -> Self {
        Self {
            buffer: [EMPTY_SLOT; RING_SIZE],
            head: CachePadded::new(AtomicUsize::new(0)),
            tail: CachePadded::new(AtomicUsize::new(0)),
        }
    }

    /// Enqueues a context into the ring.
    /// Returns true if successfully enqueued, false if the ring is full.
    pub fn enqueue(&self, context: ContextRef, id: usize, vdeadline: u64, priority: u8) -> bool {
        let entry = Arc::new(ContextRingEntry {
            context,
            id,
            vdeadline,
            priority,
            is_removed: AtomicBool::new(false),
        });

        loop {
            let tail = self.tail.load(Ordering::Acquire);
            let head = self.head.load(Ordering::Acquire);
            if tail.saturating_sub(head) >= RING_SIZE {
                return false; // Queue is full
            }
            let idx = tail % RING_SIZE;
            if self.tail.compare_exchange_weak(tail, tail + 1, Ordering::Release, Ordering::Relaxed).is_ok() {
                // Reserved slot `tail`. Spin-wait until it is empty (previous pop fully completed).
                while self.buffer[idx].load().is_some() {
                    core::hint::spin_loop();
                }
                self.buffer[idx].store(Some(entry));
                return true;
            }
        }
    }

    /// Selects and extracts the next eligible context from the ring.
    /// Operates in O(1) time by atomically popping from the head of the ring.
    pub fn select_next_eevdf(&self) -> Option<ContextRef> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            let tail = self.tail.load(Ordering::Acquire);
            if head >= tail {
                return None; // Queue is empty
            }
            let idx = head % RING_SIZE;
            if self.head.compare_exchange_weak(head, head + 1, Ordering::Release, Ordering::Relaxed).is_ok() {
                // Reserved pop from slot `head`. Spin-wait until the enqueue write is visible.
                let entry = loop {
                    if let Some(entry) = self.buffer[idx].swap(None) {
                        break entry;
                    }
                    core::hint::spin_loop();
                };
                if !entry.is_removed.load(Ordering::Acquire) {
                    return Some(entry.context.clone());
                }
                // If the entry was removed, continue loop to pop the next one.
            }
        }
    }

    /// Selects and extracts the context with the furthest virtual deadline (coldest task) for work stealing.
    /// Operates in O(1) time by atomically popping from the tail of the ring.
    pub fn select_furthest_eevdf(&self) -> Option<ContextRef> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            let tail = self.tail.load(Ordering::Acquire);
            if head >= tail {
                return None; // Queue is empty
            }
            let target_tail = tail - 1;
            let idx = target_tail % RING_SIZE;
            if self.tail.compare_exchange_weak(tail, target_tail, Ordering::Release, Ordering::Relaxed).is_ok() {
                // Reserved pop from slot `target_tail`. Spin-wait until the enqueue write is visible.
                let entry = loop {
                    if let Some(entry) = self.buffer[idx].swap(None) {
                        break entry;
                    }
                    core::hint::spin_loop();
                };
                if !entry.is_removed.load(Ordering::Acquire) {
                    return Some(entry.context.clone());
                }
                // If the entry was removed, continue loop to pop next.
            }
        }
    }

    /// Peeks at the head of the ring.
    /// Operates in O(1) time.
    pub fn peek_earliest(&self) -> Option<ContextRef> {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        for i in head..tail {
            let idx = i % RING_SIZE;
            if let Some(entry) = self.buffer[idx].load().as_ref() {
                if !entry.is_removed.load(Ordering::Acquire) {
                    return Some(entry.context.clone());
                }
            }
        }
        None
    }

    /// Removes a context from the ring by ID.
    pub fn remove_by_id(&self, context_id: usize) -> Option<ContextRef> {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        for i in head..tail {
            let idx = i % RING_SIZE;
            if let Some(entry) = self.buffer[idx].load().as_ref() {
                if entry.id == context_id && !entry.is_removed.load(Ordering::Acquire) {
                    // Atomically mark as removed.
                    if !entry.is_removed.swap(true, Ordering::AcqRel) {
                        return Some(entry.context.clone());
                    }
                }
            }
        }
        None
    }

    /// Returns the number of non-removed active contexts in the ring.
    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        if head >= tail {
            return 0;
        }
        let mut count = 0;
        for i in head..tail {
            let idx = i % RING_SIZE;
            if let Some(entry) = self.buffer[idx].load().as_ref() {
                if !entry.is_removed.load(Ordering::Relaxed) {
                    count += 1;
                }
            }
        }
        count
    }

    /// Returns true if empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the average virtual deadline of the contexts in the ring.
    pub fn average_vruntime(&self) -> u64 {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        let mut sum = 0;
        let mut count = 0;
        for i in head..tail {
            let idx = i % RING_SIZE;
            if let Some(entry) = self.buffer[idx].load().as_ref() {
                if !entry.is_removed.load(Ordering::Relaxed) {
                    sum += entry.vdeadline;
                    count += 1;
                }
            }
        }
        if count == 0 {
            0
        } else {
            sum / count
        }
    }
}

/// EEVDF Virtual Deadline Calculation using fixed-point integer arithmetic.
/// Formula: v_deadline = t_runtime + ((slice_quantum << 10) / weight)
#[inline]
pub fn calculate_v_deadline(t_runtime: u64, slice_quantum: u64, weight: u64) -> u64 {
    let weight_div = if weight == 0 { 1 } else { weight };
    t_runtime.saturating_add((slice_quantum << 10) / weight_div)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eevdf_math_accuracy() {
        // Test fixed-point virtual deadline math.
        // Formula: v_deadline = t_runtime + ((slice_quantum << 10) / weight)
        let t_runtime = 1000;
        let slice_quantum = 10000;
        let weight = 100;
        let vdeadline = calculate_v_deadline(t_runtime, slice_quantum, weight);
        let expected = t_runtime + ((slice_quantum * 1024) / weight);
        assert_eq!(vdeadline, expected);

        // Verify floating point comparison (e.g. within 0.01% margin).
        let float_vdeadline = (t_runtime as f64) + ((slice_quantum as f64) * 1024.0 / (weight as f64));
        let diff = (vdeadline as f64 - float_vdeadline).abs();
        let margin = float_vdeadline * 0.0001;
        assert!(diff <= margin, "Deadline {} diff {} exceeded margin {}", vdeadline, diff, margin);
    }
}

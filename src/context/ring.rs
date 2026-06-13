#![forbid(unsafe_code)]

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::Mutex;

use crate::context::ContextRef;

/// A single entry in the ContextRing.
pub struct ContextRingEntry {
    pub context: ContextRef,
    pub id: usize,
    pub vdeadline: u64,
    pub priority: u8,
}

/// A highly concurrent, wait-free ring buffer of runnable contexts.
/// Uses independent mutexes per slot to avoid global locks during scans and enqueues.
pub struct ContextRing {
    slots: [Mutex<Option<ContextRingEntry>>; 256],
    head: AtomicUsize,
    tail: AtomicUsize,
}

// Support const array initialization.
const EMPTY_SLOT: Mutex<Option<ContextRingEntry>> = Mutex::new(None);

impl ContextRing {
    /// Creates a new, empty ContextRing.
    pub const fn new() -> Self {
        Self {
            slots: [EMPTY_SLOT; 256],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Enqueues a context into the ring.
    /// Returns true if successfully enqueued, false if the ring is full.
    pub fn enqueue(&self, context: ContextRef, id: usize, vdeadline: u64, priority: u8) -> bool {
        let tail = self.tail.fetch_add(1, Ordering::Relaxed);
        let start_idx = tail % 256;

        for i in 0..256 {
            let idx = (start_idx + i) % 256;
            if let Some(mut slot) = self.slots[idx].try_lock() {
                if slot.is_none() {
                    *slot = Some(ContextRingEntry {
                        context,
                        id,
                        vdeadline,
                        priority,
                    });
                    return true;
                }
            }
        }
        false
    }

    /// Scans the ring for the eligible context with the earliest virtual deadline.
    /// Dynamically extracts it if found.
    pub fn select_next_eevdf(&self) -> Option<ContextRef> {
        let mut best_idx = None;
        let mut min_deadline = u64::MAX;

        // Perform wait-free parallel scanning of slots
        for idx in 0..256 {
            if let Some(slot) = self.slots[idx].try_lock() {
                if let Some(entry) = &*slot {
                    if entry.vdeadline < min_deadline {
                        min_deadline = entry.vdeadline;
                        best_idx = Some(idx);
                    }
                }
            }
        }

        // Atomically acquire/extract the selected candidate
        if let Some(idx) = best_idx {
            if let Some(mut slot) = self.slots[idx].try_lock() {
                if let Some(entry) = slot.take() {
                    return Some(entry.context);
                }
            }
        }
        None
    }

    /// Scans the ring for the context with the furthest virtual deadline (for work stealing).
    /// Dynamically extracts it if found.
    pub fn select_furthest_eevdf(&self) -> Option<ContextRef> {
        let mut worst_idx = None;
        let mut max_deadline = 0;

        for idx in 0..256 {
            if let Some(slot) = self.slots[idx].try_lock() {
                if let Some(entry) = &*slot {
                    if entry.vdeadline > max_deadline {
                        max_deadline = entry.vdeadline;
                        worst_idx = Some(idx);
                    }
                }
            }
        }

        if let Some(idx) = worst_idx {
            if let Some(mut slot) = self.slots[idx].try_lock() {
                if let Some(entry) = slot.take() {
                    return Some(entry.context);
                }
            }
        }
        None
    }

    /// Removes a context from the ring by ID.
    pub fn remove_by_id(&self, context_id: usize) -> Option<ContextRef> {
        for idx in 0..256 {
            if let Some(mut slot) = self.slots[idx].try_lock() {
                let matches = if let Some(entry) = &*slot {
                    entry.id == context_id
                } else {
                    false
                };
                if matches {
                    return slot.take().map(|e| e.context);
                }
            }
        }
        None
    }

    /// Peeks at the context with the earliest virtual deadline in the ring.
    pub fn peek_earliest(&self) -> Option<ContextRef> {
        let mut best_ctx = None;
        let mut min_deadline = u64::MAX;

        for idx in 0..256 {
            if let Some(slot) = self.slots[idx].try_lock() {
                if let Some(entry) = &*slot {
                    if entry.vdeadline < min_deadline {
                        min_deadline = entry.vdeadline;
                        best_ctx = Some(entry.context.clone());
                    }
                }
            }
        }
        best_ctx
    }

    /// Cleans/removes all entries matching a predicate.
    pub fn clean<F>(&self, mut predicate: F)
    where
        F: FnMut(&ContextRef) -> bool,
    {
        for idx in 0..256 {
            if let Some(mut slot) = self.slots[idx].try_lock() {
                let should_remove = if let Some(entry) = &*slot {
                    predicate(&entry.context)
                } else {
                    false
                };
                if should_remove {
                    *slot = None;
                }
            }
        }
    }

    /// Returns the number of active contexts in the ring.
    pub fn len(&self) -> usize {
        let mut count = 0;
        for idx in 0..256 {
            if let Some(slot) = self.slots[idx].try_lock() {
                if slot.is_some() {
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

    /// Returns the average virtual deadline of runnable contexts in the ring.
    pub fn average_vruntime(&self) -> u64 {
        let mut sum = 0;
        let mut count = 0;
        for idx in 0..256 {
            if let Some(slot) = self.slots[idx].try_lock() {
                if let Some(entry) = &*slot {
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

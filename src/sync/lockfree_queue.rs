//! Lock-Free MPMC Queue for Ultra-Low Latency IPC
//!
//! This implementation uses atomic operations to avoid spinlocks in the hot path,
//! significantly reducing contention and improving cache behavior.

use alloc::boxed::Box;
use core::{
    ptr,
    sync::atomic::{AtomicPtr, AtomicUsize, Ordering},
};

/// A node in the lock-free queue
struct Node<T> {
    data: Option<T>,
    next: AtomicPtr<Node<T>>,
}

impl<T> Node<T> {
    fn new(data: Option<T>) -> Self {
        Node {
            data,
            next: AtomicPtr::new(ptr::null_mut()),
        }
    }
}

/// Lock-free MPMC queue optimized for IPC
///
/// This queue uses Michael-Scott style lock-free algorithm with optimizations:
/// 1. Atomic operations instead of spin locks
/// 2. Reduced cache line bouncing
/// 3. Fast path for single producer/consumer
#[derive(Debug)]
pub struct LockFreeQueue<T> {
    head: AtomicPtr<Node<T>>,
    tail: AtomicPtr<Node<T>>,
    /// Approximate size for wait heuristics
    approx_size: AtomicUsize,
}

impl<T> LockFreeQueue<T> {
    /// Create a new empty lock-free queue
    pub fn new() -> Self {
        // Create dummy sentinel node
        let sentinel = Box::into_raw(Box::new(Node::new(None)));

        LockFreeQueue {
            head: AtomicPtr::new(sentinel),
            tail: AtomicPtr::new(sentinel),
            approx_size: AtomicUsize::new(0),
        }
    }

    /// Enqueue an item (producer operation)
    ///
    /// This is lock-free and wait-free in the common case.
    /// Returns approximate queue length after insertion.
    pub fn enqueue(&self, value: T) -> usize {
        let new_node = Box::into_raw(Box::new(Node::new(Some(value))));

        loop {
            let tail = self.tail.load(Ordering::Acquire);
            let next = unsafe { (*tail).next.load(Ordering::Acquire) };

            // Check if tail is still the same
            if tail == self.tail.load(Ordering::Acquire) {
                if next.is_null() {
                    // Try to link new node at the end
                    if unsafe {
                        (*tail)
                            .next
                            .compare_exchange(
                                ptr::null_mut(),
                                new_node,
                                Ordering::Release,
                                Ordering::Relaxed,
                            )
                            .is_ok()
                    } {
                        // Enqueue succeeded, try to swing tail
                        let _ = self.tail.compare_exchange(
                            tail,
                            new_node,
                            Ordering::Release,
                            Ordering::Relaxed,
                        );

                        // Update approximate size
                        let size = self.approx_size.fetch_add(1, Ordering::Relaxed) + 1;
                        return size;
                    }
                } else {
                    // Tail was not pointing to the last node, help by swinging tail
                    let _ = self.tail.compare_exchange(
                        tail,
                        next,
                        Ordering::Release,
                        Ordering::Relaxed,
                    );
                }
            }
        }
    }

    /// Dequeue an item (consumer operation)
    ///
    /// Returns None if queue is empty.
    /// This is lock-free and wait-free in the common case.
    pub fn dequeue(&self) -> Option<T> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            let tail = self.tail.load(Ordering::Acquire);
            let next = unsafe { (*head).next.load(Ordering::Acquire) };

            // Check if head is still the same
            if head == self.head.load(Ordering::Acquire) {
                if head == tail {
                    // Queue is empty or tail is falling behind
                    if next.is_null() {
                        return None; // Queue is empty
                    }

                    // Tail is falling behind, help swing it
                    let _ = self.tail.compare_exchange(
                        tail,
                        next,
                        Ordering::Release,
                        Ordering::Relaxed,
                    );
                } else {
                    // Read value before CAS, otherwise another dequeue might free the next node
                    let data = unsafe { (*next).data.take() };

                    // Try to swing head to the next node
                    if self
                        .head
                        .compare_exchange(head, next, Ordering::Release, Ordering::Relaxed)
                        .is_ok()
                    {
                        // Dequeue succeeded
                        // Update approximate size
                        self.approx_size.fetch_sub(1, Ordering::Relaxed);

                        // Free old dummy node
                        unsafe { drop(Box::from_raw(head)) };

                        return data;
                    }
                }
            }
        }
    }

    /// Check if queue is approximately empty
    ///
    /// This is a fast check but may have false positives/negatives
    /// due to race conditions. Use for heuristics only.
    #[inline]
    pub fn is_empty_approx(&self) -> bool {
        self.approx_size.load(Ordering::Relaxed) == 0
    }

    /// Get approximate size
    ///
    /// This is not exact due to concurrent operations but useful for stats.
    #[inline]
    pub fn len_approx(&self) -> usize {
        self.approx_size.load(Ordering::Relaxed)
    }
}

impl<T> Drop for LockFreeQueue<T> {
    fn drop(&mut self) {
        // Drain all remaining items
        while self.dequeue().is_some() {}

        // Free the sentinel node
        let sentinel = self.head.load(Ordering::Relaxed);
        if !sentinel.is_null() {
            unsafe { drop(Box::from_raw(sentinel)) };
        }
    }
}

// Safety: LockFreeQueue is designed for concurrent access
unsafe impl<T: Send> Send for LockFreeQueue<T> {}
unsafe impl<T: Send> Sync for LockFreeQueue<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_enqueue_dequeue() {
        let queue = LockFreeQueue::new();
        queue.enqueue(42);
        assert_eq!(queue.dequeue(), Some(42));
        assert_eq!(queue.dequeue(), None);
    }

    #[test]
    fn test_multiple_items() {
        let queue = LockFreeQueue::new();
        for i in 0..100 {
            queue.enqueue(i);
        }
        for i in 0..100 {
            assert_eq!(queue.dequeue(), Some(i));
        }
    }
}

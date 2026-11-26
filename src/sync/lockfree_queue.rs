//! Lock-Free MPMC Queue (Fallback to Mutex for Stability)
//!
//! This module provides a Multi-Producer Multi-Consumer (MPMC) queue.
//!
//! # Implementation Details
//!
//! Originally designed as a lock-free queue using atomic operations, it was found to be unsafe
//! in the current kernel environment due to the lack of a robust Epoch-Based Reclamation (EBR)
//! or Hazard Pointer system. Without proper memory reclamation, the lock-free dequeue operation
//! suffered from Use-After-Free (UAF) issues (ABA problem on head node).
//!
//! To ensure system stability and correctness ("Production Hardening"), the implementation
//! has been temporarily replaced with a `spin::Mutex` protecting a standard `VecDeque`.
//! This ensures thread safety and correctness at the cost of some performance (locking overhead).
//!
//! # Future Work
//!
//! - Implement a safe memory reclamation scheme (e.g., EBR).
//! - Restore the lock-free Michael-Scott queue algorithm once UAF prevention is in place.
//!
//! # Safety
//!
//! The current implementation uses a `Mutex`, so it is safe for concurrent access (Send + Sync).

use alloc::collections::VecDeque;
use spin::Mutex;

/// A thread-safe queue implementation (currently Mutex-backed).
#[derive(Debug)]
pub struct LockFreeQueue<T> {
    inner: Mutex<VecDeque<T>>,
}

impl<T> LockFreeQueue<T> {
    /// Create a new empty queue.
    pub fn new() -> Self {
        LockFreeQueue {
            inner: Mutex::new(VecDeque::new()),
        }
    }

    /// Enqueue an item (Producer).
    pub fn enqueue(&self, value: T) -> usize {
        let mut queue = self.inner.lock();
        queue.push_back(value);
        queue.len()
    }

    /// Dequeue an item (Consumer).
    ///
    /// Returns `None` if the queue is empty.
    pub fn dequeue(&self) -> Option<T> {
        let mut queue = self.inner.lock();
        queue.pop_front()
    }

    /// Check if queue is empty.
    pub fn is_empty_approx(&self) -> bool {
        self.inner.lock().is_empty()
    }

    /// Get approximate length.
    pub fn len_approx(&self) -> usize {
        self.inner.lock().len()
    }
}

// Safety: Mutex is safe
unsafe impl<T: Send> Send for LockFreeQueue<T> {}
unsafe impl<T: Send> Sync for LockFreeQueue<T> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::vec::Vec;

    #[test]
    #[ignore]
    fn test_basic_enqueue_dequeue() {
        let queue = LockFreeQueue::new();
        queue.enqueue(42);
        assert_eq!(queue.dequeue(), Some(42));
        assert_eq!(queue.dequeue(), None);
    }

    #[test]
    #[ignore]
    fn test_multiple_items() {
        let queue = LockFreeQueue::new();
        for i in 0..100 {
            queue.enqueue(i);
        }
        for i in 0..100 {
            assert_eq!(queue.dequeue(), Some(i));
        }
    }

    #[test]
    #[ignore]
    fn test_mpsc_stress() {
        let queue = Arc::new(LockFreeQueue::new());
        let producer_count = 8;
        let items_per_producer = 10000;

        let mut handles = Vec::new();

        for i in 0..producer_count {
            let q = queue.clone();
            handles.push(thread::spawn(move || {
                for j in 0..items_per_producer {
                    q.enqueue(i * items_per_producer + j);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let mut count = 0;
        while let Some(_) = queue.dequeue() {
            count += 1;
        }

        assert_eq!(count, producer_count * items_per_producer);
    }
}

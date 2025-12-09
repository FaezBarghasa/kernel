//! Lock-Free MPMC Queue (Fallback to Mutex for Stability)
//!
//! This implementation currently uses a Mutex because the lock-free implementation
//! was unsafe (Use-After-Free) without a proper memory reclamation strategy (e.g. EBR).
//! TODO: Re-implement using crossbeam-queue or implement EBR.

use alloc::collections::VecDeque;
use spin::Mutex;

#[derive(Debug)]
pub struct LockFreeQueue<T> {
    inner: Mutex<VecDeque<T>>,
}

impl<T> LockFreeQueue<T> {
    /// Create a new queue
    pub fn new() -> Self {
        LockFreeQueue {
            inner: Mutex::new(VecDeque::new()),
        }
    }

    /// Enqueue an item
    pub fn enqueue(&self, value: T) -> usize {
        let mut queue = self.inner.lock();
        queue.push_back(value);
        queue.len()
    }

    /// Dequeue an item
    pub fn dequeue(&self) -> Option<T> {
        let mut queue = self.inner.lock();
        queue.pop_front()
    }

    /// Check if queue is empty
    pub fn is_empty_approx(&self) -> bool {
        self.inner.lock().is_empty()
    }

    /// Get length
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

#![forbid(unsafe_code)]

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use crossbeam_utils::atomic::AtomicCell;
#[allow(unused_imports)]
use crossbeam_epoch::{self, Atomic, Owned, Shared};

use crate::sched::eevdf_types::{EevdfTask, RunqueueStats};

/// Lock-free circular ring buffer for EEVDF task scheduling.
///
/// This structure manages a fixed-size array of tasks using atomic operations
/// to allow concurrent access without locks. It maintains head and tail pointers
/// to track the active task range.
pub struct ContextRing {
    /// Fixed-size array of task slots
    tasks: Vec<AtomicCell<Option<EevdfTask>>>,

    /// Atomic head pointer (index of first valid task)
    head: AtomicUsize,

    /// Atomic tail pointer (index one past last valid task)
    tail: AtomicUsize,

    /// Maximum capacity of the ring
    capacity: usize,

    /// Atomic counter for total tasks currently in the ring
    task_count: AtomicUsize,

    /// Cached minimum vdeadline for O(1) selection
    min_vdeadline: AtomicU64,
}

impl ContextRing {
    /// Creates a new ContextRing with the specified capacity.
    ///
    /// # Arguments
    /// * `capacity` - Maximum number of tasks the ring can hold
    ///
    /// # Panics
    /// Panics if capacity is 0 or greater than 10,000
    pub fn new(capacity: usize) -> Self {
        if capacity == 0 || capacity > 10_000 {
            panic!("capacity must be between 1 and 10,000");
        }
        let mut tasks = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            tasks.push(AtomicCell::new(None));
        }
        Self {
            tasks,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            capacity,
            task_count: AtomicUsize::new(0),
            min_vdeadline: AtomicU64::new(u64::MAX),
        }
    }

    /// Inserts a task into the ring buffer.
    ///
    /// This operation is lock-free and uses atomic compare-and-swap (compare_exchange)
    /// to ensure thread safety. The task is inserted at the tail position.
    ///
    /// # Arguments
    /// * `task` - The task to insert
    ///
    /// # Returns
    /// * `Ok(())` if insertion succeeded
    /// * `Err(EevdfTask)` if the ring is full (returns the task back)
    pub fn insert(&self, task: EevdfTask) -> Result<(), EevdfTask> {
        loop {
            let count = self.task_count.load(Ordering::Acquire);
            if count >= self.capacity {
                return Err(task);
            }
            let tail = self.tail.load(Ordering::Acquire);
            let next_tail = (tail + 1) % self.capacity;

            if self
                .tail
                .compare_exchange_weak(tail, next_tail, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                // Reserved slot `tail`. Spin-wait until it is empty.
                while self.tasks[tail].load().is_some() {
                    core::hint::spin_loop();
                }
                self.tasks[tail].store(Some(task));
                self.task_count.fetch_add(1, Ordering::SeqCst);

                // Update cached min_vdeadline if this task's deadline is smaller
                let mut current_min = self.min_vdeadline.load(Ordering::Acquire);
                while task.vdeadline < current_min {
                    match self.min_vdeadline.compare_exchange_weak(
                        current_min,
                        task.vdeadline,
                        Ordering::SeqCst,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => break,
                        Err(actual) => current_min = actual,
                    }
                }
                return Ok(());
            }
        }
    }

    /// Selects the next task to schedule (earliest vdeadline).
    ///
    /// This operation scans the active range of the ring to find the task with
    /// the minimum vdeadline.
    ///
    /// # Returns
    /// * `Some(EevdfTask)` if a task is available
    /// * `None` if the ring is empty
    pub fn select_next(&self) -> Option<EevdfTask> {
        let count = self.task_count.load(Ordering::Acquire);
        if count == 0 {
            return None;
        }

        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);

        let mut min_task = None;
        let mut min_deadline = u64::MAX;

        let mut idx = head;
        loop {
            if let Some(task) = self.tasks[idx].load() {
                if task.vdeadline < min_deadline {
                    min_deadline = task.vdeadline;
                    min_task = Some(task);
                }
            }
            if idx == tail {
                break;
            }
            idx = (idx + 1) % self.capacity;
        }

        if let Some(task) = min_task {
            self.min_vdeadline.store(task.vdeadline, Ordering::Release);
            Some(task)
        } else {
            // Fallback: scan the entire buffer if concurrent removals left holes
            let mut fallback_min = None;
            let mut fallback_deadline = u64::MAX;
            for cell in &self.tasks {
                if let Some(task) = cell.load() {
                    if task.vdeadline < fallback_deadline {
                        fallback_deadline = task.vdeadline;
                        fallback_min = Some(task);
                    }
                }
            }
            if let Some(task) = fallback_min {
                self.min_vdeadline.store(task.vdeadline, Ordering::Release);
                Some(task)
            } else {
                None
            }
        }
    }

    /// Removes a task from the ring by task_id.
    ///
    /// # Arguments
    /// * `task_id` - ID of the task to remove
    ///
    /// # Returns
    /// * `Some(EevdfTask)` if task was found and removed
    /// * `None` if task was not found
    pub fn remove(&self, task_id: u64) -> Option<EevdfTask> {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);

        let mut idx = head;
        loop {
            if let Some(task) = self.tasks[idx].load() {
                if task.task_id == task_id {
                    if let Some(taken) = self.tasks[idx].take() {
                        if taken.task_id == task_id {
                            self.task_count.fetch_sub(1, Ordering::SeqCst);
                            self.recalculate_min_vdeadline();
                            return Some(taken);
                        } else {
                            // Put it back if it changed concurrently
                            self.tasks[idx].store(Some(taken));
                        }
                    }
                }
            }
            if idx == tail {
                break;
            }
            idx = (idx + 1) % self.capacity;
        }

        // Fallback scan of the entire buffer
        for idx in 0..self.capacity {
            if let Some(task) = self.tasks[idx].load() {
                if task.task_id == task_id {
                    if let Some(taken) = self.tasks[idx].take() {
                        if taken.task_id == task_id {
                            self.task_count.fetch_sub(1, Ordering::SeqCst);
                            self.recalculate_min_vdeadline();
                            return Some(taken);
                        } else {
                            self.tasks[idx].store(Some(taken));
                        }
                    }
                }
            }
        }
        None
    }

    /// Advances the head pointer after a task completes its time slice.
    ///
    /// This is called after select_next() when the selected task
    /// has finished executing and should be moved to the back of the queue.
    pub fn advance_head(&self) {
        loop {
            let head = self.head.load(Ordering::Acquire);
            let next_head = (head + 1) % self.capacity;
            if self
                .head
                .compare_exchange_weak(head, next_head, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }
    }

    /// Returns current statistics about the ring.
    pub fn stats(&self) -> RunqueueStats {
        let count = self.task_count.load(Ordering::Acquire);
        let mut sum_vdeadline = 0u128;
        let mut max_vd = 0u64;
        let mut total_w = 0u64;
        let mut active_count = 0;

        for cell in &self.tasks {
            if let Some(task) = cell.load() {
                sum_vdeadline += task.vdeadline as u128;
                if task.vdeadline > max_vd {
                    max_vd = task.vdeadline;
                }
                total_w += task.weight as u64;
                active_count += 1;
            }
        }

        let avg_vd = if active_count == 0 {
            0
        } else {
            (sum_vdeadline / active_count as u128) as u64
        };

        let min_vd = self.min_vdeadline.load(Ordering::Acquire);

        RunqueueStats {
            task_count: count,
            avg_vdeadline: avg_vd,
            min_vdeadline: if min_vd == u64::MAX { 0 } else { min_vd },
            max_vdeadline: max_vd,
            total_weight: total_w,
        }
    }

    /// Returns the number of tasks currently in the ring.
    pub fn len(&self) -> usize {
        self.task_count.load(Ordering::Relaxed)
    }

    /// Returns true if the ring is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the capacity of the ring.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Calculates the next index in the circular buffer.
    #[allow(dead_code)]
    fn next_index(&self, current: usize) -> usize {
        (current + 1) % self.capacity
    }

    /// Recalculates the minimum vdeadline across all tasks.
    fn recalculate_min_vdeadline(&self) {
        let mut min_vd = u64::MAX;
        for cell in &self.tasks {
            if let Some(task) = cell.load() {
                if task.vdeadline < min_vd {
                    min_vd = task.vdeadline;
                }
            }
        }
        self.min_vdeadline.store(min_vd, Ordering::Release);
    }
}

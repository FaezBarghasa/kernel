//! # Deadline Scheduler
//!
//! This module implements a Virtual Deadline-based scheduler.
//!
//! The scheduler aims to provide a "premium desktop feel" by prioritizing
//! interactive and low-latency tasks, similar to XanMod's MuQSS/TT.

use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::{
    context::{Context, ContextRef, Status},
    cpu_set::LogicalCpuId,
    percpu::PercpuBlock,
    sync::{CleanLockToken, Priority},
    time::monotonic,
};

/// Base time slice for tasks in nanoseconds.
/// This value can be tuned for responsiveness vs. throughput.
const BASE_TIME_SLICE_NS: u64 = 1_000_000; // 1ms

/// A single run queue for a CPU.
///
/// This will hold runnable contexts, ordered by their virtual deadline.
pub struct RunQueue {
    /// Real-time tasks, ordered by virtual deadline.
    rt_queue: VecDeque<ContextRef>,
    /// Non-real-time tasks, ordered by virtual deadline.
    non_rt_queue: VecDeque<ContextRef>,
}

impl RunQueue {
    pub fn new() -> Self {
        RunQueue {
            rt_queue: VecDeque::new(),
            non_rt_queue: VecDeque::new(),
        }
    }

    /// Adds a context to the appropriate run queue.
    ///
    /// The context is inserted into the queue based on its virtual deadline,
    /// maintaining the sorted order.
    pub fn add(&mut self, context_ref: ContextRef) {
        let new_deadline = context_ref.read().virtual_deadline;
        let is_realtime = context_ref.read().is_realtime;

        let queue = if is_realtime {
            &mut self.rt_queue
        } else {
            &mut self.non_rt_queue
        };

        // Find the correct position to insert based on virtual_deadline
        let mut i = 0;
        while i < queue.len() {
            if new_deadline < queue[i].read().virtual_deadline {
                break;
            }
            i += 1;
        }
        queue.insert(i, context_ref);
    }

    /// Removes and returns the next context to run.
    ///
    /// This will prioritize real-time tasks.
    pub fn next(&mut self) -> Option<ContextRef> {
        if let Some(ctx) = self.rt_queue.pop_front() {
            Some(ctx)
        } else {
            self.non_rt_queue.pop_front()
        }
    }

    /// Removes a specific context from the run queue.
    pub fn remove(&mut self, context_id: usize) -> Option<ContextRef> {
        let mut found_idx = None;
        for (i, c) in self.rt_queue.iter().enumerate() {
            if c.read().id() == context_id {
                found_idx = Some(i);
                break;
            }
        }
        if let Some(i) = found_idx {
            return self.rt_queue.remove(i);
        }

        found_idx = None;
        for (i, c) in self.non_rt_queue.iter().enumerate() {
            if c.read().id() == context_id {
                found_idx = Some(i);
                break;
            }
        }
        found_idx.map(|i| self.non_rt_queue.remove(i).unwrap())
    }

    pub fn is_empty(&self) -> bool {
        self.rt_queue.is_empty() && self.non_rt_queue.is_empty()
    }
}

/// Per-CPU scheduler state.
pub struct Scheduler {
    /// The run queue for this CPU.
    pub run_queue: RunQueue,
    /// The currently running context.
    pub current_context: Option<ContextRef>,
    /// The virtual deadline of the currently running context.
    pub current_virtual_deadline: AtomicU64,
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler {
            run_queue: RunQueue::new(),
            current_context: None,
            current_virtual_deadline: AtomicU64::new(0),
        }
    }

    /// Selects the next context to run.
    pub fn schedule(&mut self, token: &mut CleanLockToken) -> Option<ContextRef> {
        // Update the virtual deadline of the current context before scheduling out
        if let Some(current_ctx_ref) = &self.current_context {
            let mut current_ctx = current_ctx_ref.write(token.token());
            let now = monotonic();
            let time_spent = now.saturating_sub(current_ctx.switch_time);

            // Update last_cpu_id for cache locality
            current_ctx.last_cpu_id = Some(crate::cpu_id());

            // Calculate the virtual deadline based on effective priority
            // Lower effective_priority means higher actual priority.
            // A simple way to incorporate this is to scale the time_spent.
            // For example, higher priority tasks accumulate virtual time slower.
            let priority_factor = current_ctx.priority.effective_priority() as u64 + 1; // Avoid division by zero
            let virtual_time_increase = (time_spent as u64 * BASE_TIME_SLICE_NS) / priority_factor;

            current_ctx.virtual_deadline = current_ctx
                .virtual_deadline
                .saturating_add(virtual_time_increase);

            // If the context is still runnable, add it back to the run queue
            if current_ctx.status.is_runnable() {
                drop(current_ctx); // Drop the write lock before cloning Arc
                self.run_queue.add(current_ctx_ref.clone());
            }
        }

        let next_context = self.run_queue.next();

        if let Some(next_ctx_ref) = &next_context {
            let mut next_ctx = next_ctx_ref.write(token.token());
            next_ctx.switch_time = monotonic();
            self.current_virtual_deadline
                .store(next_ctx.virtual_deadline, Ordering::Relaxed);
        }

        self.current_context = next_context.clone();
        next_context
    }

    /// Called when a context is blocked.
    pub fn context_blocked(&mut self, context_id: usize) {
        self.run_queue.remove(context_id);
    }

    /// Called when a context becomes runnable.
    pub fn context_unblocked(&mut self, context_ref: ContextRef) {
        self.run_queue.add(context_ref);
    }
}

// Per-CPU scheduler instance
pub fn scheduler() -> &'static mut Scheduler {
    &mut PercpuBlock::current().scheduler
}

pub fn schedule_next(token: &mut CleanLockToken) -> Option<ContextRef> {
    scheduler().schedule(token)
}

pub fn add_context(context_ref: ContextRef) {
    // Determine which CPU's run queue to add the context to
    let target_cpu = {
        let context = context_ref.read();
        context.last_cpu_id.unwrap_or_else(crate::cpu_id)
    };

    // Initialize virtual_deadline for new contexts
    {
        let mut context = context_ref.write();
        context.virtual_deadline = PercpuBlock::current().scheduler.current_virtual_deadline.load(Ordering::Relaxed);
    }

    // TODO: Implement proper load balancing if target_cpu's queue is overloaded
    if target_cpu == crate::cpu_id() {
        scheduler().run_queue.add(context_ref);
    } else {
        // For now, if not the current CPU, just add to current CPU's queue
        // In a real implementation, this would involve inter-processor interrupts (IPIs)
        // to add to the target CPU's run queue.
        scheduler().run_queue.add(context_ref);
    }
}

pub fn remove_context(context_id: &usize) {
    scheduler().run_queue.remove(*context_id);
}

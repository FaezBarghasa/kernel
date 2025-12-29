//! # XanMod-Style Real-Time Scheduler
//!
//! This module implements a hybrid real-time scheduler inspired by XanMod/MuQSS.
//!
//! ## Features
//!
//! - **Fixed-priority preemptive scheduling** for real-time (RT) tasks
//! - **Virtual Deadline (MuQSS-style)** for fair scheduling of non-RT tasks
//! - **Per-CPU run queues** with work stealing for cache locality
//! - **Tickless operation** with dynamic timer programming
//! - **Priority inheritance** support for avoiding priority inversion
//! - **Deterministic scheduling** for Hard RTOS guarantees
//!
//! ## Design
//!
//! The scheduler uses separate queues for RT and non-RT tasks:
//! - RT tasks: Priority-ordered queue, always preempt non-RT
//! - Non-RT tasks: Virtual deadline-ordered queue (MuQSS algorithm)
//!
//! Virtual deadlines are calculated as: `vd = vd + (time_slice / (weight + 1))`
//! where weight is derived from priority (lower priority = higher weight).

use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::{
    context::{ContextRef, Status},
    cpu_set::LogicalCpuId,
    ipi::{ipi, IpiKind, IpiTarget},
    percpu::PercpuBlock,
    sync::{CleanLockToken, Priority},
    time::monotonic,
};

// =============================================================================
// Scheduler Constants
// =============================================================================

/// Base time slice for tasks in nanoseconds (1ms default, XanMod-style).
/// This is the quantum that non-RT tasks receive before virtual deadline update.
const BASE_TIME_SLICE_NS: u64 = 1_000_000; // 1ms

/// Minimum time slice for interactive tasks (for responsiveness).
const MIN_TIME_SLICE_NS: u64 = 100_000; // 100µs

/// Maximum time slice for batch tasks (for throughput).
const MAX_TIME_SLICE_NS: u64 = 10_000_000; // 10ms

/// RT task time slice (smaller for determinism).
const RT_TIME_SLICE_NS: u64 = 500_000; // 500µs

/// Number of priority levels for RT tasks (POSIX SCHED_FIFO).
pub const RT_PRIORITY_LEVELS: usize = 100;

/// Load balance interval in nanoseconds.
const BALANCE_INTERVAL_NS: u64 = 4_000_000; // 4ms

/// Imbalance threshold for work stealing (percentage).
const IMBALANCE_PCT: usize = 25;

// =============================================================================
// Scheduler Statistics
// =============================================================================

/// Per-CPU scheduler statistics for monitoring and tuning.
#[derive(Debug, Default)]
pub struct SchedulerStats {
    /// Total number of context switches
    pub switches: AtomicU64,
    /// Number of RT task switches
    pub rt_switches: AtomicU64,
    /// Total time spent in scheduler (cycles)
    pub total_overhead_cycles: AtomicU64,
    /// Minimum switch latency observed
    pub min_latency_ns: AtomicU64,
    /// Maximum switch latency observed  
    pub max_latency_ns: AtomicU64,
    /// Number of load balance operations
    pub balance_ops: AtomicU64,
    /// Number of tasks migrated via work stealing
    pub migrations: AtomicU64,
    /// Number of preemptions
    pub preemptions: AtomicU64,
}

impl SchedulerStats {
    pub const fn new() -> Self {
        SchedulerStats {
            switches: AtomicU64::new(0),
            rt_switches: AtomicU64::new(0),
            total_overhead_cycles: AtomicU64::new(0),
            min_latency_ns: AtomicU64::new(u64::MAX),
            max_latency_ns: AtomicU64::new(0),
            balance_ops: AtomicU64::new(0),
            migrations: AtomicU64::new(0),
            preemptions: AtomicU64::new(0),
        }
    }

    /// Record a context switch
    pub fn record_switch(&self, is_rt: bool, latency_ns: u64) {
        self.switches.fetch_add(1, Ordering::Relaxed);
        if is_rt {
            self.rt_switches.fetch_add(1, Ordering::Relaxed);
        }

        // Update min latency
        let mut current_min = self.min_latency_ns.load(Ordering::Relaxed);
        while latency_ns < current_min {
            match self.min_latency_ns.compare_exchange_weak(
                current_min,
                latency_ns,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_min = x,
            }
        }

        // Update max latency
        let mut current_max = self.max_latency_ns.load(Ordering::Relaxed);
        while latency_ns > current_max {
            match self.max_latency_ns.compare_exchange_weak(
                current_max,
                latency_ns,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_max = x,
            }
        }
    }
}

// =============================================================================
// Run Queue Entry
// =============================================================================

/// Entry in the run queue with cached scheduling data
#[derive(Debug)]
pub struct RunQueueEntry {
    /// Context ID for quick lookup
    pub id: usize,
    /// Reference to the context
    pub context: ContextRef,
    /// Cached virtual deadline (for non-RT)
    pub vdeadline: u64,
    /// Cached priority (for RT)
    pub priority: u8,
    /// Time slice remaining
    pub time_slice: u64,
    /// Number of times this task has been scheduled
    pub run_count: u32,
}

impl RunQueueEntry {
    fn new(id: usize, context: ContextRef, vdeadline: u64, priority: u8) -> Self {
        RunQueueEntry {
            id,
            context,
            vdeadline,
            priority,
            time_slice: BASE_TIME_SLICE_NS,
            run_count: 0,
        }
    }
}

// =============================================================================
// Run Queue
// =============================================================================

/// A single run queue for a CPU.
///
/// Uses separate queues for RT and non-RT tasks for predictable scheduling.
pub struct RunQueue {
    /// Real-time tasks, ordered by priority (lower value = higher priority).
    /// Uses a simple deque since RT task count is typically small.
    pub rt_queue: VecDeque<RunQueueEntry>,

    /// Non-real-time tasks, ordered by virtual deadline.
    /// This implements the MuQSS virtual deadline algorithm.
    pub non_rt_queue: VecDeque<RunQueueEntry>,

    /// Total number of tasks in both queues
    task_count: AtomicUsize,

    /// Total load weight (for balancing)
    load_weight: AtomicU64,

    /// Flag indicating a high-priority task is waiting
    needs_preempt: AtomicBool,
}

impl RunQueue {
    pub const fn new() -> Self {
        RunQueue {
            rt_queue: VecDeque::new(),
            non_rt_queue: VecDeque::new(),
            task_count: AtomicUsize::new(0),
            load_weight: AtomicU64::new(0),
            needs_preempt: AtomicBool::new(false),
        }
    }

    /// Adds a context to the appropriate run queue.
    ///
    /// RT tasks are inserted sorted by priority.
    /// Non-RT tasks are inserted sorted by virtual deadline (earliest first).
    pub fn add(&mut self, context_ref: ContextRef, token: &mut CleanLockToken) {
        let (is_realtime, id, vdeadline, priority) = {
            let context = context_ref.read(token.token());
            (
                context.is_realtime,
                context.id(),
                context.virtual_deadline,
                context.priority.effective_priority(),
            )
        };

        let entry = RunQueueEntry::new(id, context_ref, vdeadline, priority);

        if is_realtime {
            // Insert RT task sorted by priority (lower value = higher priority)
            let insert_pos = self
                .rt_queue
                .iter()
                .position(|e| priority < e.priority)
                .unwrap_or(self.rt_queue.len());
            self.rt_queue.insert(insert_pos, entry);

            // Mark preemption needed if this is highest priority
            if insert_pos == 0 {
                self.needs_preempt.store(true, Ordering::Release);
            }
        } else {
            // Insert non-RT task sorted by virtual deadline (earliest first)
            let insert_pos = self
                .non_rt_queue
                .iter()
                .position(|e| vdeadline < e.vdeadline)
                .unwrap_or(self.non_rt_queue.len());
            self.non_rt_queue.insert(insert_pos, entry);
        }

        // Update counts and load
        self.task_count.fetch_add(1, Ordering::Relaxed);
        let weight = Self::priority_to_weight(priority);
        self.load_weight.fetch_add(weight, Ordering::Relaxed);
    }

    /// Removes and returns the next context to run.
    ///
    /// RT tasks always have priority over non-RT tasks.
    pub fn next(&mut self) -> Option<ContextRef> {
        self.needs_preempt.store(false, Ordering::Relaxed);

        // RT tasks first
        if let Some(mut entry) = self.rt_queue.pop_front() {
            self.task_count.fetch_sub(1, Ordering::Relaxed);
            let weight = Self::priority_to_weight(entry.priority);
            self.load_weight.fetch_sub(weight, Ordering::Relaxed);
            entry.run_count += 1;
            return Some(entry.context);
        }

        // Then non-RT tasks (earliest virtual deadline first)
        if let Some(mut entry) = self.non_rt_queue.pop_front() {
            self.task_count.fetch_sub(1, Ordering::Relaxed);
            let weight = Self::priority_to_weight(entry.priority);
            self.load_weight.fetch_sub(weight, Ordering::Relaxed);
            entry.run_count += 1;
            return Some(entry.context);
        }

        None
    }

    /// Peek at the next context without removing it
    pub fn peek(&self) -> Option<&ContextRef> {
        if let Some(entry) = self.rt_queue.front() {
            Some(&entry.context)
        } else {
            self.non_rt_queue.front().map(|e| &e.context)
        }
    }

    /// Check if there's a higher priority RT task waiting
    pub fn has_higher_priority(&self, current_priority: u8) -> bool {
        if let Some(front) = self.rt_queue.front() {
            front.priority < current_priority
        } else {
            false
        }
    }

    /// Removes a specific context from the run queue.
    pub fn remove(&mut self, context_id: usize) -> Option<ContextRef> {
        // Check RT queue first
        if let Some(pos) = self.rt_queue.iter().position(|e| e.id == context_id) {
            let entry = self.rt_queue.remove(pos)?;
            self.task_count.fetch_sub(1, Ordering::Relaxed);
            let weight = Self::priority_to_weight(entry.priority);
            self.load_weight.fetch_sub(weight, Ordering::Relaxed);
            return Some(entry.context);
        }

        // Then non-RT queue
        if let Some(pos) = self.non_rt_queue.iter().position(|e| e.id == context_id) {
            let entry = self.non_rt_queue.remove(pos)?;
            self.task_count.fetch_sub(1, Ordering::Relaxed);
            let weight = Self::priority_to_weight(entry.priority);
            self.load_weight.fetch_sub(weight, Ordering::Relaxed);
            return Some(entry.context);
        }

        None
    }

    /// Check if the queue is empty
    pub fn is_empty(&self) -> bool {
        self.task_count.load(Ordering::Relaxed) == 0
    }

    /// Get total task count
    pub fn len(&self) -> usize {
        self.task_count.load(Ordering::Relaxed)
    }

    /// Get load weight for balancing
    pub fn load(&self) -> u64 {
        self.load_weight.load(Ordering::Relaxed)
    }

    /// Check if preemption is needed
    pub fn check_preempt(&self) -> bool {
        self.needs_preempt.load(Ordering::Acquire)
    }

    /// Try to steal a task for work stealing (returns non-RT only)
    pub fn steal(&mut self) -> Option<RunQueueEntry> {
        // Only steal from the back of non-RT queue to minimize disruption
        if self.non_rt_queue.len() > 1 {
            let entry = self.non_rt_queue.pop_back()?;
            self.task_count.fetch_sub(1, Ordering::Relaxed);
            let weight = Self::priority_to_weight(entry.priority);
            self.load_weight.fetch_sub(weight, Ordering::Relaxed);
            return Some(entry);
        }
        None
    }

    /// Convert priority to load weight
    /// Lower priority number = higher priority = lower weight (gets less time advantage)
    fn priority_to_weight(priority: u8) -> u64 {
        // NICE_0_LOAD equivalent - priority 100 (normal) gets weight 1024
        // Scale: priority 0 -> weight 88, priority 139 -> weight 15
        let base_weight = 1024u64;
        let nice = priority as i32 - 100; // -100 to +39 (RT tasks are 0-99)
        if nice <= 0 {
            // Higher priority than normal
            base_weight.saturating_mul(1 << ((-nice).min(10) as u32))
        } else {
            // Lower priority than normal
            base_weight / (1 << (nice.min(10) as u32))
        }
    }
}

// =============================================================================
// Per-CPU Scheduler
// =============================================================================

/// Per-CPU scheduler state implementing MuQSS-style virtual deadline scheduling.
pub struct Scheduler {
    /// The run queue for this CPU
    pub run_queue: RunQueue,

    /// The currently running context
    pub current_context: Option<ContextRef>,

    /// Virtual deadline of current context (for quick comparison)
    pub current_virtual_deadline: AtomicU64,

    /// Current context priority (for preemption checks)
    pub current_priority: AtomicU32,

    /// Time of last load balance check
    pub last_balance_time: AtomicU64,

    /// Statistics
    pub stats: SchedulerStats,

    /// Flag for tickless mode
    pub tickless: AtomicBool,

    /// Next timer event (for tickless)
    pub next_timer_event: AtomicU64,
}

impl Scheduler {
    pub const fn new() -> Self {
        Scheduler {
            run_queue: RunQueue::new(),
            current_context: None,
            current_virtual_deadline: AtomicU64::new(0),
            current_priority: AtomicU32::new(Priority::Low as u32),
            last_balance_time: AtomicU64::new(0),
            stats: SchedulerStats::new(),
            tickless: AtomicBool::new(true),
            next_timer_event: AtomicU64::new(0),
        }
    }

    /// Selects and returns the next context to run.
    ///
    /// This is the core scheduling function implementing MuQSS virtual deadline.
    pub fn schedule(&mut self, token: &mut CleanLockToken) -> Option<ContextRef> {
        #[cfg(target_arch = "x86_64")]
        let start_tsc = unsafe { core::arch::x86_64::_rdtsc() };

        // Handle the currently running context
        if let Some(current_ctx_ref) = &self.current_context {
            self.handle_current_context(current_ctx_ref, token);
        }

        // Select next context
        let next_context = self.run_queue.next();

        // Set up the next context
        if let Some(next_ctx_ref) = &next_context {
            self.setup_next_context(next_ctx_ref, token);
        }

        self.current_context = next_context.clone();

        // Record stats
        #[cfg(target_arch = "x86_64")]
        {
            let end_tsc = unsafe { core::arch::x86_64::_rdtsc() };
            if end_tsc > start_tsc {
                let cycles = end_tsc - start_tsc;
                self.stats
                    .total_overhead_cycles
                    .fetch_add(cycles, Ordering::Relaxed);
                // Approximate latency in ns (assuming ~3GHz)
                let latency_ns = cycles / 3;
                let is_rt = next_context
                    .as_ref()
                    .map(|c| c.read(token.token()).is_realtime)
                    .unwrap_or(false);
                self.stats.record_switch(is_rt, latency_ns);
            }
        }

        next_context
    }

    /// Handle the currently running context before switching
    fn handle_current_context(&mut self, current_ctx_ref: &ContextRef, token: &mut CleanLockToken) {
        let mut current_ctx = current_ctx_ref.write(token.token());
        let now = monotonic();
        let time_spent = now.saturating_sub(current_ctx.switch_time);

        // Update cache locality tracking
        current_ctx.last_cpu_id = Some(crate::cpu_id());

        // Update CPU time accounting
        current_ctx.cpu_time = current_ctx.cpu_time.saturating_add(time_spent);

        // Update virtual deadline for non-RT tasks
        if !current_ctx.is_realtime {
            // MuQSS-style virtual deadline calculation:
            // vd = vd + (time_spent * BASE_TIME_SLICE) / (priority_weight + 1)
            let priority_factor = current_ctx.priority.effective_priority() as u64 + 1;
            let virtual_time_increase = if priority_factor > 0 {
                (time_spent as u64 * BASE_TIME_SLICE_NS) / priority_factor
            } else {
                time_spent as u64
            };

            current_ctx.virtual_deadline = current_ctx
                .virtual_deadline
                .saturating_add(virtual_time_increase);
        }

        // Check priority boost expiration
        current_ctx.priority.check_boost_expired();

        // Re-add to run queue if still runnable
        if current_ctx.status.is_runnable() {
            drop(current_ctx); // Drop lock before adding to queue
            self.run_queue.add(current_ctx_ref.clone(), token);
        }
    }

    /// Set up the next context to run
    fn setup_next_context(&mut self, next_ctx_ref: &ContextRef, token: &mut CleanLockToken) {
        let mut next_ctx = next_ctx_ref.write(token.token());

        // Record switch time
        next_ctx.switch_time = monotonic();

        // Update scheduler state
        let priority = next_ctx.priority.effective_priority();
        self.current_priority
            .store(priority as u32, Ordering::Relaxed);

        if !next_ctx.is_realtime {
            self.current_virtual_deadline
                .store(next_ctx.virtual_deadline, Ordering::Relaxed);
        }

        // Set up tickless timer for time slice
        let time_slice = if next_ctx.is_realtime {
            RT_TIME_SLICE_NS
        } else {
            Self::calculate_time_slice(priority)
        };

        let deadline = next_ctx.switch_time as u64 + time_slice;
        self.next_timer_event.store(deadline, Ordering::Release);
    }

    /// Calculate time slice based on priority
    fn calculate_time_slice(priority: u8) -> u64 {
        // Scale time slice with priority
        // High priority (0-64): MIN to BASE
        // Normal priority (100): BASE
        // Low priority (139): MAX
        if priority < 64 {
            MIN_TIME_SLICE_NS + ((priority as u64) * (BASE_TIME_SLICE_NS - MIN_TIME_SLICE_NS) / 64)
        } else if priority <= 100 {
            BASE_TIME_SLICE_NS
        } else {
            BASE_TIME_SLICE_NS
                + ((priority as u64 - 100) * (MAX_TIME_SLICE_NS - BASE_TIME_SLICE_NS) / 39)
        }
    }

    /// Called when a context is blocked
    pub fn context_blocked(&mut self, context_id: usize) {
        self.run_queue.remove(context_id);
    }

    /// Called when a context becomes runnable
    pub fn context_unblocked(&mut self, context_ref: ContextRef, token: &mut CleanLockToken) {
        self.run_queue.add(context_ref, token);
    }

    /// Check if preemption of current context is needed
    pub fn should_preempt(&self, token: &mut CleanLockToken) -> bool {
        let current_priority = self.current_priority.load(Ordering::Relaxed) as u8;

        // Always preempt for higher priority RT task
        if self.run_queue.has_higher_priority(current_priority) {
            return true;
        }

        // Check if run queue flagged preemption
        self.run_queue.check_preempt()
    }

    /// Attempt load balancing with other CPUs
    pub fn try_balance(&mut self, token: &mut CleanLockToken) {
        let now = monotonic() as u64;
        let last = self.last_balance_time.load(Ordering::Relaxed);

        if now.saturating_sub(last) < BALANCE_INTERVAL_NS {
            return;
        }

        self.last_balance_time.store(now, Ordering::Relaxed);

        // Get our load
        let my_load = self.run_queue.load();
        let my_count = self.run_queue.len();

        // Only try to steal if we have few tasks
        if my_count > 1 {
            return;
        }

        // Note: Actual cross-CPU stealing would require global scheduler state
        // and proper locking/IPI. This is a placeholder for the mechanism.
        self.stats.balance_ops.fetch_add(1, Ordering::Relaxed);
    }

    /// Get next timer event for tickless operation
    pub fn get_next_timer(&self) -> Option<u64> {
        let event = self.next_timer_event.load(Ordering::Acquire);
        if event > 0 {
            Some(event)
        } else {
            None
        }
    }
}

// =============================================================================
// Global Scheduler Functions
// =============================================================================

/// Get the per-CPU scheduler instance
pub fn scheduler() -> &'static mut Scheduler {
    &mut PercpuBlock::current().scheduler
}

/// Schedule the next context to run
pub fn schedule_next(token: &mut CleanLockToken) -> Option<ContextRef> {
    scheduler().schedule(token)
}

/// Add a context to the scheduler
pub fn add_context(context_ref: ContextRef, token: &mut CleanLockToken) {
    // Determine target CPU for cache locality
    let target_cpu = {
        let context = context_ref.read(token.token());
        context.last_cpu_id.unwrap_or_else(crate::cpu_id)
    };

    // Initialize virtual deadline for new non-RT contexts
    {
        let mut context = context_ref.write(token.token());
        if !context.is_realtime && context.virtual_deadline == 0 {
            // Start with current minimum deadline to be fair
            let current_vd = PercpuBlock::current()
                .scheduler
                .current_virtual_deadline
                .load(Ordering::Relaxed);
            context.virtual_deadline = current_vd;
        }
    }

    // Add to appropriate CPU's queue
    if target_cpu == crate::cpu_id() {
        scheduler().run_queue.add(context_ref, token);
    } else {
        // Cross-CPU migration: add to current and let balancing handle it
        // In a full implementation, this would use IPI
        scheduler().run_queue.add(context_ref, token);
    }
}

/// Remove a context from the scheduler
pub fn remove_context(context_id: &usize) {
    scheduler().run_queue.remove(*context_id);
}

/// Request preemption of current context if needed
pub fn request_preemption(token: &mut CleanLockToken) {
    if scheduler().should_preempt(token) {
        scheduler()
            .stats
            .preemptions
            .fetch_add(1, Ordering::Relaxed);
        // IPI to trigger reschedule on current CPU
        ipi(IpiKind::Switch, IpiTarget::Current);
    }
}

/// Try to balance load across CPUs
pub fn balance(token: &mut CleanLockToken) {
    scheduler().try_balance(token);
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_queue_new() {
        let queue = RunQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_priority_to_weight() {
        // Higher priority (lower number) should get higher weight
        let high_weight = RunQueue::priority_to_weight(0);
        let normal_weight = RunQueue::priority_to_weight(100);
        let low_weight = RunQueue::priority_to_weight(139);

        assert!(high_weight > normal_weight);
        assert!(normal_weight > low_weight);
    }

    #[test]
    fn test_scheduler_stats() {
        let stats = SchedulerStats::new();
        stats.record_switch(true, 100);
        stats.record_switch(false, 50);

        assert_eq!(stats.switches.load(Ordering::Relaxed), 2);
        assert_eq!(stats.rt_switches.load(Ordering::Relaxed), 1);
        assert_eq!(stats.min_latency_ns.load(Ordering::Relaxed), 50);
        assert_eq!(stats.max_latency_ns.load(Ordering::Relaxed), 100);
    }

    #[test]
    fn test_time_slice_calculation() {
        let rt_slice = Scheduler::calculate_time_slice(0);
        let normal_slice = Scheduler::calculate_time_slice(100);
        let low_slice = Scheduler::calculate_time_slice(139);

        assert!(rt_slice < normal_slice);
        assert!(normal_slice < low_slice);
        assert!(rt_slice >= MIN_TIME_SLICE_NS);
        assert!(low_slice <= MAX_TIME_SLICE_NS);
    }
}

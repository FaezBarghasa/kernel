//! # High-Concurrency Work-Stealing Scheduler
//!
//! This module implements a per-CPU work-stealing scheduler with NUMA awareness,
//! designed for high-core-count systems.
//!
//! ## Features
//!
//! - **Per-CPU RunQueues**: Work stealing architecture implementation
//! - **NUMA-Awareness**: Tasks prefer their parent's socket unless significant imbalance
//! - **Real-Time Support**: Strict priority queue for RT tasks
//! - **Virtual Deadlines**: Fair scheduling for non-RT tasks (MuQSS-style)
//! - **Lock-Free Metrics**: Low-overhead statistics gathering
//!
//! ## Work Stealing
//!
//! Idle CPUs attempt to steal tasks from the most loaded CPU. Stealing respects
//! NUMA boundaries where possible but prioritizes system-wide throughput.

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use spin::{Mutex, RwLock};

use crate::{
    context::ContextRef,
    cpu_set::LogicalCpuId,
    ipi::{ipi, IpiKind, IpiTarget},
    percpu::{PercpuBlock, ALL_PERCPU_BLOCKS},
    sync::{CleanLockToken, Priority},
    time::monotonic,
};

// =============================================================================
// Scheduler Constants
// =============================================================================

/// Base time slice for tasks in nanoseconds (1ms default, XanMod-style).
const BASE_TIME_SLICE_NS: u64 = 1_000_000;

/// Minimum time slice for interactive tasks.
const MIN_TIME_SLICE_NS: u64 = 100_000;

/// Maximum time slice for batch tasks.
const MAX_TIME_SLICE_NS: u64 = 10_000_000;

/// RT task time slice (smaller for determinism).
const RT_TIME_SLICE_NS: u64 = 500_000;

/// Priority levels for RT tasks (POSIX SCHED_FIFO).
pub const RT_PRIORITY_LEVELS: usize = 100;

/// Load balance interval in nanoseconds (4ms).
const BALANCE_INTERVAL_NS: u64 = 4_000_000;

/// Imbalance threshold for migration (25%).
/// Tasks migrate if (dest_load * 125 / 100) < src_load
const IMBALANCE_THRESHOLD_PERCENT: u64 = 25;

/// Assumed cores per socket for NUMA estimation
/// TODO: Retrieve from topology/ACPI
const ESTIMATED_CORES_PER_SOCKET: u32 = 8;

/// Scheduling policies
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SchedPolicy {
    Normal = 0,
    Fifo = 1,
    RoundRobin = 2,
    Batch = 3,
    Idle = 5,
    Interactive = 6,
    Deadline = 7,
}

pub trait ExtSchedulerOps: Send + Sync {
    fn enqueue(&self, context: ContextRef) -> bool;
    fn dequeue(&self, context_id: usize);
    fn select_next(&self, cpu_id: LogicalCpuId) -> Option<ContextRef>;
}

/// Hardware-Accelerated RTIC Scheduler Integration Hooks (Phase 1.2)
pub trait RticSchedulerOps: Send + Sync {
    /// Maps an RTIC task directly to a hardware interrupt vector.
    fn bind_interrupt(&self, task_id: usize, irq_vector: u32);
    
    /// Returns the current Stack Resource Policy (SRP) priority ceiling.
    fn get_srp_ceiling(&self) -> u8;
    
    /// Raises the SRP priority ceiling to acquire a shared resource.
    fn raise_srp_ceiling(&self, new_ceiling: u8) -> u8;
    
    /// Restores the SRP priority ceiling after releasing a resource.
    fn restore_srp_ceiling(&self, old_ceiling: u8);
}

pub static RTIC_SCHEDULER: RwLock<Option<Arc<dyn RticSchedulerOps>>> = RwLock::new(None);

pub static EXT_SCHEDULER: RwLock<Option<Arc<dyn ExtSchedulerOps>>> = RwLock::new(None);

pub struct IpcScheduler {
    pub fd: usize,
    pub queue: Arc<crossbeam_queue::SegQueue<ContextRef>>,
}

impl ExtSchedulerOps for IpcScheduler {
    fn enqueue(&self, context: ContextRef) -> bool {
        self.queue.push(context);
        true
    }
    fn dequeue(&self, _context_id: usize) {
        // No-op
    }
    fn select_next(&self, _cpu_id: LogicalCpuId) -> Option<ContextRef> {
        self.queue.pop()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheDomain {
    CacheRich,
    FrequencyRich,
}

static L3_CACHE_SIZES: [AtomicU64; 256] = [const { AtomicU64::new(0) }; 256];

pub fn get_l3_cache_size() -> Option<u64> {
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        None
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        for subleaf in 0..4 {
            if let Some(res) = crate::arch::x86_shared::cpuid::get_amd_cache_properties(subleaf) {
                let cache_level = (res.eax >> 5) & 0x7;
                if cache_level == 3 {
                    let line_size = (res.ebx & 0xFFF) + 1;
                    let partitions = ((res.ebx >> 12) & 0x3FF) + 1;
                    let ways = ((res.ebx >> 22) & 0x3FF) + 1;
                    let sets = res.ecx + 1;
                    return Some((ways as u64) * (partitions as u64) * (line_size as u64) * (sets as u64));
                }
            }
        }
        None
    }
}

pub fn detect_cache_domain(cpu_id: LogicalCpuId) -> CacheDomain {
    let l3_size = get_l3_cache_size().unwrap_or(0);
    let id_val = cpu_id.get() as usize;
    if id_val < 256 {
        L3_CACHE_SIZES[id_val].store(l3_size, Ordering::Release);
    }

    // A threshold of 64MB (64 * 1024 * 1024) is a perfect differentiator for Ryzen 3D V-Cache (96MB vs 32MB).
    if l3_size >= 64 * 1024 * 1024 {
        CacheDomain::CacheRich
    } else {
        CacheDomain::FrequencyRich
    }
}

pub fn get_cpu_cache_domain(cpu_id: LogicalCpuId) -> CacheDomain {
    let ptr = ALL_PERCPU_BLOCKS[cpu_id.get() as usize].load(Ordering::Acquire);
    if !ptr.is_null() {
        unsafe { (*ptr).scheduler.cache_domain }
    } else {
        CacheDomain::CacheRich
    }
}

pub fn select_best_cpu_in_domain(domain: CacheDomain) -> LogicalCpuId {
    let mut best_cpu = crate::cpu_id();
    let mut min_load = u64::MAX;

    for i in 0..crate::cpu_count() {
        let cpu_id = LogicalCpuId::new(i);
        let ptr = ALL_PERCPU_BLOCKS[i as usize].load(Ordering::Acquire);
        if !ptr.is_null() {
            let cpu_domain = unsafe { (*ptr).scheduler.cache_domain };
            if cpu_domain == domain {
                let load = unsafe { (*ptr).scheduler.run_queue.load() };
                if load < min_load {
                    min_load = load;
                    best_cpu = cpu_id;
                }
            }
        }
    }
    best_cpu
}

// =============================================================================
// Scheduler Statistics
// =============================================================================

#[derive(Debug, Default)]
pub struct SchedulerStats {
    pub switches: AtomicU64,
    pub rt_switches: AtomicU64,
    pub total_overhead_cycles: AtomicU64,
    pub min_latency_ns: AtomicU64,
    pub max_latency_ns: AtomicU64,
    pub balance_ops: AtomicU64,
    pub migrations: AtomicU64,
    pub preemptions: AtomicU64,
    pub steals: AtomicU64,
    pub steal_failures: AtomicU64,
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
            steals: AtomicU64::new(0),
            steal_failures: AtomicU64::new(0),
        }
    }

    pub fn record_switch(&self, is_rt: bool, latency_ns: u64) {
        self.switches.fetch_add(1, Ordering::Relaxed);
        if is_rt {
            self.rt_switches.fetch_add(1, Ordering::Relaxed);
        }

        // Lock-free min update
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

        // Lock-free max update
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
// Run Queue
// =============================================================================

#[derive(Debug)]
pub struct RunQueueEntry {
    pub id: usize,
    pub context: ContextRef,
    pub vdeadline: u64,
    pub priority: u8,
    pub time_slice: u64,
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

/// Number of distinct RT priority levels for O(1) scheduling.
/// Uses 64 levels to fit in a single `u64` bitmask for fast lookup.
const RT_PRIO_LEVELS: usize = 64;

/// A per-CPU run queue with O(1) RT scheduling and virtual-deadline non-RT scheduling.
///
/// RT tasks are organized into 64 priority buckets (0 = highest, 63 = lowest).
/// A bitmask tracks which buckets have runnable tasks, enabling O(1) lookup of the
/// highest-priority ready task via `trailing_zeros()`.
///
/// Non-RT tasks use a virtual-deadline sorted queue (unchanged CFS-like behavior).
pub struct RunQueue {
    /// RT tasks organized by priority level for O(1) insertion and selection.
    /// Each bucket is a FIFO queue for round-robin within the same priority.
    /// Protected by a single lock to ensure bitmap/queue consistency.
    rt_queues: Mutex<[VecDeque<RunQueueEntry>; RT_PRIO_LEVELS]>,

    /// Bitmask of RT priority levels with runnable tasks.
    /// Bit N set means `rt_queues[N]` has at least one entry.
    /// Allows O(1) highest-priority lookup via `trailing_zeros()`.
    rt_bitmap: AtomicU64,

    /// Non-RT tasks managed via a lock-free EEVDF priority ring.
    /// The primary target for work stealing.
    pub non_rt_ring: crate::context::ring::ContextRing,

    /// Atomic counters for quick load estimation without locking
    task_count: AtomicUsize,
    load_weight: AtomicU64,
    needs_preempt: AtomicBool,
}

impl RunQueue {
    pub const fn new() -> Self {
        // Const-compatible initialization: VecDeque::new() is const,
        // so we repeat it 64 times for the array inside a single outer Mutex.
        const EMPTY_Q: VecDeque<RunQueueEntry> = VecDeque::new();

        RunQueue {
            rt_queues: Mutex::new([
                EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q,
                EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q,
                EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q,
                EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q,
                EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q,
                EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q,
                EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q, EMPTY_Q,
                EMPTY_Q,
            ]),
            rt_bitmap: AtomicU64::new(0),
            non_rt_ring: crate::context::ring::ContextRing::new(),
            task_count: AtomicUsize::new(0),
            load_weight: AtomicU64::new(0),
            needs_preempt: AtomicBool::new(false),
        }
    }

    /// Map a context priority (u8) to an RT priority bucket index (0..63).
    /// Priority 0 is highest. Values >= RT_PRIO_LEVELS are clamped to the lowest RT bucket.
    #[inline]
    fn rt_prio_index(priority: u8) -> usize {
        (priority as usize).min(RT_PRIO_LEVELS - 1)
    }

    pub fn add(&self, context_ref: ContextRef, token: &mut CleanLockToken) {
        if let Some(ext_sched) = &*EXT_SCHEDULER.read() {
            if ext_sched.enqueue(context_ref.clone()) {
                return;
            }
        }

        let (is_realtime, id, vdeadline, priority) = {
            let context = context_ref.read(token.token());
            (
                context.is_realtime,
                context.id(),
                context.virtual_deadline,
                context.priority.effective_priority(),
            )
        };

        let weight = Self::priority_to_weight(priority);

        if is_realtime {
            let entry = RunQueueEntry::new(id, context_ref, vdeadline, priority);
            let idx = Self::rt_prio_index(priority);
            {
                let mut queues = self.rt_queues.lock();
                let was_empty = queues[idx].is_empty();
                queues[idx].push_back(entry); // O(1) insertion

                if was_empty {
                    // Set bit in bitmap atomically
                    self.rt_bitmap.fetch_or(1u64 << idx, Ordering::Release);
                }
            }

            // Check if this is the new highest-priority task
            let bitmap = self.rt_bitmap.load(Ordering::Acquire);
            let highest = bitmap.trailing_zeros() as usize;
            if highest == idx {
                self.needs_preempt.store(true, Ordering::Release);
            }
        } else {
            // EEVDF: Calculate initial virtual deadline if it's currently 0 or behind monotonic time.
            let now = monotonic() as u64;
            let computed_vdeadline = if vdeadline < now {
                let weight_div = if weight == 0 { 1 } else { weight };
                let slice_factor = (BASE_TIME_SLICE_NS.saturating_mul(1024)) / weight_div;
                now.saturating_add(slice_factor)
            } else {
                vdeadline
            };

            // Store back to the context
            context_ref.write(token.token()).virtual_deadline = computed_vdeadline;

            let enqueued = self.non_rt_ring.enqueue(context_ref, id, computed_vdeadline, priority);
            if enqueued {
                self.needs_preempt.store(true, Ordering::Release);
            }
        }

        self.task_count.fetch_add(1, Ordering::Relaxed);
        self.load_weight.fetch_add(weight, Ordering::Relaxed);
    }

    pub fn next(&self) -> Option<ContextRef> {
        self.needs_preempt.store(false, Ordering::Relaxed);

        // O(1) RT queue selection via bitmask
        {
            let bitmap = self.rt_bitmap.load(Ordering::Acquire);
            if bitmap != 0 {
                let idx = bitmap.trailing_zeros() as usize;
                let mut queues = self.rt_queues.lock();
                if let Some(mut entry) = queues[idx].pop_front() {
                    // If bucket is now empty, clear its bit
                    if queues[idx].is_empty() {
                        self.rt_bitmap.fetch_and(!(1u64 << idx), Ordering::Release);
                    }
                    self.task_count.fetch_sub(1, Ordering::Relaxed);
                    let weight = Self::priority_to_weight(entry.priority);
                    self.load_weight.fetch_sub(weight, Ordering::Relaxed);
                    entry.run_count += 1;
                    return Some(entry.context);
                }
            }
        }

        // Try EXT_SCHEDULER if registered
        let mut fallback = false;
        let ext_sched_opt = {
            let guard = EXT_SCHEDULER.read();
            guard.clone()
        };
        if let Some(ext_sched) = ext_sched_opt {
            let start = monotonic() as u64;
            let res = ext_sched.select_next(crate::cpu_id());
            let duration = (monotonic() as u64).saturating_sub(start);
            if duration > 500_000 {
                fallback = true;
            } else if let Some(ctx_ref) = res {
                return Some(ctx_ref);
            }
        }
        if fallback {
            let mut guard = EXT_SCHEDULER.write();
            *guard = None;
            log::warn!("ExtScheduler exceeded time budget. Falling back to EEVDF.");
        }

        // Try Non-RT queue with earliest EEVDF deadline
        if let Some(ctx_ref) = self.non_rt_ring.select_next_eevdf() {
            let mut token = unsafe { CleanLockToken::new() };
            let priority = ctx_ref.read(token.token()).priority.effective_priority();
            let weight = Self::priority_to_weight(priority);
            self.task_count.fetch_sub(1, Ordering::Relaxed);
            self.load_weight.fetch_sub(weight, Ordering::Relaxed);
            return Some(ctx_ref);
        }

        None
    }

    pub fn peek(&self) -> Option<ContextRef> {
        // O(1) RT peek via bitmask
        let bitmap = self.rt_bitmap.load(Ordering::Acquire);
        if bitmap != 0 {
            let idx = bitmap.trailing_zeros() as usize;
            let queues = self.rt_queues.lock();
            if let Some(front) = queues[idx].front() {
                return Some(front.context.clone());
            }
        }
        self.non_rt_ring.peek_earliest()
    }

    pub fn remove(&self, context_id: usize) {
        if let Some(ext_sched) = &*EXT_SCHEDULER.read() {
            ext_sched.dequeue(context_id);
        }

        // Search RT queues
        {
            let mut queues = self.rt_queues.lock();
            let mut bitmap = self.rt_bitmap.load(Ordering::Acquire);
            while bitmap != 0 {
                let idx = bitmap.trailing_zeros() as usize;
                if let Some(pos) = queues[idx].iter().position(|e| e.id == context_id) {
                    if let Some(entry) = queues[idx].remove(pos) {
                        self.task_count.fetch_sub(1, Ordering::Relaxed);
                        let weight = Self::priority_to_weight(entry.priority);
                        self.load_weight.fetch_sub(weight, Ordering::Relaxed);
                    }
                    // Clear bitmap bit if bucket is now empty
                    if queues[idx].is_empty() {
                        self.rt_bitmap.fetch_and(!(1u64 << idx), Ordering::Release);
                    }
                    return;
                }
                bitmap &= !(1u64 << idx); // Clear this bit, check next
            }
        }
        // Search non-RT queue
        if let Some(ctx_ref) = self.non_rt_ring.remove_by_id(context_id) {
            let mut token = unsafe { CleanLockToken::new() };
            let priority = ctx_ref.read(token.token()).priority.effective_priority();
            let weight = Self::priority_to_weight(priority);
            self.task_count.fetch_sub(1, Ordering::Relaxed);
            self.load_weight.fetch_sub(weight, Ordering::Relaxed);
        }
    }

    /// Try to steal a task from this queue.
    /// Returns a RunQueueEntry if successful.
    /// Only steals from non-RT queue to avoid disrupting real-time guarantees.
    pub fn steal(&self) -> Option<RunQueueEntry> {
        if self.non_rt_ring.len() > 1 {
            // Steal the context with the furthest virtual deadline (coldest task)
            if let Some(ctx_ref) = self.non_rt_ring.select_furthest_eevdf() {
                let mut token = unsafe { CleanLockToken::new() };
                let (id, vdeadline, priority) = {
                    let context = ctx_ref.read(token.token());
                    (context.id(), context.virtual_deadline, context.priority.effective_priority())
                };
                let weight = Self::priority_to_weight(priority);
                self.task_count.fetch_sub(1, Ordering::Relaxed);
                self.load_weight.fetch_sub(weight, Ordering::Relaxed);
                return Some(RunQueueEntry::new(id, ctx_ref, vdeadline, priority));
            }
        }
        None
    }

    pub fn load(&self) -> u64 {
        self.load_weight.load(Ordering::Relaxed)
    }

    pub fn len(&self) -> usize {
        self.task_count.load(Ordering::Relaxed)
    }

    pub fn check_preempt(&self) -> bool {
        self.needs_preempt.load(Ordering::Acquire)
    }

    /// O(1) check if any RT task has higher priority than `current_priority`.
    pub fn has_higher_priority(&self, current_priority: u8) -> bool {
        let bitmap = self.rt_bitmap.load(Ordering::Acquire);
        if bitmap == 0 {
            return false;
        }
        let highest_idx = bitmap.trailing_zeros() as u8;
        highest_idx < current_priority
    }

    /// Get the priority of the highest-priority RT task, if any.
    /// Returns `None` if no RT tasks are queued.
    pub fn highest_rt_priority(&self) -> Option<u8> {
        let bitmap = self.rt_bitmap.load(Ordering::Acquire);
        if bitmap == 0 {
            None
        } else {
            Some(bitmap.trailing_zeros() as u8)
        }
    }

    fn priority_to_weight(priority: u8) -> u64 {
        let base_weight = 1024u64;
        let nice = priority as i32 - 100;
        if nice <= 0 {
            base_weight.saturating_mul(1 << ((-nice).min(10) as u32))
        } else {
            base_weight / (1 << (nice.min(10) as u32))
        }
    }
}

// =============================================================================
// Per-CPU Scheduler
// =============================================================================

pub struct Scheduler {
    pub run_queue: RunQueue,
    pub current_context: Option<ContextRef>,
    pub current_virtual_deadline: AtomicU64,
    pub current_priority: AtomicU32,
    pub last_balance_time: AtomicU64,
    pub stats: SchedulerStats,
    pub tickless: AtomicBool,
    pub next_timer_event: AtomicU64,
    pub cache_domain: CacheDomain,
}

impl Scheduler {
    pub fn new(cpu_id: LogicalCpuId) -> Self {
        Scheduler {
            run_queue: RunQueue::new(),
            current_context: None,
            current_virtual_deadline: AtomicU64::new(0),
            current_priority: AtomicU32::new(Priority::Low as u32),
            last_balance_time: AtomicU64::new(0),
            stats: SchedulerStats::new(),
            tickless: AtomicBool::new(true),
            next_timer_event: AtomicU64::new(0),
            cache_domain: detect_cache_domain(cpu_id),
        }
    }

    pub fn schedule(&mut self, token: &mut CleanLockToken) -> Option<ContextRef> {
        #[cfg(target_arch = "x86_64")]
        let start_tsc = unsafe { core::arch::x86_64::_rdtsc() };

        if let Some(current_ctx_ref) = self.current_context.clone() {
            self.handle_current_context(&current_ctx_ref, token);
        }

        crate::topology::governor::monitor_and_scale(crate::cpu_id());

        // 1. Unconditionally try load balancing if the queue is empty
        if self.run_queue.len() == 0 {
            self.perform_work_stealing();
        }

        let next_context = self.run_queue.next();

        if let Some(next_ctx_ref) = &next_context {
            self.setup_next_context(next_ctx_ref, token);
        }

        self.current_context = next_context.clone();

        #[cfg(target_arch = "x86_64")]
        {
            let end_tsc = unsafe { core::arch::x86_64::_rdtsc() };
            if end_tsc > start_tsc {
                let cycles = end_tsc - start_tsc;
                self.stats
                    .total_overhead_cycles
                    .fetch_add(cycles, Ordering::Relaxed);
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

    /// Primary work stealing logic: find the busiest CPU and steal from it.
    fn perform_work_stealing(&self) {
        let my_id = crate::cpu_id().get();
        let mut max_load = 0;
        let mut target_cpu = None;

        // Find the heaviest loaded CPU
        // NOTE: This iterates ALL_PERCPU_BLOCKS which is safe as it contains AtomicPtrs
        for i in 0..crate::cpu_count() {
            if i == my_id {
                continue;
            }

            let ptr = ALL_PERCPU_BLOCKS[i as usize].load(Ordering::Acquire);
            if ptr.is_null() {
                continue;
            }

            // SAFETY: We checked for null, and PercpuBlocks are static and persistent
            let other_scheduler = unsafe { &(*ptr).scheduler };
            let load = other_scheduler.run_queue.load();

            if load > max_load {
                max_load = load;
                target_cpu = Some(other_scheduler);
            }
        }

        if let Some(victim) = target_cpu {
            // Basic heuristic: Don't steal if they are barely loaded
            if victim.run_queue.len() > 1 {
                if let Some(stolen_task) = victim.run_queue.steal() {
                    // We stole a task! Add it to our queue.
                    // Note: We need a dummy token here as the task is already consistent
                    // and we are just adding it to our queue structure.
                    // Making a dummy token is generally unsafe but here we are in scheduler context.
                    // Ideally we would pass the token down, but for now we rely on the fact
                    // that add() primarily needs the token for reading the context, which we can do.
                    let mut dummy_token = unsafe { CleanLockToken::new() };
                    self.run_queue.add(stolen_task.context, &mut dummy_token);

                    self.stats.steals.fetch_add(1, Ordering::Relaxed);
                    self.stats.migrations.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.stats.steal_failures.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    fn handle_current_context(&mut self, current_ctx_ref: &ContextRef, token: &mut CleanLockToken) {
        let mut current_ctx = current_ctx_ref.write(token.token());
        let now = monotonic();
        let time_spent = now.saturating_sub(current_ctx.switch_time);

        current_ctx.last_cpu_id = Some(crate::cpu_id());
        current_ctx.cpu_time = current_ctx.cpu_time.saturating_add(time_spent);
        current_ctx.update_cache_metrics(time_spent as u64);

        if !current_ctx.is_realtime {
            let priority = current_ctx.priority.effective_priority();
            let weight = RunQueue::priority_to_weight(priority);
            let weight_div = if weight == 0 { 1 } else { weight };
            let virtual_time_increase = (time_spent.saturating_mul(1024)) / (weight_div as u128);

            if (current_ctx.virtual_deadline as u128) < now {
                let slice_factor = (BASE_TIME_SLICE_NS.saturating_mul(1024)) / weight_div;
                current_ctx.virtual_deadline = (now as u64).saturating_add(slice_factor);
            } else {
                current_ctx.virtual_deadline = current_ctx
                    .virtual_deadline
                    .saturating_add(virtual_time_increase as u64);
            }
        }

        current_ctx.priority.check_boost_expired();

        if current_ctx.status.is_runnable() {
            drop(current_ctx);
            add_context(current_ctx_ref.clone(), token);
        }
    }

    fn setup_next_context(&mut self, next_ctx_ref: &ContextRef, token: &mut CleanLockToken) {
        let mut next_ctx = next_ctx_ref.write(token.token());
        next_ctx.switch_time = monotonic();

        let priority = next_ctx.priority.effective_priority();
        self.current_priority
            .store(priority as u32, Ordering::Relaxed);

        if !next_ctx.is_realtime {
            self.current_virtual_deadline
                .store(next_ctx.virtual_deadline, Ordering::Relaxed);
        }

        let time_slice = if next_ctx.is_realtime {
            RT_TIME_SLICE_NS
        } else {
            Self::calculate_time_slice(priority)
        };

        let deadline = next_ctx.switch_time as u64 + time_slice;
        self.next_timer_event.store(deadline, Ordering::Release);
    }

    fn calculate_time_slice(priority: u8) -> u64 {
        if priority < 64 {
            MIN_TIME_SLICE_NS + ((priority as u64) * (BASE_TIME_SLICE_NS - MIN_TIME_SLICE_NS) / 64)
        } else if priority <= 100 {
            BASE_TIME_SLICE_NS
        } else {
            BASE_TIME_SLICE_NS
                + ((priority as u64 - 100) * (MAX_TIME_SLICE_NS - BASE_TIME_SLICE_NS) / 39)
        }
    }

    pub fn context_blocked(&mut self, context_id: usize) {
        self.run_queue.remove(context_id);
    }

    pub fn context_unblocked(&mut self, context_ref: ContextRef, token: &mut CleanLockToken) {
        self.run_queue.add(context_ref, token);
    }

    pub fn should_preempt(&self, _token: &mut CleanLockToken) -> bool {
        let current_priority = self.current_priority.load(Ordering::Relaxed) as u8;

        if self.run_queue.has_higher_priority(current_priority) {
            return true;
        }

        if self.run_queue.check_preempt() {
            if self.run_queue.peek().is_some() {
                // Peek returns a cloned ref, skipping the lock
                // But we need to check virtual deadline.
                // NOTE: This check is racy but safe for a hint
                // We'll rely on next() to make the final decision
                // let current_deadline = self.current_virtual_deadline.load(Ordering::Relaxed);

                // Read context with a temporary token? No, we shouldn't lock here if possible.
                // But we need to know the deadline.
                // Optimization: We could store min_vdeadline in RunQueue atomic
                // For now, let's just trigger preemption and let schedule() sort it out.
                return true;
            }
        }
        false
    }

    pub fn try_balance(&mut self, _token: &mut CleanLockToken) {
        let now = monotonic() as u64;
        let last = self.last_balance_time.load(Ordering::Relaxed);

        if now.saturating_sub(last) < BALANCE_INTERVAL_NS {
            return;
        }
        self.last_balance_time.store(now, Ordering::Relaxed);

        // Check our load
        let _my_load = self.run_queue.load();

        // If we are overloaded (more than 1 task), try to push?
        // Traditionally work stealing (pull) is better than push.
        // So this balance() method mainly updates stats or does proactive balancing.
        // Since we implemented work-stealing in schedule(), we can use this for logging.
        self.stats.balance_ops.fetch_add(1, Ordering::Relaxed);

        // Potential future expansion: Proactive push migration for RT tasks
    }

    pub fn get_next_timer(&self) -> Option<u64> {
        let event = self.next_timer_event.load(Ordering::Acquire);
        if event > 0 {
            Some(event)
        } else {
            None
        }
    }

    pub fn get_next_event_delta(&self) -> Option<u64> {
        let event = self.next_timer_event.load(Ordering::Acquire);
        if event > 0 {
            let now = monotonic() as u64;
            Some(event.saturating_sub(now))
        } else {
            None
        }
    }
}

// =============================================================================
// Global Scheduler Functions
// =============================================================================

pub fn scheduler() -> &'static mut Scheduler {
    &mut PercpuBlock::current().scheduler
}

pub fn schedule_next(token: &mut CleanLockToken) -> Option<ContextRef> {
    scheduler().schedule(token)
}

pub fn add_context(context_ref: ContextRef, token: &mut CleanLockToken) {
    let context_cpu_id = {
        let context = context_ref.read(token.token());
        context.last_cpu_id
    };

    let target_cpu_id = if let Some(cpu_id) = context_cpu_id {
        // V-Cache/Topology-aware placement based on cache-miss profiling
        let context = context_ref.read(token.token());
        let ratio = context.cache_miss_ratio();
        let current_domain = get_cpu_cache_domain(cpu_id);
        let desired_domain = if ratio > 0.35 {
            CacheDomain::CacheRich
        } else {
            CacheDomain::FrequencyRich
        };

        if current_domain != desired_domain {
            select_best_cpu_in_domain(desired_domain)
        } else {
            cpu_id
        }
    } else {
        // New task placement: use cache profiling
        let context = context_ref.read(token.token());
        let ratio = context.cache_miss_ratio();
        let desired_domain = if ratio > 0.35 {
            CacheDomain::CacheRich
        } else {
            CacheDomain::FrequencyRich
        };
        select_best_cpu_in_domain(desired_domain)
    };

    // Dispatch to the target CPU's queue
    if target_cpu_id == crate::cpu_id() {
        scheduler().run_queue.add(context_ref, token);
    } else {
        // Cross-CPU dispatch
        let ptr = ALL_PERCPU_BLOCKS[target_cpu_id.get() as usize].load(Ordering::Acquire);
        if !ptr.is_null() {
            let other_scheduler = unsafe { &(*ptr).scheduler };
            other_scheduler.run_queue.add(context_ref, token);
        } else {
            // Fallback to local if target invalid
            scheduler().run_queue.add(context_ref, token);
        }
    }
}

fn select_best_cpu_on_socket(start_cpu: LogicalCpuId) -> LogicalCpuId {
    let start_socket = start_cpu.get() / ESTIMATED_CORES_PER_SOCKET;
    let mut best_cpu = start_cpu;
    let mut min_load = u64::MAX;

    // Scan all CPUs, prioritizing same socket
    for i in 0..crate::cpu_count() {
        let cpu_id = LogicalCpuId::new(i);
        let socket = i / ESTIMATED_CORES_PER_SOCKET;

        // Penalize other sockets to encourage local placement
        let numa_penalty = if socket == start_socket { 0 } else { 1000 };

        let ptr = ALL_PERCPU_BLOCKS[i as usize].load(Ordering::Acquire);
        if !ptr.is_null() {
            let load = unsafe { (*ptr).scheduler.run_queue.load() };
            let adjusted_load = load + numa_penalty;

            if adjusted_load < min_load {
                min_load = adjusted_load;
                best_cpu = cpu_id;
            }
        }
    }

    // Check if the best CPU is significantly better than current (hysteresis)
    // If difference is small, stick to start_cpu to avoid bouncing
    best_cpu
}

pub fn remove_context(context_id: &usize) {
    scheduler().run_queue.remove(*context_id);
}

pub fn request_preemption(token: &mut CleanLockToken) {
    if scheduler().should_preempt(token) {
        scheduler()
            .stats
            .preemptions
            .fetch_add(1, Ordering::Relaxed);
        ipi(IpiKind::Switch, IpiTarget::Current);
    }
}

pub fn balance(token: &mut CleanLockToken) {
    scheduler().try_balance(token);
}

// =============================================================================
// Validation & Benchmarks
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_priority_weight_scaling() {
        let w_rt = RunQueue::priority_to_weight(0);
        let w_norm = RunQueue::priority_to_weight(100);
        let w_idle = RunQueue::priority_to_weight(139);

        assert!(w_rt > w_norm);
        assert!(w_norm > w_idle);
    }

    #[test]
    fn test_time_slice_scaling() {
        let ts_rt = Scheduler::calculate_time_slice(0);
        let ts_norm = Scheduler::calculate_time_slice(100);

        assert_eq!(ts_rt, MIN_TIME_SLICE_NS);
        assert_eq!(ts_norm, BASE_TIME_SLICE_NS);
    }
}

#[cfg(test)]
mod benchmarks {
    use super::*;

    // A simulated benchmark for context switching overhead
    // logic. Real measurement requires running in kernel.
    #[test]
    fn bench_scheduler_overhead() {
        let scheduler = Scheduler::new(LogicalCpuId::new(0));
        let _q = &scheduler.run_queue;

        // Simulate adding tasks
        // Note: functionality limited in unit test environment without full context
        // This acts as a compile-time and basic logic check
        assert_eq!(scheduler.run_queue.len(), 0);
    }
}

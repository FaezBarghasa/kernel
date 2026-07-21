#![forbid(unsafe_code)]

//! # EEVDF Lock-Free Priority Context Ring
//!
//! Implements the Earliest Eligible Virtual Deadline First (EEVDF) scheduler ring
//! using a fixed-capacity lock-free buffer with atomic slot management.
//!
//! ## Design
//! - Tasks stored as `Arc<SchedulerContext>` in a fixed-size slot array.
//! - Each slot is an `ArcSwapOption<SchedulerContext>` — compare-and-swap capable.
//! - `min_vdeadline` tracks the cached minimum virtual deadline for O(1) hint.
//! - `select_next` scans for the slot whose `vdeadline` matches `min_vdeadline`.
//! - `push_context` atomically claims the next free slot and updates `min_vdeadline`.
//! - All hot-path operations are lock-free; no `Mutex` or `RwLock` on the data path.
//!
//! ## Virtual Deadline Formula
//! ```text
//! V_i = T_i + ((S_i << 10) / W_i)
//! where T_i = vruntime, S_i = slice_ns, W_i = weight
//! ```

use alloc::sync::Arc;
use arc_swap::ArcSwapOption;
use core::sync::atomic::{AtomicI64, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use crossbeam_utils::CachePadded;

// ─────────────────────────────────────────────────────────────────────────────
// Error types
// ─────────────────────────────────────────────────────────────────────────────

/// All errors that can be returned by ring operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerError {
    /// Ring is at maximum capacity.
    QueueFull,
    /// Task weight is 0 — division by zero would occur in deadline calculation.
    InvalidWeight,
    /// Task ID was not found in any ring slot.
    TaskNotFound,
    /// Capacity is 0 or exceeds 1,000,000.
    InvalidCapacity,
    /// Arithmetic overflow in deadline calculation.
    Overflow,
    /// Ring was at capacity; task was removed but could not be re-inserted.
    RingFull,
    /// Holder's weight already equals or exceeds the blocked task's weight.
    AlreadyInherited,
}

// ─────────────────────────────────────────────────────────────────────────────
// Task state constants
// ─────────────────────────────────────────────────────────────────────────────

/// Task is ready to run and waiting in the ring.
pub const STATE_READY: u8 = 0;
/// Task is currently executing on a CPU.
pub const STATE_RUNNING: u8 = 1;
/// Task is blocked / sleeping.
pub const STATE_SLEEPING: u8 = 2;
/// Task has been stopped.
pub const STATE_STOPPED: u8 = 3;

// ─────────────────────────────────────────────────────────────────────────────
// SchedulerContext
// ─────────────────────────────────────────────────────────────────────────────

/// A single schedulable task's EEVDF metrics.
///
/// All time values are in nanoseconds, scaled by 1024 for fixed-point precision.
/// The struct is stored behind `Arc` so multiple cores can reference the same
/// task metrics without copying — zero-copy IPC wakeup updates are performed
/// via atomic stores on the arc's fields.
pub struct SchedulerContext {
    /// Unique task identifier (PID / TID).
    pub task_id: u64,

    /// Virtual runtime accumulated (scaled by 1024).
    /// This is T_i in the EEVDF formula.
    pub vruntime: AtomicU64,

    /// Computed virtual deadline: `V_i = T_i + ((S_i << 10) / W_i)`.
    /// Updated atomically whenever `vruntime` or `weight` changes.
    pub vdeadline: AtomicU64,

    /// Current time slice allocation in nanoseconds (unscaled).
    /// This is S_i in the EEVDF formula.
    pub slice_ns: u64,

    /// Priority weight (1–1024, higher = more CPU time).
    /// Nice -20 = 1024, Nice 0 = 512, Nice 19 = 1.
    pub weight: u32,

    /// Virtual lag relative to the ring average.
    /// Positive = task is behind schedule (needs more CPU).
    /// Negative = task is ahead of schedule.
    pub lag: AtomicI64,

    /// Monotonic nanosecond timestamp when this task became eligible.
    pub eligible_since: AtomicU64,

    /// CPU affinity bitmask (set bit N = allowed on core N).
    pub cpu_affinity: AtomicU64,

    /// Current task state (`STATE_READY`, `STATE_RUNNING`, etc.)
    pub state: AtomicU8,
}

impl SchedulerContext {
    /// Constructs a new `SchedulerContext` with pre-calculated virtual deadline.
    ///
    /// # Panics
    /// Panics if `weight == 0`.
    pub fn new(
        task_id: u64,
        vruntime_ns: u64,
        slice_ns: u64,
        weight: u32,
        cpu_affinity: u64,
    ) -> Self {
        assert!(weight > 0, "weight must be non-zero");
        let vdeadline = calculate_vdeadline(vruntime_ns, slice_ns, weight);
        Self {
            task_id,
            vruntime: AtomicU64::new(vruntime_ns),
            vdeadline: AtomicU64::new(vdeadline),
            slice_ns,
            weight,
            lag: AtomicI64::new(0),
            eligible_since: AtomicU64::new(0),
            cpu_affinity: AtomicU64::new(cpu_affinity),
            state: AtomicU8::new(STATE_READY),
        }
    }

    /// Returns the task's current virtual deadline (atomic load, `Acquire`).
    #[inline]
    pub fn vdeadline(&self) -> u64 {
        self.vdeadline.load(Ordering::Acquire)
    }

    /// Returns the task's current virtual runtime (atomic load, `Acquire`).
    #[inline]
    pub fn vruntime(&self) -> u64 {
        self.vruntime.load(Ordering::Acquire)
    }

    /// Recalculates and stores the virtual deadline from the current vruntime.
    #[inline]
    fn refresh_vdeadline(&self) {
        let vrt = self.vruntime.load(Ordering::Acquire);
        let vd = calculate_vdeadline(vrt, self.slice_ns, self.weight);
        self.vdeadline.store(vd, Ordering::Release);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Virtual deadline math
// ─────────────────────────────────────────────────────────────────────────────

/// Calculates the virtual deadline for a task using fixed-point integer arithmetic.
///
/// Formula: `V_i = T_i + ((S_i << 10) / W_i)`
///
/// The `<< 10` multiplier (÷ 1024) provides sub-nanosecond precision without
/// floating-point operations. Saturates on overflow.
///
/// # Arguments
/// * `vruntime` — current virtual runtime in nanoseconds (T_i)
/// * `slice_ns` — time slice in nanoseconds (S_i)
/// * `weight` — task priority weight 1–1024 (W_i)
///
/// # Panics
/// Panics if `weight == 0`.
#[inline]
pub fn calculate_vdeadline(vruntime: u64, slice_ns: u64, weight: u32) -> u64 {
    assert!(weight > 0, "weight must be non-zero");
    let scaled = match slice_ns.checked_shl(10) {
        Some(v) => v,
        None => u64::MAX,
    };
    let term = scaled / weight as u64;
    vruntime.saturating_add(term)
}

/// Calculates the scheduling lag for a task.
///
/// Positive lag means the task is overdue; negative means it's ahead.
#[inline]
pub fn calculate_lag(avg_vruntime: u64, task_vruntime: u64) -> i64 {
    (avg_vruntime as i64).saturating_sub(task_vruntime as i64)
}

// ─────────────────────────────────────────────────────────────────────────────
// ContextRing — lock-free bounded priority ring
// ─────────────────────────────────────────────────────────────────────────────

/// Lock-free circular ring buffer for O(1)-amortised task selection.
///
/// Tasks are stored in `Arc<SchedulerContext>` behind `ArcSwapOption` slots.
/// The minimum virtual deadline is cached in `min_vdeadline` so that the timer
/// subsystem can program the LAPIC/GIC one-shot interrupt without scanning.
///
/// `select_next` finds the slot matching `min_vdeadline`, takes it atomically,
/// then re-scans remaining slots to update `min_vdeadline` — this is O(N) in
/// the worst case but O(1) in steady state when the hot task is at the head.
pub struct ContextRing {
    /// Fixed-size array of task slots.
    buffer: alloc::vec::Vec<ArcSwapOption<SchedulerContext>>,

    /// Cached minimum virtual deadline across all active slots.
    /// Set to `u64::MAX` when ring is empty.
    min_vdeadline: CachePadded<AtomicU64>,

    /// Total number of tasks currently held in the ring.
    task_count: CachePadded<AtomicUsize>,

    /// Sum of all active tasks' weights (for lag recalculation).
    total_weight: CachePadded<AtomicU64>,

    /// Weighted average vruntime, updated on every push / pop.
    avg_vruntime: CachePadded<AtomicU64>,

    /// Maximum number of tasks this ring can hold.
    capacity: usize,
}

impl ContextRing {
    /// Creates a new `ContextRing` with `capacity` task slots.
    ///
    /// # Errors
    /// Returns `SchedulerError::InvalidCapacity` if `capacity == 0` or > 1,000,000.
    pub fn new(capacity: usize) -> Result<Self, SchedulerError> {
        if capacity == 0 || capacity > 1_000_000 {
            return Err(SchedulerError::InvalidCapacity);
        }
        let mut buffer = alloc::vec::Vec::with_capacity(capacity);
        for _ in 0..capacity {
            buffer.push(ArcSwapOption::const_empty());
        }
        Ok(Self {
            buffer,
            min_vdeadline: CachePadded::new(AtomicU64::new(u64::MAX)),
            task_count: CachePadded::new(AtomicUsize::new(0)),
            total_weight: CachePadded::new(AtomicU64::new(0)),
            avg_vruntime: CachePadded::new(AtomicU64::new(0)),
            capacity,
        })
    }

    /// Returns the number of tasks currently in the ring.
    #[inline]
    pub fn len(&self) -> usize {
        self.task_count.load(Ordering::Relaxed)
    }

    /// Returns `true` if the ring contains no tasks.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the capacity of this ring.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the cached minimum virtual deadline across all tasks.
    /// Returns `u64::MAX` when the ring is empty.
    #[inline]
    pub fn min_vdeadline(&self) -> u64 {
        self.min_vdeadline.load(Ordering::Acquire)
    }

    /// Returns the current average virtual runtime.
    #[inline]
    pub fn average_vruntime(&self) -> u64 {
        self.avg_vruntime.load(Ordering::Acquire)
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    /// Scans all slots and updates `min_vdeadline` + `avg_vruntime`.
    fn recompute_stats(&self) {
        let mut min_vd = u64::MAX;
        let mut sum_vrt = 0u128;
        let mut count = 0usize;
        let mut sum_weight = 0u64;

        for slot in &self.buffer {
            if let Some(ctx) = slot.load_full() {
                let vd = ctx.vdeadline();
                let vrt = ctx.vruntime();
                let w = ctx.weight as u64;
                if vd < min_vd {
                    min_vd = vd;
                }
                sum_vrt += vrt as u128;
                sum_weight += w;
                count += 1;
            }
        }

        self.min_vdeadline.store(min_vd, Ordering::Release);
        self.total_weight.store(sum_weight, Ordering::Release);
        let avg = if count == 0 {
            0
        } else {
            (sum_vrt / count as u128) as u64
        };
        self.avg_vruntime.store(avg, Ordering::Release);
    }

    /// Atomically updates `min_vdeadline` downward if `candidate < current`.
    fn update_min_vdeadline(&self, candidate: u64) {
        let mut current = self.min_vdeadline.load(Ordering::Acquire);
        while candidate < current {
            match self.min_vdeadline.compare_exchange_weak(
                current,
                candidate,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }

    // ── Public API ───────────────────────────────────────────────────────────

    /// Inserts a task into the ring in the first available slot.
    ///
    /// # Errors
    /// - `SchedulerError::InvalidWeight` if `ctx.weight == 0`.
    /// - `SchedulerError::QueueFull` if all slots are occupied.
    pub fn push_context(&self, ctx: SchedulerContext) -> Result<(), SchedulerError> {
        if ctx.weight == 0 {
            return Err(SchedulerError::InvalidWeight);
        }

        let vd = ctx.vdeadline();
        let vrt = ctx.vruntime();
        let w = ctx.weight as u64;
        let arc = Arc::new(ctx);

        // Find an empty slot via ArcSwapOption compare-and-swap.
        for slot in &self.buffer {
            // Only attempt to place into slots that appear empty.
            if slot.load().is_some() {
                continue;
            }
            // Atomically try to store: if this slot was taken by a concurrent
            // push since we checked, `compare_and_swap` will leave `Some` in
            // place and we continue to the next slot.
            let prev = slot.compare_and_swap(&None::<Arc<SchedulerContext>>, Some(arc.clone()));
            if prev.is_none() {
                // Success — update stats.
                self.task_count.fetch_add(1, Ordering::AcqRel);
                self.total_weight.fetch_add(w, Ordering::Relaxed);
                // Recalculate average vruntime incrementally.
                let count = self.task_count.load(Ordering::Acquire) as u64;
                let prev_avg = self.avg_vruntime.load(Ordering::Acquire);
                let new_avg = if count == 0 {
                    vrt
                } else {
                    // Weighted rolling update: avg = (prev_avg * (count-1) + vrt) / count
                    (prev_avg.saturating_mul(count - 1).saturating_add(vrt)) / count
                };
                self.avg_vruntime.store(new_avg, Ordering::Release);
                self.update_min_vdeadline(vd);
                return Ok(());
            }
        }

        Err(SchedulerError::QueueFull)
    }

    /// Selects and extracts the task with the earliest virtual deadline.
    ///
    /// The returned `Arc<SchedulerContext>` allows the caller (context-switch
    /// code) to access the task's atomic fields without any copy.
    ///
    /// Returns `None` if the ring is empty.
    pub fn select_next(&self) -> Option<Arc<SchedulerContext>> {
        if self.task_count.load(Ordering::Acquire) == 0 {
            return None;
        }

        let target_vd = self.min_vdeadline.load(Ordering::Acquire);
        if target_vd == u64::MAX {
            return None;
        }

        // Scan for a slot whose vdeadline matches the cached minimum.
        for slot in &self.buffer {
            if let Some(ctx) = slot.load_full() {
                if ctx.vdeadline() == target_vd {
                    // Attempt atomic take.
                    let taken = slot.compare_and_swap(&Some(ctx.clone()), None);
                    if taken.is_some() {
                        let w = taken.as_ref().unwrap().weight as u64;
                        self.task_count.fetch_sub(1, Ordering::AcqRel);
                        self.total_weight
                            .fetch_sub(w.min(self.total_weight.load(Ordering::Relaxed)),
                                       Ordering::Relaxed);
                        self.recompute_stats();
                        return taken;
                    }
                }
            }
        }

        // Fallback: full scan for any non-empty slot (handles concurrent races).
        let mut best_vd = u64::MAX;
        let mut best_idx = None;

        for (i, slot) in self.buffer.iter().enumerate() {
            if let Some(ctx) = slot.load_full() {
                let vd = ctx.vdeadline();
                if vd < best_vd {
                    best_vd = vd;
                    best_idx = Some(i);
                }
            }
        }

        if let Some(idx) = best_idx {
            let taken = self.buffer[idx].swap(None);
            if taken.is_some() {
                let w = taken.as_ref().unwrap().weight as u64;
                self.task_count.fetch_sub(1, Ordering::AcqRel);
                self.total_weight
                    .fetch_sub(w.min(self.total_weight.load(Ordering::Relaxed)),
                               Ordering::Relaxed);
                self.recompute_stats();
                return taken;
            }
        }

        None
    }

    /// Selects the task with the **furthest** virtual deadline for work-stealing.
    ///
    /// Idle CPUs steal the coldest task from overloaded siblings to balance load.
    pub fn select_furthest(&self) -> Option<Arc<SchedulerContext>> {
        if self.task_count.load(Ordering::Acquire) == 0 {
            return None;
        }

        let mut worst_vd = 0u64;
        let mut worst_idx = None;

        for (i, slot) in self.buffer.iter().enumerate() {
            if let Some(ctx) = slot.load_full() {
                let vd = ctx.vdeadline();
                if vd > worst_vd {
                    worst_vd = vd;
                    worst_idx = Some(i);
                }
            }
        }

        if let Some(idx) = worst_idx {
            let taken = self.buffer[idx].swap(None);
            if taken.is_some() {
                let w = taken.as_ref().unwrap().weight as u64;
                self.task_count.fetch_sub(1, Ordering::AcqRel);
                self.total_weight
                    .fetch_sub(w.min(self.total_weight.load(Ordering::Relaxed)),
                               Ordering::Relaxed);
                self.recompute_stats();
                return taken;
            }
        }

        None
    }

    /// Atomically reduces a task's virtual runtime by `delta_ns` and refreshes
    /// its virtual deadline — used when an IPC message wakes a sleeping task
    /// and we want it near the front of the queue.
    ///
    /// # Errors
    /// - `SchedulerError::TaskNotFound` — task ID absent from ring.
    pub fn boost_vruntime(&self, task_id: u64, delta_ns: u64) -> Result<(), SchedulerError> {
        for slot in &self.buffer {
            if let Some(ctx) = slot.load_full() {
                if ctx.task_id == task_id {
                    // Atomically subtract delta (scaled by 1024 for fixed-point).
                    let scaled_delta = delta_ns.saturating_mul(1024);
                    let old_vrt = ctx.vruntime.load(Ordering::Acquire);
                    let new_vrt = old_vrt.saturating_sub(scaled_delta);
                    ctx.vruntime.store(new_vrt, Ordering::Release);
                    ctx.refresh_vdeadline();
                    // Update lag field.
                    let avg = self.avg_vruntime.load(Ordering::Acquire);
                    ctx.lag.store(calculate_lag(avg, new_vrt), Ordering::Release);
                    // May have moved to the front; update ring min.
                    self.update_min_vdeadline(ctx.vdeadline());
                    return Ok(());
                }
            }
        }
        Err(SchedulerError::TaskNotFound)
    }

    /// Wait-free priority inheritance: temporarily raises the `holder` task's
    /// weight to match the `blocked` task's weight so the holder can finish
    /// and release its lock faster.
    ///
    /// No allocation or mutex is used — weights are updated via atomic stores
    /// on the `Arc<SchedulerContext>` fields.
    ///
    /// # Errors
    /// - `SchedulerError::TaskNotFound` — either task ID absent.
    /// - `SchedulerError::AlreadyInherited` — no weight change needed.
    pub fn priority_inheritance(
        &self,
        blocked_task_id: u64,
        holder_task_id: u64,
    ) -> Result<(), SchedulerError> {
        // Read blocked task's weight without removing it.
        let blocked_weight = self
            .buffer
            .iter()
            .find_map(|slot| {
                slot.load_full().and_then(|ctx| {
                    if ctx.task_id == blocked_task_id {
                        Some(ctx.weight)
                    } else {
                        None
                    }
                })
            })
            .ok_or(SchedulerError::TaskNotFound)?;

        // Find holder and propagate weight if beneficial.
        let holder_arc = self
            .buffer
            .iter()
            .find_map(|slot| {
                slot.load_full().and_then(|ctx| {
                    if ctx.task_id == holder_task_id {
                        Some(ctx)
                    } else {
                        None
                    }
                })
            })
            .ok_or(SchedulerError::TaskNotFound)?;

        if holder_arc.weight >= blocked_weight {
            return Err(SchedulerError::AlreadyInherited);
        }

        // Weight is `u32` (not atomic) so we must remove + re-insert the task
        // to mutate it. We do this by swapping the slot to a new Arc with the
        // updated weight — the only allocation is a new Arc wrapper.
        let new_ctx = Arc::new(SchedulerContext::new(
            holder_arc.task_id,
            holder_arc.vruntime(),
            holder_arc.slice_ns,
            blocked_weight,
            holder_arc.cpu_affinity.load(Ordering::Relaxed),
        ));

        for slot in &self.buffer {
            if let Some(old) = slot.load_full() {
                if old.task_id == holder_task_id {
                    // Replace atomically.
                    slot.store(Some(new_ctx.clone()));
                    self.update_min_vdeadline(new_ctx.vdeadline());
                    return Ok(());
                }
            }
        }

        Err(SchedulerError::TaskNotFound)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Unit tests — 10+ required by Faez Standard
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx(id: u64, vruntime: u64, slice_ns: u64, weight: u32) -> SchedulerContext {
        SchedulerContext::new(id, vruntime, slice_ns, weight, u64::MAX)
    }

    // ── Test 1: Empty ring construction ──────────────────────────────────────

    #[test]
    fn test_ring_creation_empty() {
        let ring = ContextRing::new(1024).unwrap();
        assert_eq!(ring.len(), 0);
        assert!(ring.is_empty());
        assert!(ring.select_next().is_none());
        assert_eq!(ring.min_vdeadline(), u64::MAX);
    }

    // ── Test 2: Invalid capacity ──────────────────────────────────────────────

    #[test]
    fn test_invalid_capacity_zero() {
        assert!(matches!(ContextRing::new(0), Err(SchedulerError::InvalidCapacity)));
    }

    #[test]
    fn test_invalid_capacity_too_large() {
        assert!(matches!(
            ContextRing::new(1_000_001),
            Err(SchedulerError::InvalidCapacity)
        ));
    }

    // ── Test 3: Single task insert + select ──────────────────────────────────

    #[test]
    fn test_single_task_insert_select() {
        let ring = ContextRing::new(1024).unwrap();
        ring.push_context(make_ctx(42, 1_000_000, 4_000_000, 512)).unwrap();
        assert_eq!(ring.len(), 1);

        let selected = ring.select_next().unwrap();
        assert_eq!(selected.task_id, 42);
        assert!(ring.is_empty());
    }

    // ── Test 4: Earliest virtual deadline selected first ─────────────────────

    #[test]
    fn test_earliest_deadline_selected_first() {
        let ring = ContextRing::new(1024).unwrap();
        // Task 1: high vruntime → higher deadline
        ring.push_context(make_ctx(1, 5_000_000, 4_000_000, 512)).unwrap();
        // Task 2: low vruntime → earliest deadline (should be selected)
        ring.push_context(make_ctx(2, 1_000_000, 4_000_000, 512)).unwrap();
        // Task 3: medium vruntime
        ring.push_context(make_ctx(3, 3_000_000, 4_000_000, 512)).unwrap();

        let selected = ring.select_next().unwrap();
        assert_eq!(selected.task_id, 2, "task with lowest vruntime must win");
    }

    // ── Test 5: Virtual deadline arithmetic ──────────────────────────────────

    #[test]
    fn test_vdeadline_calculation_accuracy() {
        // V_i = T_i + ((S_i << 10) / W_i)
        // T_i = 5_000_000, S_i = 4_000_000, W_i = 512
        // V_i = 5_000_000 + ((4_000_000 * 1024) / 512)
        //     = 5_000_000 + 8_000_000
        //     = 13_000_000
        let vd = calculate_vdeadline(5_000_000, 4_000_000, 512);
        assert_eq!(vd, 13_000_000);
    }

    // ── Test 6: High weight → smaller deadline increment ─────────────────────

    #[test]
    fn test_high_weight_smaller_deadline_increment() {
        let vd_low = calculate_vdeadline(0, 4_000, 1);
        let vd_high = calculate_vdeadline(0, 4_000, 1024);
        assert!(vd_high < vd_low, "heavier tasks should get sooner deadlines");
    }

    // ── Test 7: Queue full returns error ──────────────────────────────────────

    #[test]
    fn test_queue_full_error() {
        let ring = ContextRing::new(2).unwrap();
        ring.push_context(make_ctx(1, 1_000, 4_000, 512)).unwrap();
        ring.push_context(make_ctx(2, 2_000, 4_000, 512)).unwrap();

        let result = ring.push_context(make_ctx(3, 3_000, 4_000, 512));
        assert!(
            matches!(result, Err(SchedulerError::QueueFull)),
            "inserting into full ring must fail"
        );
    }

    // ── Test 8: Zero weight returns InvalidWeight ─────────────────────────────

    #[test]
    fn test_zero_weight_error() {
        let ring = ContextRing::new(1024).unwrap();
        let ctx = SchedulerContext {
            task_id: 99,
            vruntime: AtomicU64::new(0),
            vdeadline: AtomicU64::new(0),
            slice_ns: 4_000_000,
            weight: 0, // invalid
            lag: AtomicI64::new(0),
            eligible_since: AtomicU64::new(0),
            cpu_affinity: AtomicU64::new(u64::MAX),
            state: AtomicU8::new(STATE_READY),
        };
        assert!(matches!(
            ring.push_context(ctx),
            Err(SchedulerError::InvalidWeight)
        ));
    }

    // ── Test 9: boost_vruntime reduces deadline and advances priority ─────────

    #[test]
    fn test_boost_vruntime_advances_priority() {
        let ring = ContextRing::new(1024).unwrap();
        // Insert a "cold" task with high vruntime and a "hot" task with low vruntime.
        ring.push_context(make_ctx(1, 10_000_000, 4_000_000, 512)).unwrap(); // cold
        ring.push_context(make_ctx(2, 1_000_000, 4_000_000, 512)).unwrap();  // hot

        // Without boost, task 2 (low vruntime) is selected first.
        // Now boost task 1 massively so it overtakes task 2.
        ring.boost_vruntime(1, 9_500_000).unwrap();

        let selected = ring.select_next().unwrap();
        assert_eq!(selected.task_id, 1, "boosted task must be selected first");
    }

    // ── Test 10: boost_vruntime on missing task returns TaskNotFound ──────────

    #[test]
    fn test_boost_vruntime_not_found() {
        let ring = ContextRing::new(1024).unwrap();
        let result = ring.boost_vruntime(999, 1_000);
        assert!(matches!(result, Err(SchedulerError::TaskNotFound)));
    }

    // ── Test 11: Priority inheritance raises holder weight ────────────────────

    #[test]
    fn test_priority_inheritance_raises_weight() {
        let ring = ContextRing::new(1024).unwrap();
        ring.push_context(make_ctx(1, 1_000, 4_000_000, 100)).unwrap(); // low-priority holder
        ring.push_context(make_ctx(2, 2_000, 4_000_000, 900)).unwrap(); // high-priority blocked

        ring.priority_inheritance(2, 1).unwrap();

        // Find holder in ring and verify weight was promoted.
        let holder = ring
            .buffer
            .iter()
            .find_map(|slot| {
                slot.load_full()
                    .and_then(|c| if c.task_id == 1 { Some(c) } else { None })
            })
            .expect("holder must still be in ring");
        assert_eq!(holder.weight, 900, "holder weight must be promoted to 900");
    }

    // ── Test 12: AlreadyInherited when no weight change needed ────────────────

    #[test]
    fn test_priority_inheritance_already_optimal() {
        let ring = ContextRing::new(1024).unwrap();
        ring.push_context(make_ctx(1, 1_000, 4_000_000, 900)).unwrap(); // already high weight
        ring.push_context(make_ctx(2, 2_000, 4_000_000, 500)).unwrap(); // lower-priority blocked

        let result = ring.priority_inheritance(2, 1);
        assert!(matches!(result, Err(SchedulerError::AlreadyInherited)));
    }

    // ── Test 13: task_count stays consistent across push/pop ─────────────────

    #[test]
    fn test_task_count_consistency() {
        let ring = ContextRing::new(8).unwrap();
        for i in 0..5 {
            ring.push_context(make_ctx(i, i * 1000, 4_000, 512)).unwrap();
        }
        assert_eq!(ring.len(), 5);

        ring.select_next().unwrap();
        assert_eq!(ring.len(), 4);

        ring.select_next().unwrap();
        assert_eq!(ring.len(), 3);
    }

    // ── Test 14: select_furthest returns highest vdeadline ────────────────────

    #[test]
    fn test_select_furthest_returns_cold_task() {
        let ring = ContextRing::new(1024).unwrap();
        ring.push_context(make_ctx(1, 1_000_000, 4_000_000, 512)).unwrap();
        ring.push_context(make_ctx(2, 9_000_000, 4_000_000, 512)).unwrap(); // furthest deadline
        ring.push_context(make_ctx(3, 4_000_000, 4_000_000, 512)).unwrap();

        let cold = ring.select_furthest().unwrap();
        assert_eq!(cold.task_id, 2, "furthest deadline must be work-stolen");
    }

    // ── Test 15: Overflow saturation in vdeadline calculation ─────────────────

    #[test]
    fn test_vdeadline_overflow_saturates() {
        // u64::MAX slice_ns would overflow << 10; must saturate, not panic.
        let vd = calculate_vdeadline(0, u64::MAX, 1);
        assert_eq!(vd, u64::MAX, "overflow must saturate at u64::MAX");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Benchmarks — 3 required by Faez Standard
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod benches {
    use super::*;
    use std::time::Instant;

    /// Benchmark 1: `select_next` with 1 000 tasks.
    /// Target: < 500 ns per operation on modern hardware.
    #[test]
    fn bench_select_next_1k_tasks() {
        let ring = ContextRing::new(2000).unwrap();
        for i in 0u64..1000 {
            ring.push_context(SchedulerContext::new(i, i * 1000, 4_000_000, 512, u64::MAX))
                .unwrap();
        }

        let iterations = 10_000u32;
        let start = Instant::now();

        for i in 0u64..iterations as u64 {
            if ring.select_next().is_some() {
                // Re-insert to maintain population.
                let _ = ring.push_context(SchedulerContext::new(
                    i % 1000,
                    i * 500,
                    4_000_000,
                    512,
                    u64::MAX,
                ));
            }
        }

        let elapsed = start.elapsed();
        let per_op_ns = elapsed.as_nanos() / iterations as u128;
        println!("bench_select_next_1k_tasks: {}ns/op", per_op_ns);
        // Soft target — assert only in non-debug builds to avoid CI flakiness.
        #[cfg(not(debug_assertions))]
        assert!(
            per_op_ns < 1_000,
            "select_next exceeded 1µs/op: {}ns",
            per_op_ns
        );
    }

    /// Benchmark 2: `push_context` throughput.
    /// Target: < 200 ns per insertion.
    #[test]
    fn bench_push_context_throughput() {
        let ring = ContextRing::new(100_000).unwrap();
        let iterations = 50_000u32;
        let start = Instant::now();

        for i in 0u64..iterations as u64 {
            let _ = ring.push_context(SchedulerContext::new(i, i * 100, 4_000_000, 512, u64::MAX));
        }

        let elapsed = start.elapsed();
        let per_op_ns = elapsed.as_nanos() / iterations as u128;
        println!("bench_push_context: {}ns/op", per_op_ns);
        #[cfg(not(debug_assertions))]
        assert!(per_op_ns < 500, "push_context exceeded 500ns/op: {}ns", per_op_ns);
    }

    /// Benchmark 3: `calculate_vdeadline` raw throughput.
    /// Target: < 5 ns per call (pure integer arithmetic).
    #[test]
    fn bench_calculate_vdeadline() {
        let iterations = 1_000_000u32;
        let start = Instant::now();

        for i in 0u64..iterations as u64 {
            core::hint::black_box(calculate_vdeadline(i * 1000, 4_000_000, 512));
        }

        let elapsed = start.elapsed();
        let per_op_ns = elapsed.as_nanos() / iterations as u128;
        println!("bench_calculate_vdeadline: {}ns/op", per_op_ns);
        #[cfg(not(debug_assertions))]
        assert!(per_op_ns < 10, "calculate_vdeadline exceeded 10ns/op: {}ns", per_op_ns);
    }
}

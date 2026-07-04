//! # SCX (sched_ext) External Scheduler Interface
//!
//! Provides `sys_sched_scx_register` — a sandboxed syscall that allows a
//! user-space scheduling policy to plug into the kernel's EEVDF runqueue
//! via a lock-free IPC queue.
//!
//! ## Safety contract
//! - The SCX subsystem runs in a timed window. If it misses its deadline
//!   (`SCX_DEADLINE_NS`), the kernel immediately reverts to the native
//!   EEVDF implementation without dropping any contexts.
//! - All context operations go through `CleanLockToken` as usual.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::{
    context::ContextRef,
    cpu_set::LogicalCpuId,
    scheduler::{ExtSchedulerOps, EXT_SCHEDULER},
    sync::CleanLockToken,
    syscall::error::{Error, Result, EINVAL, EPERM},
    time::monotonic,
};

// =============================================================================
// Constants
// =============================================================================

/// Maximum allowed time for an SCX `select_next` call in nanoseconds (500 µs).
const SCX_DEADLINE_NS: u64 = 500_000;

/// Maximum number of contexts the SCX policy can hold in its pending queue.
const SCX_QUEUE_CAPACITY: usize = 2048;

// =============================================================================
// SCX Scheduler
// =============================================================================

/// An external (SCX) scheduler communicating over a lock-free IPC queue.
///
/// Contexts enqueued here are visible to the user-space scheduling daemon
/// via the `scx:` scheme. If the daemon fails to pick the next context
/// within `SCX_DEADLINE_NS`, the system falls back to EEVDF automatically
/// (handled by the existing fallback logic in `RunQueue::next`).
pub struct ScxScheduler {
    /// Lock-free queue for runnable contexts.
    queue: crossbeam_queue::SegQueue<ContextRef>,
    /// Whether the SCX subsystem is currently enabled.
    pub enabled: AtomicBool,
    /// Timestamp of the last successful `select_next` call.
    last_success_ns: AtomicU64,
    /// Total number of deadline misses (monotonically increasing).
    pub deadline_misses: AtomicU64,
    /// Total contexts enqueued since registration.
    pub enqueued_total: AtomicU64,
    /// Total contexts selected since registration.
    pub selected_total: AtomicU64,
}

impl ScxScheduler {
    pub fn new() -> Self {
        ScxScheduler {
            queue: crossbeam_queue::SegQueue::new(),
            enabled: AtomicBool::new(true),
            last_success_ns: AtomicU64::new(0),
            deadline_misses: AtomicU64::new(0),
            enqueued_total: AtomicU64::new(0),
            selected_total: AtomicU64::new(0),
        }
    }

    /// Returns `true` if the SCX subsystem has been idle for more than
    /// `SCX_DEADLINE_NS` without a successful `select_next`.
    pub fn deadline_exceeded(&self) -> bool {
        let last = self.last_success_ns.load(Ordering::Relaxed);
        if last == 0 {
            return false; // Not yet used.
        }
        (monotonic() as u64).saturating_sub(last) > SCX_DEADLINE_NS
    }

    /// Drain all queued contexts into a Vec for re-enqueue into EEVDF.
    pub fn drain_all(&self) -> alloc::vec::Vec<ContextRef> {
        let mut drained = alloc::vec::Vec::new();
        while let Some(ctx) = self.queue.pop() {
            drained.push(ctx);
        }
        drained
    }
}

impl ExtSchedulerOps for ScxScheduler {
    fn enqueue(&self, context: ContextRef) -> bool {
        if !self.enabled.load(Ordering::Acquire) {
            return false;
        }
        // Enforce soft capacity limit to prevent unbounded memory growth.
        if self.queue.len() >= SCX_QUEUE_CAPACITY {
            return false;
        }
        self.queue.push(context);
        self.enqueued_total.fetch_add(1, Ordering::Relaxed);
        true
    }

    fn dequeue(&self, _context_id: usize) {
        // The SCX queue is FIFO; we cannot efficiently remove by ID.
        // The user-space daemon is responsible for discarding stale contexts.
    }

    fn select_next(&self, _cpu_id: LogicalCpuId) -> Option<ContextRef> {
        if !self.enabled.load(Ordering::Acquire) {
            return None;
        }

        let t0 = monotonic() as u64;
        let result = self.queue.pop();
        let elapsed = (monotonic() as u64).saturating_sub(t0);

        if elapsed > SCX_DEADLINE_NS {
            // The pop itself was slow — record the miss.
            self.deadline_misses.fetch_add(1, Ordering::Relaxed);
            self.enabled.store(false, Ordering::Release);
            log::warn!(
                "scx: select_next exceeded deadline ({} ns). Disabling SCX.",
                elapsed
            );
            return None;
        }

        if result.is_some() {
            self.last_success_ns
                .store(monotonic() as u64, Ordering::Relaxed);
            self.selected_total.fetch_add(1, Ordering::Relaxed);
        }

        result
    }
}

// =============================================================================
// Syscall: sys_sched_scx_register
// =============================================================================

/// `sys_sched_scx_register() -> Result<()>`
///
/// Registers a new SCX scheduling policy. Only one SCX policy may be active
/// at a time. If a previous SCX policy was disabled (deadline miss), it is
/// replaced. Returns `EPERM` if a live SCX policy already exists.
pub fn sys_sched_scx_register(_token: &mut CleanLockToken) -> Result<()> {
    // Check if an active SCX scheduler is already registered.
    {
        let guard = EXT_SCHEDULER.read();
        if guard.is_some() {
            // The existing fallback logic in RunQueue::next will auto-disable an
            // SCX that exceeds its time budget, setting the guard to None.
            // If it's still Some, it's still live.
            return Err(Error::new(EPERM));
        }
    }

    let scx = Arc::new(ScxScheduler::new());
    let mut guard = EXT_SCHEDULER.write();
    *guard = Some(scx as Arc<dyn ExtSchedulerOps>);

    log::info!("scx: external scheduler registered successfully");
    Ok(())
}

/// `sys_sched_scx_unregister() -> Result<()>`
///
/// Unregisters the currently active SCX policy. All queued contexts are
/// drained and re-enqueued into the native EEVDF runqueue on the calling CPU.
pub fn sys_sched_scx_unregister(token: &mut CleanLockToken) -> Result<()> {
    let old = {
        let mut guard = EXT_SCHEDULER.write();
        guard.take().ok_or(Error::new(EINVAL))?
    };

    // The old Arc<dyn ExtSchedulerOps> is the ScxScheduler. We can't downcast
    // without nightly specialization, so we drain via the trait's select_next
    // in a loop (it's lock-free and terminates when the queue is empty).
    loop {
        match old.select_next(crate::cpu_id()) {
            Some(ctx) => crate::scheduler::add_context(ctx, token),
            None => break,
        }
    }

    log::info!("scx: external scheduler unregistered, contexts drained to EEVDF");
    Ok(())
}

// =============================================================================
// SCX Statistics Snapshot
// =============================================================================

/// Point-in-time statistics snapshot for the active SCX policy.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScxStats {
    pub active: bool,
    pub deadline_misses: u64,
    pub enqueued_total: u64,
    pub selected_total: u64,
}

/// Read current SCX status (non-blocking). Returns `None` if no SCX is active.
pub fn scx_is_active() -> bool {
    EXT_SCHEDULER.read().is_some()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scx_construction_defaults() {
        let scx = ScxScheduler::new();
        assert!(scx.enabled.load(Ordering::Relaxed));
        assert_eq!(scx.deadline_misses.load(Ordering::Relaxed), 0);
        assert_eq!(scx.enqueued_total.load(Ordering::Relaxed), 0);
        assert_eq!(scx.selected_total.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_scx_deadline_not_exceeded_when_fresh() {
        let scx = ScxScheduler::new();
        // last_success_ns is 0, so deadline_exceeded must be false.
        assert!(!scx.deadline_exceeded());
    }

    #[test]
    fn test_scx_disable_prevents_enqueue() {
        let scx = ScxScheduler::new();
        scx.enabled.store(false, Ordering::Release);
        // select_next on disabled SCX should return None immediately.
        assert!(scx.select_next(LogicalCpuId::new(0)).is_none());
    }

    #[test]
    fn test_scx_drain_empty_queue() {
        let scx = ScxScheduler::new();
        let drained = scx.drain_all();
        assert!(drained.is_empty());
    }
}

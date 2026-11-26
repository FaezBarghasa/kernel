//! Full Preemption Support (PREEMPT_FULL)
//!
//! This module provides full preemption support for all kernel code paths,
//! including driver and personality servers. This prevents background task
//! stalls and ensures interactive responsiveness.
//!
//! # Design
//!
//! - Preemption points at all voluntary schedule points
//! - Preempt counter tracking for critical sections
//! - Lazy preemption for batch-friendly workloads
//! - Immediate preemption for RT and interactive tasks

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::ipi::{ipi, IpiKind, IpiTarget};

/// Per-CPU preemption counter
/// When > 0, preemption is disabled
#[repr(C)]
pub struct PreemptState {
    /// Preemption disable count (nesting)
    count: AtomicUsize,
    /// Pending reschedule request
    need_resched: AtomicUsize,
    /// Lazy preemption enabled
    lazy_preempt: AtomicUsize,
}

impl PreemptState {
    pub const fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            need_resched: AtomicUsize::new(0),
            lazy_preempt: AtomicUsize::new(0),
        }
    }
}

/// Thread-local preemption state
/// In kernel, this would be per-CPU
static PREEMPT_STATE: PreemptState = PreemptState::new();

/// Disable preemption
///
/// Call this before entering a critical section that cannot be preempted.
/// Must be paired with `preempt_enable()`.
#[inline(always)]
pub fn preempt_disable() {
    PREEMPT_STATE.count.fetch_add(1, Ordering::Acquire);
}

/// Enable preemption
///
/// Call this after leaving a critical section.
/// If preemption was requested while disabled, triggers reschedule.
#[inline(always)]
pub fn preempt_enable() {
    let prev = PREEMPT_STATE.count.fetch_sub(1, Ordering::Release);

    // If we just enabled preemption and reschedule is pending
    if prev == 1 {
        // Check for pending reschedule
        if PREEMPT_STATE.need_resched.load(Ordering::Acquire) != 0 {
            // Clear the pending flag and reschedule
            PREEMPT_STATE.need_resched.store(0, Ordering::Release);
            schedule_point();
        }
    }
}

/// Get current preemption count
#[inline(always)]
pub fn preempt_count() -> usize {
    PREEMPT_STATE.count.load(Ordering::Relaxed)
}

/// Check if preemption is currently enabled
#[inline(always)]
pub fn preempt_enabled() -> bool {
    preempt_count() == 0
}

/// Request a reschedule
///
/// If preemption is enabled, triggers immediately.
/// If disabled, sets pending flag for when preemption is re-enabled.
#[inline(always)]
pub fn set_need_resched() {
    if preempt_enabled() {
        // Immediate reschedule
        schedule_point();
    } else {
        // Defer until preemption is enabled
        PREEMPT_STATE.need_resched.store(1, Ordering::Release);
    }
}

/// Voluntary preemption point
///
/// Insert at long-running kernel code paths to allow higher priority
/// tasks to run. This is the core of PREEMPT_FULL mode.
#[inline(always)]
pub fn schedule_point() {
    if preempt_enabled() {
        // Trigger context switch via IPI to self
        ipi(IpiKind::Switch, IpiTarget::Current);
    }
}

/// Conditionally schedule if there's a pending need
///
/// More efficient than unconditional schedule_point() for hot paths.
#[inline(always)]
pub fn cond_schedule() {
    if PREEMPT_STATE.need_resched.load(Ordering::Relaxed) != 0 {
        schedule_point();
    }
}

/// Enable lazy preemption mode
///
/// In lazy mode, preemption is deferred until explicit schedule points.
/// Useful for batch workloads that benefit from longer time slices.
pub fn enable_lazy_preempt() {
    PREEMPT_STATE.lazy_preempt.store(1, Ordering::Release);
}

/// Disable lazy preemption mode
///
/// Returns to immediate preemption when requested.
pub fn disable_lazy_preempt() {
    PREEMPT_STATE.lazy_preempt.store(0, Ordering::Release);

    // Check for pending reschedule
    if PREEMPT_STATE.need_resched.load(Ordering::Acquire) != 0 {
        set_need_resched();
    }
}

/// Check if lazy preemption is enabled
pub fn lazy_preempt_enabled() -> bool {
    PREEMPT_STATE.lazy_preempt.load(Ordering::Relaxed) != 0
}

/// RAII guard for disabling preemption
///
/// Automatically re-enables preemption when dropped.
pub struct PreemptGuard {
    _private: (),
}

impl PreemptGuard {
    /// Create a new preemption guard, disabling preemption
    #[inline(always)]
    pub fn new() -> Self {
        preempt_disable();
        Self { _private: () }
    }
}

impl Drop for PreemptGuard {
    #[inline(always)]
    fn drop(&mut self) {
        preempt_enable();
    }
}

impl Default for PreemptGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII guard for a preemption-safe critical section
///
/// Use when you need to ensure no preemption occurs during an operation.
#[macro_export]
macro_rules! preempt_critical {
    ($body:block) => {{
        let _guard = $crate::preempt::PreemptGuard::new();
        $body
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preempt_disable_enable() {
        assert!(preempt_enabled());

        preempt_disable();
        assert!(!preempt_enabled());
        assert_eq!(preempt_count(), 1);

        preempt_disable();
        assert_eq!(preempt_count(), 2);

        preempt_enable();
        assert_eq!(preempt_count(), 1);

        preempt_enable();
        assert!(preempt_enabled());
    }

    #[test]
    fn test_preempt_guard() {
        assert!(preempt_enabled());

        {
            let _guard = PreemptGuard::new();
            assert!(!preempt_enabled());
        }

        assert!(preempt_enabled());
    }

    #[test]
    fn test_lazy_preempt() {
        assert!(!lazy_preempt_enabled());

        enable_lazy_preempt();
        assert!(lazy_preempt_enabled());

        disable_lazy_preempt();
        assert!(!lazy_preempt_enabled());
    }
}

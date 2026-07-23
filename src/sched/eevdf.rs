#![forbid(unsafe_code)]

//! # EEVDF Lock-Free Priority Ring Engine
//!
//! Implements Earliest Eligible Virtual Deadline First (EEVDF) scheduling algorithms
//! with fixed-point $2^{10}$ deadline calculation, lock-free context ring,
//! zero-copy IPC vruntime boosting, and wait-free priority inheritance.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
pub use crate::sched::eevdf_ring::{ContextRing, SchedulerContext, SchedulerError};

/// Context identifier type.
pub type ContextId = u64;

/// Calculates the virtual deadline for a context using fixed-point scaling by $1024$ ($2^{10}$):
/// $$v\_deadline = t\_runtime + \frac{slice\_quantum \ll 10}{weight}$$
///
/// # Panics
/// Panics if `weight == 0`.
pub fn calculate_vdeadline(t_runtime: u64, slice_quantum: u64, weight: u32) -> u64 {
    assert!(weight > 0, "Weight cannot be zero");
    let scaled_quantum = (slice_quantum as u128) << 10;
    let deadline_delta = (scaled_quantum / (weight as u128)) as u64;
    t_runtime.saturating_add(deadline_delta)
}

/// Zero-Copy IPC Boost:
/// Decreases context runtime $T_i$ by `delta` atomically when an IPC message arrives,
/// shifting the context toward the front of the queue without reallocation.
pub fn boost_vruntime(context: &SchedulerContext, delta: u64) {
    let mut old_vruntime = context.vruntime.load(Ordering::Relaxed);
    loop {
        let new_vruntime = old_vruntime.saturating_sub(delta);
        match context.vruntime.compare_exchange_weak(
            old_vruntime,
            new_vruntime,
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            Ok(_) => {
                let weight = context.weight;
                let slice = context.slice_ns;
                let new_vdeadline = calculate_vdeadline(new_vruntime, slice, weight);
                context.vdeadline.store(new_vdeadline, Ordering::Release);
                break;
            }
            Err(actual) => old_vruntime = actual,
        }
    }
}

/// Wait-Free Priority Inheritance:
/// When a high-priority task $H$ is blocked on a mutex held by low-priority task $L$,
/// $L$'s weight $W_L$ is temporarily updated by adding $W_H$ via atomic `fetch_add`.
pub fn propagate_priority_weight(holder_weight: &AtomicU32, blocked_weight: u32) -> u32 {
    holder_weight.fetch_add(blocked_weight, Ordering::AcqRel)
}

#[cfg(test)]
mod eevdf_tests {
    use super::*;

    #[test]
    fn test_vdeadline_calculation() {
        // vruntime=1000, slice=100, weight=512
        // 1000 + (100 << 10) / 512 = 1000 + 102400 / 512 = 1000 + 200 = 1200
        let v_deadline = calculate_vdeadline(1000, 100, 512);
        assert_eq!(v_deadline, 1200);
    }

    #[test]
    fn test_boost_vruntime() {
        let ctx = SchedulerContext::new(1, 2000, 100, 512, 1);
        boost_vruntime(&ctx, 500);
        assert_eq!(ctx.vruntime.load(Ordering::Relaxed), 1500);
    }
}

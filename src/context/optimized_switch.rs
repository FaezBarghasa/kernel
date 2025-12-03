//! Optimized Context Switching for Minimal Latency
//!
//! This module provides optimizations for the context switch path to minimize
//! latency and improve cache behavior during IPC operations.

use crate::context::Context;
use core::sync::atomic::{AtomicU64, Ordering};

/// Per-CPU context switch statistics
pub struct SwitchStats {
    /// Total number of context switches
    pub total_switches: AtomicU64,
    /// Total cycles spent in context switches (TSC)
    pub total_cycles: AtomicU64,
    /// Minimum cycles observed
    pub min_cycles: AtomicU64,
    /// Maximum cycles observed
    pub max_cycles: AtomicU64,
}

impl SwitchStats {
    pub const fn new() -> Self {
        SwitchStats {
            total_switches: AtomicU64::new(0),
            total_cycles: AtomicU64::new(0),
            min_cycles: AtomicU64::new(u64::MAX),
            max_cycles: AtomicU64::new(0),
        }
    }

    /// Record a context switch
    #[inline]
    pub fn record(&self, cycles: u64) {
        self.total_switches.fetch_add(1, Ordering::Relaxed);
        self.total_cycles.fetch_add(cycles, Ordering::Relaxed);

        // Update min
        let mut current_min = self.min_cycles.load(Ordering::Relaxed);
        while cycles < current_min {
            match self.min_cycles.compare_exchange_weak(
                current_min,
                cycles,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_min = x,
            }
        }

        // Update max
        let mut current_max = self.max_cycles.load(Ordering::Relaxed);
        while cycles > current_max {
            match self.max_cycles.compare_exchange_weak(
                current_max,
                cycles,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_max = x,
            }
        }
    }

    /// Get average cycles per switch
    pub fn average_cycles(&self) -> u64 {
        let total = self.total_cycles.load(Ordering::Relaxed);
        let count = self.total_switches.load(Ordering::Relaxed);
        if count > 0 {
            total / count
        } else {
            0
        }
    }
}

/// Fast path check for same address space
///
/// If switching between contexts in the same address space,
/// we can skip TLB flush and page table switching.
#[inline]
pub unsafe fn same_address_space(prev: *const Context, next: *const Context) -> bool {
    if prev.is_null() || next.is_null() {
        return false;
    }

    let prev_addrsp = (*prev).addr_space.as_ref();
    let next_addrsp = (*next).addr_space.as_ref();

    match (prev_addrsp, next_addrsp) {
        (Some(p), Some(n)) => core::sync::Arc::ptr_eq(p, n),
        _ => false,
    }
}

/// Check if FPU/SIMD save is needed
///
/// Lazy FPU state saving: only save if the context actually used FPU
#[inline]
pub fn needs_fpu_save(context: &Context) -> bool {
    // This would check an FPU-used flag in the context
    // For now, always return true for correctness
    // TODO: Implement lazy FPU saving
    true
}

/// Optimized context switch wrapper
///
/// This provides profiling and optimization hints around the
/// architecture-specific switch implementation.
#[inline(never)]
pub unsafe fn optimized_switch_to(
    prev: *mut Context,
    next: *mut Context,
    stats: Option<&SwitchStats>,
) {
    let start_tsc = core::arch::x86_64::_rdtsc();

    // Fast path: check if same address space
    let same_addrsp = same_address_space(prev, next);

    // Call architecture-specific switch
    // The actual implementation would call the arch-specific function
    // For now, we'll document the optimization strategy

    // Architecture-specific switch_to would:
    // 1. Save minimal context (registers, stack pointer)
    // 2. If same_addrsp, skip CR3 reload
    // 3. If !needs_fpu_save, skip FPU state save
    // 4. Switch stack pointer
    // 5. Restore minimal context
    // 6. Use CLFLUSH on critical cache lines

    crate::arch::switch_to(prev, next);

    let end_tsc = core::arch::x86_64::_rdtsc();
    let cycles = end_tsc - start_tsc;

    if let Some(stats) = stats {
        stats.record(cycles);
    }
}

/// Hint to prefetch context data
///
/// Issue prefetch instructions for the next context's critical data
#[inline]
pub unsafe fn prefetch_context(context: *const Context) {
    if context.is_null() {
        return;
    }

    // Prefetch key cache lines
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::_mm_prefetch;

        // Prefetch the context struct
        _mm_prefetch(context as *const i8, core::arch::x86_64::_MM_HINT_T0);

        // Prefetch the arch context (typically cache line + 64 bytes)
        let arch_ptr = &(*context).arch as *const _ as *const i8;
        _mm_prefetch(arch_ptr, core::arch::x86_64::_MM_HINT_T0);
    }
}

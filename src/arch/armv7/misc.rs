//! ARMv7 miscellaneous utilities

use crate::cpu_set::LogicalCpuId;

/// Initialize CPU-specific state
pub fn init(_cpu_id: LogicalCpuId) {
    // CPU-specific initialization
}

/// Get current CPU ID
pub fn cpu_id() -> LogicalCpuId {
    // Read MPIDR (Multiprocessor Affinity Register)
    let mpidr: u32;
    unsafe {
        core::arch::asm!("mrc p15, 0, {}, c0, c0, 5", out(reg) mpidr);
    }
    LogicalCpuId::new((mpidr & 0xFF) as u32)
}

/// Memory barrier
#[inline(always)]
pub fn memory_barrier() {
    unsafe {
        core::arch::asm!("dmb", options(nostack, preserves_flags));
    }
}

/// Instruction synchronization barrier
#[inline(always)]
pub fn isb() {
    unsafe {
        core::arch::asm!("isb", options(nostack, preserves_flags));
    }
}

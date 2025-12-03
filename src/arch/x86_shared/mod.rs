//! This module contains code that is shared between the x86 and x86_64 architectures.

/// CPUID wrapper
pub mod cpuid;

/// Debugging support
pub mod debug;

/// Devices
pub mod device;

/// Global descriptor table
pub mod gdt;

/// Assembly macros
#[macro_use]
pub mod macros;

/// Interrupt descriptor table
pub mod idt;

/// Interrupt instructions
pub mod interrupt;

/// Inter-processor interrupts
pub mod ipi;

/// Paging
pub mod paging;

/// Page table isolation
pub mod pti;

pub mod rmm;

/// Initialization and start function
pub mod start;

/// Stop function
pub mod stop;

pub mod time;

pub mod switch;
pub use switch::switch_to;

#[cfg(target_arch = "x86")]
pub use ::rmm::X86Arch as CurrentRmmArch;

#[cfg(target_arch = "x86_64")]
pub use ::rmm::X8664Arch as CurrentRmmArch;

pub unsafe fn arch_copy_from_user(dst: *mut u8, src: *const u8, len: usize) -> usize {
    // Placeholder: just copy
    // In real kernel this needs to handle page faults
    core::ptr::copy_nonoverlapping(src, dst, len);
    0 // 0 means success/0 bytes remaining
}

pub unsafe fn arch_copy_to_user(dst: *mut u8, src: *const u8, len: usize) -> usize {
    core::ptr::copy_nonoverlapping(src, dst, len);
    0
}

use crate::percpu::PercpuBlock;

impl PercpuBlock {
    pub fn current() -> &'static mut Self {
        unsafe { &mut gdt::pcr().percpu }
    }
}

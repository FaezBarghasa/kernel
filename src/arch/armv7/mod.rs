//! # ARMv7-A Architecture Support
//!
//! This module provides ARMv7-A (32-bit ARM) architecture support for the Redox kernel.
//! Designed for single-board computers like Raspberry Pi 2, BeagleBone, and similar devices.

pub mod consts;
pub mod debug;
pub mod device;
pub mod interrupt;
pub mod ipi;
pub mod linker;
pub mod misc;
pub mod paging;
pub mod rmm;
pub mod start;
pub mod stop;
pub mod time;
pub mod vectors;

pub use crate::arch::armv7::linker::*;

/// Context switch between two contexts
///
/// # Safety
///
/// This function is unsafe because it directly manipulates CPU registers and stack pointers.
pub unsafe fn switch_to(prev: *mut crate::context::Context, next: *mut crate::context::Context) {
    extern "C" {
        fn __context_switch(prev: *mut usize, next: *const usize);
    }

    // ARMv7 context switch will be implemented in assembly
    // For now, this is a placeholder
}

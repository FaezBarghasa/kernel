//! x86_64 Architecture Module
//!
//! This module contains core architecture-specific initialization routines,
//! including security hardening features like CET.

// Existing module code continues...
pub mod alternative;
pub mod amd_3d_vcache;
pub mod topology;
pub mod consts;
pub mod ept;
pub mod flags;
pub mod interrupt;
pub mod macros;
pub mod misc;
pub mod orc_unwinder;
pub mod npt;
pub mod svm;
pub mod vmx;

pub use crate::arch::x86_shared::*;

// Placeholder for other initialization functions

// Define usercopy symbols required by src/memory/mod.rs
// Since we use a Rust implementation for copy_to/from_user, these are dummy labels.
#[cfg(not(test))]
core::arch::global_asm!(
    "
    .global __usercopy_start
    __usercopy_start:
    .global __usercopy_end
    __usercopy_end:
    "
);

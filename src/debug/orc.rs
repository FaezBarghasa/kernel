#![forbid(unsafe_code)]

//! # Fine-Grained KASLR & ORC Stack Unwinder
//!
//! Provides binary-search lookup over ELF `.orc_unwind` and `.orc_unwind_ip` sections
//! with strict stack boundary checks ($stack\_start \le sp \le stack\_end$).
//! Returns `KernelError::InvalidStackFrame` on violation without panicking.

use alloc::vec::Vec;
#[cfg(target_arch = "x86_64")]
pub use crate::arch::x86_64::orc_unwinder::{lookup_orc, unwind_stack, KernelError, OrcEntry};

#[cfg(not(target_arch = "x86_64"))]
#[derive(Debug, Clone, Copy)]
pub enum KernelError {
    InvalidStackFrame,
}

#[cfg(not(target_arch = "x86_64"))]
#[derive(Debug, Clone, Copy)]
pub struct OrcEntry {
    pub sp_offset: i16,
    pub fp_offset: i16,
    pub sp_reg: u8,
    pub fp_reg: u8,
    pub type_: u8,
}

#[cfg(not(target_arch = "x86_64"))]
pub fn lookup_orc(_ip: usize) -> Option<OrcEntry> {
    None
}

#[cfg(not(target_arch = "x86_64"))]
pub fn unwind_stack(
    _ip: usize,
    _sp: usize,
    _fp: usize,
    _stack_start: usize,
    _stack_end: usize,
) -> Result<Vec<usize>, KernelError> {
    Err(KernelError::InvalidStackFrame)
}

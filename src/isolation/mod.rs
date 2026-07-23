#![forbid(unsafe_code)]

//! # Tock-Style Memory Isolation & Syscall Validation
//!
//! Implements strict process isolation boundaries, MPU region allocation,
//! and type-safe syscall validation to ensure unprivileged processes cannot
//! corrupt kernel memory.

pub mod mpu;

use mpu::MpuRegion;

/// Type-safe Syscall Argument Sanitizer.
pub struct SyscallSanitizer;

impl SyscallSanitizer {
    /// Validates that a user-provided pointer buffer lies entirely within
    /// the process's allocated MPU region before dereferencing.
    pub fn validate_buffer(
        user_ptr: usize,
        len: usize,
        region: &MpuRegion,
    ) -> Result<(), &'static str> {
        if region.contains(user_ptr, len) {
            Ok(())
        } else {
            Err("Syscall Buffer Isolation Violation")
        }
    }
}

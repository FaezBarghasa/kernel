//! # Syscall Handlers
//!
//! This module contains the main entry point for handling system calls in the kernel.
//! It dispatches syscalls to the appropriate handlers based on the syscall number.
//!
//! ## Submodules
//!
//! - `debug`: Debugging syscalls (e.g., `sys_log`).
//! - `fs`: File system syscalls (e.g., `sys_open`, `sys_read`).
//! - `futex`: Fast userspace mutex syscalls.
//! - `privilege`: Privilege management syscalls (e.g., `sys_setuid`).
//! - `process`: Process management syscalls (e.g., `sys_fork`, `sys_exit`).
//! - `time`: Time-related syscalls (e.g., `sys_clock_gettime`).
//! - `usercopy`: Utilities for copying data between user and kernel space.
//! - `memory`: Memory management syscalls (e.g., `sys_mlockall`).

pub use self::debug::*;
pub use crate::stubs::syscall_helpers::*;
pub use ::syscall::{data, error, flag, io, number};
pub use ::syscall::{EnvRegisters, FloatRegisters, IntRegisters};
pub use ::syscall::flag::EventFlags;

pub mod debug;
pub mod fs;
pub mod futex;
pub mod privilege;
pub mod process;
pub mod time;
pub mod usercopy;
pub mod memory;

use crate::{
    sync::CleanLockToken,
    syscall::error::{Error, ENOSYS},
};

/// The main syscall entry point.
///
/// This function is called by the architecture-specific syscall entry code (e.g., in `arch/x86_64/syscall.rs`).
/// It dispatches the syscall to the appropriate handler based on `number`.
///
/// # Arguments
///
/// * `number` - The system call number (e.g., `SYS_READ`, `SYS_WRITE`).
/// * `a` - The first argument to the syscall.
/// * `b` - The second argument to the syscall.
/// * `_c` - The third argument to the syscall (unused in this stub).
/// * `_d` - The fourth argument to the syscall (unused in this stub).
/// * `_e` - The fifth argument to the syscall (unused in this stub).
/// * `_f` - The sixth argument to the syscall (unused in this stub).
///
/// # Returns
///
/// Returns the result of the system call, which is typically 0 on success or a negative error code on failure.
/// The `Error::mux` function converts the `Result` into this `usize` representation.
pub fn syscall(
    number: usize,
    a: usize,
    b: usize,
    _c: usize,
    _d: usize,
    _e: usize,
    _f: usize,
) -> usize {
    let mut token = unsafe { CleanLockToken::new() };

    let res = match number {
        number::SYS_MLOCKALL => memory::sys_mlockall(a),
        number::SYS_MUNLOCKALL => memory::sys_munlockall(),
        _ => match number {
            // Forward to other handlers if needed, or default
            _ => Err(Error::new(ENOSYS)),
        }
    };
    Error::mux(res)
}

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
//! - `personality`: Foreign ABI detection and syscall redirection.

pub use self::debug::*;
pub use crate::stubs::syscall_helpers::*;
pub use ::syscall::{
    data, error, flag, flag::EventFlags, io, number, EnvRegisters, FloatRegisters, IntRegisters,
};

pub mod debug;
pub mod fs;
pub mod futex;
pub mod memory;
pub mod personality;
pub mod privilege;
pub mod process;
pub mod time;
pub mod usercopy;

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
/// * `c` - The third argument to the syscall.
/// * `d` - The fourth argument to the syscall.
/// * `e` - The fifth argument to the syscall.
/// * `f` - The sixth argument to the syscall.
///
/// # Returns
///
/// Returns the result of the system call, which is typically 0 on success or a negative error code on failure.
/// The `Error::mux` function converts the `Result` into this `usize` representation.
pub fn syscall(number: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> usize {
    let mut token = unsafe { CleanLockToken::new() };

    // Check for foreign ABI syscalls
    let abi = personality::detect_abi(&token);
    if personality::is_foreign_syscall(abi, number) {
        let args = personality::SyscallArgs::new(number, a, b, c, d, e, f);
        let res = personality::redirect_foreign_syscall(abi, args, &mut token);
        return Error::mux(res);
    }

    // Native Redox syscall dispatch
    let res = match number {
        number::SYS_FUTEX => futex::futex(a, b, c, d, e, &mut token),
        // Linux uses 449 for futex_waitv on x86_64, but Redox might use a different number.
        // We will assume 449 for now or a new constant if defined.
        449 => futex::futex_waitv(a, b, c, d, e, &mut token),

        // TODO: Uncomment when SYS_MLOCKALL and SYS_MUNLOCKALL are added to redox_syscall crate
        // number::SYS_MLOCKALL => memory::sys_mlockall(a),
        // number::SYS_MUNLOCKALL => memory::sys_munlockall(),
        _ => {
            // Forward to other handlers if needed, or default
            Err(Error::new(ENOSYS))
        }
    };
    Error::mux(res)
}

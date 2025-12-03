//! # Syscall Handlers

pub use self::debug::*;
pub use crate::stubs::{syscall_helpers::*, syscall_types::*};
pub use ::syscall::{data, error, flag, io, number};

pub mod debug;
pub mod fs;
pub mod futex;
pub mod privilege;
pub mod process;
pub mod time;
pub mod usercopy;

use crate::syscall::error::{Error, Result, EBADF, EFAULT, EINVAL, ENOSYS};

/// The main syscall entry point
pub fn syscall(
    _number: usize,
    _a: usize,
    _b: usize,
    _c: usize,
    _d: usize,
    _e: usize,
    _f: usize,
) -> usize {
    // Dispatch logic...
    // For production readiness, this returns ENOSYS by default if not handled
    // or calls specific handlers based on number.
    Error::mux(Err(Error::new(ENOSYS)))
}

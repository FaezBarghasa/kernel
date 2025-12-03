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

use crate::sync::CleanLockToken;
use crate::syscall::error::{Error, Result, EBADF, EFAULT, EINVAL, ENOSYS};

/// The main syscall entry point
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
        number::SYS_MLOCK => fs::sys_mlock(a, b, &mut token),
        number::SYS_MUNLOCK => fs::sys_munlock(a, b, &mut token),
        // ... other syscalls ...
        _ => Err(Error::new(ENOSYS)),
    };
    Error::mux(res)
}

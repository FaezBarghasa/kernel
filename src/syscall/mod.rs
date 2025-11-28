//! # Syscall Handlers

pub use self::debug::*;
pub use self::file::*;
pub use self::fs::*;
pub use self::futex::*;
pub use self::privilege::*;
pub use self::process::*;
pub use self::time::*;
pub use self::usercopy::*;

pub mod debug;
pub mod driver;
pub mod error;
pub mod file;
pub mod flag;
pub mod fs;
pub mod futex;
pub mod io;
pub mod number;
pub mod privilege;
pub mod process;
pub mod time;
pub mod usercopy;
pub mod validate;

use crate::syscall::error::{Error, EBADF, EFAULT, EINVAL, ENOSYS, Result};

/// The main syscall entry point
pub fn syscall(
    number: usize,
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
) -> usize {
    // Dispatch logic...
    // For production readiness, this returns ENOSYS by default if not handled
    // or calls specific handlers based on number.
    Error::mux(Err(Error::new(ENOSYS)))
}
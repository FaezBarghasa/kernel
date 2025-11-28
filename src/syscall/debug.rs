//! # Syscall Debugging

use crate::syscall::error::Result;

pub fn debug_syscall(number: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) {
    #[cfg(feature = "syscall_debug")]
    {
        crate::println!(
            "SYSCALL: #{}({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x})",
            number, a, b, c, d, e, f
        );
    }
}

pub fn debug_syscall_result(result: Result<usize>) {
    #[cfg(feature = "syscall_debug")]
    {
        match result {
            Ok(val) => crate::println!(" -> Ok({:#x})", val),
            Err(err) => crate::println!(" -> Err({:?})", err),
        }
    }
}
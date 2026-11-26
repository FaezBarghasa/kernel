//! Syscall Debug Tracer
//!
//! Provides formatting and symbol resolution for active system calls.

use alloc::string::String;
use ::syscall::number::*;

#[derive(Debug, Default)]
pub struct SyscallDebugInfo;

impl SyscallDebugInfo {
    pub fn format_call(&self, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> String {
        format_call(a, b, c, d, e, f)
    }
}

pub fn format_call(a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> String {
    let name = match a {
        SYS_OPEN => "sys_open",
        SYS_OPENAT => "sys_openat",
        SYS_CLOSE => "sys_close",
        SYS_READ => "sys_read",
        SYS_READ2 => "sys_read2",
        SYS_WRITE => "sys_write",
        SYS_WRITE2 => "sys_write2",
        SYS_LSEEK => "sys_lseek",
        SYS_FPATH => "sys_fpath",
        SYS_FSTAT => "sys_fstat",
        SYS_FSTATVFS => "sys_fstatvfs",
        SYS_FSYNC => "sys_fsync",
        SYS_FTRUNCATE => "sys_ftruncate",
        SYS_FUTIMENS => "sys_futimens",
        SYS_DUP => "sys_dup",
        SYS_DUP2 => "sys_dup2",
        SYS_FCNTL => "sys_fcntl",
        SYS_FEVENT => "sys_fevent",
        SYS_CALL => "sys_call",
        SYS_SENDFD => "sys_sendfd",
        SYS_GETDENTS => "sys_getdents",
        SYS_FMAP => "sys_fmap",
        SYS_FUNMAP => "sys_funmap",
        SYS_MREMAP => "sys_mremap",
        SYS_FLINK => "sys_flink",
        SYS_FRENAME => "sys_frename",
        SYS_FCHMOD => "sys_fchmod",
        SYS_FCHOWN => "sys_fchown",
        SYS_RMDIR => "sys_rmdir",
        SYS_UNLINK => "sys_unlink",
        SYS_CLOCK_GETTIME => "sys_clock_gettime",
        SYS_NANOSLEEP => "sys_nanosleep",
        SYS_FUTEX => "sys_futex",
        SYS_YIELD => "sys_yield",
        SYS_MPROTECT => "sys_mprotect",
        SYS_MKNS => "sys_mkns",
        _ => return alloc::format!("syscall_{}({:#x}, {:#x}, {:#x}, {:#x}, {:#x})", a, b, c, d, e, f),
    };

    alloc::format!("{}({:#x}, {:#x}, {:#x}, {:#x}, {:#x})", name, b, c, d, e, f)
}

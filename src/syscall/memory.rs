//! # Memory-related system calls
//!
//! This module contains system calls for managing memory, such as `mlockall` and `munlockall`.

use crate::{
    memory::{mlockall as mlockall_impl, munlockall as munlockall_impl, MlockFlags},
    syscall::error::{Error, Result},
};

/// The `mlockall` system call.
///
/// This function locks all of the calling process's virtual address space into RAM,
/// preventing that memory from being paged to the swap area.
pub fn sys_mlockall(flags: usize) -> Result<usize> {
    let flags = MlockFlags::from_bits(flags as u32).ok_or(Error::new(syscall::EINVAL))?;
    mlockall_impl(flags).map(|_| 0)
}

/// The `munlockall` system call.
///
/// This function unlocks all of the calling process's virtual address space,
/// allowing that memory to be paged to the swap area.
pub fn sys_munlockall() -> Result<usize> {
    munlockall_impl().map(|_| 0)
}

/// Create a new DMA buffer of the specified size.
pub fn sys_dmabuf_create(size: usize) -> Result<usize> {
    crate::memory::dmabuf::sys_dmabuf_create(size)
}

/// Map the specified DMA buffer into the current process's address space.
pub fn sys_dmabuf_map(fd: usize, vaddr: usize, flags: usize) -> Result<usize> {
    let map_flags = crate::syscall::flag::MapFlags::from_bits_retain(flags);
    crate::memory::dmabuf::sys_dmabuf_map(fd, vaddr, map_flags).map(|_| 0)
}

/// Unmap the specified DMA buffer from the current process's address space.
pub fn sys_dmabuf_unmap(fd: usize, vaddr: usize) -> Result<usize> {
    crate::memory::dmabuf::sys_dmabuf_unmap(fd, vaddr).map(|_| 0)
}

/// Release the handle to the specified DMA buffer.
pub fn sys_dmabuf_release(fd: usize) -> Result<usize> {
    crate::memory::dmabuf::sys_dmabuf_release(fd).map(|_| 0)
}

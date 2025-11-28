//! # RISC-V 32-bit Constants

pub const PAGE_SIZE: usize = 4096;
pub const PAGE_MASK: usize = PAGE_SIZE - 1;

pub const KERNEL_OFFSET: usize = 0x8000_0000;
pub const PHYS_OFFSET: usize = 0x0;

pub const KERNEL_HEAP_OFFSET: usize = 0xC000_0000;
pub const KERNEL_HEAP_SIZE: usize = 64 * 1024 * 1024;
#![allow(unused)]

//! ARMv7-A architecture constants
//!
//! Memory layout for ARMv7 (32-bit address space):
//! - User space: 0x0000_0000 - 0xBFFF_FFFF (3 GB)
//! - Kernel space: 0xC000_0000 - 0xFFFF_FFFF (1 GB)

/// Page size (4 KB)
pub const PAGE_SIZE: usize = 4096;
pub const PAGE_SHIFT: u8 = 12;
pub const PAGE_MASK: usize = PAGE_SIZE - 1;

/// Section size for ARMv7 (1 MB sections)
pub const SECTION_SIZE: usize = 0x0010_0000;
pub const SECTION_MASK: usize = SECTION_SIZE - 1;

/// Kernel virtual address offset (starts at 3 GB)
pub const KERNEL_OFFSET: usize = 0xC000_0000;

/// Physical memory offset for direct mapping
/// Maps physical memory starting at PHYS_OFFSET in virtual address space
pub const PHYS_OFFSET: usize = 0xC000_0000;

/// Kernel heap offset and size
pub const KERNEL_HEAP_OFFSET: usize = 0xD000_0000;
pub const KERNEL_HEAP_SIZE: usize = 16 * 1024 * 1024; // 16 MB

/// Kernel percpu variables
pub const KERNEL_PERCPU_OFFSET: usize = 0xD100_0000;
pub const KERNEL_PERCPU_SHIFT: u8 = 16; // 64 KiB per CPU
pub const KERNEL_PERCPU_SIZE: usize = 1 << KERNEL_PERCPU_SHIFT;

/// Temporary mapping area for kernel operations
pub const KERNEL_TMP_MISC_OFFSET: usize = 0xD200_0000;

/// User space end (3 GB boundary)
pub const USER_END_OFFSET: usize = 0xC000_0000;

/// Stack size for kernel threads
pub const KERNEL_STACK_SIZE: usize = 16 * 1024; // 16 KB

/// Interrupt stack size
pub const IRQ_STACK_SIZE: usize = 8 * 1024; // 8 KB

/// Number of exception vectors
pub const EXCEPTION_VECTOR_COUNT: usize = 8;

/// Exception vector base address (typically 0x0000_0000 or 0xFFFF_0000)
pub const EXCEPTION_VECTOR_BASE: usize = 0x0000_0000;

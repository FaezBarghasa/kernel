//! ARMv7 linker script integration

/// Kernel start address
#[no_mangle]
pub static __kernel_start: usize = 0;

/// Kernel end address
#[no_mangle]
pub static __kernel_end: usize = 0;

/// Text section start
#[no_mangle]
pub static __text_start: usize = 0;

/// Text section end
#[no_mangle]
pub static __text_end: usize = 0;

/// Rodata section start
#[no_mangle]
pub static __rodata_start: usize = 0;

/// Rodata section end
#[no_mangle]
pub static __rodata_end: usize = 0;

/// Data section start
#[no_mangle]
pub static __data_start: usize = 0;

/// Data section end
#[no_mangle]
pub static __data_end: usize = 0;

/// BSS section start
#[no_mangle]
pub static __bss_start: usize = 0;

/// BSS section end
#[no_mangle]
pub static __bss_end: usize = 0;

//! # RISC-V 32-bit Interrupts

pub mod syscall;

use core::arch::asm;

#[no_mangle]
#[repr(align(4))]
pub unsafe extern "C" fn trap_handler() {
    // context switch logic
}

pub unsafe fn init() {
    // init logic
}
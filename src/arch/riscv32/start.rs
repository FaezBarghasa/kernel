//! # RISC-V 32-bit Entry Point

use crate::context;
use crate::sync::CleanLockToken;

#[no_mangle]
#[naked]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::asm!(
    "
        la sp, stack_top
        j kmain_entry
        ",
    options(noreturn)
    );
}

#[no_mangle]
extern "C" fn kmain_entry(hart_id: usize, dtb: usize) -> ! {
    kmain(hart_id, dtb);
}

pub fn kmain(hart_id: usize, _dtb: usize) -> ! {
    let mut token = unsafe { CleanLockToken::new() };

    // logic would go here

    loop {
        unsafe { crate::arch::riscv32::halt(); }
    }
}

#[repr(align(16))]
struct Stack([u8; 16384]);

#[no_mangle]
static mut stack_top: Stack = Stack([0; 16384]);
//! # RISC-V 32-bit Architecture Support

pub mod consts;
pub mod context;
pub mod interrupt;
pub mod paging;
pub mod start;

pub type Arch = crate::arch::riscv64::Arch;

pub unsafe fn halt() {
    core::arch::asm!("wfi");
}

pub unsafe fn disable_interrupts() {
    core::arch::asm!("csrci sstatus, 2");
}

pub unsafe fn enable_interrupts() {
    core::arch::asm!("csrsi sstatus, 2");
}
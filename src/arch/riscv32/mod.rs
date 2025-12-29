//! # RISC-V 32-bit Architecture Support

pub mod consts;
pub mod context;
pub mod debug;
pub mod device;
pub mod interrupt;
pub mod ipi;
pub mod misc;
pub mod paging;
pub mod rmm;
pub mod start;
pub mod stop;
pub mod time;

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

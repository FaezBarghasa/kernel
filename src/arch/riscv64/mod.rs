//! # RISC-V 64-bit Architecture

pub mod consts;
pub mod debug;
pub mod device;
pub mod interrupt;
pub mod paging;
pub mod start;
pub mod stop;
pub mod time;

pub fn console_putchar(c: u8) {
    #[allow(deprecated)]
    sbi_rt::legacy::console_putchar(c as usize);
}

pub fn console_getchar() -> isize {
    #[allow(deprecated)]
    sbi_rt::legacy::console_getchar() as isize
}

pub fn set_timer(time: u64) {
    sbi_rt::set_timer(time);
}
//! # AArch64 Architecture Support

pub mod debug;
pub mod device;
pub mod interrupt;
pub mod paging;
pub mod start;
pub mod stop;
pub mod time;
pub mod vectors;
pub mod linker;
pub mod misc;

pub use crate::arch::aarch64::linker::*;

pub unsafe fn switch_to(prev: *mut crate::context::Context, next: *mut crate::context::Context) {
    extern "C" {
        fn __context_switch(prev: *mut usize, next: *const usize);
    }
}
//! # RISC-V 32-bit Mapper

use crate::memory::{Frame, PhysicalAddress, PAGE_SIZE};
use crate::paging::{Page, PageFlags, VirtualAddress};

pub struct Mapper {}

impl Mapper {
    pub unsafe fn current() -> Self {
        Self { }
    }

    pub unsafe fn map(&mut self, page: Page, frame: Frame, flags: PageFlags) {
        // Sv32 logic
    }

    pub unsafe fn unmap(&mut self, page: Page) -> Option<Frame> {
        None
    }
}
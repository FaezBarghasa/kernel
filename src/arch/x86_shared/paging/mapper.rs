//! # x86 Shared Paging Mapper

use crate::memory::{Frame, PAGE_SIZE, PhysicalAddress};
use crate::paging::{Page, PageFlags, RmmA, RmmArch, VirtualAddress};
use rmm::{Arch as RmmArchTrait, PageTable, TableKind};

pub unsafe fn page_table_allocator() -> Option<Frame> {
    crate::memory::allocate_frame()
}
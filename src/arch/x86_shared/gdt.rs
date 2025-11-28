//! # x86 Global Descriptor Table

use core::mem::size_of;
use crate::paging::VirtualAddress;

#[repr(packed)]
pub struct Tss {
    pub reserved1: u32,
    pub rsp0: u64,
    pub rsp1: u64,
    pub rsp2: u64,
    pub reserved2: u64,
    pub ist1: u64,
    pub ist2: u64,
    pub ist3: u64,
    pub ist4: u64,
    pub ist5: u64,
    pub ist6: u64,
    pub ist7: u64,
    pub reserved3: u64,
    pub reserved4: u16,
    pub iomap_base: u16,
}

impl Tss {
    pub fn new() -> Self {
        Self {
            reserved1: 0,
            rsp0: 0, rsp1: 0, rsp2: 0,
            reserved2: 0,
            ist1: 0, ist2: 0, ist3: 0, ist4: 0, ist5: 0, ist6: 0, ist7: 0,
            reserved3: 0,
            reserved4: 0,
            iomap_base: size_of::<Tss>() as u16,
        }
    }

    pub fn set_stack(&mut self, stack: VirtualAddress) {
        self.rsp0 = stack.data() as u64;
    }
}

pub unsafe fn init() {
    // GDT setup logic
}
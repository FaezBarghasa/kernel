//! ARMv7 page table entry definitions

use bitflags::bitflags;

bitflags! {
    /// Page table entry flags for ARMv7 short descriptor format
    pub struct PageFlags: u32 {
        /// Present bit
        const PRESENT = 1 << 0;
        /// Writable
        const WRITABLE = 1 << 1;
        /// User accessible
        const USER = 1 << 2;
        /// No execute
        const NO_EXECUTE = 1 << 4;
    }
}

/// Page table entry
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct PageEntry(u32);

impl PageEntry {
    /// Create a new empty entry
    pub const fn new() -> Self {
        Self(0)
    }

    /// Check if entry is present
    pub fn present(&self) -> bool {
        self.0 & PageFlags::PRESENT.bits() != 0
    }

    /// Get physical address
    pub fn address(&self) -> usize {
        (self.0 & !0xFFF) as usize
    }

    /// Set entry
    pub fn set(&mut self, addr: usize, flags: PageFlags) {
        self.0 = (addr as u32 & !0xFFF) | flags.bits();
    }
}

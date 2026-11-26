//! ARMv7 page table mapper
//!
//! Implements ARMv7 short-descriptor format (LPAE not used)
//! - L1 table: 4096 entries (16KB), each covering 1MB
//! - L2 table: 256 entries (1KB), each covering 4KB

use super::entry::{PageTableEntry, PageTableFlags};
use crate::memory::{Frame, PhysicalAddress, PAGE_SIZE};
use core::ptr;

/// ARMv7 uses 1MB sections in L1 table
const SECTION_SIZE: usize = 1024 * 1024;
const L1_ENTRIES: usize = 4096;
const L2_ENTRIES: usize = 256;

/// Page table mapper for ARMv7
pub struct Mapper {
    /// Physical address of L1 page table (TTBR0)
    l1_table_phys: PhysicalAddress,
    /// Virtual address of L1 page table
    l1_table_virt: *mut PageTableEntry,
}

impl Mapper {
    /// Create a new mapper with given L1 table address
    pub unsafe fn new(l1_table_phys: PhysicalAddress) -> Self {
        // For now, assume identity mapping for page tables
        let l1_table_virt = l1_table_phys.data() as *mut PageTableEntry;

        Self {
            l1_table_phys,
            l1_table_virt,
        }
    }

    /// Get L1 table physical address
    pub fn l1_table_phys(&self) -> PhysicalAddress {
        self.l1_table_phys
    }

    /// Map a 4KB page
    pub unsafe fn map_page(
        &mut self,
        virt: usize,
        phys: PhysicalAddress,
        flags: PageTableFlags,
    ) -> Result<(), &'static str> {
        let l1_index = virt >> 20; // Top 12 bits
        let l2_index = (virt >> 12) & 0xFF; // Next 8 bits

        if l1_index >= L1_ENTRIES {
            return Err("Virtual address out of range");
        }

        // Get or create L2 table
        let l1_entry = &mut *self.l1_table_virt.add(l1_index);

        let l2_table_virt = if l1_entry.is_present() {
            // L2 table already exists
            let l2_phys = l1_entry.address();
            l2_phys.data() as *mut PageTableEntry
        } else {
            // Allocate new L2 table
            let l2_frame = crate::memory::allocate_frame().ok_or("Failed to allocate L2 table")?;
            let l2_phys = l2_frame.base();
            let l2_virt = l2_phys.data() as *mut PageTableEntry;

            // Zero the L2 table
            ptr::write_bytes(l2_virt, 0, L2_ENTRIES);

            // Set L1 entry to point to L2 table
            *l1_entry = PageTableEntry::new_page_table(l2_phys);

            l2_virt
        };

        // Set L2 entry
        let l2_entry = &mut *l2_table_virt.add(l2_index);
        *l2_entry = PageTableEntry::new_page(phys, flags);

        // Invalidate TLB for this address
        Self::invalidate_tlb_entry(virt);

        Ok(())
    }

    /// Map a 1MB section (faster than 4KB pages)
    pub unsafe fn map_section(
        &mut self,
        virt: usize,
        phys: PhysicalAddress,
        flags: PageTableFlags,
    ) -> Result<(), &'static str> {
        let l1_index = virt >> 20;

        if l1_index >= L1_ENTRIES {
            return Err("Virtual address out of range");
        }

        if virt & (SECTION_SIZE - 1) != 0 {
            return Err("Virtual address not section-aligned");
        }

        if phys.data() & (SECTION_SIZE - 1) != 0 {
            return Err("Physical address not section-aligned");
        }

        let l1_entry = &mut *self.l1_table_virt.add(l1_index);
        *l1_entry = PageTableEntry::new_section(phys, flags);

        Self::invalidate_tlb_entry(virt);

        Ok(())
    }

    /// Unmap a page
    pub unsafe fn unmap_page(&mut self, virt: usize) -> Result<(), &'static str> {
        let l1_index = virt >> 20;
        let l2_index = (virt >> 12) & 0xFF;

        if l1_index >= L1_ENTRIES {
            return Err("Virtual address out of range");
        }

        let l1_entry = &mut *self.l1_table_virt.add(l1_index);

        if !l1_entry.is_present() {
            return Err("Page not mapped");
        }

        let l2_table_virt = l1_entry.address().data() as *mut PageTableEntry;
        let l2_entry = &mut *l2_table_virt.add(l2_index);

        if !l2_entry.is_present() {
            return Err("Page not mapped");
        }

        // Clear L2 entry
        *l2_entry = PageTableEntry::empty();

        Self::invalidate_tlb_entry(virt);

        Ok(())
    }

    /// Translate virtual address to physical
    pub fn translate(&self, virt: usize) -> Option<PhysicalAddress> {
        let l1_index = virt >> 20;
        let l2_index = (virt >> 12) & 0xFF;
        let offset = virt & 0xFFF;

        if l1_index >= L1_ENTRIES {
            return None;
        }

        unsafe {
            let l1_entry = &*self.l1_table_virt.add(l1_index);

            if !l1_entry.is_present() {
                return None;
            }

            if l1_entry.is_section() {
                // 1MB section
                let section_base = l1_entry.address().data() & !0xFFFFF;
                let section_offset = virt & 0xFFFFF;
                return Some(PhysicalAddress::new(section_base + section_offset));
            }

            // Page table entry
            let l2_table_virt = l1_entry.address().data() as *const PageTableEntry;
            let l2_entry = &*l2_table_virt.add(l2_index);

            if !l2_entry.is_present() {
                return None;
            }

            let page_base = l2_entry.address().data();
            Some(PhysicalAddress::new(page_base + offset))
        }
    }

    /// Invalidate TLB entry for given virtual address
    #[inline]
    unsafe fn invalidate_tlb_entry(virt: usize) {
        core::arch::asm!(
            "mcr p15, 0, {0}, c8, c7, 1", // TLBIMVA - invalidate by MVA
            in(reg) virt,
            options(nostack, preserves_flags)
        );
    }

    /// Invalidate entire TLB
    #[inline]
    pub unsafe fn invalidate_tlb_all() {
        core::arch::asm!(
            "mcr p15, 0, {0}, c8, c7, 0", // TLBIALL - invalidate all
            in(reg) 0,
            options(nostack, preserves_flags)
        );
    }

    /// Enable MMU
    pub unsafe fn enable_mmu(&self) {
        // Set TTBR0 (Translation Table Base Register 0)
        core::arch::asm!(
            "mcr p15, 0, {0}, c2, c0, 0",
            in(reg) self.l1_table_phys.data(),
            options(nostack, preserves_flags)
        );

        // Set TTBCR (Translation Table Base Control Register)
        // Use TTBR0 for all addresses
        core::arch::asm!(
            "mcr p15, 0, {0}, c2, c0, 2",
            in(reg) 0,
            options(nostack, preserves_flags)
        );

        // Set Domain Access Control Register
        // Domain 0: Client (check permissions)
        core::arch::asm!(
            "mcr p15, 0, {0}, c3, c0, 0",
            in(reg) 0x01,
            options(nostack, preserves_flags)
        );

        // Enable MMU in SCTLR (System Control Register)
        let mut sctlr: u32;
        core::arch::asm!(
            "mrc p15, 0, {0}, c1, c0, 0",
            out(reg) sctlr,
            options(nostack, preserves_flags)
        );

        sctlr |= 1 << 0; // M bit: Enable MMU
        sctlr |= 1 << 2; // C bit: Enable data cache
        sctlr |= 1 << 12; // I bit: Enable instruction cache

        core::arch::asm!(
            "mcr p15, 0, {0}, c1, c0, 0",
            in(reg) sctlr,
            options(nostack, preserves_flags)
        );

        // Instruction synchronization barrier
        core::arch::asm!("isb", options(nostack, preserves_flags));
    }

    /// Disable MMU
    pub unsafe fn disable_mmu() {
        let mut sctlr: u32;
        core::arch::asm!(
            "mrc p15, 0, {0}, c1, c0, 0",
            out(reg) sctlr,
            options(nostack, preserves_flags)
        );

        sctlr &= !(1 << 0); // Clear M bit: Disable MMU

        core::arch::asm!(
            "mcr p15, 0, {0}, c1, c0, 0",
            in(reg) sctlr,
            options(nostack, preserves_flags)
        );

        core::arch::asm!("isb", options(nostack, preserves_flags));
    }
}

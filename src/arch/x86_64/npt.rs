//! Nested Page Tables (NPT) — AMD-V Nested Paging
//!
//! Mirrors the EPT 4-level structure for AMD's nested paging. Entry format is
//! identical to standard x86_64 page table entries (not EPT format).

use alloc::{boxed::Box, vec::Vec};
use core::sync::atomic::{fence, Ordering};

/// Standard page table entry permission bits (AMD NPT uses normal PTE format).
pub const NPT_PRESENT: u64 = 1 << 0;
pub const NPT_WRITE: u64 = 1 << 1;
pub const NPT_USER: u64 = 1 << 2;
pub const NPT_RWX: u64 = NPT_PRESENT | NPT_WRITE | NPT_USER;

/// Physical address mask for a 4KB-aligned address.
const NPT_PHYS_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// A single 4KB NPT page (512 × 8-byte entries).
#[repr(C, align(4096))]
pub struct NptPage {
    entries: [u64; 512],
}

impl NptPage {
    pub fn new() -> Box<Self> {
        Box::new(Self {
            entries: [0u64; 512],
        })
    }

    #[inline]
    fn entry(&self, idx: usize) -> u64 {
        self.entries[idx]
    }

    #[inline]
    fn set_entry(&mut self, idx: usize, val: u64) {
        self.entries[idx] = val;
    }

    pub fn phys_addr(&self) -> u64 {
        use crate::arch::x86_64::consts::PHYS_OFFSET;
        let virt = self as *const _ as u64;
        virt - PHYS_OFFSET as u64
    }
}

/// 4-level NPT root, owning all allocated sub-tables.
pub struct NptRoot {
    pml4: Box<NptPage>,
    _tables: Vec<Box<NptPage>>,
    /// ASID assigned to this VM (used for INVLPGA).
    asid: u32,
}

impl NptRoot {
    pub fn new(asid: u32) -> Self {
        Self {
            pml4: NptPage::new(),
            _tables: Vec::new(),
            asid,
        }
    }

    /// Physical address of the PML4 — written to VMCB `N_CR3` field.
    pub fn ncr3(&self) -> u64 {
        self.pml4.phys_addr()
    }

    /// Map `[guest_phys, guest_phys+size)` → `[host_phys, host_phys+size)`.
    pub fn map_range(&mut self, guest_phys: u64, host_phys: u64, size: u64, perms: u64) {
        assert!(size % 4096 == 0);
        assert!(guest_phys % 4096 == 0);
        assert!(host_phys % 4096 == 0);

        let mut gpa = guest_phys;
        let mut hpa = host_phys;
        let end = guest_phys + size;

        while gpa < end {
            self.map_page(gpa, hpa, perms);
            gpa += 4096;
            hpa += 4096;
        }

        fence(Ordering::SeqCst);
    }

    /// Unmap `[guest_phys, guest_phys+size)` and flush TLB.
    pub fn unmap_range(&mut self, guest_phys: u64, size: u64) {
        assert!(size % 4096 == 0);
        let mut gpa = guest_phys;
        let end = guest_phys + size;
        while gpa < end {
            self.unmap_page(gpa);
            // INVLPGA: invalidate TLB entry for this guest virtual address and ASID.
            unsafe {
                invlpga(gpa, self.asid);
            }
            gpa += 4096;
        }
    }

    fn map_page(&mut self, gpa: u64, hpa: u64, perms: u64) {
        let pml4_idx = ((gpa >> 39) & 0x1FF) as usize;
        let pdpt_idx = ((gpa >> 30) & 0x1FF) as usize;
        let pd_idx = ((gpa >> 21) & 0x1FF) as usize;
        let pt_idx = ((gpa >> 12) & 0x1FF) as usize;

        let pdpt =
            Self::get_or_create_child(&mut self._tables, self.pml4.as_mut() as *mut _, pml4_idx);
        let pd = Self::get_or_create_child(&mut self._tables, pdpt, pdpt_idx);
        let pt = Self::get_or_create_child(&mut self._tables, pd, pd_idx);

        let pt_ref = unsafe { &mut *pt };
        let leaf = (hpa & NPT_PHYS_MASK) | perms;
        pt_ref.set_entry(pt_idx, leaf);
    }

    fn unmap_page(&mut self, gpa: u64) {
        let pml4_idx = ((gpa >> 39) & 0x1FF) as usize;
        let pdpt_idx = ((gpa >> 30) & 0x1FF) as usize;
        let pd_idx = ((gpa >> 21) & 0x1FF) as usize;
        let pt_idx = ((gpa >> 12) & 0x1FF) as usize;

        let pml4_entry = self.pml4.entry(pml4_idx);
        if pml4_entry == 0 {
            return;
        }
        let pdpt = Self::entry_to_page_mut(pml4_entry);

        let pdpt_entry = unsafe { (*pdpt).entry(pdpt_idx) };
        if pdpt_entry == 0 {
            return;
        }
        let pd = Self::entry_to_page_mut(pdpt_entry);

        let pd_entry = unsafe { (*pd).entry(pd_idx) };
        if pd_entry == 0 {
            return;
        }
        let pt = Self::entry_to_page_mut(pd_entry);

        unsafe {
            (*pt).set_entry(pt_idx, 0);
        }
    }

    fn get_or_create_child(
        tables: &mut Vec<Box<NptPage>>,
        parent: *mut NptPage,
        idx: usize,
    ) -> *mut NptPage {
        let entry = unsafe { (*parent).entry(idx) };
        if entry != 0 && (entry & NPT_PRESENT) != 0 {
            return Self::entry_to_page_mut(entry);
        }
        let mut child = NptPage::new();
        let child_phys = child.phys_addr();
        let child_ptr = child.as_mut() as *mut NptPage;
        tables.push(child);
        let new_entry = (child_phys & NPT_PHYS_MASK) | NPT_RWX;
        unsafe {
            (*parent).set_entry(idx, new_entry);
        }
        child_ptr
    }

    fn entry_to_page_mut(entry: u64) -> *mut NptPage {
        use crate::arch::x86_64::consts::PHYS_OFFSET;
        let phys = entry & NPT_PHYS_MASK;
        (phys + PHYS_OFFSET as u64) as *mut NptPage
    }
}

/// Execute `INVLPGA` to invalidate a TLB entry for the given virtual address and ASID.
///
/// # Safety
/// Must be called on an AMD CPU with SVM enabled.
pub unsafe fn invlpga(va: u64, asid: u32) {
    unsafe {
        core::arch::asm!(
            "invlpga {0}, {1}",
            in(reg) va,
            in(reg) asid as u64,
            options(nostack, preserves_flags)
        );
    }
}

//! Extended Page Tables (EPT) — Intel VT-x Nested Page Tables
//!
//! Implements a 4-level EPT hierarchy (PML4 → PDPT → PD → PT) for guest physical
//! address translation. Each level uses 512 × 8-byte entries.

use alloc::{boxed::Box, vec::Vec};
use core::sync::atomic::{fence, Ordering};

/// EPT memory type: Write-Back (6) is used for normal RAM.
const EPT_MT_WB: u64 = 6;

/// EPT entry permission bits.
pub const EPT_READ: u64 = 1 << 0;
pub const EPT_WRITE: u64 = 1 << 1;
pub const EPT_EXEC: u64 = 1 << 2;
pub const EPT_RWX: u64 = EPT_READ | EPT_WRITE | EPT_EXEC;

/// Bit 7: leaf entry (large page or 4KB page).
const EPT_LEAF: u64 = 1 << 7;

/// Physical address mask for a 4KB-aligned address in an EPT entry.
const EPT_PHYS_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// A single 4KB EPT page (512 × 8-byte entries).
#[repr(C, align(4096))]
pub struct EptPage {
    entries: [u64; 512],
}

impl EptPage {
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

    /// Physical address of this page (identity-mapped in kernel physmap).
    pub fn phys_addr(&self) -> u64 {
        // In Redox the kernel physmap is at PHYS_OFFSET; kernel virtual = physical + PHYS_OFFSET.
        // Box<EptPage> is allocated in kernel heap which is in the physmap region.
        // We subtract PHYS_OFFSET to get the physical address.
        use crate::arch::x86_64::consts::PHYS_OFFSET;
        let virt = self as *const _ as u64;
        virt - PHYS_OFFSET as u64
    }
}

/// 4-level EPT root, owning all allocated sub-tables.
pub struct EptRoot {
    pml4: Box<EptPage>,
    /// Owned sub-tables to prevent deallocation while EPT is live.
    _tables: Vec<Box<EptPage>>,
}

impl EptRoot {
    /// Allocate a new, empty EPT root.
    pub fn new() -> Self {
        Self {
            pml4: EptPage::new(),
            _tables: Vec::new(),
        }
    }

    /// Physical address of the PML4 — used as the EPTP value in VMCS.
    /// Bits [5:3] encode memory type (WB=6), bit 6 enables accessed/dirty tracking.
    pub fn eptp(&self) -> u64 {
        self.pml4.phys_addr() | (EPT_MT_WB << 3) | (3 << 0) // walk length - 1 = 3
    }

    /// Map `[guest_phys, guest_phys+size)` → `[host_phys, host_phys+size)` with `perms`.
    /// `size` must be a multiple of 4096.
    pub fn map_range(&mut self, guest_phys: u64, host_phys: u64, size: u64, perms: u64) {
        assert!(size % 4096 == 0, "EPT map_range: size must be page-aligned");
        assert!(
            guest_phys % 4096 == 0,
            "EPT map_range: guest_phys must be page-aligned"
        );
        assert!(
            host_phys % 4096 == 0,
            "EPT map_range: host_phys must be page-aligned"
        );

        let mut gpa = guest_phys;
        let mut hpa = host_phys;
        let end = guest_phys + size;

        while gpa < end {
            self.map_page(gpa, hpa, perms);
            gpa += 4096;
            hpa += 4096;
        }

        // Ensure writes are visible before VMENTRY.
        fence(Ordering::SeqCst);
    }

    /// Unmap `[guest_phys, guest_phys+size)` and invalidate EPT TLB.
    pub fn unmap_range(&mut self, guest_phys: u64, size: u64) {
        assert!(size % 4096 == 0);
        let mut gpa = guest_phys;
        let end = guest_phys + size;
        while gpa < end {
            self.unmap_page(gpa);
            gpa += 4096;
        }
        // Invalidate EPT TLB for this EPTP.
        unsafe {
            invept(self.eptp());
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

        // Safety: pt is a valid pointer to an EptPage we own.
        let pt_ref = unsafe { &mut *pt };
        let leaf = (hpa & EPT_PHYS_MASK) | (EPT_MT_WB << 3) | EPT_LEAF | perms;
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

    /// Walk to or create a child page table at `parent[idx]`.
    /// Returns a raw pointer to the child `EptPage`.
    fn get_or_create_child(
        tables: &mut Vec<Box<EptPage>>,
        parent: *mut EptPage,
        idx: usize,
    ) -> *mut EptPage {
        let entry = unsafe { (*parent).entry(idx) };
        if entry != 0 {
            return Self::entry_to_page_mut(entry);
        }
        // Allocate new child page.
        let mut child = EptPage::new();
        let child_phys = child.phys_addr();
        let child_ptr = child.as_mut() as *mut EptPage;
        tables.push(child);
        // Write non-leaf entry: R|W|X + physical address.
        let new_entry = (child_phys & EPT_PHYS_MASK) | EPT_RWX;
        unsafe {
            (*parent).set_entry(idx, new_entry);
        }
        child_ptr
    }

    fn entry_to_page_mut(entry: u64) -> *mut EptPage {
        use crate::arch::x86_64::consts::PHYS_OFFSET;
        let phys = entry & EPT_PHYS_MASK;
        (phys + PHYS_OFFSET as u64) as *mut EptPage
    }
}

/// Execute `INVEPT` (single-context invalidation) for the given EPTP.
///
/// # Safety
/// Must be called with a valid EPTP on a CPU that supports EPT.
pub unsafe fn invept(eptp: u64) {
    // INVEPT descriptor: 128-bit struct { eptp: u64, reserved: u64 }
    let descriptor: [u64; 2] = [eptp, 0];
    unsafe {
        core::arch::asm!(
            "invept {0}, [{1}]",
            in(reg) 1u64,          // type 1 = single-context
            in(reg) descriptor.as_ptr(),
            options(nostack, preserves_flags)
        );
    }
}

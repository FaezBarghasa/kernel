//! ARMv7 paging and MMU support

pub mod entry;
pub mod mapper;

pub use entry::*;
pub use mapper::*;

use crate::memory::{allocate_frames, Frame, PhysicalAddress, PAGE_SIZE};

/// Size of L1 page table (16KB for 4096 entries)
const L1_TABLE_SIZE: usize = 16 * 1024;

/// Initialize paging and enable MMU
pub unsafe fn init() {
    log::info!("Initializing ARMv7 paging...");

    // Allocate L1 page table (4 frames = 16KB)
    let l1_frames = allocate_frames(4).expect("Failed to allocate L1 page table");
    let l1_phys = l1_frames[0].base();
    let l1_virt = l1_phys.data() as *mut PageTableEntry;

    // Zero the L1 table
    core::ptr::write_bytes(l1_virt, 0, 4096);

    // Create mapper
    let mut mapper = Mapper::new(l1_phys);

    // Set up identity mapping for first 1GB (kernel space)
    // Use 1MB sections for efficiency
    log::info!("Setting up identity mapping for kernel (0-1GB)...");

    for section in 0..1024 {
        let virt = section * 1024 * 1024;
        let phys = PhysicalAddress::new(virt);

        let flags = PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::EXECUTABLE
            | PageTableFlags::KERNEL;

        mapper
            .map_section(virt, phys, flags)
            .expect("Failed to map kernel section");
    }

    log::info!("Enabling MMU...");

    // Invalidate TLB before enabling MMU
    Mapper::invalidate_tlb_all();

    // Enable MMU
    mapper.enable_mmu();

    log::info!("ARMv7 MMU enabled successfully");
}

/// Get current page table base address
pub fn current_page_table() -> PhysicalAddress {
    unsafe {
        let ttbr0: u32;
        core::arch::asm!(
            "mrc p15, 0, {0}, c2, c0, 0",
            out(reg) ttbr0,
            options(nostack, preserves_flags)
        );
        PhysicalAddress::new(ttbr0 as usize & !0x3FFF) // Mask lower 14 bits
    }
}

/// Check if MMU is enabled
pub fn is_mmu_enabled() -> bool {
    unsafe {
        let sctlr: u32;
        core::arch::asm!(
            "mrc p15, 0, {0}, c1, c0, 0",
            out(reg) sctlr,
            options(nostack, preserves_flags)
        );
        (sctlr & 1) != 0
    }
}

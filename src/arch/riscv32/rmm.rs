//! RISC-V 32-bit RMM integration

use crate::paging::{PhysicalAddress, RmmArch, VirtualAddress};

/// RISC-V 32 RMM architecture implementation
pub struct RmmRiscV32;

unsafe impl RmmArch for RmmRiscV32 {
    unsafe fn init(_: usize) {
        // Initialize page tables
    }

    unsafe fn phys_to_virt(phys: PhysicalAddress) -> VirtualAddress {
        // Direct mapping in kernel space
        VirtualAddress::new(phys.data() + crate::PHYS_OFFSET)
    }

    unsafe fn virt_to_phys(virt: VirtualAddress) -> Option<PhysicalAddress> {
        if virt.data() >= crate::PHYS_OFFSET {
            Some(PhysicalAddress::new(virt.data() - crate::PHYS_OFFSET))
        } else {
            None
        }
    }
}

/// Set RMM architecture to RISC-V 32
pub type RmmA = RmmRiscV32;

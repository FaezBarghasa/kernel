//! ARMv7 RMM (Redox Memory Manager) integration

use crate::paging::{PhysicalAddress, RmmA, RmmArch, VirtualAddress, PAGE_SIZE};

/// ARMv7 RMM architecture implementation
pub struct RmmArmV7;

unsafe impl RmmArch for RmmArmV7 {
    unsafe fn init(_: usize) {
        // Initialize page tables
    }

    unsafe fn phys_to_virt(phys: PhysicalAddress) -> VirtualAddress {
        // Direct mapping: physical + PHYS_OFFSET
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

/// Set RMM architecture to ARMv7
pub type RmmA = RmmArmV7;

#![forbid(unsafe_code)]

//! # Tock-Style MPU/PMP Memory Isolation Abstraction
//!
//! Provides region allocation definitions and Hardware Memory Protection Unit
//! traits for Arm Cortex-M MPU and RISC-V PMP.

/// Memory access permissions for MPU/PMP regions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MpuPermission {
    NoAccess,
    ReadOnly,
    ReadWrite,
    ReadExecute,
}

/// Defines a memory region protected by the MPU hardware.
#[derive(Clone, Copy, Debug)]
pub struct MpuRegion {
    pub base_addr: usize,
    pub size_bytes: usize,
    pub permission: MpuPermission,
}

impl MpuRegion {
    /// Creates a new `MpuRegion`.
    pub const fn new(base_addr: usize, size_bytes: usize, permission: MpuPermission) -> Self {
        Self {
            base_addr,
            size_bytes,
            permission,
        }
    }

    /// Checks if a memory buffer `[addr, addr + len)` falls strictly within this region.
    pub fn contains(&self, addr: usize, len: usize) -> bool {
        let end_addr = match addr.checked_add(len) {
            Some(e) => e,
            None => return false,
        };
        let region_end = match self.base_addr.checked_add(self.size_bytes) {
            Some(e) => e,
            None => return false,
        };

        addr >= self.base_addr && end_addr <= region_end
    }
}

/// Abstract Hardware Memory Protection Unit Interface.
pub trait MemoryProtectionUnit {
    /// Configures a specific hardware MPU slot.
    fn configure_region(&mut self, slot: usize, region: MpuRegion) -> Result<(), &'static str>;
    /// Enables memory protection.
    fn enable(&mut self);
    /// Disables memory protection.
    fn disable(&mut self);
}

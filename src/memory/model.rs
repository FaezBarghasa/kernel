//! # Memory Model Abstraction
//!
//! Defines the interface for memory management units, supporting both
//! traditional MMU (Virtual) and No-MMU/MPU (Flat) architectures.

use crate::syscall::error::{Error, Result, EFAULT, EINVAL, ENOENT, ENOMEM};
use bitflags::bitflags;
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::Mutex;

bitflags! {
    /// Permissions for MPU regions
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct MpuPermissions: u8 {
        const READ = 1 << 0;
        const WRITE = 1 << 1;
        const EXEC = 1 << 2;

        const RW = Self::READ.bits() | Self::WRITE.bits();
        const RX = Self::READ.bits() | Self::EXEC.bits();
        const RWX = Self::READ.bits() | Self::WRITE.bits() | Self::EXEC.bits();
    }
}

/// MPU Zone definition for Flat memory model
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zone {
    pub base: usize,
    pub size: usize,
    pub permissions: MpuPermissions,
    pub active: bool,
}

impl Zone {
    pub const fn empty() -> Self {
        Zone {
            base: 0,
            size: 0,
            permissions: MpuPermissions::empty(),
            active: false,
        }
    }

    pub fn contains(&self, addr: usize) -> bool {
        self.active && addr >= self.base && addr < (self.base + self.size)
    }
}

/// Helper for managing MPU resources
#[derive(Debug)]
pub struct MpuManager {
    zones: [Zone; 8], // Typical MPU has 8 hardware regions
}

impl MpuManager {
    pub const fn new() -> Self {
        MpuManager {
            zones: [Zone::empty(); 8],
        }
    }

    pub fn add_region(
        &mut self,
        base: usize,
        size: usize,
        permissions: MpuPermissions,
    ) -> Result<usize> {
        for (i, zone) in self.zones.iter_mut().enumerate() {
            if !zone.active {
                *zone = Zone {
                    base,
                    size,
                    permissions,
                    active: true,
                };
                return Ok(i);
            }
        }
        Err(Error::new(ENOMEM))
    }

    pub fn remove_region(&mut self, index: usize) -> Result<()> {
        if index >= self.zones.len() {
            return Err(Error::new(EINVAL));
        }
        self.zones[index] = Zone::empty();
        Ok(())
    }

    pub fn find_zone(&self, addr: usize) -> Option<(usize, Zone)> {
        for (i, zone) in self.zones.iter().enumerate() {
            if zone.active && zone.contains(addr) {
                return Some((i, *zone));
            }
        }
        None
    }

    pub fn check_permission(&self, addr: usize, perm: MpuPermissions) -> Result<()> {
        if let Some((_, zone)) = self.find_zone(addr) {
            if zone.permissions.contains(perm) {
                return Ok(());
            }
        }
        // In MPU systems, default is often 'deny' if no region matches
        Err(Error::new(EFAULT))
    }

    pub fn load_zones(&mut self, zones: &[Zone]) {
        for (i, zone) in zones.iter().enumerate() {
            if i < self.zones.len() {
                self.zones[i] = *zone;
            } else {
                break;
            }
        }
        // Clear remaining zones
        for i in zones.len()..self.zones.len() {
            self.zones[i] = Zone::empty();
        }
    }

    pub fn active_zones(&self) -> &[Zone; 8] {
        &self.zones
    }
}

/// Hook type for arch-specific MPU configuration
pub type MpuConfigureFn = fn(&[Zone]) -> Result<()>;

/// Global hook for hardware MPU configuration.
/// Arch initialization code should set this.
pub static HW_MPU_CONFIGURE: AtomicUsize = AtomicUsize::new(0);

/// The MemoryModel trait abstracts over Virtual vs Flat translation
pub trait MemoryModel {
    fn name(&self) -> &'static str;

    /// Translate virtual/logical address to physical address
    fn translate(&self, virt: usize) -> Result<usize>;

    /// Define a protection boundary
    fn protect_userspace(&self, base: usize, size: usize, perm: MpuPermissions) -> Result<()>;

    /// Remove a protection boundary via its base address
    fn remove_protection(&self, base: usize) -> Result<()>;

    /// Verify access (software check for MPU, hardware trap verify for MMU)
    fn check_access(&self, addr: usize, perm: MpuPermissions) -> Result<()>;

    /// Configure hardware for a specific task's zones (Context Switch)
    fn configure_for_task(&self, zones: &[Zone]) -> Result<()>;
}

/// Standard MMU-based Virtual Memory
pub struct VirtualMemoryModel;

impl VirtualMemoryModel {
    pub const fn new() -> Self {
        VirtualMemoryModel
    }
}

impl MemoryModel for VirtualMemoryModel {
    fn name(&self) -> &'static str {
        "Virtual"
    }

    fn translate(&self, virt: usize) -> Result<usize> {
        // In a real implementation this would query page tables
        // For now we rely on the existing RMM infrastructure elsewhere
        Ok(virt) // Placeholder
    }

    fn protect_userspace(&self, _base: usize, _size: usize, _perm: MpuPermissions) -> Result<()> {
        // Handled by Page Tables via RMM
        Ok(())
    }

    fn remove_protection(&self, _base: usize) -> Result<()> {
        // Handled by Page Tables via RMM
        Ok(())
    }

    fn check_access(&self, _addr: usize, _perm: MpuPermissions) -> Result<()> {
        // Hardware handles this via Page Faults
        Ok(())
    }

    fn configure_for_task(&self, _zones: &[Zone]) -> Result<()> {
        // Handled by CR3 switch / ASID
        Ok(())
    }
}

/// No-MMU Flat Memory with MPU protection
pub struct FlatMemoryModel {
    mpu: Mutex<MpuManager>,
}

impl FlatMemoryModel {
    pub const fn new() -> Self {
        FlatMemoryModel {
            mpu: Mutex::new(MpuManager::new()),
        }
    }
}

impl MemoryModel for FlatMemoryModel {
    fn name(&self) -> &'static str {
        "Flat (No-MMU)"
    }

    fn translate(&self, virt: usize) -> Result<usize> {
        // Flat memory model is identity mapped
        Ok(virt)
    }

    fn protect_userspace(&self, base: usize, size: usize, perm: MpuPermissions) -> Result<()> {
        let mut mpu = self.mpu.lock();
        mpu.add_region(base, size, perm)?;
        Ok(())
    }

    fn remove_protection(&self, base: usize) -> Result<()> {
        let mut mpu = self.mpu.lock();
        // Find region by base address
        let mut target_idx = None;
        for (i, zone) in mpu.active_zones().iter().enumerate() {
            if zone.active && zone.base == base {
                target_idx = Some(i);
                break;
            }
        }

        if let Some(idx) = target_idx {
            mpu.remove_region(idx)
        } else {
            Err(Error::new(ENOENT))
        }
    }

    fn check_access(&self, addr: usize, perm: MpuPermissions) -> Result<()> {
        // Software verification of MPU boundaries
        let mpu = self.mpu.lock();
        mpu.check_permission(addr, perm)
    }

    fn configure_for_task(&self, zones: &[Zone]) -> Result<()> {
        // 1. Update software state
        {
            let mut mpu = self.mpu.lock();
            mpu.load_zones(zones);
        }

        // 2. Program Hardware
        let hook_addr = HW_MPU_CONFIGURE.load(Ordering::Relaxed);
        if hook_addr != 0 {
            let hook: MpuConfigureFn = unsafe { core::mem::transmute(hook_addr) };
            hook(zones)
        } else {
            // If no hook is registered, we assume software-only checking is sufficient
            // or MPU is not yet initialized.
            Ok(())
        }
    }
}

// Global accessor alias
#[cfg(feature = "no-mmu")]
pub type SystemMemoryModel = FlatMemoryModel;

#[cfg(not(feature = "no-mmu"))]
pub type SystemMemoryModel = VirtualMemoryModel;

// Global static instance
pub static MEMORY_MODEL: SystemMemoryModel = SystemMemoryModel::new();

#[cfg(feature = "no-mmu")]
// =============================================================================
// Arch-specific MPU Hook Registration
// =============================================================================

/// Register a hardware MPU configuration function.
/// Called during arch-specific init to wire the HAL MPU/PMP driver
/// into the kernel's `FlatMemoryModel`.
pub fn register_hw_mpu_hook(hook: MpuConfigureFn) {
    HW_MPU_CONFIGURE.store(hook as usize, Ordering::Release);
}

// =============================================================================
// MPU Fault Handler
// =============================================================================

/// Handle an MPU fault by checking the faulting address against the active zones.
///
/// Called from arch-specific fault handlers (e.g. Xtensa LoadStoreError,
/// RISC-V store/load access fault) to determine whether the access was
/// a legitimate MPU violation.
///
/// Returns `Ok(())` if the address is covered by a zone with matching permissions
/// (i.e. the fault should be retried after reconfiguration), or `Err(EFAULT)` if
/// the access violates MPU policy and the task should be terminated.
pub fn handle_mpu_fault(addr: usize, access: MpuPermissions) -> Result<()> {
    MEMORY_MODEL.check_access(addr, access)
}

// =============================================================================
// Validation
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_check() {
        let model = FlatMemoryModel::new();

        // RW region
        assert!(model
            .protect_userspace(0x1000, 0x1000, MpuPermissions::RW)
            .is_ok());

        // RX region
        assert!(model
            .protect_userspace(0x2000, 0x1000, MpuPermissions::RX)
            .is_ok());

        // Check RW region
        assert!(model.check_access(0x1050, MpuPermissions::READ).is_ok());
        assert!(model.check_access(0x1050, MpuPermissions::WRITE).is_ok());
        assert!(model.check_access(0x1050, MpuPermissions::EXEC).is_err());

        // Check RX region
        assert!(model.check_access(0x2050, MpuPermissions::READ).is_ok());
        assert!(model.check_access(0x2050, MpuPermissions::WRITE).is_err());
        assert!(model.check_access(0x2050, MpuPermissions::EXEC).is_ok());
    }

    #[test]
    fn test_zone_removal() {
        let model = FlatMemoryModel::new();

        assert!(model
            .protect_userspace(0x1000, 0x1000, MpuPermissions::RW)
            .is_ok());
        assert!(model.check_access(0x1050, MpuPermissions::READ).is_ok());

        assert!(model.remove_protection(0x1000).is_ok());

        assert!(model.check_access(0x1050, MpuPermissions::READ).is_err());
    }

    #[test]
    fn test_zone_overlap() {
        let model = FlatMemoryModel::new();

        // Broad Read-Only
        assert!(model
            .protect_userspace(0x0000, 0x4000, MpuPermissions::READ)
            .is_ok());

        // Specific Read-Write inside
        assert!(model
            .protect_userspace(0x1000, 0x1000, MpuPermissions::RW)
            .is_ok());

        // 0x500 in zone 0 (READ): read ok, write fail
        assert!(model.check_access(0x500, MpuPermissions::READ).is_ok());
        assert!(model.check_access(0x500, MpuPermissions::WRITE).is_err());

        // 0x1500: first-match is zone 0 (READ). Write still fails.
        // Documents current first-match-wins behavior.
        assert!(model.check_access(0x1500, MpuPermissions::WRITE).is_err());
    }

    #[test]
    fn test_identity_translation() {
        let model = FlatMemoryModel::new();
        assert_eq!(model.translate(0x123456).unwrap(), 0x123456);
        assert_eq!(model.translate(0).unwrap(), 0);
        assert_eq!(model.translate(usize::MAX).unwrap(), usize::MAX);
    }

    #[test]
    fn test_task_zone_loading() {
        let model = FlatMemoryModel::new();
        let zones = [
            Zone {
                base: 0x1000,
                size: 0x1000,
                permissions: MpuPermissions::READ,
                active: true,
            },
            Zone {
                base: 0x2000,
                size: 0x1000,
                permissions: MpuPermissions::WRITE,
                active: true,
            },
        ];

        assert!(model.configure_for_task(&zones).is_ok());

        {
            let mpu = model.mpu.lock();
            let active = mpu.active_zones();
            assert!(active[0].active);
            assert_eq!(active[0].base, 0x1000);
            assert!(active[1].active);
            assert_eq!(active[1].base, 0x2000);
            assert!(!active[2].active); // Rest cleared
        }
    }

    #[test]
    fn test_mpu_exhaustion() {
        let model = FlatMemoryModel::new();

        // Fill all 8 zones
        for i in 0..8 {
            assert!(model
                .protect_userspace(0x1000 * (i + 1), 0x1000, MpuPermissions::READ)
                .is_ok());
        }

        // 9th zone should fail
        assert_eq!(
            model.protect_userspace(0x9000, 0x1000, MpuPermissions::READ),
            Err(Error::new(ENOMEM))
        );
    }

    /// Verify exact boundary semantics: first byte inside region passes,
    /// last byte inside passes, first byte outside fails.
    #[test]
    fn test_mpu_boundary_violations() {
        let model = FlatMemoryModel::new();

        // Region: 0x4000..0x5000 (size 0x1000)
        assert!(model
            .protect_userspace(0x4000, 0x1000, MpuPermissions::RW)
            .is_ok());

        // First byte (0x4000) — inside
        assert!(model.check_access(0x4000, MpuPermissions::READ).is_ok());

        // Last byte (0x4FFF) — inside
        assert!(model.check_access(0x4FFF, MpuPermissions::READ).is_ok());

        // One past end (0x5000) — outside
        assert!(model.check_access(0x5000, MpuPermissions::READ).is_err());

        // One before start (0x3FFF) — outside
        assert!(model.check_access(0x3FFF, MpuPermissions::READ).is_err());

        // Execute should fail (region is RW, not RWX)
        assert!(model.check_access(0x4500, MpuPermissions::EXEC).is_err());
    }

    /// Simulate an MPU fault handler invocation and verify it returns
    /// EFAULT for unpermitted access and Ok for permitted access.
    #[test]
    fn test_mpu_fault_handler() {
        let model = FlatMemoryModel::new();

        // Setup: .text RX zone and .data RW zone
        assert!(model
            .protect_userspace(0x10000, 0x2000, MpuPermissions::RX)
            .is_ok());
        assert!(model
            .protect_userspace(0x20000, 0x1000, MpuPermissions::RW)
            .is_ok());

        // Simulate fault: write to .text region → EFAULT
        let result = model.check_access(0x10500, MpuPermissions::WRITE);
        assert_eq!(result, Err(Error::new(EFAULT)));

        // Simulate fault: exec in .data region → EFAULT
        let result = model.check_access(0x20500, MpuPermissions::EXEC);
        assert_eq!(result, Err(Error::new(EFAULT)));

        // Simulate fault: read from .text → Ok (allowed)
        let result = model.check_access(0x10500, MpuPermissions::READ);
        assert!(result.is_ok());

        // Simulate fault: write to .data → Ok (allowed)
        let result = model.check_access(0x20500, MpuPermissions::WRITE);
        assert!(result.is_ok());

        // Simulate fault: access to unmapped region → EFAULT
        let result = model.check_access(0x99999, MpuPermissions::READ);
        assert_eq!(result, Err(Error::new(EFAULT)));
    }

    /// Verify that `configure_for_task` replaces all previous zones.
    #[test]
    fn test_configure_for_task_clears_previous() {
        let model = FlatMemoryModel::new();

        // First: fill 4 zones
        for i in 0..4 {
            assert!(model
                .protect_userspace(0x1000 * (i + 1), 0x1000, MpuPermissions::READ)
                .is_ok());
        }

        // Verify zone 0 is at 0x1000
        assert!(model.check_access(0x1500, MpuPermissions::READ).is_ok());

        // Context switch: load only 1 new zone at 0xA000
        let new_zones = [Zone {
            base: 0xA000,
            size: 0x1000,
            permissions: MpuPermissions::RWX,
            active: true,
        }];
        assert!(model.configure_for_task(&new_zones).is_ok());

        // Old zone at 0x1000 should be gone
        assert!(model.check_access(0x1500, MpuPermissions::READ).is_err());

        // New zone at 0xA000 should be active
        assert!(model.check_access(0xA500, MpuPermissions::READ).is_ok());
        assert!(model.check_access(0xA500, MpuPermissions::WRITE).is_ok());
        assert!(model.check_access(0xA500, MpuPermissions::EXEC).is_ok());

        // Slots 1..7 should be cleared
        {
            let mpu = model.mpu.lock();
            for i in 1..8 {
                assert!(!mpu.active_zones()[i].active);
            }
        }
    }
}

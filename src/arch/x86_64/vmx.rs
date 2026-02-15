use crate::{
    scheme::vmm::VmExitReason,
    syscall::error::{Error, Result, EINVAL, EIO},
};
use alloc::boxed::Box;
#![allow(dead_code)]
use core::arch::asm;

// VMX Instructions
pub unsafe fn vmxon(addr: u64) -> Result<()> {
    let ret: u64;
    unsafe {
        asm!("vmxon [{}]", in(reg) &addr, lateout("rax") ret, options(nostack));
    }
    if ret != 0 {
        Err(Error::new(EIO))
    } else {
        Ok(())
    }
}

pub unsafe fn vmxoff() {
    unsafe {
        asm!("vmxoff", options(nostack));
    }
}

pub unsafe fn vmclear(addr: u64) -> Result<()> {
    let mut flags: u64;
    unsafe {
        asm!(
            "vmclear [{}]",
            "pushfq",
            "pop {}",
            in(reg) &addr,
            out(reg) flags
        );
    }
    // VF (bit 1) or CF (bit 0) set on failure
    if (flags & 0x3) != 0 {
        Err(Error::new(EIO))
    } else {
        Ok(())
    }
}

pub unsafe fn vmptrld(addr: u64) -> Result<()> {
    let mut flags: u64;
    unsafe {
        asm!(
            "vmptrld [{}]",
            "pushfq",
            "pop {}",
            in(reg) &addr,
            out(reg) flags
        );
    }
    if (flags & 0x3) != 0 {
        Err(Error::new(EIO))
    } else {
        Ok(())
    }
}

pub unsafe fn vmptrst() -> Result<u64> {
    let mut addr: u64 = 0;
    let mut flags: u64;
    unsafe {
        asm!(
            "vmptrst [{}]",
            "pushfq",
            "pop {}",
            in(reg) &mut addr,
            out(reg) flags
        );
    }
    if (flags & 0x3) != 0 {
        Err(Error::new(EIO))
    } else {
        Ok(addr)
    }
}

// VMCS Field Encodings (simplified subset)
pub const VMCS_GUEST_RIP: u32 = 0x0000681E;
pub const VMCS_GUEST_RSP: u32 = 0x0000681C;
pub const VMCS_GUEST_RFLAGS: u32 = 0x00006820;
pub const VMCS_EXIT_REASON: u32 = 0x00004402;

pub unsafe fn vmread(field: u32) -> Result<u64> {
    let mut val: u64;
    let mut flags: u64;
    unsafe {
        asm!(
            "vmread {}, {}",
            "pushfq",
            "pop {}",
            out(reg) val,
            in(reg) field as u64,
            out(reg) flags
        );
    }
    if (flags & 0x3) != 0 {
        Err(Error::new(EIO))
    } else {
        Ok(val)
    }
}

pub unsafe fn vmwrite(field: u32, val: u64) -> Result<()> {
    let mut flags: u64;
    unsafe {
        asm!(
            "vmwrite {}, {}",
            "pushfq",
            "pop {}",
            in(reg) field as u64,
            in(reg) val,
            out(reg) flags
        );
    }
    if (flags & 0x3) != 0 {
        Err(Error::new(EIO))
    } else {
        Ok(())
    }
}

/// Helper struct for a 4KB aligned physical page, used for VMXON/VMCS regions
#[repr(C, align(4096))]
pub struct VmxRegion {
    pub revision_id: u32,
    pub data: [u8; 4092],
}

impl VmxRegion {
    pub fn new() -> Box<Self> {
        Box::new(Self {
            revision_id: 1, // Must be read from MSR_IA32_VMX_BASIC in real hw
            data: [0; 4092],
        })
    }
}

pub struct Vmx {
    vmxon_region: Box<VmxRegion>,
    vmcs_region: Box<VmxRegion>,
    active: bool,
}

impl Vmx {
    pub fn new() -> Self {
        Self {
            vmxon_region: VmxRegion::new(),
            vmcs_region: VmxRegion::new(),
            active: false,
        }
    }

    pub fn init(&mut self) -> Result<()> {
        let vmxon_phys = self.vmxon_region.as_ref() as *const _ as u64; // In real kernel need virtual_to_physical
        unsafe {
            vmxon(vmxon_phys)?;
        }
        self.active = true;

        let vmcs_phys = self.vmcs_region.as_ref() as *const _ as u64;
        unsafe {
            vmclear(vmcs_phys)?;
        }
        unsafe {
            vmptrld(vmcs_phys)?;
        }

        Ok(())
    }

    pub fn enter_guest(&mut self) -> Result<VmExitReason> {
        if !self.active {
            self.init()?;
        }

        unsafe {
            // Simplified launch/resume
            // In reality, need to save host state, load guest state, handle VM exits loop
            // for this task, we will try to execute vmlaunch
            let mut flags: u64;
            asm!(
                "vmlaunch",
                "pushfq",
                "pop {}",
                out(reg) flags,
                options(nostack)
            );

            // If we are here, VMLAUNCH failed
            if (flags & 0x3) != 0 {
                // Read error
                Err(Error::new(EIO))
            } else {
                // Should invoke handling logic
                Ok(VmExitReason::Unknown)
            }
        }
    }
}

impl Drop for Vmx {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                vmxoff();
            }
        }
    }
}

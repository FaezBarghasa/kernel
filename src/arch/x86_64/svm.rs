use crate::{scheme::vmm::VmExitReason, syscall::error::Result};
use alloc::boxed::Box;
use core::arch::asm;

// SVM Instructions
pub unsafe fn vmrun(vmcb_phys: u64) {
    unsafe {
        asm!("vmrun", in("rax") vmcb_phys, options(nostack));
    }
}

pub unsafe fn vmsave(vmcb_phys: u64) {
    unsafe {
        asm!("vmsave", in("rax") vmcb_phys, options(nostack));
    }
}

pub unsafe fn vmload(vmcb_phys: u64) {
    unsafe {
        asm!("vmload", in("rax") vmcb_phys, options(nostack));
    }
}

// VMCB Layout (simplified)
#[repr(C, align(4096))]
pub struct Vmcb {
    pub control_area: [u8; 1024],
    pub state_save_area: [u8; 3072],
}

impl Vmcb {
    pub fn new() -> Box<Self> {
        Box::new(Self {
            control_area: [0; 1024],
            state_save_area: [0; 3072],
        })
    }
}

pub struct Svm {
    vmcb: Box<Vmcb>,
    frame: Box<Vmcb>, // Host state save area
}

impl Svm {
    pub fn new() -> Self {
        Self {
            vmcb: Vmcb::new(),
            frame: Vmcb::new(),
        }
    }

    pub fn enter_guest(&mut self) -> Result<VmExitReason> {
        // Physical address of VMCB
        let vmcb_phys = self.vmcb.as_ref() as *const _ as u64; // Todo: translation

        // Physical address of Host State
        let host_phys = self.frame.as_ref() as *const _ as u64;

        unsafe {
            // Save host state
            vmsave(host_phys);

            // Load guest state (optional, usually done by hardware on VMRUN)
            vmload(vmcb_phys);

            // Run guest
            vmrun(vmcb_phys);

            // Restore host state
            vmload(host_phys);

            // Save guest state
            vmsave(vmcb_phys);
        }

        // Read exit code from VMCB control area (offset 0x70)
        let exit_code = unsafe {
            let ptr = self.vmcb.control_area.as_ptr().add(0x70) as *const u64;
            *ptr
        };

        Ok(VmExitReason::from(exit_code as u32))
    }
}

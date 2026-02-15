use alloc::{collections::BTreeMap, vec::Vec};
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::RwLock;

use crate::{
    context::file::InternalFlags,
    scheme::{CallerCtx, KernelScheme, OpenResult},
    sync::CleanLockToken,
    syscall::error::{Error, Result, EBADF, EINVAL, ENOTTY},
};

use crate::arch::x86_64::svm::Svm;
#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::vmx::Vmx;

// VMM IOCTL definitions
pub const VMM_IOCTL_BASE: usize = 0x7000;
pub const VMM_IOCTL_SET_MEMORY: usize = VMM_IOCTL_BASE + 0x01;
pub const VMM_IOCTL_CREATE_VCPU: usize = VMM_IOCTL_BASE + 0x02;
pub const VMM_IOCTL_RUN_VCPU: usize = VMM_IOCTL_BASE + 0x03;
pub const VMM_IOCTL_GET_REGS: usize = VMM_IOCTL_BASE + 0x04;
pub const VMM_IOCTL_SET_REGS: usize = VMM_IOCTL_BASE + 0x05;

/// Guest memory region descriptor
#[derive(Clone, Debug)]
pub struct MemoryRegion {
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub userspace_addr: u64,
    pub flags: u32,
}

/// Virtual CPU state
#[derive(Clone, Debug)]
pub struct VcpuState {
    pub id: u32,
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

impl VcpuState {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rsp: 0,
            rbp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: 0,
            rflags: 0x2, // Reserved bit always set
        }
    }
}

/// VM exit reasons (simplified)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum VmExitReason {
    ExternalInterrupt = 1,
    IoInstruction = 30,
    MmioAccess = 48,
    Halt = 12,
    Shutdown = 26,
    Unknown = 0,
}

impl From<u32> for VmExitReason {
    fn from(value: u32) -> Self {
        match value {
            1 => VmExitReason::ExternalInterrupt,
            30 => VmExitReason::IoInstruction,
            48 => VmExitReason::MmioAccess,
            12 => VmExitReason::Halt,
            26 => VmExitReason::Shutdown,
            _ => VmExitReason::Unknown,
        }
    }
}

/// VM instance representing a single virtual machine
pub struct VmInstance {
    pub id: usize,
    pub memory_regions: Vec<MemoryRegion>,
    pub vcpus: BTreeMap<u32, VcpuState>,
    #[cfg(target_arch = "x86_64")]
    pub vmx: Option<Vmx>,
    #[cfg(target_arch = "x86_64")]
    pub svm: Option<Svm>,
}

impl VmInstance {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            memory_regions: Vec::new(),
            vcpus: BTreeMap::new(),
            #[cfg(target_arch = "x86_64")]
            vmx: None,
            #[cfg(target_arch = "x86_64")]
            svm: None,
        }
    }

    pub fn set_memory(&mut self, region: MemoryRegion) -> Result<()> {
        // Validate region
        if region.memory_size == 0 {
            return Err(Error::new(EINVAL));
        }

        // Check for overlaps
        for existing in &self.memory_regions {
            let existing_end = existing.guest_phys_addr + existing.memory_size;
            let new_end = region.guest_phys_addr + region.memory_size;

            if region.guest_phys_addr < existing_end && new_end > existing.guest_phys_addr {
                return Err(Error::new(EINVAL));
            }
        }

        self.memory_regions.push(region);
        Ok(())
    }

    pub fn create_vcpu(&mut self, vcpu_id: u32) -> Result<()> {
        if self.vcpus.contains_key(&vcpu_id) {
            return Err(Error::new(EINVAL));
        }

        let vcpu = VcpuState::new(vcpu_id);
        self.vcpus.insert(vcpu_id, vcpu);

        #[cfg(target_arch = "x86_64")]
        {
            // Simple hypervisor detection logic (placeholder)
            // In a real kernel we check CPUID. For now, try VMX, if fail try SVM, or just init both if they are mutually exclusive globally.
            // As this is per-VM, we can try to init based on a flag or just defaults.
            // Let's assume we try VMX first, then SVM.

            if self.vmx.is_none() && self.svm.is_none() {
                // Try VMX
                let mut vmx = Vmx::new();
                if vmx.init().is_ok() {
                    self.vmx = Some(vmx);
                } else {
                    // Try SVM
                    // Svm struct doesn't have init() that can fail in our current stub, but real one would check CPUID.
                    // We'll just instantiate it.
                    self.svm = Some(Svm::new());
                }
            }
        }

        Ok(())
    }

    pub fn run_vcpu(&mut self, vcpu_id: u32) -> Result<VmExitReason> {
        let _vcpu = self.vcpus.get_mut(&vcpu_id).ok_or(Error::new(EINVAL))?;

        #[cfg(target_arch = "x86_64")]
        {
            if let Some(ref mut vmx) = self.vmx {
                return vmx.enter_guest();
            }
            if let Some(ref mut svm) = self.svm {
                return svm.enter_guest();
            }
        }

        // Fallback: simulate a halt exit
        Ok(VmExitReason::Halt)
    }

    pub fn get_vcpu_regs(&self, vcpu_id: u32) -> Result<VcpuState> {
        self.vcpus.get(&vcpu_id).cloned().ok_or(Error::new(EINVAL))
    }

    pub fn set_vcpu_regs(&mut self, vcpu_id: u32, regs: VcpuState) -> Result<()> {
        if let Some(vcpu) = self.vcpus.get_mut(&vcpu_id) {
            *vcpu = regs;
            Ok(())
        } else {
            Err(Error::new(EINVAL))
        }
    }
}

/// VMM Scheme: Provides virtualization interface to userspace
pub struct VmmScheme {
    next_id: AtomicUsize,
    vms: RwLock<BTreeMap<usize, VmInstance>>,
}

impl VmmScheme {
    pub fn new() -> Self {
        Self {
            next_id: AtomicUsize::new(0),
            vms: RwLock::new(BTreeMap::new()),
        }
    }
}

impl KernelScheme for VmmScheme {
    fn kopen(
        &self,
        _path: &str,
        _flags: usize,
        _ctx: CallerCtx,
        _token: &mut CleanLockToken,
    ) -> Result<OpenResult> {
        let vm_id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let vm = VmInstance::new(vm_id);
        self.vms.write().insert(vm_id, vm);
        Ok(OpenResult::SchemeLocal(vm_id, InternalFlags::empty()))
    }

    fn fcntl(
        &self,
        file: usize,
        cmd: usize,
        arg: usize,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        let mut vms = self.vms.write();
        let vm = vms.get_mut(&file).ok_or(Error::new(EBADF))?;

        match cmd {
            VMM_IOCTL_SET_MEMORY => {
                // In a real implementation, we would copy MemoryRegion from userspace
                // For now, create a mock region based on arg
                let region = MemoryRegion {
                    guest_phys_addr: 0,
                    memory_size: arg as u64,
                    userspace_addr: 0,
                    flags: 0,
                };
                vm.set_memory(region)?;
                Ok(0)
            }
            VMM_IOCTL_CREATE_VCPU => {
                let vcpu_id = arg as u32;
                vm.create_vcpu(vcpu_id)?;
                Ok(vcpu_id as usize)
            }
            VMM_IOCTL_RUN_VCPU => {
                let vcpu_id = arg as u32;
                let exit_reason = vm.run_vcpu(vcpu_id)?;
                Ok(exit_reason as usize)
            }
            VMM_IOCTL_GET_REGS => {
                let vcpu_id = arg as u32;
                let _regs = vm.get_vcpu_regs(vcpu_id)?;
                // In a real implementation, copy regs to userspace
                Ok(0)
            }
            VMM_IOCTL_SET_REGS => {
                let vcpu_id = arg as u32;
                // In a real implementation, copy regs from userspace
                let regs = VcpuState::new(vcpu_id);
                vm.set_vcpu_regs(vcpu_id, regs)?;
                Ok(0)
            }
            _ => Err(Error::new(ENOTTY)),
        }
    }

    fn close(&self, file: usize, _token: &mut CleanLockToken) -> Result<()> {
        self.vms.write().remove(&file);
        Ok(())
    }
}

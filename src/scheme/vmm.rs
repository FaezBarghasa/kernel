//! `vmm:` scheme — kernel-level virtualization interface.
//!
//! Provides a file-descriptor-based API for userspace VMMs:
//!   - `open("vmm:")` → creates a new VM instance, returns an fd
//!   - `write(fd, VmmCommand)` → configure memory, create vCPUs, run
//!   - `read(fd, VmmEvent)` → receive VM-exit events, serial output
//!   - `fcntl(fd, IOCTL, arg)` → IOCTL dispatch with userspace pointer copies
//!   - `kfmap(fd, ...)` → map guest physical memory into the VM's EPT/NPT
//!   - `close(fd)` → destroy the VM and free all resources

use alloc::{
    boxed::Box,
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    vec::Vec,
};
use core::{
    mem,
    sync::atomic::{AtomicUsize, Ordering},
};
use spin::RwLock;

use crate::{
    context::{file::InternalFlags, memory::AddrSpaceWrapper},
    scheme::{CallerCtx, KernelScheme, OpenResult},
    sync::CleanLockToken,
    syscall::{
        data::Map,
        error::{Error, Result, EBADF, EINVAL, EIO, ENOTTY},
        flag::MunmapFlags,
        usercopy::{UserSliceRo, UserSliceWo},
    },
};

#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::{
    ept::{EptRoot, EPT_RWX},
    svm::Svm,
    vmx::Vmx,
};

// ─── IOCTL codes ─────────────────────────────────────────────────────────────

pub const VMM_IOCTL_BASE: usize = 0x7000;
pub const VMM_IOCTL_SET_MEMORY: usize = VMM_IOCTL_BASE + 0x01;
pub const VMM_IOCTL_CREATE_VCPU: usize = VMM_IOCTL_BASE + 0x02;
pub const VMM_IOCTL_RUN_VCPU: usize = VMM_IOCTL_BASE + 0x03;
pub const VMM_IOCTL_GET_REGS: usize = VMM_IOCTL_BASE + 0x04;
pub const VMM_IOCTL_SET_REGS: usize = VMM_IOCTL_BASE + 0x05;
pub const VMM_IOCTL_DESTROY_VM: usize = VMM_IOCTL_BASE + 0x06;
pub const VMM_IOCTL_GET_SERIAL: usize = VMM_IOCTL_BASE + 0x07;

// ─── Wire types (repr(C) for userspace ABI) ───────────────────────────────────

/// Serialized command header written to the scheme fd.
///
/// `tag` selects the operation; `vcpu_id` is used by vCPU-specific commands.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct VmmCommandHeader {
    pub tag: u32,
    pub vcpu_id: u32,
}

/// Guest physical memory region descriptor.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct MemoryRegion {
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    /// Host physical address (or userspace virtual, treated as host phys in this impl).
    pub userspace_addr: u64,
    pub flags: u32,
    pub _pad: u32,
}

/// Virtual CPU register state.
///
/// `#[repr(C)]` is **required**: `svm.rs` and `vmx.rs` access fields by
/// hardcoded byte offsets in inline assembly.
///
/// Layout (offsets from struct base, all u64):
///   0x00 rax, 0x08 rbx, 0x10 rcx, 0x18 rdx
///   0x20 rsi, 0x28 rdi, 0x30 rsp, 0x38 rbp
///   0x40 r8,  0x48 r9,  0x50 r10, 0x58 r11
///   0x60 r12, 0x68 r13, 0x70 r14, 0x78 r15
///   0x80 rip, 0x88 rflags
#[derive(Clone, Debug, Default)]
#[repr(C)]
pub struct VcpuState {
    pub rax: u64,    // 0x00
    pub rbx: u64,    // 0x08
    pub rcx: u64,    // 0x10
    pub rdx: u64,    // 0x18
    pub rsi: u64,    // 0x20
    pub rdi: u64,    // 0x28
    pub rsp: u64,    // 0x30
    pub rbp: u64,    // 0x38
    pub r8: u64,     // 0x40
    pub r9: u64,     // 0x48
    pub r10: u64,    // 0x50
    pub r11: u64,    // 0x58
    pub r12: u64,    // 0x60
    pub r13: u64,    // 0x68
    pub r14: u64,    // 0x70
    pub r15: u64,    // 0x78
    pub rip: u64,    // 0x80
    pub rflags: u64, // 0x88
}

impl VcpuState {
    pub fn new() -> Self {
        Self {
            rflags: 0x2, // Reserved bit always set
            ..Default::default()
        }
    }
}

/// High-level VM-exit reason returned to userspace.
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

/// Event read back by userspace after a VM-exit.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct VmmEvent {
    /// High-level exit reason (VmExitReason discriminant).
    pub reason: u32,
    pub vcpu_id: u32,
    /// Exit qualification (port number for I/O, GPA for MMIO, etc.).
    pub exit_info: u64,
}

// ─── VmInstance ───────────────────────────────────────────────────────────────

/// Per-VM state: memory regions, vCPUs, EPT/NPT, and the hardware backend.
pub struct VmInstance {
    pub id: usize,
    /// vCPU states keyed by vCPU ID.
    pub vcpus: BTreeMap<u32, VcpuState>,
    pub memory_regions: Vec<MemoryRegion>,
    /// Captured UART serial output (port 0x3F8 I/O exits).
    pub serial_buf: VecDeque<u8>,
    /// Set when the guest has halted or shut down.
    pub exit_pending: bool,
    /// Pending events to be read back by userspace.
    pub event_queue: VecDeque<VmmEvent>,
    #[cfg(target_arch = "x86_64")]
    pub vmx: Option<Box<Vmx>>,
    #[cfg(target_arch = "x86_64")]
    pub svm: Option<Box<Svm>>,
    /// Standalone EPT root for memory isolation tracking.
    /// Used to register regions before the hardware backend is initialized,
    /// and as the authoritative isolation record for the VM.
    #[cfg(target_arch = "x86_64")]
    pub ept: EptRoot,
}

impl VmInstance {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            vcpus: BTreeMap::new(),
            memory_regions: Vec::new(),
            serial_buf: VecDeque::new(),
            exit_pending: false,
            event_queue: VecDeque::new(),
            #[cfg(target_arch = "x86_64")]
            vmx: None,
            #[cfg(target_arch = "x86_64")]
            svm: None,
            #[cfg(target_arch = "x86_64")]
            ept: EptRoot::new(),
        }
    }

    /// Register a guest physical memory region.
    ///
    /// Rejects zero-size regions and overlaps with existing regions.
    /// On x86_64, immediately maps the region into the standalone EPT root
    /// for isolation tracking.
    pub fn set_memory(&mut self, region: MemoryRegion) -> Result<()> {
        if region.memory_size == 0 {
            return Err(Error::new(EINVAL));
        }
        let new_end = region
            .guest_phys_addr
            .checked_add(region.memory_size)
            .ok_or(Error::new(EINVAL))?;
        for existing in &self.memory_regions {
            let existing_end = existing.guest_phys_addr + existing.memory_size;
            if region.guest_phys_addr < existing_end && new_end > existing.guest_phys_addr {
                return Err(Error::new(EINVAL));
            }
        }
        // Map into the standalone EPT root immediately for isolation tracking.
        #[cfg(target_arch = "x86_64")]
        self.ept.map_range(
            region.guest_phys_addr,
            region.userspace_addr,
            region.memory_size,
            EPT_RWX,
        );
        self.memory_regions.push(region);
        Ok(())
    }

    /// Create a new vCPU with the given ID and initialize the hardware backend.
    ///
    /// On the first vCPU creation, tries VMX then SVM. All already-registered
    /// memory regions are mapped into the hardware backend's EPT/NPT.
    pub fn create_vcpu(&mut self, vcpu_id: u32) -> Result<()> {
        if self.vcpus.contains_key(&vcpu_id) {
            return Err(Error::new(EINVAL));
        }
        self.vcpus.insert(vcpu_id, VcpuState::new());

        #[cfg(target_arch = "x86_64")]
        if self.vmx.is_none() && self.svm.is_none() {
            // Try VMX first; fall back to SVM.
            if let Ok(mut vmx) = unsafe { Vmx::new() } {
                if unsafe { vmx.init() }.is_ok() {
                    let _ = vmx.setup_ept(&self.memory_regions);
                    self.vmx = Some(Box::new(vmx));
                }
            }

            if self.vmx.is_none() {
                if let Ok(mut svm) = unsafe { Svm::new() } {
                    let _ = svm.setup_npt(&self.memory_regions);
                    self.svm = Some(Box::new(svm));
                }
            }
        }

        Ok(())
    }

    /// Run a vCPU until the next VM-exit.
    ///
    /// Drains any serial output from the hardware backend into `self.serial_buf`
    /// and pushes a `VmmEvent` onto `self.event_queue`.
    pub fn run_vcpu(&mut self, vcpu_id: u32) -> Result<VmExitReason> {
        if self.exit_pending {
            return Ok(VmExitReason::Halt);
        }

        let vcpu = self.vcpus.get_mut(&vcpu_id).ok_or(Error::new(EINVAL))?;

        #[cfg(target_arch = "x86_64")]
        {
            if let Some(ref mut vmx) = self.vmx {
                let info = unsafe { vmx.enter_guest(vcpu)? };
                let reason = VmExitReason::from(info.reason);
                // Drain serial bytes from VMX backend.
                while let Some(b) = vmx.serial_buf.pop_front() {
                    self.serial_buf.push_back(b);
                }
                if matches!(reason, VmExitReason::Halt | VmExitReason::Shutdown) {
                    self.exit_pending = true;
                }
                self.event_queue.push_back(VmmEvent {
                    reason: reason as u32,
                    vcpu_id,
                    exit_info: info.qualification,
                });
                return Ok(reason);
            }
            if let Some(ref mut svm) = self.svm {
                let (exitcode, exitinfo1, _) = unsafe { svm.enter_guest(vcpu)? };
                let reason = svm.handle_vmexit(exitcode, exitinfo1, vcpu)?;
                // Drain serial bytes from SVM backend.
                while let Some(b) = svm.serial_buf.pop_front() {
                    self.serial_buf.push_back(b);
                }
                if matches!(reason, VmExitReason::Halt | VmExitReason::Shutdown) {
                    self.exit_pending = true;
                }
                self.event_queue.push_back(VmmEvent {
                    reason: reason as u32,
                    vcpu_id,
                    exit_info: exitinfo1,
                });
                return Ok(reason);
            }
        }

        // No hardware backend available.
        Err(Error::new(EIO))
    }

    /// Run the guest until HLT or shutdown, collecting all serial output.
    pub fn run_until_halt(&mut self, vcpu_id: u32) -> Result<VmExitReason> {
        loop {
            let reason = self.run_vcpu(vcpu_id)?;
            match reason {
                VmExitReason::Halt | VmExitReason::Shutdown => return Ok(reason),
                _ => {}
            }
        }
    }

    pub fn get_vcpu_regs(&self, vcpu_id: u32) -> Result<VcpuState> {
        self.vcpus.get(&vcpu_id).cloned().ok_or(Error::new(EINVAL))
    }

    pub fn set_vcpu_regs(&mut self, vcpu_id: u32, regs: VcpuState) -> Result<()> {
        let vcpu = self.vcpus.get_mut(&vcpu_id).ok_or(Error::new(EINVAL))?;
        *vcpu = regs;
        Ok(())
    }

    /// Drain all captured serial output into a byte vector.
    pub fn drain_serial(&mut self) -> Vec<u8> {
        self.serial_buf.drain(..).collect()
    }
}

// ─── VmmScheme ────────────────────────────────────────────────────────────────

/// The `vmm:` kernel scheme.
///
/// Each `open` call creates a new isolated VM instance.
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
    // ── open ─────────────────────────────────────────────────────────────────

    fn kopen(
        &self,
        _path: &str,
        _flags: usize,
        _ctx: CallerCtx,
        _token: &mut CleanLockToken,
    ) -> Result<OpenResult> {
        let vm_id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.vms.write().insert(vm_id, VmInstance::new(vm_id));
        Ok(OpenResult::SchemeLocal(vm_id, InternalFlags::empty()))
    }

    // ── write: accept VmmCommand structs ─────────────────────────────────────
    //
    // Wire format:
    //   [VmmCommandHeader (8 bytes)] [payload (variable)]
    //
    // tag values:
    //   1 = SetMemory   → payload: MemoryRegion  (32 bytes)
    //   2 = CreateVcpu  → no payload; vcpu_id in header
    //   3 = SetVcpuRegs → payload: VcpuState     (144 bytes)
    //   4 = RunVcpu     → no payload; vcpu_id in header

    fn kwrite(
        &self,
        file: usize,
        buf: UserSliceRo,
        _flags: u32,
        _stored_flags: u32,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        // Maximum command size: header + VcpuState (largest payload).
        const MAX_CMD: usize = mem::size_of::<VmmCommandHeader>() + mem::size_of::<VcpuState>();

        let total = buf.len();
        if total < mem::size_of::<VmmCommandHeader>() {
            return Err(Error::new(EINVAL));
        }
        if total > MAX_CMD {
            return Err(Error::new(EINVAL));
        }

        // Read the entire command into a stack buffer.
        let mut cmd_buf = [0u8; MAX_CMD];
        buf.copy_common_bytes_to_slice(&mut cmd_buf[..total])?;

        // Parse the header.
        // SAFETY: VmmCommandHeader is repr(C), all bit patterns valid, buffer is large enough.
        let hdr: VmmCommandHeader =
            unsafe { core::ptr::read(cmd_buf.as_ptr() as *const VmmCommandHeader) };

        let hdr_size = mem::size_of::<VmmCommandHeader>();

        let mut vms = self.vms.write();
        let vm = vms.get_mut(&file).ok_or(Error::new(EBADF))?;

        match hdr.tag {
            // SetMemory
            1 => {
                let payload_size = mem::size_of::<MemoryRegion>();
                if total < hdr_size + payload_size {
                    return Err(Error::new(EINVAL));
                }
                let region: MemoryRegion =
                    unsafe { core::ptr::read(cmd_buf[hdr_size..].as_ptr() as *const MemoryRegion) };
                vm.set_memory(region)?;
            }
            // CreateVcpu
            2 => {
                vm.create_vcpu(hdr.vcpu_id)?;
            }
            // SetVcpuRegs
            3 => {
                let payload_size = mem::size_of::<VcpuState>();
                if total < hdr_size + payload_size {
                    return Err(Error::new(EINVAL));
                }
                let regs: VcpuState =
                    unsafe { core::ptr::read(cmd_buf[hdr_size..].as_ptr() as *const VcpuState) };
                vm.set_vcpu_regs(hdr.vcpu_id, regs)?;
            }
            // RunVcpu
            4 => {
                vm.run_vcpu(hdr.vcpu_id)?;
            }
            _ => return Err(Error::new(EINVAL)),
        }

        Ok(total)
    }

    // ── read: return VmmEvent structs ─────────────────────────────────────────
    //
    // Each read returns one `VmmEvent` (16 bytes). If the event queue is empty,
    // returns 0 bytes (non-blocking; callers should poll or use fevent).

    fn kread(
        &self,
        file: usize,
        buf: UserSliceWo,
        _flags: u32,
        _stored_flags: u32,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        let event_size = mem::size_of::<VmmEvent>();
        if buf.len() < event_size {
            return Err(Error::new(EINVAL));
        }

        let mut vms = self.vms.write();
        let vm = vms.get_mut(&file).ok_or(Error::new(EBADF))?;

        if let Some(event) = vm.event_queue.pop_front() {
            // SAFETY: VmmEvent is repr(C), all bit patterns valid.
            let event_bytes: [u8; mem::size_of::<VmmEvent>()] =
                unsafe { core::mem::transmute(event) };
            buf.limit(event_size)
                .ok_or(Error::new(EINVAL))?
                .copy_from_slice(&event_bytes)?;
            Ok(event_size)
        } else {
            Ok(0)
        }
    }

    // ── kfmap: map guest physical memory into the VM's EPT ───────────────────
    //
    // `map.offset` carries the guest physical address.
    // `map.size` is the region size in bytes (must be page-aligned).
    // The region is registered as a MemoryRegion and mapped into the EPT.
    // Returns the guest physical address (used as the scheme-local handle).

    fn kfmap(
        &self,
        file: usize,
        _addr_space: &Arc<AddrSpaceWrapper>,
        map: &Map,
        _consume: bool,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        let guest_phys = map.offset as u64;
        let size = map.size as u64;
        if size == 0 || size % 4096 != 0 {
            return Err(Error::new(EINVAL));
        }

        let mut vms = self.vms.write();
        let vm = vms.get_mut(&file).ok_or(Error::new(EBADF))?;

        // Use guest_phys as the host_phys (identity mapping).
        // A production implementation would allocate host frames here.
        let region = MemoryRegion {
            guest_phys_addr: guest_phys,
            memory_size: size,
            userspace_addr: guest_phys,
            flags: 0,
            _pad: 0,
        };
        vm.set_memory(region)?;

        Ok(guest_phys as usize)
    }

    // ── kfunmap: remove a previously mapped guest physical region ─────────────

    fn kfunmap(
        &self,
        number: usize,
        _offset: usize,
        size: usize,
        _flags: MunmapFlags,
        _token: &mut CleanLockToken,
    ) -> Result<()> {
        let guest_phys = number as u64;
        let size = size as u64;
        let mut vms = self.vms.write();
        for vm in vms.values_mut() {
            vm.memory_regions
                .retain(|r| !(r.guest_phys_addr == guest_phys && r.memory_size == size));
        }
        Ok(())
    }

    // ── fcntl: IOCTL dispatch with userspace pointer copies ──────────────────

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
                // `arg` is a userspace pointer to a MemoryRegion.
                let region = unsafe { core::ptr::read_volatile(arg as *const MemoryRegion) };
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
                let reason = vm.run_vcpu(vcpu_id)?;
                Ok(reason as usize)
            }
            VMM_IOCTL_GET_REGS => {
                // arg encodes: high 32 bits = vcpu_id, low 32 bits = output pointer.
                let vcpu_id = (arg >> 32) as u32;
                let out_ptr = (arg & 0xFFFF_FFFF) as usize;
                let regs = vm.get_vcpu_regs(vcpu_id)?;
                unsafe {
                    core::ptr::write_volatile(out_ptr as *mut VcpuState, regs);
                }
                Ok(0)
            }
            VMM_IOCTL_SET_REGS => {
                // arg encodes: high 32 bits = vcpu_id, low 32 bits = input pointer.
                let vcpu_id = (arg >> 32) as u32;
                let ptr = (arg & 0xFFFF_FFFF) as usize;
                let regs = unsafe { core::ptr::read_volatile(ptr as *const VcpuState) };
                vm.set_vcpu_regs(vcpu_id, regs)?;
                Ok(0)
            }
            VMM_IOCTL_DESTROY_VM => {
                // Mark the VM as terminated; actual cleanup happens on close().
                vm.exit_pending = true;
                Ok(0)
            }
            VMM_IOCTL_GET_SERIAL => {
                // arg is a userspace pointer to a [u8; 64] buffer.
                // Returns the number of bytes written.
                let out_ptr = arg as *mut u8;
                let bytes = vm.drain_serial();
                let n = bytes.len().min(64);
                unsafe {
                    core::ptr::copy_nonoverlapping(bytes.as_ptr(), out_ptr, n);
                }
                Ok(n)
            }
            _ => Err(Error::new(ENOTTY)),
        }
    }

    // ── close: destroy the VM and free all resources ──────────────────────────

    fn close(&self, file: usize, _token: &mut CleanLockToken) -> Result<()> {
        self.vms.write().remove(&file);
        Ok(())
    }
}

#[cfg(test)]
#[path = "vmm/tests.rs"]
mod tests;

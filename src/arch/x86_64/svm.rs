//! SVM (AMD-V) — Full VMCB Management, NPT, and VM-Exit Handling
//!
//! Implements the complete SVM lifecycle:
//!   1. Check CPUID for SVM support and set EFER.SVME
//!   2. Allocate host save area and VMCB
//!   3. Configure VMCB: intercepts, guest state, NPT
//!   4. VMRUN loop with VMEXIT dispatch
//!   5. Teardown: clear EFER.SVME
//!
//! # Safety
//! All SVM instructions are inherently unsafe and isolated to `unsafe` blocks.
//! `rbx` and `rbp` are LLVM-reserved and must never appear as named asm operands;
//! they are saved/restored on the stack inside asm blocks instead.

#![allow(dead_code)]

use alloc::{boxed::Box, collections::VecDeque};
use core::arch::asm;

use crate::{
    arch::x86_64::npt::{NptRoot, NPT_RWX},
    scheme::vmm::{MemoryRegion, VcpuState, VmExitReason},
    syscall::error::{Error, Result, EINVAL, EIO, ENODEV},
};

// ─── MSR addresses ────────────────────────────────────────────────────────────

const MSR_EFER: u32 = 0xC000_0080;
const MSR_VM_HSAVE: u32 = 0xC001_0117;
const EFER_SVME: u64 = 1 << 12;

// ─── VMCB control area offsets (AMD APM Vol 2, Appendix B) ───────────────────

const VMCB_INTERCEPT_CR: usize = 0x000;
const VMCB_INTERCEPT_DR: usize = 0x004;
const VMCB_INTERCEPT_EXCEPTION: usize = 0x008;
const VMCB_INTERCEPT_MISC1: usize = 0x00C;
const VMCB_INTERCEPT_MISC2: usize = 0x010;
const VMCB_IOPM_BASE: usize = 0x040;
const VMCB_MSRPM_BASE: usize = 0x048;
const VMCB_TSC_OFFSET: usize = 0x050;
const VMCB_ASID: usize = 0x058;
const VMCB_TLB_CONTROL: usize = 0x05C;
const VMCB_VINTR: usize = 0x060;
const VMCB_INTR_SHADOW: usize = 0x068;
const VMCB_EXITCODE: usize = 0x070;
const VMCB_EXITINFO1: usize = 0x078;
const VMCB_EXITINFO2: usize = 0x080;
const VMCB_EXITINTINFO: usize = 0x088;
const VMCB_NP_ENABLE: usize = 0x090;
const VMCB_AVIC_APIC_BAR: usize = 0x098;
const VMCB_N_CR3: usize = 0x0B0;
const VMCB_LBR_VIRT: usize = 0x0B8;
const VMCB_VMCB_CLEAN: usize = 0x0C0;
const VMCB_NEXT_RIP: usize = 0x0C8;
const VMCB_INSN_BYTES: usize = 0x0D0;

// ─── VMCB state save area offsets ────────────────────────────────────────────

const VMCB_SS_ES_SEL: usize = 0x400;
const VMCB_SS_ES_ATTR: usize = 0x402;
const VMCB_SS_ES_LIMIT: usize = 0x404;
const VMCB_SS_ES_BASE: usize = 0x408;
const VMCB_SS_CS_SEL: usize = 0x410;
const VMCB_SS_CS_ATTR: usize = 0x412;
const VMCB_SS_CS_LIMIT: usize = 0x414;
const VMCB_SS_CS_BASE: usize = 0x418;
const VMCB_SS_SS_SEL: usize = 0x420;
const VMCB_SS_SS_ATTR: usize = 0x422;
const VMCB_SS_SS_LIMIT: usize = 0x424;
const VMCB_SS_SS_BASE: usize = 0x428;
const VMCB_SS_DS_SEL: usize = 0x430;
const VMCB_SS_DS_ATTR: usize = 0x432;
const VMCB_SS_DS_LIMIT: usize = 0x434;
const VMCB_SS_DS_BASE: usize = 0x438;
const VMCB_SS_FS_SEL: usize = 0x440;
const VMCB_SS_FS_ATTR: usize = 0x442;
const VMCB_SS_FS_LIMIT: usize = 0x444;
const VMCB_SS_FS_BASE: usize = 0x448;
const VMCB_SS_GS_SEL: usize = 0x450;
const VMCB_SS_GS_ATTR: usize = 0x452;
const VMCB_SS_GS_LIMIT: usize = 0x454;
const VMCB_SS_GS_BASE: usize = 0x458;
const VMCB_SS_GDTR_ATTR: usize = 0x462;
const VMCB_SS_GDTR_LIMIT: usize = 0x464;
const VMCB_SS_GDTR_BASE: usize = 0x468;
const VMCB_SS_LDTR_SEL: usize = 0x470;
const VMCB_SS_LDTR_ATTR: usize = 0x472;
const VMCB_SS_LDTR_LIMIT: usize = 0x474;
const VMCB_SS_LDTR_BASE: usize = 0x478;
const VMCB_SS_IDTR_ATTR: usize = 0x482;
const VMCB_SS_IDTR_LIMIT: usize = 0x484;
const VMCB_SS_IDTR_BASE: usize = 0x488;
const VMCB_SS_TR_SEL: usize = 0x490;
const VMCB_SS_TR_ATTR: usize = 0x492;
const VMCB_SS_TR_LIMIT: usize = 0x494;
const VMCB_SS_TR_BASE: usize = 0x498;
const VMCB_SS_CPL: usize = 0x4CB;
const VMCB_SS_EFER: usize = 0x4D0;
const VMCB_SS_CR4: usize = 0x548;
const VMCB_SS_CR3: usize = 0x550;
const VMCB_SS_CR0: usize = 0x558;
const VMCB_SS_DR7: usize = 0x560;
const VMCB_SS_DR6: usize = 0x568;
const VMCB_SS_RFLAGS: usize = 0x570;
const VMCB_SS_RIP: usize = 0x578;
const VMCB_SS_RSP: usize = 0x5D8;
const VMCB_SS_RAX: usize = 0x5F8;
const VMCB_SS_PAT: usize = 0x668;

// ─── Intercept bits ───────────────────────────────────────────────────────────

const INTERCEPT_INTR: u32 = 1 << 0;
const INTERCEPT_NMI: u32 = 1 << 1;
const INTERCEPT_CPUID: u32 = 1 << 18;
const INTERCEPT_HLT: u32 = 1 << 24;
const INTERCEPT_IOIO: u32 = 1 << 27;
const INTERCEPT_MSR: u32 = 1 << 28;
const INTERCEPT_SHUTDOWN: u32 = 1 << 31;
const INTERCEPT_VMRUN: u32 = 1 << 0; // MISC2
const INTERCEPT_VMMCALL: u32 = 1 << 1; // MISC2

// ─── Exit codes ───────────────────────────────────────────────────────────────

const VMEXIT_INTR: u64 = 0x60;
const VMEXIT_NMI: u64 = 0x61;
const VMEXIT_CPUID: u64 = 0x72;
const VMEXIT_HLT: u64 = 0x78;
const VMEXIT_IOIO: u64 = 0x7B;
const VMEXIT_MSR: u64 = 0x7C;
const VMEXIT_SHUTDOWN: u64 = 0x7F;
const VMEXIT_NPFAULT: u64 = 0x400;

const UART_TX_PORT: u16 = 0x3F8;

// ─── VcpuState field offsets (must match #[repr(C)] VcpuState in vmm.rs) ─────
//
// VcpuState layout (all u64, 8 bytes each):
//   0x00 rax, 0x08 rbx, 0x10 rcx, 0x18 rdx
//   0x20 rsi, 0x28 rdi, 0x30 rsp, 0x38 rbp
//   0x40 r8,  0x48 r9,  0x50 r10, 0x58 r11
//   0x60 r12, 0x68 r13, 0x70 r14, 0x78 r15
//   0x80 rip, 0x88 rflags

const VCPU_RAX: usize = 0x00;
const VCPU_RBX: usize = 0x08;
const VCPU_RCX: usize = 0x10;
const VCPU_RDX: usize = 0x18;
const VCPU_RSI: usize = 0x20;
const VCPU_RDI: usize = 0x28;
const VCPU_RSP: usize = 0x30;
const VCPU_RBP: usize = 0x38;
const VCPU_R8: usize = 0x40;
const VCPU_R9: usize = 0x48;
const VCPU_R10: usize = 0x50;
const VCPU_R11: usize = 0x58;
const VCPU_R12: usize = 0x60;
const VCPU_R13: usize = 0x68;
const VCPU_R14: usize = 0x70;
const VCPU_R15: usize = 0x78;

// ─── VMCB page ────────────────────────────────────────────────────────────────

/// A 4KB-aligned VMCB or host save area page.
#[repr(C, align(4096))]
pub struct VmcbPage {
    data: [u8; 4096],
}

impl VmcbPage {
    pub fn new() -> Box<Self> {
        Box::new(Self { data: [0u8; 4096] })
    }

    pub fn phys_addr(&self) -> u64 {
        use crate::arch::x86_64::consts::PHYS_OFFSET;
        let virt = self as *const _ as u64;
        virt - PHYS_OFFSET as u64
    }

    pub fn read_u8(&self, off: usize) -> u8 {
        self.data[off]
    }
    pub fn read_u16(&self, off: usize) -> u16 {
        u16::from_le_bytes([self.data[off], self.data[off + 1]])
    }
    pub fn read_u32(&self, off: usize) -> u32 {
        u32::from_le_bytes(self.data[off..off + 4].try_into().unwrap())
    }
    pub fn read_u64(&self, off: usize) -> u64 {
        u64::from_le_bytes(self.data[off..off + 8].try_into().unwrap())
    }
    pub fn write_u8(&mut self, off: usize, v: u8) {
        self.data[off] = v;
    }
    pub fn write_u16(&mut self, off: usize, v: u16) {
        self.data[off..off + 2].copy_from_slice(&v.to_le_bytes());
    }
    pub fn write_u32(&mut self, off: usize, v: u32) {
        self.data[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    pub fn write_u64(&mut self, off: usize, v: u64) {
        self.data[off..off + 8].copy_from_slice(&v.to_le_bytes());
    }
}

// ─── SVM instructions ─────────────────────────────────────────────────────────

/// Execute VMRUN with the given VMCB physical address.
///
/// # Safety
/// EFER.SVME must be set, VMCB must be valid and 4KB-aligned.
#[inline]
pub unsafe fn vmrun(vmcb_phys: u64) {
    unsafe {
        asm!("vmrun", in("rax") vmcb_phys, options(nostack));
    }
}

/// Execute VMSAVE to save host state to the given physical address.
///
/// # Safety
/// EFER.SVME must be set.
#[inline]
pub unsafe fn vmsave(phys: u64) {
    unsafe {
        asm!("vmsave", in("rax") phys, options(nostack));
    }
}

/// Execute VMLOAD to load guest state from the given physical address.
///
/// # Safety
/// EFER.SVME must be set.
#[inline]
pub unsafe fn vmload(phys: u64) {
    unsafe {
        asm!("vmload", in("rax") phys, options(nostack));
    }
}

/// Read an MSR.
///
/// # Safety
/// Must be called with a valid MSR address.
#[inline]
unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") lo,
            out("edx") hi,
            options(nostack, nomem)
        );
    }
    ((hi as u64) << 32) | lo as u64
}

/// Write an MSR.
///
/// # Safety
/// Must be called with a valid MSR address and value.
#[inline]
unsafe fn wrmsr(msr: u32, val: u64) {
    let lo = val as u32;
    let hi = (val >> 32) as u32;
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") lo,
            in("edx") hi,
            options(nostack, nomem)
        );
    }
}

// ─── Main Svm struct ─────────────────────────────────────────────────────────

/// Per-VM SVM state.
pub struct Svm {
    vmcb: Box<VmcbPage>,
    host_save: Box<VmcbPage>,
    npt: NptRoot,
    active: bool,
    /// Captured serial output bytes (UART port 0x3F8).
    pub serial_buf: VecDeque<u8>,
}

impl Svm {
    /// Create and initialize a new SVM instance.
    ///
    /// Checks CPUID for SVM support, sets EFER.SVME, allocates VMCB and host save area.
    ///
    /// # Safety
    /// Must be called on an AMD CPU with SVM support.
    pub unsafe fn new() -> Result<Self> {
        // Check CPUID[0x80000001].ECX bit 2 for SVM support.
        // rbx cannot be a named asm operand (LLVM-reserved); push/pop it inside the block.
        let svm_support: u32;
        unsafe {
            asm!(
                "push rbx",
                "mov eax, 0x80000001",
                "cpuid",
                "pop rbx",
                out("eax") _,
                out("ecx") svm_support,
                out("edx") _,
                options(nostack, nomem)
            );
        }
        if svm_support & (1 << 2) == 0 {
            return Err(Error::new(ENODEV));
        }

        // Set EFER.SVME.
        let efer = unsafe { rdmsr(MSR_EFER) };
        unsafe {
            wrmsr(MSR_EFER, efer | EFER_SVME);
        }

        let host_save = VmcbPage::new();
        let host_phys = host_save.phys_addr();

        // Write host save area physical address to MSR_VM_HSAVE.
        unsafe {
            wrmsr(MSR_VM_HSAVE, host_phys);
        }

        // Save host state into the host save area.
        unsafe {
            vmsave(host_phys);
        }

        Ok(Self {
            vmcb: VmcbPage::new(),
            host_save,
            npt: NptRoot::new(1), // ASID 1
            active: true,
            serial_buf: VecDeque::new(),
        })
    }

    /// Map guest memory regions into the NPT.
    pub fn setup_npt(&mut self, regions: &[MemoryRegion]) -> Result<()> {
        for region in regions {
            if region.memory_size == 0 {
                return Err(Error::new(EINVAL));
            }
            self.npt.map_range(
                region.guest_phys_addr,
                region.userspace_addr,
                region.memory_size,
                NPT_RWX,
            );
        }
        Ok(())
    }

    /// Configure VMCB intercepts, guest state, and NPT.
    pub fn setup(&mut self, vcpu: &VcpuState, regions: &[MemoryRegion]) -> Result<()> {
        self.setup_npt(regions)?;
        self.setup_intercepts();
        self.setup_guest_state(vcpu);
        self.setup_npt_control();
        Ok(())
    }

    fn setup_intercepts(&mut self) {
        let misc1 = INTERCEPT_INTR
            | INTERCEPT_NMI
            | INTERCEPT_CPUID
            | INTERCEPT_HLT
            | INTERCEPT_IOIO
            | INTERCEPT_MSR
            | INTERCEPT_SHUTDOWN;
        let misc2 = INTERCEPT_VMRUN | INTERCEPT_VMMCALL;
        self.vmcb.write_u32(VMCB_INTERCEPT_MISC1, misc1);
        self.vmcb.write_u32(VMCB_INTERCEPT_MISC2, misc2);
        self.vmcb.write_u32(VMCB_ASID, 1);
        self.vmcb.write_u8(VMCB_TLB_CONTROL, 1);
    }

    fn setup_guest_state(&mut self, vcpu: &VcpuState) {
        let efer: u64 = 0xD01;
        let cr0: u64 = 0x80050033;
        let cr4: u64 = 0x000006A0;
        let cs_attr: u16 = 0x029B;
        let ds_attr: u16 = 0x0C93;
        let tr_attr: u16 = 0x008B;
        let ldtr_attr: u16 = 0x0082;

        self.vmcb.write_u64(VMCB_SS_EFER, efer);
        self.vmcb.write_u64(VMCB_SS_CR0, cr0);
        self.vmcb.write_u64(VMCB_SS_CR3, 0);
        self.vmcb.write_u64(VMCB_SS_CR4, cr4);
        self.vmcb.write_u64(VMCB_SS_DR7, 0x400);
        self.vmcb.write_u64(VMCB_SS_DR6, 0xFFFF0FF0);
        self.vmcb.write_u64(VMCB_SS_RFLAGS, vcpu.rflags);
        self.vmcb.write_u64(VMCB_SS_RIP, vcpu.rip);
        self.vmcb.write_u64(VMCB_SS_RSP, vcpu.rsp);
        self.vmcb.write_u64(VMCB_SS_RAX, vcpu.rax);
        self.vmcb.write_u8(VMCB_SS_CPL, 0);

        // CS
        self.vmcb.write_u16(VMCB_SS_CS_SEL, 0x08);
        self.vmcb.write_u16(VMCB_SS_CS_ATTR, cs_attr);
        self.vmcb.write_u32(VMCB_SS_CS_LIMIT, 0xFFFF_FFFF);
        self.vmcb.write_u64(VMCB_SS_CS_BASE, 0);

        // DS/ES/FS/GS/SS
        for (sel, attr, limit, base) in [
            (
                VMCB_SS_DS_SEL,
                VMCB_SS_DS_ATTR,
                VMCB_SS_DS_LIMIT,
                VMCB_SS_DS_BASE,
            ),
            (
                VMCB_SS_ES_SEL,
                VMCB_SS_ES_ATTR,
                VMCB_SS_ES_LIMIT,
                VMCB_SS_ES_BASE,
            ),
            (
                VMCB_SS_FS_SEL,
                VMCB_SS_FS_ATTR,
                VMCB_SS_FS_LIMIT,
                VMCB_SS_FS_BASE,
            ),
            (
                VMCB_SS_GS_SEL,
                VMCB_SS_GS_ATTR,
                VMCB_SS_GS_LIMIT,
                VMCB_SS_GS_BASE,
            ),
            (
                VMCB_SS_SS_SEL,
                VMCB_SS_SS_ATTR,
                VMCB_SS_SS_LIMIT,
                VMCB_SS_SS_BASE,
            ),
        ] {
            self.vmcb.write_u16(sel, 0x10);
            self.vmcb.write_u16(attr, ds_attr);
            self.vmcb.write_u32(limit, 0xFFFF_FFFF);
            self.vmcb.write_u64(base, 0);
        }

        // TR
        self.vmcb.write_u16(VMCB_SS_TR_SEL, 0x18);
        self.vmcb.write_u16(VMCB_SS_TR_ATTR, tr_attr);
        self.vmcb.write_u32(VMCB_SS_TR_LIMIT, 0x67);
        self.vmcb.write_u64(VMCB_SS_TR_BASE, 0);

        // LDTR
        self.vmcb.write_u16(VMCB_SS_LDTR_SEL, 0);
        self.vmcb.write_u16(VMCB_SS_LDTR_ATTR, ldtr_attr);
        self.vmcb.write_u32(VMCB_SS_LDTR_LIMIT, 0);
        self.vmcb.write_u64(VMCB_SS_LDTR_BASE, 0);

        // GDTR / IDTR
        self.vmcb.write_u32(VMCB_SS_GDTR_LIMIT, 0x27);
        self.vmcb.write_u64(VMCB_SS_GDTR_BASE, 0);
        self.vmcb.write_u32(VMCB_SS_IDTR_LIMIT, 0xFFF);
        self.vmcb.write_u64(VMCB_SS_IDTR_BASE, 0);

        // PAT default
        self.vmcb.write_u64(VMCB_SS_PAT, 0x0007040600070406);
    }

    fn setup_npt_control(&mut self) {
        self.vmcb.write_u64(VMCB_NP_ENABLE, 1);
        self.vmcb.write_u64(VMCB_N_CR3, self.npt.ncr3());
    }

    fn sync_guest_gprs_to_vmcb(&mut self, vcpu: &VcpuState) {
        self.vmcb.write_u64(VMCB_SS_RAX, vcpu.rax);
        self.vmcb.write_u64(VMCB_SS_RSP, vcpu.rsp);
        self.vmcb.write_u64(VMCB_SS_RIP, vcpu.rip);
        self.vmcb.write_u64(VMCB_SS_RFLAGS, vcpu.rflags);
    }

    fn sync_guest_gprs_from_vmcb(&self, vcpu: &mut VcpuState) {
        vcpu.rax = self.vmcb.read_u64(VMCB_SS_RAX);
        vcpu.rsp = self.vmcb.read_u64(VMCB_SS_RSP);
        vcpu.rip = self.vmcb.read_u64(VMCB_SS_RIP);
        vcpu.rflags = self.vmcb.read_u64(VMCB_SS_RFLAGS);
    }

    /// Execute VMRUN, save/restore host state, and return exit info.
    ///
    /// # Safety
    /// `setup()` must have been called.
    ///
    /// # Register strategy
    /// `rbx` and `rbp` are LLVM-reserved and cannot be named asm operands.
    /// We save/restore them on the stack inside the asm block.
    ///
    /// The vcpu pointer is passed in `r10` (caller-saved, not LLVM-reserved).
    /// Before loading guest r10, we push the vcpu pointer onto the stack so we
    /// can recover it after VMRUN to write back the guest GPRs.
    ///
    /// Stack layout during VMRUN (after the 3 entry pushes):
    ///   [rsp+0]  = vcpu_ptr   (pushed last, so at top)
    ///   [rsp+8]  = host rbp
    ///   [rsp+16] = host rbx
    pub unsafe fn enter_guest(&mut self, vcpu: &mut VcpuState) -> Result<(u64, u64, u64)> {
        if !self.active {
            return Err(Error::new(EIO));
        }

        self.sync_guest_gprs_to_vmcb(vcpu);

        let vmcb_phys = self.vmcb.phys_addr();
        let host_phys = self.host_save.phys_addr();
        let vcpu_ptr = vcpu as *mut VcpuState as u64;

        unsafe {
            asm!(
                // ── Entry: save host rbx, rbp, and the vcpu pointer ──────────
                "push rbx",          // [rsp+16] = host rbx
                "push rbp",          // [rsp+8]  = host rbp
                "push r10",          // [rsp+0]  = vcpu_ptr (r10 = input operand)

                // ── Load guest GPRs from VcpuState[r10] ──────────────────────
                // rbx and rbp are loaded via memory (not named operands).
                // r10 is used as the pointer; we load guest r10 last.
                "mov rbx, [r10 + 0x08]",  // vcpu.rbx
                "mov rcx, [r10 + 0x10]",  // vcpu.rcx
                "mov rdx, [r10 + 0x18]",  // vcpu.rdx
                "mov rsi, [r10 + 0x20]",  // vcpu.rsi
                "mov rdi, [r10 + 0x28]",  // vcpu.rdi
                // rsp is managed by VMCB; skip.
                "mov rbp, [r10 + 0x38]",  // vcpu.rbp
                "mov r8,  [r10 + 0x40]",  // vcpu.r8
                "mov r9,  [r10 + 0x48]",  // vcpu.r9
                // r10 guest value loaded last (after we no longer need it as ptr).
                "mov r11, [r10 + 0x58]",  // vcpu.r11 (loaded into r11 temporarily)
                "mov r12, [r10 + 0x60]",  // vcpu.r12
                "mov r13, [r10 + 0x68]",  // vcpu.r13
                "mov r14, [r10 + 0x70]",  // vcpu.r14
                "mov r15, [r10 + 0x78]",  // vcpu.r15
                "mov r10, [r10 + 0x50]",  // vcpu.r10 — overwrites our pointer

                // ── VMLOAD + VMRUN + VMSAVE ───────────────────────────────────
                // rax = vmcb_phys (input operand, untouched so far).
                "vmload rax",
                "vmrun rax",
                // On VMEXIT, hardware restores host state from the host save area.
                "vmsave rax",

                // ── Save guest GPRs back to VcpuState ────────────────────────
                // We need the vcpu pointer. It's at [rsp+0] (top of stack).
                // Use rax as a scratch register (vmcb_phys is no longer needed).
                "mov rax, [rsp]",         // rax = vcpu_ptr
                "mov [rax + 0x08], rbx",  // vcpu.rbx
                "mov [rax + 0x10], rcx",  // vcpu.rcx
                "mov [rax + 0x18], rdx",  // vcpu.rdx
                "mov [rax + 0x20], rsi",  // vcpu.rsi
                "mov [rax + 0x28], rdi",  // vcpu.rdi
                // rsp is synced from VMCB after the asm block; skip here.
                "mov [rax + 0x38], rbp",  // vcpu.rbp
                "mov [rax + 0x40], r8",   // vcpu.r8
                "mov [rax + 0x48], r9",   // vcpu.r9
                "mov [rax + 0x50], r10",  // vcpu.r10
                "mov [rax + 0x58], r11",  // vcpu.r11
                "mov [rax + 0x60], r12",  // vcpu.r12
                "mov [rax + 0x68], r13",  // vcpu.r13
                "mov [rax + 0x70], r14",  // vcpu.r14
                "mov [rax + 0x78], r15",  // vcpu.r15

                // ── Exit: restore host rbx and rbp ───────────────────────────
                // Stack: [rsp+0]=vcpu_ptr, [rsp+8]=host rbp, [rsp+16]=host rbx
                "add rsp, 8",             // discard vcpu_ptr slot
                "pop rbp",                // restore host rbp
                "pop rbx",                // restore host rbx

                // Inputs and clobbers.
                inout("rax") vmcb_phys => _,
                inout("r10") vcpu_ptr => _,
                // Clobbers: all caller-saved regs (they hold guest values after VMRUN).
                out("rcx") _,
                out("rdx") _,
                out("rsi") _,
                out("rdi") _,
                out("r8")  _,
                out("r9")  _,
                out("r11") _,
                out("r12") _,
                out("r13") _,
                out("r14") _,
                out("r15") _,
                options(nostack)
            );
        }

        // Restore host state (VMLOAD from host save area).
        unsafe {
            vmload(host_phys);
        }

        // Sync RAX, RSP, RIP, RFLAGS from VMCB (they're not saved in the asm above).
        self.sync_guest_gprs_from_vmcb(vcpu);

        let exitcode = self.vmcb.read_u64(VMCB_EXITCODE);
        let exitinfo1 = self.vmcb.read_u64(VMCB_EXITINFO1);
        let exitinfo2 = self.vmcb.read_u64(VMCB_EXITINFO2);

        Ok((exitcode, exitinfo1, exitinfo2))
    }

    /// Handle a VMEXIT, returning the high-level `VmExitReason`.
    pub fn handle_vmexit(
        &mut self,
        exitcode: u64,
        exitinfo1: u64,
        vcpu: &mut VcpuState,
    ) -> Result<VmExitReason> {
        let next_rip = self.vmcb.read_u64(VMCB_NEXT_RIP);

        match exitcode {
            VMEXIT_INTR => Ok(VmExitReason::ExternalInterrupt),
            VMEXIT_HLT => {
                if next_rip != 0 {
                    vcpu.rip = next_rip;
                }
                Ok(VmExitReason::Halt)
            }
            VMEXIT_SHUTDOWN => Ok(VmExitReason::Shutdown),
            VMEXIT_IOIO => {
                // exitinfo1 bit 0: 1=IN, 0=OUT; bits [31:16]: port number.
                let is_in = exitinfo1 & 1 != 0;
                let port = ((exitinfo1 >> 16) & 0xFFFF) as u16;
                if !is_in && port == UART_TX_PORT {
                    self.serial_buf.push_back(vcpu.rax as u8);
                }
                if next_rip != 0 {
                    vcpu.rip = next_rip;
                }
                Ok(VmExitReason::IoInstruction)
            }
            VMEXIT_MSR => {
                // exitinfo1: 0=RDMSR, 1=WRMSR.
                if exitinfo1 == 0 {
                    vcpu.rax = 0;
                    vcpu.rdx = 0;
                }
                if next_rip != 0 {
                    vcpu.rip = next_rip;
                }
                Ok(VmExitReason::Unknown)
            }
            VMEXIT_CPUID => {
                vcpu.rax = 0;
                vcpu.rbx = 0;
                vcpu.rcx = 0;
                vcpu.rdx = 0;
                if next_rip != 0 {
                    vcpu.rip = next_rip;
                }
                Ok(VmExitReason::Unknown)
            }
            VMEXIT_NPFAULT => Err(Error::new(EINVAL)),
            _ => {
                if next_rip != 0 {
                    vcpu.rip = next_rip;
                }
                Ok(VmExitReason::Unknown)
            }
        }
    }

    /// Run the guest until HLT or shutdown.
    ///
    /// # Safety
    /// `setup()` must have been called.
    pub unsafe fn run_until_halt(&mut self, vcpu: &mut VcpuState) -> Result<VmExitReason> {
        loop {
            let (exitcode, exitinfo1, _) = unsafe { self.enter_guest(vcpu)? };
            let reason = self.handle_vmexit(exitcode, exitinfo1, vcpu)?;
            match reason {
                VmExitReason::Halt | VmExitReason::Shutdown => return Ok(reason),
                _ => {}
            }
        }
    }
}

impl Drop for Svm {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                let efer = rdmsr(MSR_EFER);
                wrmsr(MSR_EFER, efer & !EFER_SVME);
            }
            self.active = false;
        }
    }
}

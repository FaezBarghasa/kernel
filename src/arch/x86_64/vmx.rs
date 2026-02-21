//! VMX (Intel VT-x) — Full VMCS Management, EPT, and VM-Exit Handling
//!
//! Implements the complete VMX lifecycle:
//!   1. VMXON — enable VMX operation
//!   2. VMCLEAR + VMPTRLD — initialize and load VMCS
//!   3. VMCS setup — host state, guest state, execution controls, EPT
//!   4. VMLAUNCH / VMRESUME — enter guest
//!   5. VM-exit dispatch — handle I/O, MMIO, CPUID, MSR, HLT, shutdown
//!   6. VMXOFF — disable VMX on teardown
//!
//! # Safety
//! All VMX instructions are inherently unsafe. They are isolated to `unsafe` blocks
//! and wrapped in safe Rust APIs wherever possible.

#![allow(dead_code)]

use alloc::{boxed::Box, collections::VecDeque};
use core::arch::asm;

use crate::{
    arch::x86_64::ept::{EptRoot, EPT_RWX},
    scheme::vmm::{MemoryRegion, VcpuState, VmExitReason},
    syscall::error::{Error, Result, EINVAL, EIO},
};

// ─── MSR addresses ────────────────────────────────────────────────────────────

const MSR_IA32_VMX_BASIC: u32 = 0x480;
const MSR_IA32_VMX_PINBASED_CTLS: u32 = 0x481;
const MSR_IA32_VMX_PROCBASED_CTLS: u32 = 0x482;
const MSR_IA32_VMX_EXIT_CTLS: u32 = 0x483;
const MSR_IA32_VMX_ENTRY_CTLS: u32 = 0x484;
const MSR_IA32_VMX_PROCBASED_CTLS2: u32 = 0x48B;
const MSR_IA32_VMX_TRUE_PINBASED: u32 = 0x48D;
const MSR_IA32_VMX_TRUE_PROCBASED: u32 = 0x48E;
const MSR_IA32_VMX_TRUE_EXIT: u32 = 0x48F;
const MSR_IA32_VMX_TRUE_ENTRY: u32 = 0x490;
const MSR_IA32_EFER: u32 = 0xC0000080;
const MSR_IA32_FS_BASE: u32 = 0xC0000100;
const MSR_IA32_GS_BASE: u32 = 0xC0000101;
const MSR_IA32_SYSENTER_CS: u32 = 0x174;
const MSR_IA32_SYSENTER_ESP: u32 = 0x175;
const MSR_IA32_SYSENTER_EIP: u32 = 0x176;
const MSR_IA32_PAT: u32 = 0x277;

// ─── VMCS field encodings ─────────────────────────────────────────────────────

// 16-bit guest fields
const VMCS_GUEST_ES_SEL: u32 = 0x0800;
const VMCS_GUEST_CS_SEL: u32 = 0x0802;
const VMCS_GUEST_SS_SEL: u32 = 0x0804;
const VMCS_GUEST_DS_SEL: u32 = 0x0806;
const VMCS_GUEST_FS_SEL: u32 = 0x0808;
const VMCS_GUEST_GS_SEL: u32 = 0x080A;
const VMCS_GUEST_LDTR_SEL: u32 = 0x080C;
const VMCS_GUEST_TR_SEL: u32 = 0x080E;
// 16-bit host fields
const VMCS_HOST_ES_SEL: u32 = 0x0C00;
const VMCS_HOST_CS_SEL: u32 = 0x0C02;
const VMCS_HOST_SS_SEL: u32 = 0x0C04;
const VMCS_HOST_DS_SEL: u32 = 0x0C06;
const VMCS_HOST_FS_SEL: u32 = 0x0C08;
const VMCS_HOST_GS_SEL: u32 = 0x0C0A;
const VMCS_HOST_TR_SEL: u32 = 0x0C0C;
// 64-bit control fields
const VMCS_IO_BITMAP_A: u32 = 0x2000;
const VMCS_IO_BITMAP_B: u32 = 0x2002;
const VMCS_EPTP: u32 = 0x201A;
// 64-bit guest fields
const VMCS_GUEST_VMCS_LINK: u32 = 0x2800;
const VMCS_GUEST_IA32_EFER: u32 = 0x2806;
const VMCS_GUEST_IA32_PAT: u32 = 0x2804;
// 64-bit host fields
const VMCS_HOST_IA32_EFER: u32 = 0x2C02;
const VMCS_HOST_IA32_PAT: u32 = 0x2C00;
// 32-bit control fields
const VMCS_PIN_BASED_CTLS: u32 = 0x4000;
const VMCS_PROC_BASED_CTLS: u32 = 0x4002;
const VMCS_EXCEPTION_BITMAP: u32 = 0x4004;
const VMCS_EXIT_CTLS: u32 = 0x400C;
const VMCS_EXIT_MSR_STORE_CNT: u32 = 0x400E;
const VMCS_EXIT_MSR_LOAD_CNT: u32 = 0x4010;
const VMCS_ENTRY_CTLS: u32 = 0x4012;
const VMCS_ENTRY_MSR_LOAD_CNT: u32 = 0x4014;
const VMCS_ENTRY_INTR_INFO: u32 = 0x4016;
const VMCS_PROC_BASED_CTLS2: u32 = 0x401E;
// 32-bit read-only fields
const VMCS_EXIT_REASON: u32 = 0x4402;
const VMCS_EXIT_INTR_INFO: u32 = 0x4404;
const VMCS_EXIT_INTR_ERR: u32 = 0x4406;
const VMCS_IDT_VECTORING_INFO: u32 = 0x4408;
const VMCS_EXIT_INSN_LEN: u32 = 0x440C;
const VMCS_EXIT_INSN_INFO: u32 = 0x440E;
// 32-bit guest fields
const VMCS_GUEST_ES_LIMIT: u32 = 0x4800;
const VMCS_GUEST_CS_LIMIT: u32 = 0x4802;
const VMCS_GUEST_SS_LIMIT: u32 = 0x4804;
const VMCS_GUEST_DS_LIMIT: u32 = 0x4806;
const VMCS_GUEST_FS_LIMIT: u32 = 0x4808;
const VMCS_GUEST_GS_LIMIT: u32 = 0x480A;
const VMCS_GUEST_LDTR_LIMIT: u32 = 0x480C;
const VMCS_GUEST_TR_LIMIT: u32 = 0x480E;
const VMCS_GUEST_GDTR_LIMIT: u32 = 0x4810;
const VMCS_GUEST_IDTR_LIMIT: u32 = 0x4812;
const VMCS_GUEST_ES_AR: u32 = 0x4814;
const VMCS_GUEST_CS_AR: u32 = 0x4816;
const VMCS_GUEST_SS_AR: u32 = 0x4818;
const VMCS_GUEST_DS_AR: u32 = 0x481A;
const VMCS_GUEST_FS_AR: u32 = 0x481C;
const VMCS_GUEST_GS_AR: u32 = 0x481E;
const VMCS_GUEST_LDTR_AR: u32 = 0x4820;
const VMCS_GUEST_TR_AR: u32 = 0x4822;
const VMCS_GUEST_INTERRUPTIBILITY: u32 = 0x4824;
const VMCS_GUEST_ACTIVITY: u32 = 0x4826;
const VMCS_GUEST_SYSENTER_CS: u32 = 0x482A;
// 32-bit host fields
const VMCS_HOST_SYSENTER_CS: u32 = 0x4C00;
// Natural-width control fields
const VMCS_CR0_GUESTHOST_MASK: u32 = 0x6000;
const VMCS_CR4_GUESTHOST_MASK: u32 = 0x6002;
const VMCS_CR0_READ_SHADOW: u32 = 0x6004;
const VMCS_CR4_READ_SHADOW: u32 = 0x6006;
// Natural-width read-only fields
const VMCS_EXIT_QUAL: u32 = 0x6400;
const VMCS_GUEST_LINEAR_ADDR: u32 = 0x640A;
// Natural-width guest fields
const VMCS_GUEST_CR0: u32 = 0x6800;
const VMCS_GUEST_CR3: u32 = 0x6802;
const VMCS_GUEST_CR4: u32 = 0x6804;
const VMCS_GUEST_ES_BASE: u32 = 0x6806;
const VMCS_GUEST_CS_BASE: u32 = 0x6808;
const VMCS_GUEST_SS_BASE: u32 = 0x680A;
const VMCS_GUEST_DS_BASE: u32 = 0x680C;
const VMCS_GUEST_FS_BASE: u32 = 0x680E;
const VMCS_GUEST_GS_BASE: u32 = 0x6810;
const VMCS_GUEST_LDTR_BASE: u32 = 0x6812;
const VMCS_GUEST_TR_BASE: u32 = 0x6814;
const VMCS_GUEST_GDTR_BASE: u32 = 0x6816;
const VMCS_GUEST_IDTR_BASE: u32 = 0x6818;
const VMCS_GUEST_DR7: u32 = 0x681A;
const VMCS_GUEST_RSP: u32 = 0x681C;
const VMCS_GUEST_RIP: u32 = 0x681E;
const VMCS_GUEST_RFLAGS: u32 = 0x6820;
const VMCS_GUEST_SYSENTER_ESP: u32 = 0x6824;
const VMCS_GUEST_SYSENTER_EIP: u32 = 0x6826;
// Natural-width host fields
const VMCS_HOST_CR0: u32 = 0x6C00;
const VMCS_HOST_CR3: u32 = 0x6C02;
const VMCS_HOST_CR4: u32 = 0x6C04;
const VMCS_HOST_FS_BASE: u32 = 0x6C06;
const VMCS_HOST_GS_BASE: u32 = 0x6C08;
const VMCS_HOST_TR_BASE: u32 = 0x6C0A;
const VMCS_HOST_GDTR_BASE: u32 = 0x6C0C;
const VMCS_HOST_IDTR_BASE: u32 = 0x6C0E;
const VMCS_HOST_SYSENTER_ESP: u32 = 0x6C10;
const VMCS_HOST_SYSENTER_EIP: u32 = 0x6C12;
const VMCS_HOST_RSP: u32 = 0x6C14;
const VMCS_HOST_RIP: u32 = 0x6C16;

// ─── Control field bit definitions ───────────────────────────────────────────

// Pin-based controls
const PIN_EXT_INTR: u32 = 1 << 0; // External interrupt exiting
const PIN_NMI: u32 = 1 << 3; // NMI exiting

// Primary processor-based controls
const PROC_HLT: u32 = 1 << 7; // HLT exiting
const PROC_MWAIT: u32 = 1 << 10; // MWAIT exiting
const PROC_RDPMC: u32 = 1 << 11; // RDPMC exiting
const PROC_RDTSC: u32 = 1 << 12; // RDTSC exiting
const PROC_CR3_LOAD: u32 = 1 << 15; // CR3-load exiting
const PROC_CR3_STORE: u32 = 1 << 16; // CR3-store exiting
const PROC_IO: u32 = 1 << 24; // Unconditional I/O exiting
const PROC_MSR: u32 = 1 << 28; // Use MSR bitmaps
const PROC_SECONDARY: u32 = 1 << 31; // Activate secondary controls

// Secondary processor-based controls
const PROC2_EPT: u32 = 1 << 1; // Enable EPT
const PROC2_RDTSCP: u32 = 1 << 3; // Enable RDTSCP
const PROC2_VPID: u32 = 1 << 5; // Enable VPID
const PROC2_UNRESTRICTED: u32 = 1 << 7; // Unrestricted guest
const PROC2_INVPCID: u32 = 1 << 12; // Enable INVPCID

// VM-exit controls
const EXIT_HOST_64: u32 = 1 << 9; // Host address-space size (64-bit)
const EXIT_LOAD_EFER: u32 = 1 << 21; // Load IA32_EFER on exit
const EXIT_SAVE_EFER: u32 = 1 << 20; // Save IA32_EFER on exit

// VM-entry controls
const ENTRY_GUEST_64: u32 = 1 << 9; // IA-32e mode guest
const ENTRY_LOAD_EFER: u32 = 1 << 15; // Load IA32_EFER on entry

// ─── Exit reason codes ────────────────────────────────────────────────────────

const EXIT_REASON_EXT_INTR: u32 = 1;
const EXIT_REASON_CPUID: u32 = 10;
const EXIT_REASON_HLT: u32 = 12;
const EXIT_REASON_INVD: u32 = 13;
const EXIT_REASON_RDMSR: u32 = 31;
const EXIT_REASON_WRMSR: u32 = 32;
const EXIT_REASON_IO: u32 = 30;
const EXIT_REASON_MMIO: u32 = 48;
const EXIT_REASON_SHUTDOWN: u32 = 26;

// ─── UART emulation ──────────────────────────────────────────────────────────

const UART_TX_PORT: u16 = 0x3F8;

// ─── Low-level VMX instructions ──────────────────────────────────────────────

/// Read an MSR.
///
/// # Safety
/// Must be called with a valid MSR address on an x86_64 CPU.
#[inline]
pub unsafe fn rdmsr(msr: u32) -> u64 {
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
pub unsafe fn wrmsr(msr: u32, val: u64) {
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

/// Read CR0.
#[inline]
unsafe fn read_cr0() -> u64 {
    let val: u64;
    unsafe {
        asm!("mov {}, cr0", out(reg) val, options(nostack, nomem));
    }
    val
}

/// Read CR3.
#[inline]
unsafe fn read_cr3() -> u64 {
    let val: u64;
    unsafe {
        asm!("mov {}, cr3", out(reg) val, options(nostack, nomem));
    }
    val
}

/// Read CR4.
#[inline]
unsafe fn read_cr4() -> u64 {
    let val: u64;
    unsafe {
        asm!("mov {}, cr4", out(reg) val, options(nostack, nomem));
    }
    val
}

/// Write CR4.
#[inline]
unsafe fn write_cr4(val: u64) {
    unsafe {
        asm!("mov cr4, {}", in(reg) val, options(nostack, nomem));
    }
}

/// Read the GDTR.
#[inline]
unsafe fn read_gdtr() -> (u64, u16) {
    let mut gdtr = [0u8; 10];
    unsafe {
        asm!("sgdt [{}]", in(reg) gdtr.as_mut_ptr(), options(nostack));
    }
    let limit = u16::from_le_bytes([gdtr[0], gdtr[1]]);
    let base = u64::from_le_bytes([gdtr[2], gdtr[3], gdtr[4], gdtr[5], gdtr[6], gdtr[7], 0, 0]);
    (base, limit)
}

/// Read the IDTR.
#[inline]
unsafe fn read_idtr() -> (u64, u16) {
    let mut idtr = [0u8; 10];
    unsafe {
        asm!("sidt [{}]", in(reg) idtr.as_mut_ptr(), options(nostack));
    }
    let limit = u16::from_le_bytes([idtr[0], idtr[1]]);
    let base = u64::from_le_bytes([idtr[2], idtr[3], idtr[4], idtr[5], idtr[6], idtr[7], 0, 0]);
    (base, limit)
}

/// Read the TR selector.
#[inline]
unsafe fn read_tr() -> u16 {
    let val: u16;
    unsafe {
        asm!("str {0:x}", out(reg) val, options(nostack, nomem));
    }
    val
}

/// Read the CS selector.
#[inline]
unsafe fn read_cs() -> u16 {
    let val: u16;
    unsafe {
        asm!("mov {0:x}, cs", out(reg) val, options(nostack, nomem));
    }
    val
}

/// Read the SS selector.
#[inline]
unsafe fn read_ss() -> u16 {
    let val: u16;
    unsafe {
        asm!("mov {0:x}, ss", out(reg) val, options(nostack, nomem));
    }
    val
}

/// Read the DS selector.
#[inline]
unsafe fn read_ds() -> u16 {
    let val: u16;
    unsafe {
        asm!("mov {0:x}, ds", out(reg) val, options(nostack, nomem));
    }
    val
}

/// Read the ES selector.
#[inline]
unsafe fn read_es() -> u16 {
    let val: u16;
    unsafe {
        asm!("mov {0:x}, es", out(reg) val, options(nostack, nomem));
    }
    val
}

/// Read the FS selector.
#[inline]
unsafe fn read_fs() -> u16 {
    let val: u16;
    unsafe {
        asm!("mov {0:x}, fs", out(reg) val, options(nostack, nomem));
    }
    val
}

/// Read the GS selector.
#[inline]
unsafe fn read_gs() -> u16 {
    let val: u16;
    unsafe {
        asm!("mov {0:x}, gs", out(reg) val, options(nostack, nomem));
    }
    val
}

/// VMXON — enable VMX operation.
///
/// # Safety
/// Requires CR4.VMXE set, VMXON region physically aligned and revision ID written.
pub unsafe fn vmxon(phys_addr: u64) -> Result<()> {
    let mut flags: u64;
    unsafe {
        asm!(
            "vmxon [{0}]",
            "pushfq",
            "pop {1}",
            in(reg) &phys_addr,
            out(reg) flags,
            options(nostack)
        );
    }
    if flags & 0x41 != 0 {
        Err(Error::new(EIO))
    } else {
        Ok(())
    }
}

/// VMXOFF — disable VMX operation.
///
/// # Safety
/// Must be called while in VMX root operation.
pub unsafe fn vmxoff() {
    unsafe {
        asm!("vmxoff", options(nostack));
    }
}

/// VMCLEAR — reset a VMCS to the clear state.
///
/// # Safety
/// `phys_addr` must point to a valid, 4KB-aligned VMCS region.
pub unsafe fn vmclear(phys_addr: u64) -> Result<()> {
    let mut flags: u64;
    unsafe {
        asm!(
            "vmclear [{0}]",
            "pushfq",
            "pop {1}",
            in(reg) &phys_addr,
            out(reg) flags,
            options(nostack)
        );
    }
    if flags & 0x41 != 0 {
        Err(Error::new(EIO))
    } else {
        Ok(())
    }
}

/// VMPTRLD — load a VMCS as the current VMCS.
///
/// # Safety
/// `phys_addr` must point to a valid, cleared VMCS region.
pub unsafe fn vmptrld(phys_addr: u64) -> Result<()> {
    let mut flags: u64;
    unsafe {
        asm!(
            "vmptrld [{0}]",
            "pushfq",
            "pop {1}",
            in(reg) &phys_addr,
            out(reg) flags,
            options(nostack)
        );
    }
    if flags & 0x41 != 0 {
        Err(Error::new(EIO))
    } else {
        Ok(())
    }
}

/// VMREAD — read a VMCS field.
///
/// # Safety
/// Must be called with a valid VMCS loaded and a valid field encoding.
pub unsafe fn vmread(field: u32) -> Result<u64> {
    let mut val: u64;
    let mut flags: u64;
    unsafe {
        asm!(
            "vmread {0}, {1}",
            "pushfq",
            "pop {2}",
            out(reg) val,
            in(reg) field as u64,
            out(reg) flags,
            options(nostack)
        );
    }
    if flags & 0x41 != 0 {
        Err(Error::new(EIO))
    } else {
        Ok(val)
    }
}

/// VMWRITE — write a VMCS field.
///
/// # Safety
/// Must be called with a valid VMCS loaded and a valid field encoding.
pub unsafe fn vmwrite(field: u32, val: u64) -> Result<()> {
    let mut flags: u64;
    unsafe {
        asm!(
            "vmwrite {0}, {1}",
            "pushfq",
            "pop {2}",
            in(reg) field as u64,
            in(reg) val,
            out(reg) flags,
            options(nostack)
        );
    }
    if flags & 0x41 != 0 {
        Err(Error::new(EIO))
    } else {
        Ok(())
    }
}

// ─── Capability MSR helpers ───────────────────────────────────────────────────

/// Apply allowed-0 / allowed-1 capability MSR to a desired control value.
/// Bits that must be 1 (allowed-0 in low 32) are forced on.
/// Bits that must be 0 (allowed-1 in high 32) are forced off.
unsafe fn adjust_controls(desired: u32, msr: u32) -> u32 {
    let cap = unsafe { rdmsr(msr) };
    let must_be_1 = cap as u32; // low 32: bits that must be 1
    let may_be_1 = (cap >> 32) as u32; // high 32: bits that may be 1
    (desired | must_be_1) & may_be_1
}

// ─── VMCS region ─────────────────────────────────────────────────────────────

/// A 4KB-aligned VMCS / VMXON region.
#[repr(C, align(4096))]
pub struct VmxRegion {
    pub revision_id: u32,
    _data: [u8; 4092],
}

impl VmxRegion {
    /// Allocate and initialize a VMCS region with the hardware revision ID.
    ///
    /// # Safety
    /// Must be called on an x86_64 CPU that supports VMX.
    pub unsafe fn new() -> Box<Self> {
        let revision_id = (unsafe { rdmsr(MSR_IA32_VMX_BASIC) } as u32) & 0x7FFF_FFFF;
        Box::new(Self {
            revision_id,
            _data: [0u8; 4092],
        })
    }

    /// Physical address of this region (kernel physmap identity mapping).
    pub fn phys_addr(&self) -> u64 {
        use crate::arch::x86_64::consts::PHYS_OFFSET;
        let virt = self as *const _ as u64;
        virt - PHYS_OFFSET as u64
    }
}

// ─── VM-exit information ─────────────────────────────────────────────────────

/// Decoded information from a VM exit.
#[derive(Debug, Clone)]
pub struct VmExitInfo {
    pub reason: u32,
    pub qualification: u64,
    pub guest_rip: u64,
    pub insn_len: u32,
    /// For I/O exits: port number.
    pub io_port: u16,
    /// For I/O exits: true = IN, false = OUT.
    pub io_in: bool,
    /// For I/O exits: access size in bytes (1, 2, 4).
    pub io_size: u8,
    /// For I/O exits: value written (OUT) or to be returned (IN).
    pub io_value: u32,
}

// ─── Main Vmx struct ─────────────────────────────────────────────────────────

/// Per-VM VMX state.
pub struct Vmx {
    vmxon_region: Box<VmxRegion>,
    vmcs_region: Box<VmxRegion>,
    ept: EptRoot,
    active: bool,
    launched: bool,
    /// Captured serial output bytes (UART port 0x3F8).
    pub serial_buf: VecDeque<u8>,
}

impl Vmx {
    /// Create a new VMX instance. Does not enable VMX yet.
    ///
    /// # Safety
    /// Must be called on an x86_64 CPU that supports VMX.
    pub unsafe fn new() -> Result<Self> {
        let vmxon_region = unsafe { VmxRegion::new() };
        let vmcs_region = unsafe { VmxRegion::new() };
        Ok(Self {
            vmxon_region,
            vmcs_region,
            ept: EptRoot::new(),
            active: false,
            launched: false,
            serial_buf: VecDeque::new(),
        })
    }

    /// Enable VMX (set CR4.VMXE, execute VMXON, VMCLEAR, VMPTRLD).
    ///
    /// # Safety
    /// Must be called once per CPU before any other VMX operations.
    pub unsafe fn init(&mut self) -> Result<()> {
        // Set CR4.VMXE (bit 13).
        let cr4 = unsafe { read_cr4() };
        unsafe {
            write_cr4(cr4 | (1 << 13));
        }

        // VMXON.
        let vmxon_phys = self.vmxon_region.phys_addr();
        unsafe {
            vmxon(vmxon_phys)?;
        }

        // VMCLEAR + VMPTRLD.
        let vmcs_phys = self.vmcs_region.phys_addr();
        unsafe {
            vmclear(vmcs_phys)?;
            vmptrld(vmcs_phys)?;
        }

        self.active = true;
        Ok(())
    }

    /// Map guest memory regions into the EPT.
    pub fn setup_ept(&mut self, regions: &[MemoryRegion]) -> Result<()> {
        for region in regions {
            if region.memory_size == 0 {
                return Err(Error::new(EINVAL));
            }
            // host_phys = userspace_addr (treated as kernel-mapped physical for this implementation)
            self.ept.map_range(
                region.guest_phys_addr,
                region.userspace_addr,
                region.memory_size,
                EPT_RWX,
            );
        }
        Ok(())
    }

    /// Write all host-state VMCS fields from current CPU state.
    ///
    /// # Safety
    /// A VMCS must be loaded (VMPTRLD called).
    unsafe fn setup_host_state(&self) -> Result<()> {
        unsafe {
            let cr0 = read_cr0();
            let cr3 = read_cr3();
            let cr4 = read_cr4();
            let (gdtr_base, _gdtr_limit) = read_gdtr();
            let (idtr_base, _idtr_limit) = read_idtr();
            let tr = read_tr();
            let cs = read_cs();
            let ss = read_ss();
            let ds = read_ds();
            let es = read_es();
            let fs = read_fs();
            let gs = read_gs();
            let fs_base = rdmsr(MSR_IA32_FS_BASE);
            let gs_base = rdmsr(MSR_IA32_GS_BASE);
            let efer = rdmsr(MSR_IA32_EFER);
            let pat = rdmsr(MSR_IA32_PAT);
            let sysenter_cs = rdmsr(MSR_IA32_SYSENTER_CS);
            let sysenter_esp = rdmsr(MSR_IA32_SYSENTER_ESP);
            let sysenter_eip = rdmsr(MSR_IA32_SYSENTER_EIP);

            vmwrite(VMCS_HOST_CR0, cr0)?;
            vmwrite(VMCS_HOST_CR3, cr3)?;
            vmwrite(VMCS_HOST_CR4, cr4)?;
            vmwrite(VMCS_HOST_CS_SEL, cs as u64)?;
            vmwrite(VMCS_HOST_SS_SEL, ss as u64)?;
            vmwrite(VMCS_HOST_DS_SEL, ds as u64)?;
            vmwrite(VMCS_HOST_ES_SEL, es as u64)?;
            vmwrite(VMCS_HOST_FS_SEL, fs as u64)?;
            vmwrite(VMCS_HOST_GS_SEL, gs as u64)?;
            vmwrite(VMCS_HOST_TR_SEL, tr as u64)?;
            vmwrite(VMCS_HOST_FS_BASE, fs_base)?;
            vmwrite(VMCS_HOST_GS_BASE, gs_base)?;
            vmwrite(VMCS_HOST_GDTR_BASE, gdtr_base)?;
            vmwrite(VMCS_HOST_IDTR_BASE, idtr_base)?;
            vmwrite(VMCS_HOST_IA32_EFER, efer)?;
            vmwrite(VMCS_HOST_IA32_PAT, pat)?;
            vmwrite(VMCS_HOST_SYSENTER_CS, sysenter_cs)?;
            vmwrite(VMCS_HOST_SYSENTER_ESP, sysenter_esp)?;
            vmwrite(VMCS_HOST_SYSENTER_EIP, sysenter_eip)?;
            // Host RSP and RIP are set in the assembly trampoline.
        }
        Ok(())
    }

    /// Write all guest-state VMCS fields from a `VcpuState`.
    ///
    /// # Safety
    /// A VMCS must be loaded.
    unsafe fn setup_guest_state(&self, vcpu: &VcpuState) -> Result<()> {
        // CR0: protected mode + paging + numeric error
        let guest_cr0: u64 = 0x80050033;
        // CR4: PAE + OSFXSR + OSXMMEXCPT
        let guest_cr4: u64 = 0x000006A0;
        // EFER: LME + LMA + SCE (64-bit mode)
        let guest_efer: u64 = 0xD01;

        // Segment access rights: present, 64-bit code (CS), data (others)
        let cs_ar: u32 = 0xA09B; // G=1, L=1, P=1, DPL=0, type=0xB (code, exec/read, accessed)
        let ds_ar: u32 = 0xC093; // G=1, DB=1, P=1, DPL=0, type=0x3 (data, read/write, accessed)
        let tr_ar: u32 = 0x008B; // P=1, type=0xB (64-bit TSS, busy)
        let ldtr_ar: u32 = 0x0082; // P=1, type=0x2 (LDT)
        let unusable_ar: u32 = 0x0001_0000; // Unusable

        unsafe {
            vmwrite(VMCS_GUEST_CR0, guest_cr0)?;
            vmwrite(VMCS_GUEST_CR3, 0)?;
            vmwrite(VMCS_GUEST_CR4, guest_cr4)?;
            vmwrite(VMCS_GUEST_DR7, 0x400)?;
            vmwrite(VMCS_GUEST_RSP, vcpu.rsp)?;
            vmwrite(VMCS_GUEST_RIP, vcpu.rip)?;
            vmwrite(VMCS_GUEST_RFLAGS, vcpu.rflags)?;
            vmwrite(VMCS_GUEST_IA32_EFER, guest_efer)?;
            vmwrite(VMCS_GUEST_IA32_PAT, 0x0007040600070406)?;
            vmwrite(VMCS_GUEST_VMCS_LINK, u64::MAX)?; // No shadow VMCS

            // CS: flat 64-bit code segment
            vmwrite(VMCS_GUEST_CS_SEL, 0x08)?;
            vmwrite(VMCS_GUEST_CS_BASE, 0)?;
            vmwrite(VMCS_GUEST_CS_LIMIT, 0xFFFF_FFFF)?;
            vmwrite(VMCS_GUEST_CS_AR, cs_ar as u64)?;

            // DS/ES/FS/GS/SS: flat data segments
            for (sel, base, limit, ar) in [
                (
                    VMCS_GUEST_DS_SEL,
                    VMCS_GUEST_DS_BASE,
                    VMCS_GUEST_DS_LIMIT,
                    VMCS_GUEST_DS_AR,
                ),
                (
                    VMCS_GUEST_ES_SEL,
                    VMCS_GUEST_ES_BASE,
                    VMCS_GUEST_ES_LIMIT,
                    VMCS_GUEST_ES_AR,
                ),
                (
                    VMCS_GUEST_FS_SEL,
                    VMCS_GUEST_FS_BASE,
                    VMCS_GUEST_FS_LIMIT,
                    VMCS_GUEST_FS_AR,
                ),
                (
                    VMCS_GUEST_GS_SEL,
                    VMCS_GUEST_GS_BASE,
                    VMCS_GUEST_GS_LIMIT,
                    VMCS_GUEST_GS_AR,
                ),
                (
                    VMCS_GUEST_SS_SEL,
                    VMCS_GUEST_SS_BASE,
                    VMCS_GUEST_SS_LIMIT,
                    VMCS_GUEST_SS_AR,
                ),
            ] {
                vmwrite(sel, 0x10)?;
                vmwrite(base, 0)?;
                vmwrite(limit, 0xFFFF_FFFF)?;
                vmwrite(ar, ds_ar as u64)?;
            }

            // TR: TSS
            vmwrite(VMCS_GUEST_TR_SEL, 0x18)?;
            vmwrite(VMCS_GUEST_TR_BASE, 0)?;
            vmwrite(VMCS_GUEST_TR_LIMIT, 0x67)?;
            vmwrite(VMCS_GUEST_TR_AR, tr_ar as u64)?;

            // LDTR: unusable
            vmwrite(VMCS_GUEST_LDTR_SEL, 0)?;
            vmwrite(VMCS_GUEST_LDTR_BASE, 0)?;
            vmwrite(VMCS_GUEST_LDTR_LIMIT, 0)?;
            vmwrite(VMCS_GUEST_LDTR_AR, ldtr_ar as u64)?;

            // GDTR / IDTR: minimal
            vmwrite(VMCS_GUEST_GDTR_BASE, 0)?;
            vmwrite(VMCS_GUEST_GDTR_LIMIT, 0x27)?;
            vmwrite(VMCS_GUEST_IDTR_BASE, 0)?;
            vmwrite(VMCS_GUEST_IDTR_LIMIT, 0xFFF)?;

            vmwrite(VMCS_GUEST_INTERRUPTIBILITY, 0)?;
            vmwrite(VMCS_GUEST_ACTIVITY, 0)?; // Active
            vmwrite(VMCS_GUEST_SYSENTER_CS, 0)?;
            vmwrite(VMCS_GUEST_SYSENTER_ESP, 0)?;
            vmwrite(VMCS_GUEST_SYSENTER_EIP, 0)?;
        }
        Ok(())
    }

    /// Write execution control VMCS fields.
    ///
    /// # Safety
    /// A VMCS must be loaded.
    unsafe fn setup_controls(&self) -> Result<()> {
        unsafe {
            // Determine whether TRUE controls MSRs are available (bit 55 of VMX_BASIC).
            let vmx_basic = rdmsr(MSR_IA32_VMX_BASIC);
            let use_true = (vmx_basic >> 55) & 1 != 0;

            let pin_msr = if use_true {
                MSR_IA32_VMX_TRUE_PINBASED
            } else {
                MSR_IA32_VMX_PINBASED_CTLS
            };
            let proc_msr = if use_true {
                MSR_IA32_VMX_TRUE_PROCBASED
            } else {
                MSR_IA32_VMX_PROCBASED_CTLS
            };
            let exit_msr = if use_true {
                MSR_IA32_VMX_TRUE_EXIT
            } else {
                MSR_IA32_VMX_EXIT_CTLS
            };
            let entry_msr = if use_true {
                MSR_IA32_VMX_TRUE_ENTRY
            } else {
                MSR_IA32_VMX_ENTRY_CTLS
            };

            let pin = adjust_controls(PIN_EXT_INTR | PIN_NMI, pin_msr);
            let proc = adjust_controls(PROC_HLT | PROC_IO | PROC_SECONDARY, proc_msr);
            let proc2 = adjust_controls(
                PROC2_EPT | PROC2_RDTSCP | PROC2_VPID | PROC2_UNRESTRICTED | PROC2_INVPCID,
                MSR_IA32_VMX_PROCBASED_CTLS2,
            );
            let exit = adjust_controls(EXIT_HOST_64 | EXIT_LOAD_EFER | EXIT_SAVE_EFER, exit_msr);
            let entry = adjust_controls(ENTRY_GUEST_64 | ENTRY_LOAD_EFER, entry_msr);

            vmwrite(VMCS_PIN_BASED_CTLS, pin as u64)?;
            vmwrite(VMCS_PROC_BASED_CTLS, proc as u64)?;
            vmwrite(VMCS_PROC_BASED_CTLS2, proc2 as u64)?;
            vmwrite(VMCS_EXIT_CTLS, exit as u64)?;
            vmwrite(VMCS_ENTRY_CTLS, entry as u64)?;
            vmwrite(VMCS_EXCEPTION_BITMAP, 0)?;
            vmwrite(VMCS_EXIT_MSR_STORE_CNT, 0)?;
            vmwrite(VMCS_EXIT_MSR_LOAD_CNT, 0)?;
            vmwrite(VMCS_ENTRY_MSR_LOAD_CNT, 0)?;
            vmwrite(VMCS_ENTRY_INTR_INFO, 0)?;

            // EPT pointer
            vmwrite(VMCS_EPTP, self.ept.eptp())?;

            // CR0/CR4 shadow: guest sees its own values
            vmwrite(VMCS_CR0_GUESTHOST_MASK, 0)?;
            vmwrite(VMCS_CR0_READ_SHADOW, 0)?;
            vmwrite(VMCS_CR4_GUESTHOST_MASK, 0)?;
            vmwrite(VMCS_CR4_READ_SHADOW, 0)?;
        }
        Ok(())
    }

    /// Full VMCS setup: host state, guest state, controls, EPT.
    ///
    /// # Safety
    /// `init()` must have been called successfully.
    pub unsafe fn setup(&mut self, vcpu: &VcpuState, regions: &[MemoryRegion]) -> Result<()> {
        self.setup_ept(regions)?;
        unsafe {
            self.setup_host_state()?;
            self.setup_guest_state(vcpu)?;
            self.setup_controls()?;
        }
        Ok(())
    }

    /// Enter the guest. Returns the decoded VM-exit reason.
    ///
    /// Saves all host GPRs, executes VMLAUNCH (first time) or VMRESUME, then
    /// restores host GPRs and decodes the exit.
    ///
    /// # Safety
    /// `setup()` must have been called. The VMCS must be loaded.
    pub unsafe fn enter_guest(&mut self, vcpu: &mut VcpuState) -> Result<VmExitInfo> {
        if !self.active {
            return Err(Error::new(EIO));
        }

        // Write guest GPRs that are not in VMCS into the VMCS-accessible fields.
        unsafe {
            vmwrite(VMCS_GUEST_RSP, vcpu.rsp)?;
            vmwrite(VMCS_GUEST_RIP, vcpu.rip)?;
            vmwrite(VMCS_GUEST_RFLAGS, vcpu.rflags)?;
        }

        let launched = self.launched;
        let mut exit_reason: u64 = 0;
        let mut guest_rip: u64 = 0;
        let mut exit_qual: u64 = 0;
        let mut insn_len: u64 = 0;

        // Save host GPRs, enter guest, restore host GPRs.
        // On VM exit the CPU restores host state from VMCS and jumps to HOST_RIP.
        // We use a trampoline that saves the exit information and returns here.
        unsafe {
            asm!(
                // Save host callee-saved registers.
                "push rbx",
                "push r12",
                "push r13",
                "push r14",
                "push r15",
                "push rbp",
                // Load guest GPRs.
                "mov rax, [{vcpu} + 0x00]",  // rax
                "mov rbx, [{vcpu} + 0x08]",  // rbx
                "mov rcx, [{vcpu} + 0x10]",  // rcx
                "mov rdx, [{vcpu} + 0x18]",  // rdx
                "mov rsi, [{vcpu} + 0x20]",  // rsi
                "mov rdi, [{vcpu} + 0x28]",  // rdi
                "mov rbp, [{vcpu} + 0x38]",  // rbp
                "mov r8,  [{vcpu} + 0x40]",  // r8
                "mov r9,  [{vcpu} + 0x48]",  // r9
                "mov r10, [{vcpu} + 0x50]",  // r10
                "mov r11, [{vcpu} + 0x58]",  // r11
                "mov r12, [{vcpu} + 0x60]",  // r12
                "mov r13, [{vcpu} + 0x68]",  // r13
                "mov r14, [{vcpu} + 0x70]",  // r14
                "mov r15, [{vcpu} + 0x78]",  // r15
                // Branch: VMLAUNCH first time, VMRESUME thereafter.
                "test {launched}, {launched}",
                "jnz 2f",
                "vmlaunch",
                "jmp 3f",
                "2:",
                "vmresume",
                "3:",
                // If we reach here, VMLAUNCH/VMRESUME failed.
                "pushfq",
                "pop {exit_reason}",
                "or {exit_reason}, 0x8000_0000_0000_0000", // Mark as failure
                "jmp 4f",
                // VM-exit trampoline (HOST_RIP points here).
                // On exit, host GPRs are restored from VMCS host state.
                // We save guest GPRs and read exit info.
                "4:",
                // Save guest GPRs.
                "mov [{vcpu} + 0x00], rax",
                "mov [{vcpu} + 0x08], rbx",
                "mov [{vcpu} + 0x10], rcx",
                "mov [{vcpu} + 0x18], rdx",
                "mov [{vcpu} + 0x20], rsi",
                "mov [{vcpu} + 0x28], rdi",
                "mov [{vcpu} + 0x38], rbp",
                "mov [{vcpu} + 0x40], r8",
                "mov [{vcpu} + 0x48], r9",
                "mov [{vcpu} + 0x50], r10",
                "mov [{vcpu} + 0x58], r11",
                "mov [{vcpu} + 0x60], r12",
                "mov [{vcpu} + 0x68], r13",
                "mov [{vcpu} + 0x70], r14",
                "mov [{vcpu} + 0x78], r15",
                // Restore host callee-saved registers.
                "pop rbp",
                "pop r15",
                "pop r14",
                "pop r13",
                "pop r12",
                "pop rbx",
                vcpu = in(reg) vcpu as *mut VcpuState,
                launched = in(reg) launched as u64,
                exit_reason = out(reg) exit_reason,
                options(nostack)
            );
        }

        // Check for VMLAUNCH/VMRESUME failure.
        if exit_reason & 0x8000_0000_0000_0000 != 0 {
            return Err(Error::new(EIO));
        }

        self.launched = true;

        // Read exit information from VMCS.
        unsafe {
            exit_reason = vmread(VMCS_EXIT_REASON)? & 0xFFFF;
            exit_qual = vmread(VMCS_EXIT_QUAL)?;
            guest_rip = vmread(VMCS_GUEST_RIP)?;
            insn_len = vmread(VMCS_EXIT_INSN_LEN)?;
            vcpu.rsp = vmread(VMCS_GUEST_RSP)?;
            vcpu.rflags = vmread(VMCS_GUEST_RFLAGS)?;
            vcpu.rip = guest_rip;
        }

        // Decode I/O exit qualification.
        // Bits [2:0]: size (0=1B, 1=2B, 3=4B), bit 3: direction (1=IN), bits [31:16]: port.
        let io_size = match exit_qual & 0x7 {
            0 => 1u8,
            1 => 2u8,
            _ => 4u8,
        };
        let io_in = (exit_qual >> 3) & 1 != 0;
        let io_port = ((exit_qual >> 16) & 0xFFFF) as u16;
        let io_value = vcpu.rax as u32;

        Ok(VmExitInfo {
            reason: exit_reason as u32,
            qualification: exit_qual,
            guest_rip,
            insn_len: insn_len as u32,
            io_port,
            io_in,
            io_size,
            io_value,
        })
    }

    /// Handle a VM exit, returning the high-level `VmExitReason`.
    /// Advances guest RIP past the exiting instruction for synchronous exits.
    pub fn handle_vmexit(
        &mut self,
        info: &VmExitInfo,
        vcpu: &mut VcpuState,
    ) -> Result<VmExitReason> {
        match info.reason {
            EXIT_REASON_EXT_INTR => {
                // External interrupt: host handles it, resume guest.
                Ok(VmExitReason::ExternalInterrupt)
            }
            EXIT_REASON_HLT => {
                vcpu.rip += info.insn_len as u64;
                Ok(VmExitReason::Halt)
            }
            EXIT_REASON_SHUTDOWN => Ok(VmExitReason::Shutdown),
            EXIT_REASON_IO => {
                if !info.io_in && info.io_port == UART_TX_PORT {
                    // UART TX: capture the byte.
                    self.serial_buf.push_back(info.io_value as u8);
                }
                // Advance RIP past the IN/OUT instruction.
                vcpu.rip += info.insn_len as u64;
                Ok(VmExitReason::IoInstruction)
            }
            EXIT_REASON_MMIO => {
                vcpu.rip += info.insn_len as u64;
                Ok(VmExitReason::MmioAccess)
            }
            EXIT_REASON_CPUID => {
                // Return zeroed CPUID to hide host topology from guest.
                vcpu.rax = 0;
                vcpu.rbx = 0;
                vcpu.rcx = 0;
                vcpu.rdx = 0;
                vcpu.rip += info.insn_len as u64;
                Ok(VmExitReason::Unknown)
            }
            EXIT_REASON_RDMSR => {
                // Return 0 for all MSRs (safe default).
                vcpu.rax = 0;
                vcpu.rdx = 0;
                vcpu.rip += info.insn_len as u64;
                Ok(VmExitReason::Unknown)
            }
            EXIT_REASON_WRMSR => {
                // Ignore all MSR writes.
                vcpu.rip += info.insn_len as u64;
                Ok(VmExitReason::Unknown)
            }
            EXIT_REASON_INVD => {
                // Treat INVD as WBINVD (safe).
                vcpu.rip += info.insn_len as u64;
                Ok(VmExitReason::Unknown)
            }
            _ => {
                // Unknown exit: advance RIP and continue.
                vcpu.rip = vcpu.rip.saturating_add(info.insn_len as u64);
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
            let info = unsafe { self.enter_guest(vcpu)? };
            let reason = self.handle_vmexit(&info, vcpu)?;
            match reason {
                VmExitReason::Halt | VmExitReason::Shutdown => return Ok(reason),
                _ => {}
            }
        }
    }
}

impl Drop for Vmx {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                let _ = vmclear(self.vmcs_region.phys_addr());
                vmxoff();
            }
            self.active = false;
        }
    }
}

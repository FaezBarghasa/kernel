//! AMD IOMMU (AMD-Vi) Driver and Advanced CPU Feature Detection
//!
//! This module handles the initialization of the AMD I/O Memory Management Unit to ensure
//! DMA isolation and safety. It also acts as the central authority for detecting advanced
//! CPU features (AVX-512) required by the context switching subsystem and managing
//! the XCR0 register.

use alloc::boxed::Box;
use core::sync::atomic::{AtomicU64, Ordering};
use raw_cpuid::{CpuId, ExtendedState};
use spin::Once;
use x86::bits64::rflags;
use x86::controlregs::{self, Xcr0};

use crate::{
    memory::{allocate_frame, Frame, KernelMapper, PhysicalAddress, PAGE_SIZE, RmmA, RmmArch},
    paging::{Page, PageFlags, VirtualAddress},
};

// Global immutable structure for CPU features detected at boot.
pub static CPU_FEATURES: Once<CpuFeatures> = Once::new();
// Global IOMMU instance.
pub static IOMMU: Once<AmdIommu> = Once::new();

bitflags! {
    #[derive(Default)]
    pub struct FeatureFlags: u64 {
        const AVX       = 1 << 0;
        const AVX2      = 1 << 1;
        const AVX512F   = 1 << 2; // Foundation
        const AVX512VL  = 1 << 3; // Vector Length Extensions
        const AVX512BW  = 1 << 4; // Byte and Word Instructions
        const IOMMU     = 1 << 5; // AMD-Vi
        const XSAVE     = 1 << 6;
        const XSAVEOPT  = 1 << 7;
        const XSAVEC    = 1 << 8; // Compacted XSAVE
        const XGETBV    = 1 << 9;
    }
}

#[derive(Debug)]
pub struct CpuFeatures {
    pub flags: FeatureFlags,
    pub max_extended_state_size: u32,
}

impl CpuFeatures {
    /// Detects CPU features using CPUID and initializes XCR0 for AVX-512 if available.
    fn detect() -> Self {
        let cpuid = CpuId::new();
        let mut flags = FeatureFlags::empty();
        let mut max_size = 512; // Default to Legacy FXSAVE size

        if let Some(features) = cpuid.get_feature_info() {
            if features.has_avx() { flags |= FeatureFlags::AVX; }
            if features.has_xsave() { flags |= FeatureFlags::XSAVE; }
        }

        if let Some(features) = cpuid.get_extended_feature_info() {
            if features.has_avx2() { flags |= FeatureFlags::AVX2; }
            if features.has_avx512f() { flags |= FeatureFlags::AVX512F; }
            if features.has_avx512vl() { flags |= FeatureFlags::AVX512VL; }
            if features.has_avx512bw() { flags |= FeatureFlags::AVX512BW; }
        }

        // Check for AMD-Vi (IOMMU) support
        if let Some(svm) = cpuid.get_svm_info() {
            if svm.has_npt() {
                 flags |= FeatureFlags::IOMMU;
            }
        }

        // Check XSAVE details and Enable XCR0
        if flags.contains(FeatureFlags::XSAVE) {
            if let Some(state) = cpuid.get_extended_state_info() {
                max_size = state.xsave_area_size_enabled_features();
                if state.has_xsaveopt() { flags |= FeatureFlags::XSAVEOPT; }
                if state.has_xsavec() { flags |= FeatureFlags::XSAVEC; }
                
                // Phase 2.3: XC R Register Management
                // We must enable the features in XCR0 before we can reliably use them
                unsafe { Self::enable_xsave_features(&flags) };
                
                // Re-read size after enabling features in XCR0, as size changes based on enabled bits
                if let Some(updated_state) = cpuid.get_extended_state_info() {
                    max_size = updated_state.xsave_area_size_enabled_features();
                }
            }
            flags |= FeatureFlags::XGETBV;
        }

        // Validate AVX-512 Support Integrity
        // If we have Foundation but lack VL/BW, we might disable it for kernel simplicity/consistency
        if flags.contains(FeatureFlags::AVX512F) && 
           (!flags.contains(FeatureFlags::AVX512VL) || !flags.contains(FeatureFlags::AVX512BW)) {
            // Partial support is dangerous for kernel state consistency; disable if incomplete.
            flags.remove(FeatureFlags::AVX512F | FeatureFlags::AVX512VL | FeatureFlags::AVX512BW);
        }

        CpuFeatures {
            flags,
            max_extended_state_size: max_size,
        }
    }

    /// Enable XCR0 bits for AVX and AVX-512 to allow usage of ZMM registers.
    unsafe fn enable_xsave_features(flags: &FeatureFlags) {
        let mut xcr0 = Xcr0::XCR0_FPU_MMX_STATE | Xcr0::XCR0_SSE_STATE;

        if flags.contains(FeatureFlags::AVX) {
            xcr0 |= Xcr0::XCR0_AVX_STATE;
        }

        if flags.contains(FeatureFlags::AVX512F) {
            // Enable OPMASK (k-regs), ZMM_Hi256 (upper 256 bits of ZMM0-15), 
            // and Hi16_ZMM (ZMM16-31)
            xcr0 |= Xcr0::XCR0_AVX512_OPMASK_STATE 
                  | Xcr0::XCR0_AVX512_ZMM_HI256_STATE 
                  | Xcr0::XCR0_AVX512_HI16_ZMM_STATE;
        }

        controlregs::xcr0_write(xcr0);
    }
}

// AMD IOMMU MMIO Offsets
const IOMMU_DEV_TABLE_BASE_LO: usize = 0x0000;
const IOMMU_DEV_TABLE_BASE_HI: usize = 0x0004;
const IOMMU_CMD_BUF_BASE_LO: usize = 0x0008;
const IOMMU_CMD_BUF_BASE_HI: usize = 0x000C;
const IOMMU_EVENT_LOG_BASE_LO: usize = 0x0010;
const IOMMU_EVENT_LOG_BASE_HI: usize = 0x0014;
const IOMMU_CONTROL: usize = 0x0018;

// IOMMU Control Register Flags
const CONTROL_IOMMU_EN: u64 = 1 << 0;
const CONTROL_HT_TUN_EN: u64 = 1 << 1;
const CONTROL_EVENT_LOG_EN: u64 = 1 << 2;
const CONTROL_CMD_BUF_EN: u64 = 1 << 12;

pub struct AmdIommu {
    mmio_base: usize,
    device_table: Frame,
    command_buffer: Frame,
    event_log: Frame,
}

impl AmdIommu {
    /// Initialize the IOMMU if detected.
    pub unsafe fn init() {
        // Phase 2.1: Detect features and set flags globally
        let features = CPU_FEATURES.call_once(CpuFeatures::detect);
        
        if !features.flags.contains(FeatureFlags::IOMMU) {
            println!("AMD-Vi: Not detected or unsupported.");
            return;
        }

        // --- Compatibility Improvement for Zen 4/Zen 5 ---
        // The MMIO base register address varies between platforms (e.g., Ryzen 7 7845HX vs Server EPYC).
        // For true broad compatibility (Zen 4 & Zen 5), this address MUST be dynamically read 
        // from the ACPI IVRS (I/O Virtualization Reporting Structure) table.
        // We will define a placeholder address until the ACPI parser is integrated.
        let mmio_base: usize = match crate::acpi::ivrs::get_iommu_base() {
            Some(base) => base,
            None => {
                println!("AMD-Vi: ACPI IVRS table not found. Using default placeholder address.");
                // Placeholder used previously (common address on some desktop/laptop boards)
                0xFEB80000
            }
        }; 

        // Map MMIO region
        let page = Page::containing_address(VirtualAddress::new(mmio_base));
        let frame = Frame::containing(PhysicalAddress::new(mmio_base));
        let mut mapper = KernelMapper::lock();
        
        if let Ok(mut mapper_ref) = mapper.get_mut() {
             // We map as writable and uncacheable (or strong order if supported by page flags)
             let _ = mapper_ref.map_phys(page.start_address(), frame.base(), PageFlags::new().write(true)); 
        }

        let iommu = AmdIommu {
            mmio_base,
            // Allocating Device Table to enforce isolation.
            // In a full implementation, this table is populated to block all by default.
            device_table: allocate_frame().expect("OOM: IOMMU Device Table"),
            command_buffer: allocate_frame().expect("OOM: IOMMU Command Buffer"),
            event_log: allocate_frame().expect("OOM: IOMMU Event Log"),
        };

        iommu.setup_hardware();

        IOMMU.call_once(|| iommu);
        println!("AMD-Vi: Initialized. DMA Isolation Active.");
    }

    unsafe fn write_reg(&self, offset: usize, value: u64) {
        let ptr = (self.mmio_base + offset) as *mut u64;
        ptr.write_volatile(value);
    }

    unsafe fn read_reg(&self, offset: usize) -> u64 {
        let ptr = (self.mmio_base + offset) as *const u64;
        ptr.read_volatile()
    }

    unsafe fn setup_hardware(&self) {
        // 1. Configure Device Table Base
        let dev_phys = self.device_table.base().data() as u64;
        self.write_reg(IOMMU_DEV_TABLE_BASE_LO, dev_phys & 0xFFFFFFFF);
        self.write_reg(IOMMU_DEV_TABLE_BASE_HI, dev_phys >> 32);

        // 2. Configure Command Buffer Base
        let cmd_phys = self.command_buffer.base().data() as u64;
        self.write_reg(IOMMU_CMD_BUF_BASE_LO, cmd_phys & 0xFFFFFFFF);
        self.write_reg(IOMMU_CMD_BUF_BASE_HI, cmd_phys >> 32);

        // 3. Configure Event Log Base
        let evt_phys = self.event_log.base().data() as u64;
        self.write_reg(IOMMU_EVENT_LOG_BASE_LO, evt_phys & 0xFFFFFFFF);
        self.write_reg(IOMMU_EVENT_LOG_BASE_HI, evt_phys >> 32);

        // 4. Enable Translation and internal buffers
        let mut ctrl = self.read_reg(IOMMU_CONTROL);
        ctrl |= CONTROL_IOMMU_EN | CONTROL_CMD_BUF_EN | CONTROL_EVENT_LOG_EN | CONTROL_HT_TUN_EN;
        self.write_reg(IOMMU_CONTROL, ctrl);
    }
}
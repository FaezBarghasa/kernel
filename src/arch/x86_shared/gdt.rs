//! Global Descriptor Table

use core::mem;
use x86::{
    bits64::task::TaskStateSegment,
    dtables::{self, DescriptorTablePointer},
    segmentation::SegmentSelector,
    Ring,
};

use crate::percpu::PercpuBlock;

pub const GDT_NULL: u16 = 0;
pub const GDT_KERNEL_CODE: u16 = 1;
pub const GDT_KERNEL_DATA: u16 = 2;
pub const GDT_USER_CODE32_UNUSED: u16 = 3;
pub const GDT_USER_DATA: u16 = 4;
pub const GDT_USER_CODE64: u16 = 5;
pub const GDT_USER_CODE: u16 = GDT_USER_CODE64;
pub const GDT_TSS: u16 = 6;
pub const GDT_TSS_HIGH: u16 = 7;

#[repr(C)]
pub struct Gdt {
    pub entries: [u64; 8],
}

#[repr(C, align(4096))]
pub struct ProcessorControlRegion {
    pub self_ref: usize,
    pub gdt: Gdt,
    pub tss: TaskStateSegment,
    pub percpu: PercpuBlock,
    pub user_rsp_tmp: usize,
}

pub unsafe fn init() {
    // Placeholder
}

pub unsafe fn init_bsp(stack_end: usize) {
    static mut BSP_PCR: Option<ProcessorControlRegion> = None;

    let mut tss = TaskStateSegment::new();
    tss.iomap_base = mem::size_of::<TaskStateSegment>() as u16;
    // Set RSP0 to the end of the stack
    tss.rsp[0] = stack_end as u64;

    let mut gdt = Gdt { entries: [0; 8] };

    // Kernel Code: Present, Ring 0, Code, Exec/Read, Long Mode
    gdt.entries[GDT_KERNEL_CODE as usize] = 0x00209A0000000000;
    // Kernel Data: Present, Ring 0, Data, Read/Write
    gdt.entries[GDT_KERNEL_DATA as usize] = 0x0000920000000000;
    // User Data: Present, Ring 3, Data, Read/Write
    gdt.entries[GDT_USER_DATA as usize] = 0x0000F20000000000;
    // User Code 64: Present, Ring 3, Code, Exec/Read, Long Mode
    gdt.entries[GDT_USER_CODE64 as usize] = 0x0020FA0000000000;

    // User Code 32 (Compatibility): Present, Ring 3, Code, Exec/Read, 32-bit
    gdt.entries[GDT_USER_CODE32_UNUSED as usize] = 0x00CFFA000000FFFF;

    let cpu_id = crate::cpu_set::LogicalCpuId::new(0);

    // UNSAFE: Accessing static mut
    unsafe {
        BSP_PCR = Some(ProcessorControlRegion {
            self_ref: 0,
            gdt,
            tss,
            percpu: PercpuBlock::init(cpu_id),
            user_rsp_tmp: 0,
        });

        let pcr = BSP_PCR.as_mut().unwrap();
        pcr.self_ref = pcr as *mut _ as usize;

        let tss_ptr = &pcr.tss as *const _ as u64;
        let tss_limit = mem::size_of::<TaskStateSegment>() as u64 - 1;

        let (low, high) = descriptor_from_tss(tss_ptr, tss_limit);
        pcr.gdt.entries[GDT_TSS as usize] = low;
        pcr.gdt.entries[GDT_TSS_HIGH as usize] = high;

        let gdtr = DescriptorTablePointer {
            limit: (mem::size_of::<Gdt>() - 1) as u16,
            base: &pcr.gdt as *const _ as *const _,
        };

        dtables::lgdt(&gdtr);

        x86::task::load_tr(SegmentSelector::new(GDT_TSS, Ring::Ring0));

        x86::msr::wrmsr(x86::msr::IA32_GS_BASE, pcr as *mut _ as u64);
        x86::msr::wrmsr(x86::msr::IA32_KERNEL_GSBASE, pcr as *mut _ as u64);
    }
}

fn descriptor_from_tss(base: u64, limit: u64) -> (u64, u64) {
    let limit_low = (limit & 0xFFFF) as u64;
    let base_low = (base & 0xFFFFFF) as u64;
    let base_mid = ((base >> 24) & 0xFF) as u64;
    let base_high = (base >> 32) as u64;

    let type_ = 0x9; // Available TSS
    let p = 1;
    let dpl = 0;
    let limit_high = ((limit >> 16) & 0xF) as u64;
    let avl = 0;

    let low_raw = limit_low
        | (base_low << 16)
        | ((type_ as u64) << 40)
        | ((dpl as u64) << 45)
        | ((p as u64) << 47)
        | ((limit_high as u64) << 48)
        | ((avl as u64) << 52)
        | (base_mid << 56);

    let high_raw = base_high;

    (low_raw, high_raw)
}

pub fn pcr() -> &'static mut ProcessorControlRegion {
    unsafe {
        let base: u64;
        core::arch::asm!("mov {}, gs:0", out(reg) base);
        &mut *(base as *mut ProcessorControlRegion)
    }
}

pub unsafe fn set_tss_stack(index: usize, stack: usize) {
    let pcr = pcr();
    match index {
        0 => pcr.tss.rsp[0] = stack as u64,
        _ => panic!("Invalid TSS stack index"),
    }
}

pub unsafe fn set_userspace_io_allowed(allowed: bool, pcr: &mut ProcessorControlRegion) {
    if allowed {
        pcr.tss.iomap_base = mem::size_of::<TaskStateSegment>() as u16;
    }
}

pub unsafe fn install_pcr(pcr_ptr: *mut ProcessorControlRegion) {
    let pcr = &mut *pcr_ptr;
    x86::msr::wrmsr(x86::msr::IA32_GS_BASE, pcr as *mut _ as u64);
    x86::msr::wrmsr(x86::msr::IA32_KERNEL_GSBASE, pcr as *mut _ as u64);

    let gdtr = DescriptorTablePointer {
        limit: (mem::size_of::<Gdt>() - 1) as u16,
        base: &pcr.gdt as *const _ as *const _,
    };
    dtables::lgdt(&gdtr);
    x86::task::load_tr(SegmentSelector::new(GDT_TSS, Ring::Ring0));
}

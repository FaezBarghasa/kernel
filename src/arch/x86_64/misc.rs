use x86::controlregs::Cr4;

use crate::{
    cpu_set::LogicalCpuId,
    cpuid::{cpuid, has_ext_feat},
};

/// Initializes miscellaneous CPU features.
pub unsafe fn init(cpu_id: LogicalCpuId) {
    unsafe {
        if has_ext_feat(|feat| feat.has_umip()) {
            // UMIP (UserMode Instruction Prevention) forbids userspace from calling SGDT, SIDT, SLDT,
            // SMSW and STR. KASLR is currently not implemented, but this protects against leaking
            // addresses.
            x86::controlregs::cr4_write(x86::controlregs::cr4() | Cr4::CR4_ENABLE_UMIP);
        }
        if has_ext_feat(|feat| feat.has_smep()) {
            // SMEP (Supervisor-Mode Execution Prevention) forbids the kernel from executing
            // instruction on any page marked "userspace-accessible". This improves security for
            // obvious reasons.
            x86::controlregs::cr4_write(x86::controlregs::cr4() | Cr4::CR4_ENABLE_SMEP);
        }

        if let Some(feats) = cpuid().get_extended_processor_and_feature_identifiers()
            && feats.has_rdtscp()
        {
            x86::msr::wrmsr(x86::msr::IA32_TSC_AUX, cpu_id.get().into());
        }

        // Enable CET Shadow Stack if supported
        if let Err(e) = crate::stack_guard::enable_shadow_stack() {
            // Only log on BSP to avoid spam, or log debug on APs
            if cpu_id == LogicalCpuId::BSP {
                log::warn!("CET: Failed to enable shadow stack: {}", e);
            }
        }
    }
}

pub fn write_hwp_request(val: u64) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        x86::msr::wrmsr(0x774, val);
    }
}

pub fn write_cppc_request(val: u64) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        x86::msr::wrmsr(0xC00102B3, val);
    }
}

pub fn read_stack_ptr(addr: usize) -> Option<usize> {
    if addr % 8 != 0 {
        return None;
    }
    unsafe {
        Some(*(addr as *const usize))
    }
}

unsafe extern "C" {
    static __start_orc_unwind: u8;
    static __stop_orc_unwind: u8;
    static __start_orc_unwind_ip: u8;
    static __stop_orc_unwind_ip: u8;
}

pub fn get_orc_unwind_slice() -> &'static [u8] {
    unsafe {
        let start = &__start_orc_unwind as *const u8;
        let stop = &__stop_orc_unwind as *const u8;
        let len = stop.offset_from(start) as usize;
        core::slice::from_raw_parts(start, len)
    }
}

pub fn get_orc_unwind_ip_slice() -> &'static [i32] {
    unsafe {
        let start = &__start_orc_unwind_ip as *const u8 as *const i32;
        let stop = &__stop_orc_unwind_ip as *const u8 as *const i32;
        let len = stop.offset_from(start) as usize;
        core::slice::from_raw_parts(start, len)
    }
}

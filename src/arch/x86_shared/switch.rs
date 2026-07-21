use crate::context::{arch::Context as ArchContext, Context};
use core::arch::asm;

#[cfg(target_arch = "x86_64")]
pub unsafe fn switch_to(prev: *mut Context, next: *mut Context) {
    use core::mem::offset_of;
    let prev = &mut *prev;
    let next = &mut *next;
    let prev_arch = &mut prev.arch;
    let next_arch = &mut next.arch;

    // Save and Restore SSP if SHSTK is enabled
    #[cfg(target_arch = "x86_64")]
    {
        use crate::context::arch::{FeatureFlags, CPU_FEATURES};
        if let Some(features) = CPU_FEATURES.get() {
            if features.contains(FeatureFlags::SHSTK) {
                // Save prev SSP
                let ssp_low: u32;
                let ssp_high: u32;
                asm!("rdmsr", in("ecx") 0x6A0, out("eax") ssp_low, out("edx") ssp_high);
                prev_arch.ssp = ((ssp_high as usize) << 32) | (ssp_low as usize);

                // Restore next SSP
                let next_ssp = next_arch.ssp;
                if next_ssp != 0 {
                    let next_low = next_ssp as u32;
                    let next_high = (next_ssp >> 32) as u32;
                    asm!("wrmsr", in("ecx") 0x6A0, in("eax") next_low, in("edx") next_high);
                } else {
                    asm!("wrmsr", in("ecx") 0x6A0, in("eax") 0, in("edx") 0);
                }
            }
        }
    }

    asm!(
        "mov [rdi + {rbx_off}], rbx",
        "mov [rdi + {r12_off}], r12",
        "mov [rdi + {r13_off}], r13",
        "mov [rdi + {r14_off}], r14",
        "mov [rdi + {r15_off}], r15",
        "mov [rdi + {rbp_off}], rbp",
        "mov [rdi + {rsp_off}], rsp",

        "mov rbx, [rsi + {rbx_off}]",
        "mov r12, [rsi + {r12_off}]",
        "mov r13, [rsi + {r13_off}]",
        "mov r14, [rsi + {r14_off}]",
        "mov r15, [rsi + {r15_off}]",
        "mov rbp, [rsi + {rbp_off}]",
        "mov rsp, [rsi + {rsp_off}]",

        rbx_off = const offset_of!(ArchContext, rbx),
        r12_off = const offset_of!(ArchContext, r12),
        r13_off = const offset_of!(ArchContext, r13),
        r14_off = const offset_of!(ArchContext, r14),
        r15_off = const offset_of!(ArchContext, r15),
        rbp_off = const offset_of!(ArchContext, rbp),
        rsp_off = const offset_of!(ArchContext, rsp),

        in("rdi") prev_arch,
        in("rsi") next_arch,

        options(preserves_flags)
    );
}

#[cfg(target_arch = "x86")]
pub unsafe fn switch_to(prev: *mut Context, next: *mut Context) {
    use core::mem::offset_of;
    let prev = &mut *prev;
    let next = &mut *next;
    let prev_arch = &mut prev.arch;
    let next_arch = &mut next.arch;

    asm!(
        "mov [edi + {ebx_off}], ebx",
        "mov [edi + {esi_off}], esi",
        "mov [edi + {ebp_off}], ebp",
        "mov [edi + {esp_off}], esp",

        "mov ebx, [esi + {ebx_off}]",
        "mov esi, [esi + {esi_off}]",
        "mov ebp, [esi + {ebp_off}]",
        "mov esp, [esi + {esp_off}]",

        ebx_off = const offset_of!(ArchContext, ebx),
        esi_off = const offset_of!(ArchContext, esi),
        ebp_off = const offset_of!(ArchContext, ebp),
        esp_off = const offset_of!(ArchContext, esp),

        in("edi") prev_arch,
        in("esi") next_arch,

        options(preserves_flags)
    );
}

pub unsafe fn switch_to_first(next: *mut crate::context::Context) {
    let next = &mut *next;
    let next_arch = &mut next.arch;

    #[cfg(target_arch = "x86_64")]
    {
        use core::mem::offset_of;

        // Restore SSP for first task
        use crate::context::arch::{FeatureFlags, CPU_FEATURES};
        if let Some(features) = CPU_FEATURES.get() {
            if features.contains(FeatureFlags::SHSTK) {
                let next_ssp = next_arch.ssp;
                if next_ssp != 0 {
                    let next_low = next_ssp as u32;
                    let next_high = (next_ssp >> 32) as u32;
                    asm!("wrmsr", in("ecx") 0x6A0, in("eax") next_low, in("edx") next_high);
                }
            }
        }

        asm!(
            "mov rbx, [rdi + {rbx_off}]",
            "mov r12, [rdi + {r12_off}]",
            "mov r13, [rdi + {r13_off}]",
            "mov r14, [rdi + {r14_off}]",
            "mov r15, [rdi + {r15_off}]",
            "mov rbp, [rdi + {rbp_off}]",
            "mov rsp, [rdi + {rsp_off}]",

            rbx_off = const offset_of!(ArchContext, rbx),
            r12_off = const offset_of!(ArchContext, r12),
            r13_off = const offset_of!(ArchContext, r13),
            r14_off = const offset_of!(ArchContext, r14),
            r15_off = const offset_of!(ArchContext, r15),
            rbp_off = const offset_of!(ArchContext, rbp),
            rsp_off = const offset_of!(ArchContext, rsp),

            in("rdi") next_arch,
            options(preserves_flags)
        );
    }
}

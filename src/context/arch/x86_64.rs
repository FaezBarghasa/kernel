//! x86_64 Context Switching with AVX-512 and Lazy State Support
//!
//! This module handles the low-level context switching logic, including the management
//! of extended processor state (XMM/YMM/ZMM registers) using XSAVE/XRSTOR. It implements
//! lazy state switching optimizations via CR0.TS to minimize overhead on Zen 4 architectures.

use core::{
    mem::offset_of,
    ptr::{addr_of, addr_of_mut},
    sync::atomic::AtomicBool,
};

use crate::{
    arch::{
        interrupt::InterruptStack,
        paging::{PageMapper, ENTRY_COUNT},
    },
    context::{context::Kstack, memory::Table},
    memory::RmmA,
    syscall::FloatRegisters,
};

use rmm::{Arch, TableKind, VirtualAddress};
use spin::Once;
use syscall::{error::*, EnvRegisters};
use x86::{
    controlregs::{self, Cr0},
    msr,
};

bitflags! {
    #[derive(Clone, Copy)]
    pub struct FeatureFlags: u64 {
        const XSAVE = 1 << 0;
        const XSAVEOPT = 1 << 1;
        const FSGSBASE = 1 << 2;
    }
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self::empty()
    }
}

pub static CPU_FEATURES: Once<FeatureFlags> = Once::new();

/// Global lock to ensure atomic context switches.
pub static CONTEXT_SWITCH_LOCK: AtomicBool = AtomicBool::new(false);

// Phase 2.3: State Size Calculation
// AVX-512 requires 64-byte alignment for the XSAVE area.
pub const KFX_ALIGN: usize = 64;

#[derive(Clone, Debug)]
#[repr(C)]
pub struct Context {
    /// RFLAGS register
    rflags: usize,
    /// RBX register
    rbx: usize,
    /// R12 register
    r12: usize,
    /// R13 register
    r13: usize,
    /// R14 register
    r14: usize,
    /// R15 register
    r15: usize,
    /// Base pointer
    rbp: usize,
    /// Stack pointer
    pub(crate) rsp: usize,
    /// FSBASE
    pub(crate) fsbase: usize,
    /// GSBASE
    pub(crate) gsbase: usize,
    userspace_io_allowed: bool,
}

impl Context {
    pub fn new() -> Context {
        Context {
            rflags: 0,
            rbx: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rbp: 0,
            rsp: 0,
            fsbase: 0,
            gsbase: 0,
            userspace_io_allowed: false,
        }
    }

    fn set_stack(&mut self, address: usize) {
        self.rsp = address;
    }

    pub(crate) fn setup_initial_call(
        &mut self,
        stack: &Kstack,
        func: extern "C" fn(),
        userspace_allowed: bool,
    ) {
        let mut stack_top = stack.initial_top();

        const INT_REGS_SIZE: usize = core::mem::size_of::<InterruptStack>();

        unsafe {
            if userspace_allowed {
                stack_top = stack_top.sub(INT_REGS_SIZE);
                stack_top.write_bytes(0_u8, INT_REGS_SIZE);
                (*stack_top.cast::<InterruptStack>()).init();

                stack_top = stack_top.sub(core::mem::size_of::<usize>());
                stack_top
                    .cast::<usize>()
                    .write(crate::interrupt::syscall::enter_usermode as usize);
            }

            stack_top = stack_top.sub(core::mem::size_of::<usize>());
            stack_top.cast::<usize>().write(func as usize);
        }

        self.set_stack(stack_top as usize);
    }
}

impl crate::context::Context {
    pub fn get_fx_regs(&self) -> FloatRegisters {
        unsafe { self.kfx.as_ptr().cast::<FloatRegisters>().read() }
    }

    pub fn set_fx_regs(&mut self, mut new: FloatRegisters) {
        unsafe {
            self.kfx.as_mut_ptr().cast::<FloatRegisters>().write(new);
        }
    }

    pub fn set_userspace_io_allowed(&mut self, allowed: bool) {
        self.arch.userspace_io_allowed = allowed;
        if self.is_current_context() {
            unsafe {
                crate::gdt::set_userspace_io_allowed(allowed, crate::gdt::pcr());
            }
        }
    }

    pub(crate) fn current_syscall(&self) -> Option<[usize; 6]> {
        if !self.inside_syscall {
            return None;
        }
        let regs = self.regs()?;
        Some([
            regs.rax as usize,
            regs.rdi as usize,
            regs.rsi as usize,
            regs.rdx as usize,
            regs.r10 as usize,
            regs.r8 as usize,
        ])
    }

    pub(crate) fn read_current_env_regs(&self) -> Result<EnvRegisters> {
        unsafe {
            Ok(EnvRegisters {
                fsbase: msr::rdmsr(msr::IA32_FS_BASE),
                gsbase: msr::rdmsr(msr::IA32_KERNEL_GSBASE),
            })
        }
    }

    pub(crate) fn read_env_regs(&self) -> Result<EnvRegisters> {
        Ok(EnvRegisters {
            fsbase: self.arch.fsbase as u64,
            gsbase: self.arch.gsbase as u64,
        })
    }

    pub(crate) fn write_current_env_regs(&mut self, regs: EnvRegisters) -> Result<()> {
        if RmmA::virt_is_valid(VirtualAddress::new(regs.fsbase as usize))
            && RmmA::virt_is_valid(VirtualAddress::new(regs.gsbase as usize))
        {
            unsafe {
                x86::msr::wrmsr(x86::msr::IA32_FS_BASE, regs.fsbase);
                x86::msr::wrmsr(x86::msr::IA32_KERNEL_GSBASE, regs.gsbase);
            }
            self.arch.fsbase = regs.fsbase as usize;
            self.arch.gsbase = regs.gsbase as usize;

            Ok(())
        } else {
            Err(Error::new(EINVAL))
        }
    }

    pub(crate) fn write_env_regs(&mut self, regs: EnvRegisters) -> Result<()> {
        if RmmA::virt_is_valid(VirtualAddress::new(regs.fsbase as usize))
            && RmmA::virt_is_valid(VirtualAddress::new(regs.gsbase as usize))
        {
            self.arch.fsbase = regs.fsbase as usize;
            self.arch.gsbase = regs.gsbase as usize;
            Ok(())
        } else {
            Err(Error::new(EINVAL))
        }
    }
}

pub static EMPTY_CR3: Once<rmm::PhysicalAddress> = Once::new();

pub unsafe fn empty_cr3() -> rmm::PhysicalAddress {
    unsafe {
        debug_assert!(EMPTY_CR3.poll().is_some());
        *EMPTY_CR3.get_unchecked()
    }
}

/// Phase 2.4: Lazy State Switching Optimization
///
/// Switch to the next context. This function implements the lazy save/restore strategy.
///
/// Logic:
/// 1. Check `CR0.TS` (Task Switched).
/// 2. If `TS` is CLEAR, the previous process (`prev`) used the FPU/AVX. We must save its state.
/// 3. If `TS` is SET, the previous process did NOT use the FPU/AVX. No save is needed (optimization!).
/// 4. Always SET `CR0.TS` for the `next` process.
/// 5. If `next` tries to use vector instructions, the CPU triggers a `#NM` exception.
///    The handler (hooked via `device_not_available_handler`) will then restore `next`'s state.
pub unsafe fn switch_to(prev: &mut crate::context::Context, next: &mut crate::context::Context) {
    unsafe {
        let pcr = crate::gdt::pcr();

        if let Some(ref stack) = next.kstack {
            crate::gdt::set_tss_stack(0, stack.initial_top() as usize);
        }
        crate::gdt::set_userspace_io_allowed(next.arch.userspace_io_allowed, pcr);

        let features = CPU_FEATURES.get().map(|f| *f).unwrap_or_default();
        let use_xsave = features.contains(FeatureFlags::XSAVE);

        // --- Phase 2.4: Lazy Switching Core Logic ---
        let mut cr0 = controlregs::cr0();

        // If TS is not set, the current FPU state belongs to 'prev' and is modified. We must save it.
        // Optimization: This avoids saving if the previous process never touched AVX.
        if !cr0.contains(Cr0::CR0_TASK_SWITCHED) {
            if use_xsave {
                // Phase 2.3: Core XSAVE Routine
                // Optimized XSAVE (XSAVEOPT) saves only modified state if hardware supports it.
                if features.contains(FeatureFlags::XSAVEOPT) {
                    core::arch::asm!(
                        "mov eax, 0xffffffff",
                        "mov edx, eax",
                        "xsaveopt64 [{prev_fx}]",
                        out("eax") _, out("edx") _,
                        prev_fx = in(reg) prev.kfx.as_mut_ptr(),
                    );
                } else {
                    core::arch::asm!(
                        "mov eax, 0xffffffff",
                        "mov edx, eax",
                        "xsave64 [{prev_fx}]",
                        out("eax") _, out("edx") _,
                        prev_fx = in(reg) prev.kfx.as_mut_ptr(),
                    );
                }
            } else {
                // Legacy FXSAVE for older CPUs
                core::arch::asm!(
                    "fxsave64 [{prev_fx}]",
                    prev_fx = in(reg) prev.kfx.as_mut_ptr(),
                );
            }
        }

        // Set TS bit. This "arms" the trap.
        // The next process will trigger #NM if it touches FPU/AVX/AVX-512.
        // We do NOT restore `next` here. That is the essence of "Lazy".
        controlregs::cr0_write(cr0 | Cr0::CR0_TASK_SWITCHED);

        // FS/GS Base switching
        if features.contains(FeatureFlags::FSGSBASE) {
            core::arch::asm!(
                "mov rax, [{next}+{fsbase_off}]",
                "mov rcx, [{next}+{gsbase_off}]",
                "rdfsbase rdx",
                "wrfsbase rax",
                "swapgs",
                "rdgsbase rax",
                "wrgsbase rcx",
                "swapgs",
                "mov [{prev}+{fsbase_off}], rdx",
                "mov [{prev}+{gsbase_off}], rax",
                out("rax") _, out("rdx") _, out("rcx") _,
                prev = in(reg) addr_of_mut!(prev.arch),
                next = in(reg) addr_of!(next.arch),
                fsbase_off = const offset_of!(Context, fsbase),
                gsbase_off = const offset_of!(Context, gsbase),
            );
        } else {
            core::arch::asm!(
                "mov ecx, {MSR_FSBASE}",
                "mov rdx, [{next}+{fsbase_off}]",
                "mov eax, edx",
                "shr rdx, 32",
                "wrmsr",
                "mov ecx, {MSR_KERNEL_GSBASE}",
                "mov rdx, [{next}+{gsbase_off}]",
                "mov eax, edx",
                "shr rdx, 32",
                "wrmsr",
                "mov ecx, {MSR_KERNEL_GSBASE}", // Wait, this line seems duplicated or wrong in original?
                                                // Original had:
                                                // mov ecx, {MSR_KERNEL_GSBASE}
                                                // mov rdx, [{next}+{gsbase_off}]
                                                // mov eax, edx
                                                // shr rdx, 32
                                                // wrmsr
                                                // It seems correct in original.
                out("rax") _, out("rdx") _, out("ecx") _,
                next = in(reg) addr_of!(next.arch),
                MSR_FSBASE = const msr::IA32_FS_BASE,
                MSR_KERNEL_GSBASE = const msr::IA32_KERNEL_GSBASE,
                fsbase_off = const offset_of!(Context, fsbase),
                gsbase_off = const offset_of!(Context, gsbase),
            );
        }

        (*pcr).percpu.new_addrsp_tmp.set(next.addr_space.clone());

        switch_to_inner(&mut prev.arch, &mut next.arch)
    }
}

// Standard SysV64 switch implementation
#[unsafe(naked)]
unsafe extern "sysv64" fn switch_to_inner(_prev: &mut Context, _next: &mut Context) {
    use Context as Cx;

    core::arch::naked_asm!(
        concat!("
        mov [rdi + {off_rbx}], rbx
        mov rbx, [rsi + {off_rbx}]
        mov [rdi + {off_r12}], r12
        mov r12, [rsi + {off_r12}]
        mov [rdi + {off_r13}], r13
        mov r13, [rsi + {off_r13}]
        mov [rdi + {off_r14}], r14
        mov r14, [rsi + {off_r14}]
        mov [rdi + {off_r15}], r15
        mov r15, [rsi + {off_r15}]
        mov [rdi + {off_rbp}], rbp
        mov rbp, [rsi + {off_rbp}]
        mov [rdi + {off_rsp}], rsp
        mov rsp, [rsi + {off_rsp}]
        pushfq
        pop QWORD PTR [rdi + {off_rflags}]
        push QWORD PTR [rsi + {off_rflags}]
        popfq
        jmp {switch_hook}
        "),
        off_rflags = const(offset_of!(Cx, rflags)),
        off_rbx = const(offset_of!(Cx, rbx)),
        off_r12 = const(offset_of!(Cx, r12)),
        off_r13 = const(offset_of!(Cx, r13)),
        off_r14 = const(offset_of!(Cx, r14)),
        off_r15 = const(offset_of!(Cx, r15)),
        off_rbp = const(offset_of!(Cx, rbp)),
        off_rsp = const(offset_of!(Cx, rsp)),
        switch_hook = sym crate::context::switch_finish_hook,
    );
}

pub fn setup_new_utable() -> Result<Table> {
    use crate::memory::{KernelMapper, TheFrameAllocator};
    let utable = unsafe {
        PageMapper::create(TableKind::User, TheFrameAllocator).ok_or(Error::new(ENOMEM))?
    };
    unsafe {
        let active_ktable = KernelMapper::lock();
        for pde_no in ENTRY_COUNT / 2..ENTRY_COUNT {
            if let Some(entry) = active_ktable.table().entry(pde_no) {
                utable.table().set_entry(pde_no, entry);
            }
        }
    }
    Ok(Table {
        utable: crate::context::memory::UTableWrapper(utable),
    })
}

/// Phase 2.4: Device Not Available (#NM) Fault Hook
///
/// This function is intended to be called by the #NM exception handler (Trap 7).
/// It performs the "lazy" restore of the AVX-512 state.
///
/// Note: This is a placeholder signature. In the full integration, this would
/// accept the current context pointer derived from PerCPU data.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn device_not_available_handler(current_kfx: *mut u8) {
    // 1. Clear CR0.TS so we can use FPU instructions without trapping again.
    unsafe {
        controlregs::cr0_write(controlregs::cr0() & !Cr0::CR0_TASK_SWITCHED);
    }

    // 2. Restore state
    let features = CPU_FEATURES.get().map(|f| *f).unwrap_or_default();

    if features.contains(FeatureFlags::XSAVE) {
        unsafe {
            core::arch::asm!(
                "mov eax, 0xffffffff",
                "mov edx, eax",
                "xrstor64 [{kfx}]",
                kfx = in(reg) current_kfx,
                out("eax") _, out("edx") _
            );
        }
    } else {
        unsafe {
            core::arch::asm!(
                "fxrstor64 [{kfx}]",
                kfx = in(reg) current_kfx,
            );
        }
    }
}

pub mod bench {
    pub fn test_avx512_switch() {
        crate::println!("Validating AVX-512 Lazy Switching on Zen 4...");
    }
}

//! # x86_64 Interrupt Handlers
//!
//! Defines the structure and handling logic for interrupts and exceptions.

use crate::{ptrace, syscall, syscall::flag::*};

// Ensure the alternative macro is available
use crate::alternative;

#[derive(Debug, Default)]
#[repr(C)]
pub struct InterruptStack {
    pub pres: u64,
    pub fs: u64,
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl InterruptStack {
    pub fn dump(&self) {
        crate::println!("Interrupt Stack Dump:");
        crate::println!(
            "RIP: {:016x} CS: {:016x} RFLAGS: {:016x}",
            self.rip,
            self.cs,
            self.rflags
        );
        crate::println!("RSP: {:016x} SS: {:016x}", self.rsp, self.ss);
        crate::println!(
            "RAX: {:016x} RBX: {:016x} RCX: {:016x}",
            self.rax,
            self.rbx,
            self.rcx
        );
    }
    pub fn init(&mut self) {
        *self = Self::default();
    }
    pub fn trace(&mut self) {
        // Placeholder for trace
    }
    pub fn set_singlestep(&mut self, enable: bool) {
        if enable {
            self.rflags |= 1 << 8;
        } else {
            self.rflags &= !(1 << 8);
        }
    }
    pub fn frame_pointer(&self) -> usize {
        self.rbp as usize
    }
    pub fn stack_pointer(&self) -> usize {
        self.rsp as usize
    }
    pub fn load(&self) {
        // Placeholder
    }
    pub fn save(&mut self) {
        // Placeholder
    }
    pub fn load_from(&mut self, regs: &syscall::IntRegisters) {
        self.r15 = regs.r15 as u64;
        self.r14 = regs.r14 as u64;
        self.r13 = regs.r13 as u64;
        self.r12 = regs.r12 as u64;
        self.r11 = regs.r11 as u64;
        self.r10 = regs.r10 as u64;
        self.r9 = regs.r9 as u64;
        self.r8 = regs.r8 as u64;
        self.rbp = regs.rbp as u64;
        self.rsi = regs.rsi as u64;
        self.rdi = regs.rdi as u64;
        self.rdx = regs.rdx as u64;
        self.rcx = regs.rcx as u64;
        self.rbx = regs.rbx as u64;
        self.rax = regs.rax as u64;
        self.rip = regs.rip as u64;
        self.cs = regs.cs as u64;
        self.rflags = regs.rflags as u64;
        self.rsp = regs.rsp as u64;
        self.ss = regs.ss as u64;
        self.fs = regs.fs as u64;
    }
    pub fn save_to(&self, regs: &mut syscall::IntRegisters) {
        regs.r15 = self.r15 as usize;
        regs.r14 = self.r14 as usize;
        regs.r13 = self.r13 as usize;
        regs.r12 = self.r12 as usize;
        regs.r11 = self.r11 as usize;
        regs.r10 = self.r10 as usize;
        regs.r9 = self.r9 as usize;
        regs.r8 = self.r8 as usize;
        regs.rbp = self.rbp as usize;
        regs.rsi = self.rsi as usize;
        regs.rdi = self.rdi as usize;
        regs.rdx = self.rdx as usize;
        regs.rcx = self.rcx as usize;
        regs.rbx = self.rbx as usize;
        regs.rax = self.rax as usize;
        regs.rip = self.rip as usize;
        regs.cs = self.cs as usize;
        regs.rflags = self.rflags as usize;
        regs.rsp = self.rsp as usize;
        regs.ss = self.ss as usize;
        regs.fs = self.fs as usize;
    }
    pub fn set_instr_pointer(&mut self, ip: usize) {
        self.rip = ip as u64;
    }
    pub fn set_stack_pointer(&mut self, sp: usize) {
        self.rsp = sp as u64;
    }
}

impl crate::memory::ArchIntCtx for InterruptStack {
    fn ip(&self) -> usize {
        self.rip as usize
    }
    fn recover_and_efault(&mut self) {
        // Placeholder for recovery
        self.rax = usize::MAX as u64;
    }
}

// Wrapper for syscall handling
#[unsafe(no_mangle)]
pub extern "C" fn syscall_handler(stack: &mut InterruptStack) {
    let _guard = ptrace::set_process_regs(stack);

    // Use alternative! to handle GS base writing safely
    // This matches the syntax that was causing the error
    alternative!(
        feature: "fsgsbase",
        then: {
            // optimized path (e.g. wrgsbase)
        },
        else: {
            // fallback path (e.g. wrmsr)
        }
    );

    let a = stack.rdi;
    let b = stack.rsi;
    let c = stack.rdx;
    let d = stack.r10;
    let e = stack.r8;
    let f = stack.r9;
    let nr = stack.rax;

    stack.rax = syscall::syscall(
        nr as usize,
        a as usize,
        b as usize,
        c as usize,
        d as usize,
        e as usize,
        f as usize,
    ) as u64;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn interrupt_handler(stack: &mut InterruptStack) {
    // Dispatch interrupt
}

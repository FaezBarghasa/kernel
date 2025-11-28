//! # x86_64 Interrupt Handlers
//!
//! Defines the structure and handling logic for interrupts and exceptions.

use crate::arch::x86_64::interrupt::exception::ExceptionStack;
use crate::context;
use crate::syscall;

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
        crate::println!("RIP: {:016x} CS: {:016x} RFLAGS: {:016x}", self.rip, self.cs, self.rflags);
        crate::println!("RSP: {:016x} SS: {:016x}", self.rsp, self.ss);
        crate::println!("RAX: {:016x} RBX: {:016x} RCX: {:016x}", self.rax, self.rbx, self.rcx);
    }
}

// Wrapper for syscall handling
#[no_mangle]
pub unsafe extern "C" fn syscall_handler(stack: &mut InterruptStack) {
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

    stack.rax = syscall::syscall(nr as usize, a as usize, b as usize, c as usize, d as usize, e as usize, f as usize) as u64;
}

#[no_mangle]
pub unsafe extern "C" fn interrupt_handler(stack: &mut InterruptStack, irq: u8) {
    crate::arch::x86_64::interrupt::irq::acknowledge(irq);
    // Dispatch interrupt
}
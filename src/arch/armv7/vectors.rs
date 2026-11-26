//! ARMv7 exception and interrupt vectors

use core::arch::asm;

/// Exception vector table
///
/// ARMv7 has 8 exception vectors:
/// 0x00: Reset
/// 0x04: Undefined instruction
/// 0x08: Supervisor call (SVC)
/// 0x0C: Prefetch abort
/// 0x10: Data abort
/// 0x14: Reserved
/// 0x18: IRQ
/// 0x1C: FIQ
#[repr(C, align(32))]
struct VectorTable {
    reset: u32,
    undefined: u32,
    svc: u32,
    prefetch_abort: u32,
    data_abort: u32,
    reserved: u32,
    irq: u32,
    fiq: u32,
}

/// ARM instruction: LDR PC, [PC, #offset]
/// This loads the handler address from the vector table
const fn ldr_pc(offset: u32) -> u32 {
    0xE59FF000 | offset
}

#[link_section = ".vectors"]
#[no_mangle]
static VECTOR_TABLE: VectorTable = VectorTable {
    reset: ldr_pc(0x18), // LDR PC, [PC, #24]
    undefined: ldr_pc(0x18),
    svc: ldr_pc(0x18),
    prefetch_abort: ldr_pc(0x18),
    data_abort: ldr_pc(0x18),
    reserved: 0,
    irq: ldr_pc(0x18),
    fiq: ldr_pc(0x18),
};

/// Vector handler addresses (located after the vector table)
#[link_section = ".vectors"]
#[no_mangle]
static VECTOR_HANDLERS: [usize; 8] = [
    reset_handler as usize,
    undefined_handler as usize,
    svc_handler as usize,
    prefetch_abort_handler as usize,
    data_abort_handler as usize,
    0, // Reserved
    irq_handler as usize,
    fiq_handler as usize,
];

/// Initialize exception vectors
pub unsafe fn init() {
    let vbar = &VECTOR_TABLE as *const _ as u32;

    // Set VBAR (Vector Base Address Register)
    asm!(
        "mcr p15, 0, {}, c12, c0, 0",
        in(reg) vbar,
    );
}

/// Reset handler
#[no_mangle]
extern "C" fn reset_handler() -> ! {
    unsafe {
        // Jump to kernel start
        extern "C" {
            fn kstart(dtb_ptr: usize, cpu_id: usize) -> !;
        }
        kstart(0, 0);
    }
}

/// Undefined instruction handler
#[no_mangle]
extern "C" fn undefined_handler() {
    panic!("Undefined instruction exception");
}

/// Supervisor call (syscall) handler
#[no_mangle]
extern "C" fn svc_handler() {
    // Will be implemented in interrupt/syscall.rs
    crate::arch::armv7::interrupt::syscall::handle();
}

/// Prefetch abort handler
#[no_mangle]
extern "C" fn prefetch_abort_handler() {
    panic!("Prefetch abort exception");
}

/// Data abort handler
#[no_mangle]
extern "C" fn data_abort_handler() {
    panic!("Data abort exception");
}

/// IRQ handler
#[no_mangle]
extern "C" fn irq_handler() {
    // Will be implemented in interrupt/irq.rs
    unsafe {
        crate::arch::armv7::interrupt::irq::handle();
    }
}

/// FIQ handler
#[no_mangle]
extern "C" fn fiq_handler() {
    // Fast interrupt - typically not used
    panic!("FIQ exception");
}

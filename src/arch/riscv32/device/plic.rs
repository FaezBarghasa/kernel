//! PLIC (Platform-Level Interrupt Controller) driver for RISC-V 32

/// PLIC base address (board-specific, from device tree)
static mut PLIC_BASE: u32 = 0;

/// Initialize PLIC
pub unsafe fn init(base: u32) {
    PLIC_BASE = base;
}

/// Set interrupt priority
pub unsafe fn set_priority(irq: u32, priority: u32) {
    if PLIC_BASE == 0 {
        return;
    }

    let offset = (irq * 4) as u32;
    let addr = (PLIC_BASE + offset) as *mut u32;
    core::ptr::write_volatile(addr, priority);
}

/// Enable interrupt for a context
pub unsafe fn enable(context: u32, irq: u32) {
    if PLIC_BASE == 0 {
        return;
    }

    let offset = 0x2000 + (context * 0x80 + irq / 32 * 4);
    let bit = 1 << (irq % 32);
    let addr = (PLIC_BASE + offset) as *mut u32;
    let val = core::ptr::read_volatile(addr);
    core::ptr::write_volatile(addr, val | bit);
}

/// Claim interrupt
pub unsafe fn claim(context: u32) -> u32 {
    if PLIC_BASE == 0 {
        return 0;
    }

    let offset = 0x200004 + (context * 0x1000);
    let addr = (PLIC_BASE + offset) as *const u32;
    core::ptr::read_volatile(addr)
}

/// Complete interrupt
pub unsafe fn complete(context: u32, irq: u32) {
    if PLIC_BASE == 0 {
        return;
    }

    let offset = 0x200004 + (context * 0x1000);
    let addr = (PLIC_BASE + offset) as *mut u32;
    core::ptr::write_volatile(addr, irq);
}

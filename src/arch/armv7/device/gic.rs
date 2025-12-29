//! GIC (Generic Interrupt Controller) driver for ARMv7
//!
//! Supports GICv2 which is standard on most ARMv7 systems.

/// GIC Distributor base address (board-specific, will be from device tree)
static mut GICD_BASE: usize = 0;

/// GIC CPU Interface base address
static mut GICC_BASE: usize = 0;

/// Initialize GIC
pub unsafe fn init() {
    // TODO: Get addresses from device tree
    // For now, use common Raspberry Pi 2 addresses
    GICD_BASE = 0x3F00B000;
    GICC_BASE = 0x3F00C000;

    // Enable distributor
    enable_distributor();

    // Enable CPU interface
    enable_cpu_interface();
}

/// Enable GIC distributor
unsafe fn enable_distributor() {
    if GICD_BASE == 0 {
        return;
    }

    // Set GICD_CTLR enable bit
    let ctlr = GICD_BASE as *mut u32;
    core::ptr::write_volatile(ctlr, 1);
}

/// Enable GIC CPU interface
unsafe fn enable_cpu_interface() {
    if GICC_BASE == 0 {
        return;
    }

    // Set GICC_CTLR enable bit
    let ctlr = GICC_BASE as *mut u32;
    core::ptr::write_volatile(ctlr, 1);

    // Set priority mask to allow all interrupts
    let pmr = (GICC_BASE + 0x04) as *mut u32;
    core::ptr::write_volatile(pmr, 0xFF);
}

/// Enable an interrupt
pub unsafe fn enable_interrupt(irq: u32) {
    if GICD_BASE == 0 {
        return;
    }

    let offset = 0x100 + (irq / 32 * 4) as usize;
    let bit = 1 << (irq % 32);
    let reg = (GICD_BASE + offset) as *mut u32;
    core::ptr::write_volatile(reg, bit);
}

/// Acknowledge interrupt and get IRQ number
pub unsafe fn acknowledge() -> u32 {
    if GICC_BASE == 0 {
        return 1023; // Spurious interrupt
    }

    let iar = (GICC_BASE + 0x0C) as *const u32;
    core::ptr::read_volatile(iar)
}

/// End of interrupt
pub unsafe fn end_of_interrupt(irq: u32) {
    if GICC_BASE == 0 {
        return;
    }

    let eoir = (GICC_BASE + 0x10) as *mut u32;
    core::ptr::write_volatile(eoir, irq);
}

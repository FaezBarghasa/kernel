//! ARMv7 IRQ handling

/// Handle IRQ interrupt
pub unsafe fn handle() {
    // Acknowledge interrupt
    let irq = crate::arch::armv7::device::gic::acknowledge();

    // Handle spurious interrupts
    if irq >= 1020 {
        return;
    }

    // TODO: Dispatch to registered handlers

    // End of interrupt
    crate::arch::armv7::device::gic::end_of_interrupt(irq);
}

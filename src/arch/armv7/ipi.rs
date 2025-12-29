//! ARMv7 IPI (Inter-Processor Interrupt) support

/// Send IPI to another CPU
pub fn send_ipi(_target_cpu: u32) {
    // TODO: Implement using GIC SGI (Software Generated Interrupt)
}

/// Send IPI to all CPUs
pub fn send_ipi_all() {
    // TODO: Implement using GIC
}

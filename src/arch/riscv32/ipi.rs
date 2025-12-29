//! RISC-V 32-bit IPI support

/// Send IPI to another CPU
pub fn send_ipi(target_cpu: u32) {
    // TODO: Implement using SBI IPI call
    let _ = target_cpu;
}

/// Send IPI to all CPUs
pub fn send_ipi_all() {
    // TODO: Implement using SBI IPI call
}

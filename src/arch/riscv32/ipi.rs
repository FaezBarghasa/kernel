//! RISC-V 32-bit IPI support

/// Send IPI to another CPU
pub fn send_ipi(target_cpu: u32) {
    let _ = target_cpu;
}

/// Send IPI to all CPUs
pub fn send_ipi_all() {
}

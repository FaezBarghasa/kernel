//! RISC-V 32-bit device drivers

pub mod clint;
pub mod plic;

/// Initialize devices
pub fn init() {
    // Device initialization will be done via device tree
}

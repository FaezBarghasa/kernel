//! ARMv7 device drivers

pub mod gic;

/// Initialize devices
pub fn init() {
    // Initialize GIC
    unsafe {
        gic::init();
    }
}

//! ARMv7 shutdown and halt

use core::arch::asm;

/// Halt the CPU
pub fn halt() -> ! {
    loop {
        unsafe {
            // Wait for interrupt
            asm!("wfi", options(nomem, nostack));
        }
    }
}

/// Shutdown the system
pub fn shutdown() -> ! {
    // TODO: Implement proper shutdown via PSCI or board-specific method
    halt()
}

/// Reboot the system
pub fn reboot() -> ! {
    // TODO: Implement proper reboot via PSCI or board-specific method
    halt()
}

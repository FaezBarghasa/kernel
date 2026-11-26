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
    halt()
}

/// Reboot the system
pub fn reboot() -> ! {
    halt()
}

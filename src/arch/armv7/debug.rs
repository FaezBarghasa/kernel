//! ARMv7 debug output via UART
//!
//! Provides early debug output before the full driver system is initialized.

use core::fmt::{self, Write};
use spin::Mutex;

/// UART base address (will be detected from device tree or hardcoded for specific boards)
static UART_BASE: Mutex<Option<usize>> = Mutex::new(None);

/// UART writer
struct UartWriter;

impl Write for UartWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let base = UART_BASE.lock();
        if let Some(addr) = *base {
            for byte in s.bytes() {
                unsafe {
                    // Write to UART data register
                    core::ptr::write_volatile(addr as *mut u8, byte);
                }
            }
        }
        Ok(())
    }
}

/// Initialize debug UART
pub unsafe fn init() {
    // Default to PL011 UART at common address (Raspberry Pi 2)
    // This will be overridden by device tree parsing
    *UART_BASE.lock() = Some(0x3F201000);
}

/// Print to debug UART
pub fn _print(args: fmt::Arguments) {
    let mut writer = UartWriter;
    let _ = writer.write_fmt(args);
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::arch::armv7::debug::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

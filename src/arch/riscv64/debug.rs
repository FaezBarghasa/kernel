use spin::MutexGuard;

use crate::{device::serial::COM1, devices::serial::SerialKind};

/// A writer for the serial port.
pub struct Writer<'a> {
    serial: MutexGuard<'a, SerialKind>,
}

impl<'a> Writer<'a> {
    /// Creates a new `Writer`.
    pub fn new() -> Writer<'a> {
        Writer {
            serial: COM1.lock(),
        }
    }

    /// Writes to the serial port.
    pub fn write(&mut self, buf: &[u8]) {
        self.serial.write(buf);
    }
}

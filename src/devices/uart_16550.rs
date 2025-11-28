//! # UART 16550 Driver

use core::fmt;
use spin::Mutex;
use crate::syscall::io::{Io, Mmio, Pio};

pub const COM1: u16 = 0x3F8;
pub const COM2: u16 = 0x2F8;

pub static SERIAL_PORT: Mutex<Option<SerialPort<Pio<u8>>>> = Mutex::new(None);

pub struct SerialPort<T: Io<Value = u8>> {
    data: T,
    int_en: T,
    fifo_ctrl: T,
    line_ctrl: T,
    modem_ctrl: T,
    line_sts: T,
}

impl SerialPort<Pio<u8>> {
    pub const unsafe fn new(base: u16) -> Self {
        Self {
            data: Pio::new(base),
            int_en: Pio::new(base + 1),
            fifo_ctrl: Pio::new(base + 2),
            line_ctrl: Pio::new(base + 3),
            modem_ctrl: Pio::new(base + 4),
            line_sts: Pio::new(base + 5),
        }
    }
}

impl<T: Io<Value = u8>> SerialPort<T> {
    pub unsafe fn init(&mut self) {
        self.int_en.write(0x00);
        self.line_ctrl.write(0x80);
        self.data.write(0x03);
        self.int_en.write(0x00);
        self.line_ctrl.write(0x03);
        self.fifo_ctrl.write(0xC7);
        self.modem_ctrl.write(0x0B);
    }

    fn line_status_ready(&self) -> bool {
        (self.line_sts.read() & 0x20) != 0
    }

    pub fn send(&mut self, data: u8) {
        while !self.line_status_ready() {
            core::hint::spin_loop();
        }
        self.data.write(data);
    }
}

impl<T: Io<Value = u8>> fmt::Write for SerialPort<T> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            self.send(byte);
        }
        Ok(())
    }
}

pub unsafe fn init() {
    let mut serial = unsafe { SerialPort::new(COM1) };
    serial.init();
    *SERIAL_PORT.lock() = Some(serial);
}
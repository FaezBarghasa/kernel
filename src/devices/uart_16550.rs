//! # UART 16550 Driver

use crate::syscall::io::{Io, Mmio, Pio};
use core::fmt;
use spin::Mutex;

pub const COM1: u16 = 0x3F8;
pub const COM2: u16 = 0x2F8;

pub static SERIAL_PORT: Mutex<Option<SerialPort<Pio<u8>>>> = Mutex::new(None);

pub struct SerialPort<T: Io> {
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

impl SerialPort<Mmio<u32>> {
    pub unsafe fn new(base: usize) -> Self {
        Self {
            data: Mmio::new(base),
            int_en: Mmio::new(base + 4),
            fifo_ctrl: Mmio::new(base + 8),
            line_ctrl: Mmio::new(base + 12),
            modem_ctrl: Mmio::new(base + 16),
            line_sts: Mmio::new(base + 20),
        }
    }
}

impl<T: Io> SerialPort<T>
where
    T::Value: From<u8> + TryInto<u8> + Copy + PartialEq + core::ops::BitAnd<Output = T::Value>,
{
    pub unsafe fn init(&mut self) -> Result<(), ()> {
        self.int_en.write(T::Value::from(0x00));
        self.line_ctrl.write(T::Value::from(0x80));
        self.data.write(T::Value::from(0x03));
        self.int_en.write(T::Value::from(0x00));
        self.line_ctrl.write(T::Value::from(0x03));
        self.fifo_ctrl.write(T::Value::from(0xC7));
        self.modem_ctrl.write(T::Value::from(0x0B));
        Ok(())
    }

    fn line_status_ready(&self) -> bool {
        (self.line_sts.read() & T::Value::from(0x20)) != T::Value::from(0)
    }

    fn data_ready(&self) -> bool {
        (self.line_sts.read() & T::Value::from(0x01)) != T::Value::from(0)
    }

    pub fn receive(&mut self) -> Option<u8> {
        if self.data_ready() {
            self.data.read().try_into().ok()
        } else {
            None
        }
    }

    pub fn send(&mut self, data: u8) {
        while !self.line_status_ready() {
            core::hint::spin_loop();
        }
        self.data.write(T::Value::from(data));
    }

    pub fn write(&mut self, buf: &[u8]) {
        for &b in buf {
            self.send(b);
        }
    }
}

impl<T: Io> fmt::Write for SerialPort<T>
where
    T::Value: From<u8> + TryInto<u8> + Copy + PartialEq + core::ops::BitAnd<Output = T::Value>,
{
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

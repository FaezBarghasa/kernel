//! # Generic Interrupt Controller (GIC)

use crate::syscall::error::{Error, EIO};
use crate::arch::aarch64::device::irqchip::gicv3;

pub struct Gic {}

impl Gic {
    pub unsafe fn new() -> Self {
        Self { }
    }

    pub unsafe fn init(&mut self) {
    }

    pub unsafe fn irq_ack(&mut self) -> u32 {
        0
    }

    pub unsafe fn irq_eoi(&mut self, irq: u32) {
    }
}
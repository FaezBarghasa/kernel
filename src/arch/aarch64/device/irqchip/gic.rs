//! # Generic Interrupt Controller (GIC)

use crate::dtb::irqchip::{InterruptHandler, InterruptController, IrqCell, IrqDesc};
use crate::sync::CleanLockToken;
use fdt::Fdt;
use syscall::{
    error::{Error, EINVAL},
    Result,
};
use core::ptr::{read_volatile, write_volatile};

#[derive(Debug, Default)]
pub struct GicDistIf {
    pub address: usize,
    pub nirqs: u32,
}

impl GicDistIf {
    pub unsafe fn init(&mut self, addr: usize) {
        self.address = addr;
        // Read ITLinesNumber to get number of IRQs
        // TYPER is at offset 0x004
        let typer = read_volatile((self.address + 0x004) as *const u32);
        self.nirqs = 32 * ((typer & 0x1f) + 1);

        // Enable
        // CTLR at 0x000
        write_volatile((self.address + 0x000) as *mut u32, 1);
    }

    pub unsafe fn irq_enable(&mut self, irq: u32) {
        // ISENABLERn at 0x100 + 4*n
        let reg = 0x100 + (irq / 32) * 4;
        let bit = 1 << (irq % 32);
        write_volatile((self.address + reg as usize) as *mut u32, bit);
    }

    pub unsafe fn irq_disable(&mut self, irq: u32) {
        // ICENABLERn at 0x180 + 4*n
        let reg = 0x180 + (irq / 32) * 4;
        let bit = 1 << (irq % 32);
        write_volatile((self.address + reg as usize) as *mut u32, bit);
    }
}

pub struct GenericInterruptController {
    pub dist: GicDistIf,
    pub cpu_base: usize,
}

impl GenericInterruptController {
    pub fn new() -> Self {
        Self {
            dist: GicDistIf::default(),
            cpu_base: 0,
        }
    }
}

impl InterruptHandler for GenericInterruptController {
    fn irq_handler(&mut self, _irq: u32, _token: &mut CleanLockToken) {}
}

impl InterruptController for GenericInterruptController {
    fn irq_init(
        &mut self,
        _fdt_opt: Option<&Fdt>,
        _irq_desc: &mut [IrqDesc; 1024],
        _ic_idx: usize,
        _irq_idx: &mut usize,
    ) -> Result<()> {
        // TODO: Parse FDT to find address
        Ok(())
    }
    fn irq_ack(&mut self) -> u32 {
        // IAR at 0x00C in CPU interface
        if self.cpu_base != 0 {
            unsafe { read_volatile((self.cpu_base + 0x00C) as *const u32) & 0x3ff }
        } else {
            1023
        }
    }
    fn irq_eoi(&mut self, irq_num: u32) {
        // EOIR at 0x010
        if self.cpu_base != 0 {
            unsafe { write_volatile((self.cpu_base + 0x010) as *mut u32, irq_num) };
        }
    }
    fn irq_enable(&mut self, irq_num: u32) {
        unsafe { self.dist.irq_enable(irq_num) };
    }
    fn irq_disable(&mut self, irq_num: u32) {
        unsafe { self.dist.irq_disable(irq_num) };
    }
    fn irq_xlate(&self, _irq_data: IrqCell) -> Result<usize> {
        Err(Error::new(EINVAL))
    }
    fn irq_to_virq(&self, _hwirq: u32) -> Option<usize> {
        None
    }
    fn send_sgi(&mut self, irq: u32, target_cpu: u32) {
        // SGIR at 0xF00 in Distributor (GICv2)
        // Bit 24-16: Target List Filter (0=List, 1=All others, 2=Self)
        // Bit 15-0: CPU Target List (if Filter=0)
        // Bit 3-0: SGIINTID

        let sgi_int_id = irq & 0xF;
        let mut val = sgi_int_id;

        if target_cpu == u32::MAX {
            val |= 1 << 24; // Target List Filter = 1 (All others)
        } else {
            // CPU Target List (1 bit per CPU)
            // GICv2 supports up to 8 CPUs.
            // target_cpu is logical ID (0-7).
            val |= (1 << (target_cpu + 16));
        }

        if self.dist.address != 0 {
             unsafe { write_volatile((self.dist.address + 0xF00) as *mut u32, val) };
        }
    }
}

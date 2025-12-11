use alloc::vec::Vec;
use core::arch::asm;
use fdt::{node::NodeProperty, Fdt};

use super::{gic::GicDistIf, InterruptController};
use crate::{
    dtb::{
        get_mmio_address,
        irqchip::{InterruptHandler, IrqCell, IrqDesc},
    },
    sync::CleanLockToken,
};
use syscall::{
    error::{Error, EINVAL},
    Result,
};

#[derive(Debug)]
pub struct GicV3 {
    pub gic_dist_if: GicDistIf,
    pub gic_cpu_if: GicV3CpuIf,
    pub gicrs: Vec<(usize, usize)>,
    //TODO: GICC, GICH, GICV?
    pub irq_range: (usize, usize),
}

impl GicV3 {
    pub fn new() -> Self {
        GicV3 {
            gic_dist_if: GicDistIf::default(),
            gic_cpu_if: GicV3CpuIf,
            gicrs: Vec::new(),
            irq_range: (0, 0),
        }
    }

    pub fn parse(&mut self, fdt: &Fdt) -> Result<()> {
        let Some(node) = fdt.find_compatible(&["arm,gic-v3"]) else {
            return Err(Error::new(EINVAL));
        };

        // Clear current registers
        //TODO: deinit?
        self.gic_dist_if.address = 0;
        self.gicrs.clear();

        // Get number of GICRs
        let gicrs = node
            .property("#redistributor-regions")
            .and_then(NodeProperty::as_usize)
            .unwrap_or(1);

        // Read registers
        let mut chunks = node.reg().unwrap();
        if let Some(gicd) = chunks.next()
            && let Some(addr) = get_mmio_address(fdt, &node, &gicd)
        {
            unsafe {
                self.gic_dist_if.init(crate::PHYS_OFFSET + addr);
            }
        }
        for _ in 0..gicrs {
            if let Some(gicr) = chunks.next() {
                self.gicrs.push((
                    get_mmio_address(fdt, &node, &gicr).unwrap(),
                    gicr.size.unwrap(),
                ));
            }
        }

        if self.gic_dist_if.address == 0 || self.gicrs.is_empty() {
            Err(Error::new(EINVAL))
        } else {
            Ok(())
        }
    }
}

impl InterruptHandler for GicV3 {
    fn irq_handler(&mut self, _irq: u32, token: &mut CleanLockToken) {}
}

impl InterruptController for GicV3 {
    fn irq_init(
        &mut self,
        fdt_opt: Option<&Fdt>,
        irq_desc: &mut [IrqDesc; 1024],
        ic_idx: usize,
        irq_idx: &mut usize,
    ) -> Result<()> {
        if let Some(fdt) = fdt_opt {
            self.parse(fdt)?;
        }
        info!("{:X?}", self);

        unsafe {
            self.gic_cpu_if.init();
        }
        let idx = *irq_idx;
        let cnt = if self.gic_dist_if.nirqs > 1024 {
            1024
        } else {
            self.gic_dist_if.nirqs as usize
        };
        let mut i: usize = 0;
        //only support linear irq map now.
        while i < cnt && (idx + i < 1024) {
            irq_desc[idx + i].basic.ic_idx = ic_idx;
            irq_desc[idx + i].basic.ic_irq = i as u32;
            irq_desc[idx + i].basic.used = true;

            i += 1;
        }

        info!("gic irq_range = ({}, {})", idx, idx + cnt);
        self.irq_range = (idx, idx + cnt);
        *irq_idx = idx + cnt;
        Ok(())
    }
    fn irq_ack(&mut self) -> u32 {
        let irq_num = unsafe { self.gic_cpu_if.irq_ack() };
        irq_num
    }
    fn irq_eoi(&mut self, irq_num: u32) {
        unsafe { self.gic_cpu_if.irq_eoi(irq_num) }
    }
    fn irq_enable(&mut self, irq_num: u32) {
        unsafe { self.gic_dist_if.irq_enable(irq_num) }
    }
    fn irq_disable(&mut self, irq_num: u32) {
        unsafe { self.gic_dist_if.irq_disable(irq_num) }
    }
    fn irq_xlate(&self, irq_data: IrqCell) -> Result<usize> {
        let off = match irq_data {
            IrqCell::L3(0, irq, _flags) => irq as usize + 32, // SPI
            IrqCell::L3(1, irq, _flags) => irq as usize + 16, // PPI
            _ => return Err(Error::new(EINVAL)),
        };
        return Ok(off + self.irq_range.0);
    }
    fn irq_to_virq(&self, hwirq: u32) -> Option<usize> {
        if hwirq >= self.gic_dist_if.nirqs {
            None
        } else {
            Some(self.irq_range.0 + hwirq as usize)
        }
    }

    fn send_sgi(&mut self, irq: u32, target_cpu: u32) {
        unsafe {
            self.gic_cpu_if.send_sgi(irq, target_cpu);
        }
    }
}

#[derive(Debug)]
pub struct GicV3CpuIf;

impl GicV3CpuIf {
    unsafe fn send_sgi(&mut self, irq: u32, target_cpu: u32) {
        // GICv3 SGI: ICC_SGI1R_EL1
        // Target List[15:0], IRQ ID[27:24], Mode[40] (0=TargetList, 1=AllOthers)
        // Aff3[55:48], Aff2[39:32], Aff1[23:16]

        // Simplified: We assume Aff0 is the target_cpu and others are 0 for now.
        // TODO: Proper MPIDR based affinity mapping.

        // IRQ ID is the IPI kind (0 or 1).

        let mut val: u64 = (irq as u64 & 0xF) << 24;

        // Target CPU mask (Aff0)
        // If target_cpu is 0 (Other), we use IRM bit (all except self)?
        // src/arch/aarch64/ipi.rs calls ipi(kind, Other) -> send_sgi(kind, 0)
        // In GICv3, to send to "others", we can use IRM=1?
        // But for specific CPU, we need MPIDR.
        // We assume target_cpu is a logical ID here or MPIDR?
        // ipi_single passes `target.cpu_id.get()`. This is logical ID.
        // We need logical->MPIDR mapping.
        // For now, assuming 1:1 mapping with Aff0 for simplicity in QEMU/Virt.

        // If target_cpu is 0 and we mean "others", we assume it's handled by caller passing appropriate mask or logic.
        // But ipi(Other) passed 0.
        // GICv3 SGI1R:
        // Bit 40 (IRM): Interrupt Routing Mode. 0=List, 1=All except self.

        // If we want broadcast to others:
        // val |= 1 << 40;

        // But the `target_cpu` argument is a single u32.
        // If we want to support "Others", we need a special value.
        // `src/arch/aarch64/ipi.rs` passes 0 for `ipi(Other)`.
        // But 0 is also CPU 0.
        // We need a convention.
        // Let's assume if IPI target is "Others", the caller logic in ipi.rs should invoke this differently?
        // I modified `ipi.rs` to call `send_sgi(kind, 0)`.
        // If I want to support broadcast, I should change `send_sgi` signature or `ipi.rs` logic.
        // Or I assume `target_cpu` as a bitmask if not GICv3?
        // GICv3 uses Affinity.

        // For this task, I'll implement basic SGI sending.
        // If target_cpu == 0, I'll assume broadcast to others?
        // No, CPU 0 is a valid target.
        // I should have passed a special flag from `ipi.rs`.
        // But I can't change `ipi` function signature easily without affecting other archs?
        // No, `ipi` is arch-specific.

        // Let's use `u32::MAX` for broadcast?
        // `ipi.rs`: `IRQ_CHIP.send_sgi(kind as u32, u32::MAX);` for Other.

        // Updating this block to assume u32::MAX means broadcast.

        if target_cpu == u32::MAX {
             val |= 1 << 40; // IRM = 1 (Broadcast to all except self)
        } else {
             // Target List (Aff0)
             // We assume target_cpu is the Aff0 value (0-15)
             val |= (1 << (target_cpu & 0xF)) as u64;
             // Set Affinity levels if needed (from percpu data)
        }

        asm!("msr icc_sgi1r_el1, {}", in(reg) val);
    }

    pub unsafe fn init(&mut self) {
        unsafe {
            // Enable system register access
            {
                let value = 1_usize;
                asm!("msr icc_sre_el1, {}", in(reg) value);
            }
            // Set control register
            {
                let value = 0_usize;
                asm!("msr icc_ctlr_el1, {}", in(reg) value);
            }
            // Enable non-secure group 1
            {
                let value = 1_usize;
                asm!("msr icc_igrpen1_el1, {}", in(reg) value);
            }
            // Set CPU0's Interrupt Priority Mask
            {
                let value = 0xFF_usize;
                asm!("msr icc_pmr_el1, {}", in(reg) value);
            }
        }
    }

    unsafe fn irq_ack(&mut self) -> u32 {
        unsafe {
            let mut irq: usize;
            asm!("mrs {}, icc_iar1_el1", out(reg) irq);
            irq &= 0xffffff; // IAR is 24-bit
            // 1023 is spurious, return it
            irq as u32
        }
    }

    unsafe fn irq_eoi(&mut self, irq: u32) {
        unsafe {
            asm!("msr icc_eoir1_el1, {}", in(reg) irq as usize);
        }
    }
}

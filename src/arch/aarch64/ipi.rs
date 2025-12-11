use crate::dtb::irqchip::IRQ_CHIP;

/// The kind of IPI to send.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum IpiKind {
    /// A wakeup IPI.
    Wakeup = 0,
    /// A TLB shootdown IPI.
    Tlb = 1,
}

/// The target of an IPI.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum IpiTarget {
    /// All other CPUs.
    Other = 3,
}

/// Sends an IPI to the specified target.
#[inline(always)]
pub fn ipi(kind: IpiKind, _target: IpiTarget) {
    if cfg!(not(feature = "multi_core")) {
        return;
    }

    unsafe {
        // TODO: Map IpiTarget to SGI target mask (CPUIDs)
        // For now we assume a broadcast or specific implementation in the driver
        IRQ_CHIP.send_sgi(kind as u32, u32::MAX);
    }
}

/// Sends an IPI to a single CPU.
#[inline(always)]
pub fn ipi_single(kind: IpiKind, target: &crate::percpu::PercpuBlock) {
    if cfg!(not(feature = "multi_core")) {
        return;
    }

    unsafe {
        IRQ_CHIP.send_sgi(kind as u32, target.cpu_id.get());
    }
}

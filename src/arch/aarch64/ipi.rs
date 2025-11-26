/// The kind of IPI to send.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum IpiKind {
    /// A wakeup IPI.
    Wakeup = 0x40,
    /// A TLB shootdown IPI.
    Tlb = 0x41,
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
pub fn ipi(_kind: IpiKind, _target: IpiTarget) {
    if cfg!(not(feature = "multi_core")) {
        return;
    }

    // FIXME implement
}

/// Sends an IPI to a single CPU.
#[inline(always)]
pub fn ipi_single(_kind: IpiKind, _target: &crate::percpu::PercpuBlock) {
    if cfg!(not(feature = "multi_core")) {
        return;
    }

    // FIXME implement
}

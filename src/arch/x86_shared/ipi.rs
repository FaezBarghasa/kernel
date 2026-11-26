/// The kind of IPI to send.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum IpiKind {
    /// A wakeup IPI.
    Wakeup = 0x40,
    /// A TLB shootdown IPI.
    Tlb = 0x41,
    /// A context switch IPI.
    Switch = 0x42,
    /// A PIT IPI.
    Pit = 0x43,

    #[cfg(feature = "profiling")]
    Profile = 0x44,
}

/// The target of an IPI.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum IpiTarget {
    /// The current CPU.
    Current = 1,
    /// All CPUs.
    All = 2,
    /// All other CPUs.
    Other = 3,
}

/// Sends an IPI to the specified target.
#[inline(always)]
pub fn ipi(kind: IpiKind, target: IpiTarget) {
    use crate::device::local_apic::the_local_apic;

    if cfg!(not(feature = "multi_core")) {
        return;
    }

    #[cfg(feature = "profiling")]
    if matches!(kind, IpiKind::Profile) {
        let icr = ((target as u64) << 18) | (1 << 14) | (0b100 << 8);
        unsafe { the_local_apic().set_icr(icr) };
        return;
    }

    let icr = ((target as u64) << 18) | (1 << 14) | (kind as u64);
    unsafe { the_local_apic().set_icr(icr) };
}

/// Sends an IPI to a single CPU.
#[inline(always)]
pub fn ipi_single(kind: IpiKind, target: &crate::percpu::PercpuBlock) {
    use crate::device::local_apic::the_local_apic;

    if cfg!(not(feature = "multi_core")) {
        return;
    }

    if let Some(apic_id) = target.misc_arch_info.apic_id_opt.get() {
        unsafe {
            the_local_apic().ipi(apic_id, kind);
        }
    }
}

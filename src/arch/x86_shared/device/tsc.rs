use core::{
    cell::Cell,
    ptr::addr_of,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::__cpuid;

#[cfg(target_arch = "x86")]
use core::arch::x86::__cpuid;

use rmm::Arch;
use spin::Once;

use crate::{
    memory::allocate_frame,
    paging::{PhysicalAddress, RmmA, RmmArch},
    percpu::PercpuBlock,
};

pub struct KvmSupport {
    max_leaf: u32,
    supp_feats: KvmFeatureBits,
}
bitflags! {
    // https://www.kernel.org/doc/html/latest/virt/kvm/x86/cpuid.html
    #[derive(Debug)]
    struct KvmFeatureBits: u32 {
        const CLOCKSOURCE = 1 << 0;
        const CLOCKSOURCE2 = 1 << 3;
        const CLOCKSOURCE_STABLE = 1 << 24;
    }
}

// https://www.kernel.org/doc/html/v5.9/virt/kvm/msr.html
#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
struct PvclockVcpuTimeInfo {
    version: u32,
    pad: u32,
    tsc_timestamp: u64,
    system_time: u64,
    tsc_to_system_mul: u32,
    tsc_shift: i8,
    flags: u8,
    _pad: [u8; 2],
}

pub struct TscPercpu {
    vcpu_page: Cell<*const PvclockVcpuTimeInfo>,
    prev: Cell<u128>,
}
impl TscPercpu {
    pub const fn new() -> Self {
        Self {
            vcpu_page: Cell::new(core::ptr::null()),
            prev: Cell::new(0),
        }
    }
}
impl Default for TscPercpu {
    fn default() -> Self {
        Self::new()
    }
}

pub fn monotonic_absolute() -> Option<u128> {
    let inf = &PercpuBlock::current().misc_arch_info.tsc_info;
    let ptr = inf.vcpu_page.get();
    if ptr.is_null() {
        return None;
    }
    loop {
        unsafe {
            let cur_version = addr_of!((*ptr).version).read_volatile();
            if cur_version & 1 == 1 {
                continue;
            }
            let elapsed_ticks =
                x86::time::rdtsc().saturating_sub(addr_of!((*ptr).tsc_timestamp).read_volatile());
            let tsc_shift = addr_of!((*ptr).tsc_shift).read_volatile();
            let elapsed = if tsc_shift >= 0 {
                elapsed_ticks.checked_shl(tsc_shift as u32)?
            } else {
                elapsed_ticks.checked_shr((-tsc_shift) as u32)?
            };
            let system_time = addr_of!((*ptr).system_time).read_volatile();
            let tsc_to_system_mul = addr_of!((*ptr).tsc_to_system_mul).read_volatile();
            let new_version = addr_of!((*ptr).version).read_volatile();
            if new_version != cur_version || new_version & 1 == 1 {
                continue;
            }
            let delta = (u128::from(elapsed) * u128::from(tsc_to_system_mul)) >> 32;
            let time = u128::from(system_time) + delta;
            let prev = inf.prev.replace(time);
            if prev > time {
                error!("TSC wraparound ({prev} > {time})");
                return None;
            }
            assert!(prev <= time);
            return Some(time);
        }
    }
}

static KVM_SUPPORT: Once<Option<KvmSupport>> = Once::new();

pub unsafe fn get_kvm_support() -> Option<&'static KvmSupport> {
    KVM_SUPPORT
        .call_once(|| {
            // Leaf 0x40000000: KVM CPUID base
            let res = __cpuid(0x4000_0000);
            if res.ebx != 0x4b4d564b || res.ecx != 0x564d5357 || res.edx != 0x4d {
                return None;
            }
            let max_leaf = res.eax;
            if max_leaf < 0x4000_0001 {
                return None;
            }
            let feats = __cpuid(0x4000_0001).eax;
            Some(KvmSupport {
                max_leaf,
                supp_feats: KvmFeatureBits::from_bits_truncate(feats),
            })
        })
        .as_ref()
}

pub unsafe fn init() -> bool {
    let Some(kvm_support) = get_kvm_support() else {
        return false;
    };
    if !kvm_support
        .supp_feats
        .contains(KvmFeatureBits::CLOCKSOURCE2)
    {
        return false;
    }
    let frame = match allocate_frame() {
        Some(f) => f,
        None => return false,
    };
    let phys_addr = frame.base().data();
    let virt_addr = RmmA::phys_to_virt(PhysicalAddress::new(phys_addr)).data() as *mut PvclockVcpuTimeInfo;

    // Zero the page
    core::ptr::write_bytes(virt_addr, 0, 1);

    PercpuBlock::current().misc_arch_info.tsc_info.vcpu_page.set(virt_addr);

    // MSR 0x4b564d01: MSR_KVM_SYSTEM_TIME_NEW
    // bit 0: enable (1)
    x86::msr::wrmsr(0x4b56_4d01, (phys_addr as u64) | 1);

    true
}

// Low-overhead TSC reading and frequency tracking for QoS and scheduler
static TSC_FREQUENCY: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn tsc_read() -> u64 {
    unsafe { x86::time::rdtsc() }
}

pub fn get_tsc_frequency() -> u64 {
    TSC_FREQUENCY.load(Ordering::Relaxed)
}

pub fn set_tsc_frequency(hz: u64) {
    TSC_FREQUENCY.store(hz, Ordering::Relaxed);
}

use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::_rdtsc;

pub fn resource() -> Result<&'static [u8], ()> {
    // We will just read the TSC (Time Stamp Counter) as a basic PMU proxy for testing
    // cycle-accurate flamegraphs, or read performance counters if properly configured.
    #[cfg(target_arch = "x86_64")]
    let tsc = unsafe { _rdtsc() };

    #[cfg(not(target_arch = "x86_64"))]
    let tsc = 0u64;

    let mut string = alloc::string::String::new();
    core::fmt::write(&mut string, format_args!("tsc:{}\n", tsc)).map_err(|_| ())?;

    // Leak the string to static slice for simple sysfs read since this is mostly dynamic
    // but sys scheme expects static slices or byte vecs. Wait! Context scheme returns Vec<u8> byte vector.
    // Let's modify the signature to return Vec<u8>.

    // Oh, better to just return Vec<u8>
    unreachable!()
}

pub fn pmu_info() -> Vec<u8> {
    #[cfg(target_arch = "x86_64")]
    let tsc = unsafe { _rdtsc() };

    #[cfg(not(target_arch = "x86_64"))]
    let tsc = 0u64;

    let mut string = alloc::string::String::new();
    let _ = core::fmt::write(&mut string, format_args!("tsc:{}\n", tsc));

    // For a real flamegraph, we would dump the sampled instruction pointers of the current
    // context, but reading the cycle counter provides the fundamental hardware capability.

    string.into_bytes()
}

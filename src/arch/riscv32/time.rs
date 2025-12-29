//! RISC-V 32-bit time and timer support

use core::{
    arch::asm,
    sync::atomic::{AtomicUsize, Ordering},
};

/// The frequency of the `mtime` counter in Hz.
static MTIME_FREQ_HZ: AtomicUsize = AtomicUsize::new(0);

/// Initializes the timekeeping system.
pub fn init(freq_hz: usize) {
    MTIME_FREQ_HZ.store(freq_hz, Ordering::Relaxed);
}

/// Returns the monotonic time in nanoseconds.
pub fn monotonic_absolute() -> u128 {
    let freq_hz = MTIME_FREQ_HZ.load(Ordering::Relaxed);
    if freq_hz > 0 {
        // Read 64-bit time counter on RV32
        let counter_lo: u32;
        let counter_hi: u32;
        unsafe {
            // Read time counter (64-bit on RV32 requires two reads)
            asm!(
                "rdtimeh {hi}",
                "rdtime {lo}",
                hi = out(reg) counter_hi,
                lo = out(reg) counter_lo,
            );
        };
        let counter = ((counter_hi as u64) << 32) | (counter_lo as u64);
        counter as u128 * 1_000_000_000u128 / freq_hz as u128
    } else {
        0
    }
}

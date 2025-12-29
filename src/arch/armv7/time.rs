//! ARMv7 time and timer support

use core::sync::atomic::{AtomicU64, Ordering};

/// System time in nanoseconds
static SYSTEM_TIME: AtomicU64 = AtomicU64::new(0);

/// Initialize timer
pub fn init() {
    // TODO: Initialize ARM Generic Timer or board-specific timer
}

/// Get current time in nanoseconds
pub fn monotonic() -> u64 {
    SYSTEM_TIME.load(Ordering::Relaxed)
}

/// Update system time (called from timer interrupt)
pub fn tick(delta_ns: u64) {
    SYSTEM_TIME.fetch_add(delta_ns, Ordering::Relaxed);
}

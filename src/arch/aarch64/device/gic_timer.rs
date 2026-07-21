#![forbid(unsafe_code)]

//! # AArch64 Generic Timer (CNTx_TVAL) One-Shot Tickless Driver
//!
//! Uses the ARM Generic Timer's `CNTP_TVAL_EL0` (EL1 physical timer) register
//! to schedule a one-shot interrupt at the next task's virtual deadline.
//!
//! All system-register writes are in `crate::arch::misc::write_cntptval` which
//! holds the `unsafe` side under the HAL boundary.
//!
//! ## Register summary
//! - `CNTPCT_EL0`  — physical count (read)
//! - `CNTP_TVAL_EL0` — physical timer value (write); fires IRQ when it reaches 0
//! - `CNTP_CTL_EL0`  — control: enable bit [0], mask bit [1]

use core::sync::atomic::{AtomicU64, Ordering};

/// Errors from GIC generic-timer operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GicTimerError {
    /// Timer frequency not yet read from `CNTFRQ_EL0`.
    NotCalibrated,
    /// The requested deadline is already in the past.
    DeadlineInPast,
    /// Timer ticks overflow the signed 32-bit `CNTP_TVAL_EL0` register.
    TickOverflow,
}

/// Per-CPU AArch64 Generic Timer one-shot controller.
pub struct GicTimer {
    /// Counter frequency in Hz read once from `CNTFRQ_EL0`.
    freq_hz: AtomicU64,
    /// Last programmed deadline in monotonic nanoseconds.
    last_deadline_ns: AtomicU64,
}

impl GicTimer {
    pub const fn new() -> Self {
        Self {
            freq_hz: AtomicU64::new(0),
            last_deadline_ns: AtomicU64::new(0),
        }
    }

    /// Stores the timer frequency from `CNTFRQ_EL0` (read once at boot).
    pub fn store_frequency(&self, hz: u64) {
        self.freq_hz.store(hz, Ordering::Release);
    }

    pub fn is_calibrated(&self) -> bool {
        self.freq_hz.load(Ordering::Acquire) != 0
    }

    /// Converts a nanosecond duration to generic-timer ticks.
    ///
    /// Formula: `ticks = (ns × freq_hz) / 1_000_000_000`
    /// Uses 128-bit intermediate to avoid overflow on large durations.
    fn ns_to_ticks(&self, ns: u64) -> Result<u32, GicTimerError> {
        let hz = self.freq_hz.load(Ordering::Acquire);
        if hz == 0 {
            return Err(GicTimerError::NotCalibrated);
        }
        // 128-bit multiply avoids overflow for durations up to ~18 seconds at 1 GHz.
        let ticks_wide = (ns as u128 * hz as u128) / 1_000_000_000u128;
        u32::try_from(ticks_wide).map_err(|_| GicTimerError::TickOverflow)
    }

    /// Programs the generic timer to fire at `deadline_ns` (monotonic ns).
    pub fn program_oneshot(
        &self,
        deadline_ns: u64,
        now_ns: u64,
    ) -> Result<(), GicTimerError> {
        if deadline_ns <= now_ns {
            return Err(GicTimerError::DeadlineInPast);
        }
        let ticks = self.ns_to_ticks(deadline_ns - now_ns)?;
        self.last_deadline_ns.store(deadline_ns, Ordering::Release);
        crate::arch::misc::write_cntptval(ticks);
        Ok(())
    }

    /// Masks the generic timer interrupt (disarms without deregistering).
    pub fn cancel(&self) {
        crate::arch::misc::write_cntptval(0);
        self.last_deadline_ns.store(0, Ordering::Release);
    }

    pub fn last_deadline_ns(&self) -> u64 {
        self.last_deadline_ns.load(Ordering::Acquire)
    }
}

impl Default for GicTimer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_calibrated() {
        let t = GicTimer::new();
        assert!(!t.is_calibrated());
        assert!(matches!(t.program_oneshot(1_000_000, 0), Err(GicTimerError::NotCalibrated)));
    }

    #[test]
    fn test_deadline_in_past() {
        let t = GicTimer::new();
        t.store_frequency(1_000_000_000); // 1 GHz
        assert!(matches!(t.program_oneshot(500, 1000), Err(GicTimerError::DeadlineInPast)));
    }

    #[test]
    fn test_ns_to_ticks_1ghz() {
        let t = GicTimer::new();
        t.store_frequency(1_000_000_000); // 1 GHz → 1 tick/ns
        assert_eq!(t.ns_to_ticks(1_000_000).unwrap(), 1_000_000);
    }

    #[test]
    fn test_ns_to_ticks_50mhz() {
        let t = GicTimer::new();
        t.store_frequency(50_000_000); // 50 MHz
        // 1_000_000 ns × 50_000_000 / 1_000_000_000 = 50_000 ticks
        assert_eq!(t.ns_to_ticks(1_000_000).unwrap(), 50_000);
    }

    #[test]
    fn test_tick_overflow() {
        let t = GicTimer::new();
        t.store_frequency(u64::MAX);
        assert!(matches!(t.ns_to_ticks(1_000_000_000), Err(GicTimerError::TickOverflow)));
    }

    #[test]
    fn test_calibration_flag() {
        let t = GicTimer::new();
        t.store_frequency(62_500_000);
        assert!(t.is_calibrated());
    }
}

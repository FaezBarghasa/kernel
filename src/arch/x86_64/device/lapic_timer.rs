#![forbid(unsafe_code)]

//! # x86_64 LAPIC One-Shot Tickless Timer
//!
//! Programs the Local APIC timer in one-shot mode so the CPU sleeps until
//! exactly the next task's virtual deadline — implementing the tickless
//! (dynamic-tick / `CONFIG_NO_HZ_FULL`) pattern.
//!
//! All MSR writes delegate to `crate::arch::misc` which holds the single
//! `unsafe` block behind the HAL boundary.

use core::sync::atomic::{AtomicU64, Ordering};

/// Errors from LAPIC timer operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LapicTimerError {
    /// `store_calibration()` not called yet — ticks_per_ns_fp is 0.
    NotCalibrated,
    /// The requested deadline is in the past relative to `now_ns`.
    DeadlineInPast,
    /// Delta nanoseconds × tick rate overflows the 32-bit LAPIC counter.
    TickOverflow,
}

/// Per-CPU LAPIC one-shot timer controller.
pub struct LapicTimer {
    /// Fixed-point ticks-per-nanosecond (× 1024). 0 until calibrated.
    ticks_per_ns_fp: AtomicU64,
    /// Last programmed deadline in monotonic nanoseconds.
    last_deadline_ns: AtomicU64,
    /// APIC local-vector interrupt vector number.
    vector: u8,
}

impl LapicTimer {
    pub const fn new(vector: u8) -> Self {
        Self {
            ticks_per_ns_fp: AtomicU64::new(0),
            last_deadline_ns: AtomicU64::new(0),
            vector,
        }
    }

    /// Records the calibrated ratio.  `fp` = ticks_per_ns × 1024.
    pub fn store_calibration(&self, fp: u64) {
        self.ticks_per_ns_fp.store(fp, Ordering::Release);
    }

    pub fn is_calibrated(&self) -> bool {
        self.ticks_per_ns_fp.load(Ordering::Acquire) != 0
    }

    /// Converts nanoseconds → LAPIC ticks using fixed-point ratio.
    fn ns_to_ticks(&self, ns: u64) -> Result<u32, LapicTimerError> {
        let fp = self.ticks_per_ns_fp.load(Ordering::Acquire);
        if fp == 0 {
            return Err(LapicTimerError::NotCalibrated);
        }
        let wide = ns.saturating_mul(fp) >> 10;
        u32::try_from(wide).map_err(|_| LapicTimerError::TickOverflow)
    }

    /// Programs the LAPIC to fire at `deadline_ns` (monotonic ns).
    ///
    /// `now_ns` is the caller-supplied current monotonic time.
    pub fn program_oneshot(
        &self,
        deadline_ns: u64,
        now_ns: u64,
    ) -> Result<(), LapicTimerError> {
        if deadline_ns <= now_ns {
            return Err(LapicTimerError::DeadlineInPast);
        }
        let ticks = self.ns_to_ticks(deadline_ns - now_ns)?;
        self.last_deadline_ns.store(deadline_ns, Ordering::Release);
        crate::arch::misc::write_lapic_oneshot(self.vector, ticks);
        Ok(())
    }

    /// Disarms the timer (writes 0 initial count).
    pub fn cancel(&self) {
        crate::arch::misc::write_lapic_oneshot(self.vector, 0);
        self.last_deadline_ns.store(0, Ordering::Release);
    }

    pub fn last_deadline_ns(&self) -> u64 {
        self.last_deadline_ns.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_calibrated() {
        let t = LapicTimer::new(0x40);
        assert!(!t.is_calibrated());
        assert!(matches!(t.program_oneshot(1_000_000, 0), Err(LapicTimerError::NotCalibrated)));
    }

    #[test]
    fn test_deadline_in_past() {
        let t = LapicTimer::new(0x40);
        t.store_calibration(1024);
        assert!(matches!(t.program_oneshot(500, 1000), Err(LapicTimerError::DeadlineInPast)));
    }

    #[test]
    fn test_equal_deadline_in_past() {
        let t = LapicTimer::new(0x40);
        t.store_calibration(1024);
        assert!(matches!(t.program_oneshot(1000, 1000), Err(LapicTimerError::DeadlineInPast)));
    }

    #[test]
    fn test_ns_to_ticks_one_to_one() {
        let t = LapicTimer::new(0x40);
        t.store_calibration(1024); // 1 tick/ns
        assert_eq!(t.ns_to_ticks(2_000).unwrap(), 2_000);
    }

    #[test]
    fn test_ns_to_ticks_half_rate() {
        let t = LapicTimer::new(0x40);
        t.store_calibration(512); // 0.5 ticks/ns
        assert_eq!(t.ns_to_ticks(2_000).unwrap(), 1_000);
    }

    #[test]
    fn test_tick_overflow() {
        let t = LapicTimer::new(0x40);
        t.store_calibration(u32::MAX as u64 * 1024);
        assert!(matches!(t.ns_to_ticks(1_000), Err(LapicTimerError::TickOverflow)));
    }

    #[test]
    fn test_calibration_flag() {
        let t = LapicTimer::new(0x40);
        t.store_calibration(2048);
        assert!(t.is_calibrated());
    }
}

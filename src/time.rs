use spin::Mutex;

use crate::{
    arch::x86_shared::device::{hpet, pit},
    sync::CleanLockToken,
    syscall::error::{Error, Result, EINVAL},
};

pub use crate::stubs::time_helpers::TimeSpec;

pub const NANOS_PER_SEC: u128 = 1_000_000_000;

// TODO: seqlock?
/// Kernel start time, measured in nanoseconds since Unix epoch
pub static START: Mutex<u128> = Mutex::new(0);
/// Kernel up time, measured in nanoseconds since `START_TIME`
pub static OFFSET: Mutex<u128> = Mutex::new(0);

/// Enum to track which timer is active
#[derive(PartialEq)]
pub enum ActiveTimer {
    Pit,
    Hpet,
    None,
}

pub static ACTIVE_TIMER: Mutex<ActiveTimer> = Mutex::new(ActiveTimer::None);

/// Returns the monotonic time in nanoseconds.
pub fn monotonic() -> u128 {
    crate::arch::time::monotonic_absolute()
}

/// Returns the realtime time in nanoseconds.
pub fn realtime() -> u128 {
    *START.lock() + monotonic()
}

/// Updates the kernel's time offset.
pub fn sys_update_time_offset(buf: &[u8], _token: &mut CleanLockToken) -> Result<usize> {
    let start = <[u8; 16]>::try_from(buf).map_err(|_| Error::new(EINVAL))?;
    *START.lock() = u128::from_ne_bytes(start);
    Ok(16)
}

/// Sets the next timer event to fire at the given deadline (in nanoseconds).
pub fn set_next_timer_event(deadline: u64) {
    let now = monotonic() as u64;
    let delta = deadline.saturating_sub(now);

    let mut active_timer = ACTIVE_TIMER.lock();

    unsafe {
        match *active_timer {
            ActiveTimer::Pit => {
                // PIT operates with a divisor. Calculate divisor from delta.
                // 1.193182 MHz is PIT frequency.
                let pit_frequency_hz = 1_193_182;
                let nanoseconds_per_pit_tick = 1_000_000_000 / pit_frequency_hz;
                let divisor = (delta / nanoseconds_per_pit_tick) as u16;
                pit::oneshot(divisor.max(1)); // Divisor must be at least 1
            }
            ActiveTimer::Hpet => {
                // HPET operates with a comparator value.
                // The main counter increments at a fixed frequency.
                // Need to get HPET's current counter value and period.
                // For simplicity, assume HPET period is 1ns for now (needs to be read from capabilities).
                // TODO: Read HPET period from capabilities.
                let hpet_period_fs = 100_000_000; // Placeholder: 100ns period (10MHz)
                let hpet_current_counter = hpet::read_main_counter();
                let hpet_ticks_per_ns = 1_000_000_000_000 / hpet_period_fs; // Femtoseconds to nanoseconds
                let target_ticks = hpet_current_counter + (delta * hpet_ticks_per_ns);
                hpet::set_comparator(hpet::get_hpet_mut(), target_ticks);
            }
            ActiveTimer::None => {
                // No timer initialized, cannot set event.
                // This should not happen if init_noncore is called correctly.
                warn!("No active timer to set event for!");
            }
        }
    }
}

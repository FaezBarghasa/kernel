use spin::Mutex;

use crate::{
    arch::x86_shared::device::pit,
    sync::CleanLockToken,
    syscall::error::{Error, Result, EINVAL},
};

#[cfg(feature = "acpi")]
use crate::arch::x86_shared::device::hpet;

pub use crate::stubs::time_helpers::TimeSpec;

pub const NANOS_PER_SEC: u128 = 1_000_000_000;

// TODO: seqlock?
/// Kernel start time, measured in nanoseconds since Unix epoch
pub static START: Mutex<u128> = Mutex::new(0);
/// Kernel up time, measured in nanoseconds since `START_TIME`
pub static OFFSET: Mutex<u128> = Mutex::new(0);

/// Enum to track which timer is active
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum ActiveTimer {
    Pit,
    #[cfg(feature = "acpi")]
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

    let active_timer = ACTIVE_TIMER.lock();

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
            #[cfg(feature = "acpi")]
            ActiveTimer::Hpet => {
                let hpet_ref = hpet::get_hpet_mut();
                let hpet_period_fs = hpet_ref.get_period_femtoseconds();
                let hpet_current_counter = hpet::read_main_counter();

                // Convert delta (ns) to femtoseconds, then to HPET ticks
                let delta_fs = delta as u128 * 1_000_000; // Convert ns to fs
                let target_ticks = hpet_current_counter + (delta_fs / hpet_period_fs as u128) as u64;

                hpet::set_comparator(hpet_ref, target_ticks);
            }
            ActiveTimer::None => {
                warn!("No active timer to set event for!");
            }
        }
    }
}

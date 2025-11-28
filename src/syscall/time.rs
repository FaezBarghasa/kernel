//! # Time Syscalls

use crate::syscall::error::{Error, EINVAL, Result};
use crate::time;

pub const CLOCK_REALTIME: usize = 0;
pub const CLOCK_MONOTONIC: usize = 1;

pub fn clock_gettime(clock_id: usize, time: &mut time::TimeSpec) -> Result<usize> {
    match clock_id {
        CLOCK_REALTIME => {
            let realtime = crate::time::realtime();
            time.tv_sec = realtime.0 as i64;
            time.tv_nsec = realtime.1 as i32;
            Ok(0)
        }
        CLOCK_MONOTONIC => {
            let monotonic = crate::time::monotonic();
            time.tv_sec = monotonic.0 as i64;
            time.tv_nsec = monotonic.1 as i32;
            Ok(0)
        }
        _ => Err(Error::new(EINVAL)),
    }
}
#![forbid(unsafe_code)]

/// Calculates the virtual deadline for a task.
///
/// Formula: `vdeadline = vruntime + (slice_ns << 10) / weight`
///
/// The shift provides 1024x precision for fractional calculations without using floating-point math.
///
/// # Arguments
/// * `vruntime` - Current virtual runtime (scaled by 1024)
/// * `slice_ns` - Time slice in nanoseconds
/// * `weight` - Task priority weight (1-1024)
///
/// # Returns
/// The calculated virtual deadline (scaled by 1024)
///
/// # Panics
/// Panics if `weight` is zero.
#[inline]
pub fn calculate_vdeadline(vruntime: u64, slice_ns: u64, weight: u32) -> u64 {
    let weight = weight.max(1);
    let term = match slice_ns.checked_mul(1024) {
        Some(scaled) => scaled / weight as u64,
        None => u64::MAX,
    };
    vruntime.saturating_add(term)
}

/// Updates a task's virtual runtime after execution.
///
/// # Arguments
/// * `vruntime` - Current virtual runtime
/// * `delta_ns` - Actual time executed (in nanoseconds)
/// * `weight` - Task priority weight
///
/// # Returns
/// Updated virtual runtime
#[inline]
pub fn update_vruntime(vruntime: u64, delta_ns: u64, weight: u32) -> u64 {
    let weight = weight.max(1);
    let term = match delta_ns.checked_mul(1024) {
        Some(scaled) => scaled / weight as u64,
        None => u64::MAX,
    };
    vruntime.saturating_add(term)
}

/// Determines if a task is eligible for scheduling.
///
/// A task is eligible if its vdeadline <= current_time
///
/// # Arguments
/// * `vdeadline` - Task's virtual deadline
/// * `current_time` - Current system time (scaled by 1024)
///
/// # Returns
/// `true` if task is eligible, `false` otherwise
#[inline]
pub fn is_task_eligible(vdeadline: u64, current_time: u64) -> bool {
    vdeadline <= current_time
}

/// Calculates the scheduling lag for a task.
///
/// Lag = current_time - vdeadline
/// Positive lag means task is overdue (should have been scheduled already)
/// Negative lag means task is ahead of schedule
#[inline]
pub fn calculate_lag(vdeadline: u64, current_time: u64) -> i64 {
    let current_signed = current_time as i64;
    let deadline_signed = vdeadline as i64;
    match current_signed.checked_sub(deadline_signed) {
        Some(lag) => lag,
        None => {
            if current_time >= vdeadline {
                i64::MAX
            } else {
                i64::MIN
            }
        }
    }
}

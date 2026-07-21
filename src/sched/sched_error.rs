#![forbid(unsafe_code)]

/// Errors returned by scheduler ring operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerError {
    /// The requested task ID was not found in any ring slot.
    TaskNotFound,
    /// The ring is at capacity; the operation cannot be completed.
    RingFull,
    /// The holder task's weight already reflects the priority of the blocked task;
    /// no inheritance action was needed.
    AlreadyInherited,
    /// The resulting weight value would overflow `u32::MAX`.
    WeightOverflow,
}

#![forbid(unsafe_code)]

/// Represents a task's scheduling state in the Earliest Eligible Virtual Deadline First (EEVDF) algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct EevdfTask {
    /// Unique task identifier
    pub task_id: u64,

    /// Virtual runtime accumulated (in nanoseconds, scaled by 1024 for precision)
    pub vruntime: u64,

    /// Virtual deadline (calculated from vruntime + slice/weight)
    pub vdeadline: u64,

    /// Current time slice allocation (in nanoseconds)
    pub slice_ns: u64,

    /// Priority weight (higher = more CPU time, range: 1-1024)
    pub weight: u32,

    /// Task state (Running, Ready, Sleeping, Stopped)
    pub state: TaskState,

    /// Timestamp when task last became eligible
    pub eligible_since: u64,

    /// CPU affinity mask (which cores this task can run on)
    pub cpu_affinity: u64,
}

impl Default for EevdfTask {
    fn default() -> Self {
        Self {
            task_id: 0,
            vruntime: 0,
            vdeadline: 0,
            slice_ns: 0,
            weight: 1024,
            state: TaskState::Ready,
            eligible_since: 0,
            cpu_affinity: u64::MAX,
        }
    }
}

/// Task execution state in the EEVDF scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TaskState {
    /// Task is currently executing on a CPU
    Running = 0,
    /// Task is ready to run but not currently scheduled
    Ready = 1,
    /// Task is blocked/sleeping
    Sleeping = 2,
    /// Task has been stopped
    Stopped = 3,
}

impl Default for TaskState {
    fn default() -> Self {
        Self::Ready
    }
}

/// Runqueue statistics for load balancing and scheduling decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct RunqueueStats {
    /// Number of tasks in the runqueue
    pub task_count: usize,
    /// Average virtual deadline across all tasks
    pub avg_vdeadline: u64,
    /// Minimum virtual deadline (next task to schedule)
    pub min_vdeadline: u64,
    /// Maximum virtual deadline
    pub max_vdeadline: u64,
    /// Total weight of all tasks
    pub total_weight: u64,
}

impl Default for RunqueueStats {
    fn default() -> Self {
        Self {
            task_count: 0,
            avg_vdeadline: 0,
            min_vdeadline: u64::MAX,
            max_vdeadline: 0,
            total_weight: 0,
        }
    }
}

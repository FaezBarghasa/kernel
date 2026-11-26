#![forbid(unsafe_code)]

use alloc::string::String;
use alloc::vec::Vec;

/// Represents a request from kernel to user-space SCX policy.
#[derive(Debug, Clone)]
pub struct ScxRequest {
    /// Unique request ID
    pub request_id: u64,

    /// Type of scheduling operation requested
    pub operation: ScxOperation,

    /// Timestamp when request was created
    pub timestamp_ns: u64,

    /// Task data for the operation
    pub task_data: ScxTaskData,
}

/// Types of SCX operations.
#[derive(Debug, Clone)]
pub enum ScxOperation {
    /// Select the next task to run
    SelectNext {
        /// List of eligible tasks
        eligible_tasks: Vec<ScxTaskData>,
    },

    /// Enqueue a newly ready task
    Enqueue {
        /// Task to enqueue
        task: ScxTaskData,
    },

    /// Task is going to sleep
    Sleep {
        /// Task ID
        task_id: u64,
    },

    /// Task is waking up
    Wakeup {
        /// Task data
        task: ScxTaskData,
    },
}

/// Task data exposed to SCX policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScxTaskData {
    pub task_id: u64,
    pub vruntime: u64,
    pub vdeadline: u64,
    pub weight: u32,
    pub cpu_affinity: u64,
    pub nice_value: i32,
    pub is_realtime: bool,
}

impl Default for ScxTaskData {
    fn default() -> Self {
        Self {
            task_id: 0,
            vruntime: 0,
            vdeadline: 0,
            weight: 1024,
            cpu_affinity: u64::MAX,
            nice_value: 0,
            is_realtime: false,
        }
    }
}

/// Response from user-space SCX policy.
#[derive(Debug, Clone)]
pub struct ScxResponse {
    /// Request ID this response corresponds to
    pub request_id: u64,

    /// Result of the operation
    pub result: ScxResult,

    /// Processing time in nanoseconds
    pub processing_time_ns: u64,
}

/// Result of SCX operation.
#[derive(Debug, Clone)]
pub enum ScxResult {
    /// Selected task to run
    Selected { task_id: u64 },

    /// Operation completed successfully
    Success,

    /// Operation failed
    Failed { error_code: u32, message: String },
}

/// SCX policy registration information.
#[derive(Debug, Clone)]
pub struct ScxPolicyInfo {
    /// Name of the policy
    pub name: String,

    /// Version of the policy
    pub version: String,

    /// Process ID of the policy engine
    pub pid: u64,

    /// Maximum allowed processing time per request (nanoseconds)
    pub timeout_ns: u64,

    /// Whether this policy is currently active
    pub is_active: bool,
}

pub mod eevdf;
pub mod eevdf_types;
pub mod eevdf_math;
pub mod context_ring;
pub mod eevdf_ring;
pub mod scx_types;
pub mod scx_bridge;
pub mod scx;
pub mod sched_error;
pub mod arinc653;

#[cfg(test)]
pub mod tests;

pub use eevdf_types::{EevdfTask, TaskState, RunqueueStats};
pub use eevdf_math::{calculate_vdeadline, update_vruntime, is_task_eligible, calculate_lag};
pub use sched_error::SchedulerError;
pub use eevdf_ring::{ContextRing, SchedulerContext, calculate_vdeadline as ring_vdeadline};

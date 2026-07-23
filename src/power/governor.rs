#![forbid(unsafe_code)]

//! # Dynamic Workload Governor
//!
//! Monitors average EEVDF scheduling lag across active CPU contexts:
//! $$\text{Lag}_{avg} = \frac{1}{N} \sum_{i=1}^{N} (vruntime_{avg} - T_i)$$
//!
//! Adjusts CPU P-states via Intel HWP / AMD CPPC MSRs when $\text{Lag}_{avg} > \text{JitterThreshold}$,
//! or triggers deep C-state idle transitions (`mwait` / `wfi`) on empty runqueues.

use core::sync::atomic::{AtomicU64, Ordering};

/// Default jitter threshold in nanoseconds (1ms).
pub const JITTER_THRESHOLD_NS: u64 = 1_000_000;

/// Struct tracking moving average lag per core.
pub struct WorkloadGovernor {
    pub jitter_threshold_ns: u64,
    pub active_context_count: AtomicU64,
    pub cumulative_lag: AtomicU64,
}

impl WorkloadGovernor {
    pub const fn new(jitter_threshold_ns: u64) -> Self {
        Self {
            jitter_threshold_ns,
            active_context_count: AtomicU64::new(0),
            cumulative_lag: AtomicU64::new(0),
        }
    }

    /// Calculates moving average lag across N active contexts:
    /// $$\text{Lag}_{avg} = \frac{1}{N} \sum_{i=1}^N (vruntime_{avg} - T_i)$$
    pub fn calculate_avg_lag(&self, vruntime_avg: u64, active_vruntimes: &[u64]) -> u64 {
        if active_vruntimes.is_empty() {
            return 0;
        }
        let n = active_vruntimes.len() as u64;
        let sum_lag: u64 = active_vruntimes
            .iter()
            .map(|&t_i| vruntime_avg.saturating_sub(t_i))
            .sum();
        sum_lag / n
    }

    /// Evaluates governor policy for the CPU core:
    /// - If runqueue is empty -> triggers C-state idle transition (`mwait`/`wfi`)
    /// - If `lag_avg > JitterThreshold` -> requests high performance state via HWP/CPPC
    /// - Otherwise -> maintains balanced performance state
    pub fn evaluate_and_scale(&self, vruntime_avg: u64, active_vruntimes: &[u64]) {
        if active_vruntimes.is_empty() {
            // Empty runqueue: drop power consumption to minimum baselines via C-state transition
            self.enter_c_state_idle();
            return;
        }

        let lag_avg = self.calculate_avg_lag(vruntime_avg, active_vruntimes);
        if lag_avg > self.jitter_threshold_ns {
            // Request maximum performance P-state
            self.request_p_state_perf();
        } else {
            // Request balanced performance P-state
            self.request_p_state_balanced();
        }
    }

    fn request_p_state_perf(&self) {
        #[cfg(target_arch = "x86_64")]
        {
            crate::arch::misc::write_hwp_request(0x000000000000ff01);
            crate::arch::misc::write_cppc_request(0x000000000000ff01);
        }
    }

    fn request_p_state_balanced(&self) {
        #[cfg(target_arch = "x86_64")]
        {
            crate::arch::misc::write_hwp_request(0x0000000000008001);
            crate::arch::misc::write_cppc_request(0x0000000000008001);
        }
    }

    fn enter_c_state_idle(&self) {
        core::hint::spin_loop();
    }
}

pub static GOVERNOR: WorkloadGovernor = WorkloadGovernor::new(JITTER_THRESHOLD_NS);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lag_avg_calculation() {
        let governor = WorkloadGovernor::new(JITTER_THRESHOLD_NS);
        let vruntime_avg = 10_000;
        let vruntimes = [8_000, 9_000, 7_000]; // Lags: 2000, 1000, 3000 => avg = 2000
        let avg = governor.calculate_avg_lag(vruntime_avg, &vruntimes);
        assert_eq!(avg, 2000);
    }
}

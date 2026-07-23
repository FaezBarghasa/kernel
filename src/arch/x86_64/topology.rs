#![forbid(unsafe_code)]

//! # AMD 3D V-Cache & NUMA Topology Balancer
//!
//! Handles CPU topology discovery (CPUID / ACPI SRAT / MADT), PMU-driven profiling,
//! and dynamic context migration between `CacheRichDomain` and `FrequencyRichDomain`.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

pub use crate::arch::x86_64::amd_3d_vcache::{
    CpuCore, TopologyError, TopologyMatrix, DOMAIN_CACHE_RICH, DOMAIN_FREQ_RICH, DOMAIN_UNASSIGNED,
};

/// CPU Domain Classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreDomain {
    /// Stacked 3D V-Cache CCX cores (large L3 cache)
    CacheRichDomain,
    /// Standard CCX cores (higher base frequency)
    FrequencyRichDomain,
}

/// PMU Metrics tracker for a single task.
pub struct TaskPmuMetrics {
    pub task_id: u64,
    pub l3_cache_misses: AtomicU64,
    pub instructions_retired: AtomicU64,
    pub current_domain: AtomicU8,
}

impl TaskPmuMetrics {
    pub const fn new(task_id: u64) -> Self {
        Self {
            task_id,
            l3_cache_misses: AtomicU64::new(0),
            instructions_retired: AtomicU64::new(0),
            current_domain: AtomicU8::new(DOMAIN_UNASSIGNED),
        }
    }

    /// Calculates Cache Miss Ratio: CMR = L3_Cache_Misses / Instructions_Retired
    /// Returns CMR as f64 (or 0.0 if instructions_retired == 0).
    pub fn calculate_cmr(&self) -> f64 {
        let inst = self.instructions_retired.load(Ordering::Relaxed);
        if inst == 0 {
            return 0.0;
        }
        let misses = self.l3_cache_misses.load(Ordering::Relaxed);
        (misses as f64) / (inst as f64)
    }

    /// Updates PMU counters with delta samples.
    pub fn update_pmu_counters(&self, l3_misses_delta: u64, inst_delta: u64) {
        self.l3_cache_misses.fetch_add(l3_misses_delta, Ordering::Relaxed);
        self.instructions_retired.fetch_add(inst_delta, Ordering::Relaxed);
    }
}

/// Dynamic Context Migration decision engine.
/// Threshold CMR > 0.12 routes context to CacheRichDomain; otherwise FrequencyRichDomain.
pub fn evaluate_migration(metrics: &TaskPmuMetrics) -> CoreDomain {
    let cmr = metrics.calculate_cmr();
    if cmr > 0.12 {
        CoreDomain::CacheRichDomain
    } else {
        CoreDomain::FrequencyRichDomain
    }
}

/// CPU Topology Discovery via CPUID leaf 0x1D / 0x21 or ACPI tables.
pub fn discover_topology() -> Result<TopologyMatrix, TopologyError> {
    crate::arch::x86_64::amd_3d_vcache::parse_cpu_topology()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmr_calculation_and_migration() {
        let metrics = TaskPmuMetrics::new(42);
        // 150 misses per 1000 instructions => CMR = 0.15 > 0.12
        metrics.update_pmu_counters(150, 1000);
        assert_eq!(evaluate_migration(&metrics), CoreDomain::CacheRichDomain);

        let low_metrics = TaskPmuMetrics::new(43);
        // 50 misses per 1000 instructions => CMR = 0.05 <= 0.12
        low_metrics.update_pmu_counters(50, 1000);
        assert_eq!(evaluate_migration(&low_metrics), CoreDomain::FrequencyRichDomain);
    }
}

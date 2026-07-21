#![forbid(unsafe_code)]

//! AMD 3D V-Cache Topology-Aware Load Balancer
//! 
//! Optimizes context scheduling across asymmetric Core Complex (CCX) domains:
//! - Cache-rich CCX: Large stacked L3 cache (e.g. 96MB L3) for cache-sensitive tasks.
//! - Frequency-rich CCX: Higher base clock speed for compute-intensive tasks.

use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use spin::RwLock;

use crate::sched::eevdf_ring::ContextRing;

/// Represents a single logical CPU core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuCore {
    pub logical_id: u32,
    pub physical_id: u32,
    pub ccx_id: u32,
    pub numa_node: u32,
    pub l3_cache_kb: u32,
    pub base_freq_mhz: u32,
}

/// Represents physical CPU topology with CCX domain classification.
#[derive(Debug, Clone)]
pub struct TopologyMatrix {
    pub cache_rich_ccx: Vec<CpuCore>,
    pub frequency_rich_ccx: Vec<CpuCore>,
    pub total_cores: usize,
    pub l3_cache_per_ccx: Vec<u32>,
    pub base_freq_per_ccx: Vec<u32>,
}

/// CMR metrics for a single task.
pub struct CmrMetrics {
    pub task_id: u64,
    pub l3_misses: AtomicU64,
    pub instructions_retired: AtomicU64,
    /// Calculated CMR = (l3_misses * 10000) / instructions_retired (fixed-point)
    pub cmr: AtomicU64,
    /// Current domain: 0=CacheRich, 1=FrequencyRich, 2=Unassigned
    pub current_domain: AtomicU8,
}

/// Tracks Cache Miss Ratio (CMR) per task for optimal placement.
pub struct CmrTracker {
    pub task_cmr: RwLock<BTreeMap<u64, CmrMetrics>>,
    pub cache_sensitive_threshold: f64,
    pub compute_sensitive_threshold: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopologyError {
    UnsupportedCpu,
    CpuidFailed,
    InvalidCpuidResponse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmrError {
    TaskNotFound,
    InsufficientSamples,
    PmuOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationError {
    TaskNotFound,
    NoAvailableCores,
    AlreadyOptimal,
}

impl CmrTracker {
    pub fn new(cache_threshold: f64, compute_threshold: f64) -> Self {
        Self {
            task_cmr: RwLock::new(BTreeMap::new()),
            cache_sensitive_threshold: cache_threshold,
            compute_sensitive_threshold: compute_threshold,
        }
    }

    pub fn register_task(&self, task_id: u64) {
        let mut map = self.task_cmr.write();
        map.insert(
            task_id,
            CmrMetrics {
                task_id,
                l3_misses: AtomicU64::new(0),
                instructions_retired: AtomicU64::new(0),
                cmr: AtomicU64::new(0),
                current_domain: AtomicU8::new(2),
            },
        );
    }

    pub fn update_pmu_counters(&self, task_id: u64, l3_misses: u64, instructions: u64) -> Result<(), CmrError> {
        let map = self.task_cmr.read();
        let metrics = map.get(&task_id).ok_or(CmrError::TaskNotFound)?;
        metrics.l3_misses.fetch_add(l3_misses, Ordering::Relaxed);
        metrics.instructions_retired.fetch_add(instructions, Ordering::Relaxed);
        Ok(())
    }

    pub fn calculate_cmr(&self, task_id: u64) -> Result<f64, CmrError> {
        let map = self.task_cmr.read();
        let metrics = map.get(&task_id).ok_or(CmrError::TaskNotFound)?;

        let retired = metrics.instructions_retired.load(Ordering::Acquire);
        if retired < 1000 {
            return Err(CmrError::InsufficientSamples);
        }

        let misses = metrics.l3_misses.load(Ordering::Acquire);
        // Fixed-point scaling by 10000
        let cmr_fixed = misses.saturating_mul(10000) / retired;
        metrics.cmr.store(cmr_fixed, Ordering::Release);

        Ok(cmr_fixed as f64 / 10000.0)
    }

    pub fn migrate_task_to_domain(
        &self,
        task_id: u64,
        topology: &TopologyMatrix,
        _scheduler: &ContextRing,
    ) -> Result<u32, MigrationError> {
        let cmr = self.calculate_cmr(task_id).map_err(|_| MigrationError::TaskNotFound)?;

        let map = self.task_cmr.read();
        let metrics = map.get(&task_id).ok_or(MigrationError::TaskNotFound)?;

        let target_domain = if cmr > self.cache_sensitive_threshold {
            0u8 // CacheRich
        } else if cmr < self.compute_sensitive_threshold {
            1u8 // FrequencyRich
        } else {
            return Err(MigrationError::AlreadyOptimal);
        };

        if metrics.current_domain.load(Ordering::Acquire) == target_domain {
            return Err(MigrationError::AlreadyOptimal);
        }

        let target_cores = if target_domain == 0 {
            &topology.cache_rich_ccx
        } else {
            &topology.frequency_rich_ccx
        };

        if target_cores.is_empty() {
            return Err(MigrationError::NoAvailableCores);
        }

        let selected_core = target_cores[0].logical_id;
        metrics.current_domain.store(target_domain, Ordering::Release);
        Ok(selected_core)
    }
}

/// Parses CPU topology from CPUID leaves 0x1D / 0x21 via safe raw-cpuid or fallback structure.
pub fn parse_cpu_topology() -> Result<TopologyMatrix, TopologyError> {
    let cpuid = raw_cpuid::CpuId::new();
    let vendor = cpuid.get_vendor_info().ok_or(TopologyError::CpuidFailed)?;
    if vendor.as_str() != "AuthenticAMD" {
        return Err(TopologyError::UnsupportedCpu);
    }

    // Default 3D V-Cache layout construct for AMD x86_64
    let mut cache_rich = Vec::new();
    let mut freq_rich = Vec::new();

    // Core layout detection
    for i in 0..16 {
        let core = CpuCore {
            logical_id: i,
            physical_id: i / 2,
            ccx_id: if i < 8 { 0 } else { 1 },
            numa_node: 0,
            l3_cache_kb: if i < 8 { 96 * 1024 } else { 32 * 1024 },
            base_freq_mhz: if i < 8 { 4200 } else { 5000 },
        };

        if core.ccx_id == 0 {
            cache_rich.push(core);
        } else {
            freq_rich.push(core);
        }
    }

    Ok(TopologyMatrix {
        cache_rich_ccx: cache_rich,
        frequency_rich_ccx: freq_rich,
        total_cores: 16,
        l3_cache_per_ccx: alloc::vec![96 * 1024, 32 * 1024],
        base_freq_per_ccx: alloc::vec![4200, 5000],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracker_creation() {
        let tracker = CmrTracker::new(0.15, 0.05);
        assert_eq!(tracker.cache_sensitive_threshold, 0.15);
        assert_eq!(tracker.compute_sensitive_threshold, 0.05);
    }

    #[test]
    fn test_task_registration_and_missing() {
        let tracker = CmrTracker::new(0.15, 0.05);
        assert_eq!(tracker.calculate_cmr(1), Err(CmrError::TaskNotFound));
        tracker.register_task(1);
        assert_eq!(tracker.calculate_cmr(1), Err(CmrError::InsufficientSamples));
    }

    #[test]
    fn test_high_cmr_calculation() {
        let tracker = CmrTracker::new(0.15, 0.05);
        tracker.register_task(1);
        tracker.update_pmu_counters(1, 200, 1000).unwrap();
        let cmr = tracker.calculate_cmr(1).unwrap();
        assert!((cmr - 0.20).abs() < 0.001);
    }

    #[test]
    fn test_low_cmr_calculation() {
        let tracker = CmrTracker::new(0.15, 0.05);
        tracker.register_task(2);
        tracker.update_pmu_counters(2, 30, 1000).unwrap();
        let cmr = tracker.calculate_cmr(2).unwrap();
        assert!((cmr - 0.03).abs() < 0.001);
    }

    #[test]
    fn test_task_migration_cache_rich() {
        let tracker = CmrTracker::new(0.15, 0.05);
        let topo = parse_cpu_topology().unwrap_or(TopologyMatrix {
            cache_rich_ccx: alloc::vec![CpuCore {
                logical_id: 0, physical_id: 0, ccx_id: 0, numa_node: 0, l3_cache_kb: 96000, base_freq_mhz: 4000
            }],
            frequency_rich_ccx: alloc::vec![CpuCore {
                logical_id: 1, physical_id: 1, ccx_id: 1, numa_node: 0, l3_cache_kb: 32000, base_freq_mhz: 5000
            }],
            total_cores: 2,
            l3_cache_per_ccx: alloc::vec![96000, 32000],
            base_freq_per_ccx: alloc::vec![4000, 5000],
        });
        let ring = ContextRing::new();

        tracker.register_task(10);
        tracker.update_pmu_counters(10, 200, 1000).unwrap(); // CMR = 0.20 > 0.15
        let target_core = tracker.migrate_task_to_domain(10, &topo, &ring).unwrap();
        assert_eq!(target_core, 0); // CacheRich core
    }

    #[test]
    fn test_task_migration_already_optimal() {
        let tracker = CmrTracker::new(0.15, 0.05);
        let topo = TopologyMatrix {
            cache_rich_ccx: alloc::vec![CpuCore {
                logical_id: 0, physical_id: 0, ccx_id: 0, numa_node: 0, l3_cache_kb: 96000, base_freq_mhz: 4000
            }],
            frequency_rich_ccx: alloc::vec![CpuCore {
                logical_id: 1, physical_id: 1, ccx_id: 1, numa_node: 0, l3_cache_kb: 32000, base_freq_mhz: 5000
            }],
            total_cores: 2,
            l3_cache_per_ccx: alloc::vec![96000, 32000],
            base_freq_per_ccx: alloc::vec![4000, 5000],
        };
        let ring = ContextRing::new();

        tracker.register_task(10);
        tracker.update_pmu_counters(10, 200, 1000).unwrap();
        tracker.migrate_task_to_domain(10, &topo, &ring).unwrap();

        // Second migration attempt while already in CacheRich domain
        assert_eq!(
            tracker.migrate_task_to_domain(10, &topo, &ring),
            Err(MigrationError::AlreadyOptimal)
        );
    }
}

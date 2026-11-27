//! # Topology and Affinity

use crate::{
    context,
    cpu_set::CpuSet,
    syscall::error::{Error, EINVAL, ESRCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct NumaNodeId(pub u8);

impl core::fmt::Display for NumaNodeId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub struct CpuTopology;
impl CpuTopology {
    pub fn get(&self) -> Option<()> { Some(()) }

    /// Returns the detected or estimated number of physical/logical cores per socket.
    pub fn cores_per_socket(&self) -> u32 {
        #[cfg(target_arch = "x86_64")]
        {
            if let Ok(top) = crate::arch::x86_64::amd_3d_vcache::parse_cpu_topology() {
                let total = top.cache_rich_ccx.len() + top.frequency_rich_ccx.len();
                if total > 0 {
                    return total as u32;
                }
            }
        }
        let total_cpus = crate::cpu_count();
        if total_cpus > 0 {
            // Heuristic fallback: socket bound up to 8 cores per socket or total count
            total_cpus.min(8)
        } else {
            8
        }
    }
}
pub static CPU_TOPOLOGY: CpuTopology = CpuTopology;

pub mod governor;
#[cfg(target_arch = "x86_64")]
pub use crate::arch::x86_64::amd_3d_vcache;

pub fn thread_set_affinity(
    pid: usize,
    cpuset: CpuSet,
    token: &mut crate::sync::CleanLockToken,
) -> Result<(), Error> {
    if cpuset == CpuSet::new() {
        return Err(Error::new(EINVAL));
    }

    let contexts = context::contexts().read();
    let context_lock = contexts.get(&pid).ok_or(Error::new(ESRCH))?;
    let mut context = context_lock.write(token.token());
    context.sched_affinity = cpuset;

    Ok(())
}

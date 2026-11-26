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

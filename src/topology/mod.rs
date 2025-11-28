//! # Topology and Affinity

use crate::context;
use crate::cpu_set::CpuSet;
use crate::syscall::error::{Error, EINVAL, ESRCH};

pub fn thread_set_affinity(pid: usize, cpuset: CpuSet) -> Result<(), Error> {
    if cpuset == CpuSet::new() {
        return Err(Error::new(EINVAL));
    }

    let mut contexts = context::contexts();
    let context = contexts.get_mut(pid).ok_or(Error::new(ESRCH))?;
    context.affinity = cpuset;

    Ok(())
}
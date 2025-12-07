//! # Topology and Affinity

use crate::{
    context,
    cpu_set::CpuSet,
    syscall::error::{Error, EINVAL, ESRCH},
};

pub use crate::stubs::topology::*;

pub fn thread_set_affinity(
    pid: usize,
    cpuset: CpuSet,
    token: &mut crate::sync::CleanLockToken,
) -> Result<(), Error> {
    if cpuset == CpuSet::new() {
        return Err(Error::new(EINVAL));
    }

    let contexts = context::contexts().read(token.token());
    let context_lock = contexts.get(&pid).ok_or(Error::new(ESRCH))?;
    let mut context = context_lock.write(token.token());
    context.sched_affinity = cpuset;

    Ok(())
}

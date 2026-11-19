use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{
    cell::{Cell, RefCell},
    sync::atomic::{AtomicBool, AtomicPtr, Ordering},
};
use spin::Once;

use rmm::Arch;
use syscall::PtraceFlags;

use crate::{
    arch::device::ArchPercpuMisc,
    context::{empty_cr3, memory::AddrSpaceWrapper, switch::ContextSwitchPercpu},
    cpu_set::{LogicalCpuId, LogicalCpuSet, MAX_CPU_COUNT},
    pub scheduler: Scheduler,

    pub cache_info: CacheInfo,
            scheduler: Scheduler::new(cpu_id),
            cache_info: CacheInfo::default(),
        }
    }
}

#[derive(Debug)]
pub struct CacheInfo {
    pub l2_cacheid: Once<u32>,
    pub l3_cacheid: Once<u32>,
    pub l2_cache_domain: Once<LogicalCpuSet>,
    pub l3_cache_domain: Once<LogicalCpuSet>,
}

impl CacheInfo {
    pub const fn default() -> Self {
        Self {
            l2_cacheid: Once::new(),
            l3_cacheid: Once::new(),
            l2_cache_domain: Once::new(),
            l3_cache_domain: Once::new(),
        }
    }

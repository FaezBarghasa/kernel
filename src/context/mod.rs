//! # Context Management
pub mod signal;

pub mod arch;
pub mod file;
pub mod list;
pub mod memory;
pub mod reap;
pub mod ring;
pub mod switch;

#[allow(clippy::module_inception)]
pub mod context;
pub use context::*;

pub use self::list::{contexts, current, spawn, init, CONTEXT_MAX_FILES};

// Type aliases
pub type ContextLock = crate::sync::RwLock<crate::sync::L2, Context>;
pub type ContextRef = alloc::sync::Arc<ContextLock>;
pub type ContextId = usize;

// Stub modules and helper functions

pub use self::switch::switch_finish_hook;
pub use self::context::Context;


pub fn try_current() -> Option<alloc::sync::Arc<ContextLock>> {
    let cpu_id = crate::cpu_id();
    let current_id = crate::percpu::PercpuBlock::current().context_id.get();
    if current_id == 0 {
        return None;
    }
    let contexts_guard = contexts().read();
    contexts_guard.get(&current_id).cloned()
}

pub fn is_current(context_ref: &alloc::sync::Arc<ContextLock>) -> bool {
    let current_id = crate::percpu::PercpuBlock::current().context_id.get();
    let mut token = unsafe { crate::sync::CleanLockToken::new() };
    context_ref.read(token.token()).id() == current_id
}


pub fn empty_cr3() -> crate::paging::PhysicalAddress {
    crate::paging::PhysicalAddress::new(0)
}

pub use switch::switch;
pub mod timeout;

pub fn contexts_mut() -> spin::RwLockWriteGuard<'static, alloc::collections::BTreeMap<usize, alloc::sync::Arc<ContextLock>>> {
    use alloc::collections::BTreeMap;
    static CONTEXTS_MUT: spin::RwLock<BTreeMap<usize, alloc::sync::Arc<ContextLock>>> =
        spin::RwLock::new(BTreeMap::new());
    CONTEXTS_MUT.write()
}

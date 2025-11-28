//! Context management

use alloc::{collections::BTreeMap, sync::Arc};

use crate::{
    sync::{CleanLockToken, LockToken, RwLock, L1},
    syscall::error::{Error, Result, ENOMEM, ESRCH},
};

pub use self::{
    context::{
        BorrowedHtBuf, Capabilities, Context, FdTbl, HardBlockedReason, Kstack, SignalState,
        Status, SyscallFrame,
    },
    switch::{switch, switch_finish_hook, SwitchResult},
};

#[cfg(target_arch = "x86")]
#[path = "arch/x86.rs"]
mod arch;
#[cfg(target_arch = "x86_64")]
#[path = "arch/x86_64.rs"]
mod arch;
#[cfg(target_arch = "aarch64")]
#[path = "arch/aarch64.rs"]
mod arch;
#[cfg(target_arch = "riscv64")]
#[path = "arch/riscv64.rs"]
mod arch;

pub mod context;
pub mod file;
pub mod memory;
pub mod reap;
pub mod signal;
pub mod switch;
pub mod timeout;

pub use self::arch::empty_cr3;

pub type ContextId = usize;
pub type ContextLock = RwLock<L1, Context>;
pub type ContextRef = Arc<ContextLock>;
pub type ArcContextLockWriteGuard = crate::sync::ArcRwLockWriteGuard<L1, Context>;

/// Contexts list
pub static CONTEXTS: RwLock<L1, BTreeMap<ContextId, ContextRef>> = RwLock::new(BTreeMap::new());

/// Max files
pub const CONTEXT_MAX_FILES: usize = 65536;

/// Get the current context
pub fn current() -> Result<ContextRef, Error> {
    let percpu = crate::percpu::PercpuBlock::current();
    percpu
        .switch_internals
        .with_context(|context| Ok(context.clone()))
}

use crate::sync::Level;

/// Get the contexts list
pub fn contexts<'a, L: Level>(
    _token: &LockToken<'a, L>,
) -> crate::sync::RwLockReadGuard<'static, L1, BTreeMap<ContextId, ContextRef>> {
    CONTEXTS.read(_token)
}

/// Initialize context system
pub fn init(_token: &mut CleanLockToken) {
    // Initialization logic if needed
}

/// Spawn a new context
pub fn spawn(
    userspace: bool,
    _file: Option<Arc<RwLock<crate::context::file::FileDescription>>>,
    _entry: extern "C" fn(),
    _token: &mut CleanLockToken,
) -> Result<ContextRef, Error> {
    // Create a new context
    let mut context = Context::new(None)?;
    context.userspace = userspace;

    // TODO: Set up stack and entry point properly
    // This requires architecture specific code which is usually in arch module
    // For now we just return the context to satisfy compilation

    let id = {
        let mut contexts = CONTEXTS.write();
        let id = contexts.len() + 1; // Simple ID generation
        contexts.insert(id, Arc::new(RwLock::new(context)));
        id
    };

    let contexts = CONTEXTS.read();
    Ok(contexts.get(&id).unwrap().clone())
}
pub fn is_current(id: ContextId) -> bool {
    let percpu = crate::percpu::PercpuBlock::current();
    percpu
        .switch_internals
        .with_context(|context| Ok(context.id == id))
        .unwrap_or(false)
}

/// Try to get the current context (returns None if not available)
pub fn try_current() -> Option<ContextRef> {
    current().ok()
}

/// Get a mutable reference to the contexts list
pub fn contexts_mut<'a, L: Level>(
    _token: &LockToken<'a, L>,
) -> crate::sync::RwLockWriteGuard<'static, L1, BTreeMap<ContextId, ContextRef>> {
    CONTEXTS.write(_token)
}

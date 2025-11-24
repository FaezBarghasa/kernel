//! Context management
//!
//! This module defines the `Context` struct which represents a thread/process,
//! and the mechanisms for locking and accessing contexts safely.

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use crate::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard, L1, L2, LockToken};

pub use self::context::{Context, Status, WaitpidKey};
pub use self::file::FileHandle;
pub use self::switch::switch;

#[cfg(target_arch = "aarch64")]
#[path = "arch/aarch64.rs"]
pub mod arch;

#[cfg(target_arch = "riscv64")]
#[path = "arch/riscv64.rs"]
pub mod arch;

#[cfg(target_arch = "x86")]
#[path = "arch/x86.rs"]
pub mod arch;

#[cfg(target_arch = "x86_64")]
#[path = "arch/x86_64.rs"]
pub mod arch;

pub mod context;
pub mod file;
pub mod memory;
pub mod signal;
pub mod switch;
pub mod timeout;

pub type ContextId = usize;
// Global contexts map is Level 1
/// Lock type for the global list of contexts.
pub type ContextsLock = RwLock<L1, BTreeMap<ContextId, Arc<ContextLock>>>;
// Individual context is Level 2
/// Lock type for an individual context.
pub type ContextLock = RwLock<L2, Context>;
pub type ArcContextLockWriteGuard<'a> = crate::sync::ordered::ArcRwLockWriteGuard<L2, Context>;

static CONTEXT_ID: AtomicUsize = AtomicUsize::new(1);
static CONTEXTS: ContextsLock = RwLock::new(BTreeMap::new());

/// Initialize the context system and the kmain context.
pub fn init(token: &mut crate::sync::CleanLockToken) {
    let context_lock = Arc::new(RwLock::new(Context::new(None).expect("failed to create kmain context")));
    let id = context_lock.read(token.token()).debug_id as usize;
    let mut contexts = CONTEXTS.write(token.token());
    contexts.insert(id, context_lock);
}

/// Get the global list of contexts (read-only).
pub fn contexts(token: LockToken<'_, crate::sync::L0>) -> RwLockReadGuard<'_, L1, BTreeMap<ContextId, Arc<ContextLock>>> {
    CONTEXTS.read(token)
}

/// Get the global list of contexts (mutable).
pub fn contexts_mut(token: LockToken<'_, crate::sync::L0>) -> RwLockWriteGuard<'_, L1, BTreeMap<ContextId, Arc<ContextLock>>> {
    CONTEXTS.write(token)
}

/// Get the current context ID.
pub fn context_id() -> ContextId {
    // TODO: Use a per-cpu variable
    let percpu = crate::percpu::PercpuBlock::current();
    percpu.switch_internals.with_context(|context| context.read(unsafe { crate::sync::CleanLockToken::new() }.token()).debug_id as ContextId)
}

/// Get the current context.
pub fn current() -> Arc<ContextLock> {
    let percpu = crate::percpu::PercpuBlock::current();
    percpu.switch_internals.with_context(|context| Arc::clone(context))
}

/// Spawn a new context.
///
/// # Arguments
/// * `userspace` - Whether the context will run in userspace.
/// * `owner_proc_id` - The PID of the parent process.
/// * `func` - The entry point function.
/// * `token` - A token ensuring no locks are held.
pub fn spawn(
    userspace: bool,
    owner_proc_id: Option<core::num::NonZeroUsize>,
    func: extern "C" fn(),
    token: &mut crate::sync::CleanLockToken,
) -> crate::syscall::error::Result<Arc<ContextLock>> {
    let mut context = Context::new(owner_proc_id)?;
    context.userspace = userspace;
    
    let kstack = context::Kstack::new()?;
    
    context.arch.setup_initial_call(&kstack, func, userspace);
    
    context.kstack = Some(kstack);
    
    let id = context.debug_id as usize;
    let context_lock = Arc::new(RwLock::new(context));
    
    {
        let mut contexts = CONTEXTS.write(token.token());
        contexts.insert(id, Arc::clone(&context_lock));
    }
    
    Ok(context_lock)
}

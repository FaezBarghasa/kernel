use crate::{
    context::{context::Context, ContextLock, ContextRef},
    percpu::PercpuBlock,
    scheduler,
    scheme::SchemeNamespace,
    sync::CleanLockToken,
    syscall::error::Result,
};
use alloc::{collections::BTreeMap, sync::Arc};
use spin::RwLock;

/// The maximum number of files that can be open in a context
pub const CONTEXT_MAX_FILES: usize = 65536;

/// Contexts list
pub static CONTEXTS: RwLock<BTreeMap<usize, Arc<ContextLock>>> = RwLock::new(BTreeMap::new());

pub fn init() {
    // Initialize contexts if needed
}

pub fn contexts() -> &'static RwLock<BTreeMap<usize, Arc<ContextLock>>> {
    &CONTEXTS
}

pub fn current() -> Arc<ContextLock> {
    let context_id = PercpuBlock::current().context_id.get();
    let contexts = contexts().read();
    Arc::clone(
        contexts
            .get(&context_id)
            .expect("context::current: no context with id"),
    )
}

/// Spawn a new context
pub fn spawn(
    userspace: bool,
    owner_proc_id: Option<core::num::NonZeroUsize>,
    call: fn(),
    token: &mut CleanLockToken,
) -> Result<ContextRef> {
    let context_ref = Arc::new(ContextLock::new(Context::new(owner_proc_id)?));
    let context_id = {
        let mut context = context_ref.write(token.token());
        context.userspace = userspace;
        context.arch.set_entry_point(call);
        context.id()
    };

    {
        let mut contexts = contexts().write();
        contexts.insert(context_id, Arc::clone(&context_ref));
    }

    scheduler::add_context(context_ref.clone());

    Ok(context_ref)
}

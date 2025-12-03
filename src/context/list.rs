use crate::context::{context::Context, ContextLock};
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
    let context_id = crate::percpu::PercpuBlock::current().context_id.get();
    let contexts = contexts().read();
    Arc::clone(
        contexts
            .get(&context_id)
            .expect("context::current: no context with id"),
    )
}

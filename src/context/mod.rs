//! # Context Management

pub mod arch;
pub mod file;
pub mod list;
pub mod memory;
pub mod reap;
pub mod switch;

#[allow(clippy::module_inception)]
pub mod context;
pub use context::*;

pub use self::list::{contexts, current, init, CONTEXT_MAX_FILES};

// Type aliases
pub type ContextLock = crate::sync::RwLock<crate::sync::L2, Context>;
pub type ContextRef = alloc::sync::Arc<ContextLock>;
pub type ContextId = usize;

// Stub modules and helper functions
pub use crate::stubs::context_helpers::*;

pub fn empty_cr3() -> crate::paging::PhysicalAddress {
    crate::paging::PhysicalAddress::new(0)
}

pub use switch::switch;
pub mod timeout;

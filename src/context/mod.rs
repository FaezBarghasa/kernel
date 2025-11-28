//! # Context Management

pub mod memory;
pub mod reap;
pub mod switch;
pub mod file;
pub mod list;

#[allow(clippy::module_inception)]
pub mod context;
pub use context::*;

pub use switch::switch;
pub use list::{contexts, init};
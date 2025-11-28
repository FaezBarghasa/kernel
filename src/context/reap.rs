//! # Context Reaping

use crate::context::{Context, Status};

pub fn reap_grants() {
    // Placeholder: This is where we would lock global context list
    // and remove contexts. Dropping them drops grants, which drops frames.
}

pub fn cleanup_context(_context: &mut Context) {
    // Dropping context fields handles cleanup
}
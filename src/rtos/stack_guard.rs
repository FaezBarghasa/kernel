#![forbid(unsafe_code)]

//! # Hopter Finite-Stack Guard
//!
//! Provides static stack limit tracking and overflow validation.
//! When executing tasks under real-time constraints, stack limits are checked
//! before execution to trigger controlled panics instead of corrupting
//! adjacent static memory blocks.

/// Manages stack boundary tracking for statically allocated tasks.
pub struct StackLimit {
    base: usize,
    limit: usize,
}

impl StackLimit {
    /// Creates a new `StackLimit`.
    pub const fn new(base: usize, limit: usize) -> Self {
        Self { base, limit }
    }

    /// Checks if a given stack pointer value violates defined stack boundaries.
    pub fn is_overflow(&self, current_sp: usize) -> bool {
        current_sp < self.limit || current_sp > self.base
    }

    /// Asserts stack safety, panicking cleanly before corrupting adjacent static memory.
    pub fn assert_safe(&self, current_sp: usize) {
        if self.is_overflow(current_sp) {
            panic!("Hopter Stack Guard: Bounded Stack Overflow Detected!");
        }
    }
}

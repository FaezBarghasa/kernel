//! Synchronization primitives for the kernel.
//!
//! This module contains synchronization types essential for thread safety and real-time guarantees.

use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
};
use spin::{Mutex as SpinMutex, MutexGuard as SpinMutexGuard};

use crate::context::Context;

// Declare submodules
mod ordered;
mod wait_condition;
mod wait_queue;

// Lock-free and optimized synchronization primitives
mod lockfree_queue;
mod optimized_wait_queue;
mod priority;

// Re-export ordered lock types
pub use ordered::{
    check_no_locks, ArcRwLockWriteGuard, CleanLockToken, Higher, Level, LockToken, Lower,
    Mutex as OrderedMutex, MutexGuard as OrderedMutexGuard, RwLock, RwLockReadGuard,
    RwLockWriteGuard, L0, L1, L2, L3, L4, L5,
};

// Re-export wait queue types
pub use wait_condition::WaitCondition;
pub use wait_queue::WaitQueue;

// Re-export lock-free and optimized types
pub use lockfree_queue::LockFreeQueue;
pub use optimized_wait_queue::OptimizedWaitQueue;
pub use priority::{IpcCriticalGuard, Priority, PriorityTracker};

/// A Mutex wrapper implementing a placeholder for the Priority Inheritance Protocol (PIP).
///
/// In a full PIP implementation, the `priority_ceiling` would automatically be raised
/// to the priority of the highest-priority waiting task upon contention.
pub struct Mutex<T: ?Sized> {
    /// Tracks the highest priority of any task currently waiting for this lock.
    /// Used by the scheduler to adjust the priority of the lock owner (priority inheritance).
    pub priority_ceiling: UnsafeCell<u8>,
    inner: SpinMutex<T>,
}

impl<T> Mutex<T> {
    pub const fn new(user_data: T) -> Self {
        Mutex {
            priority_ceiling: UnsafeCell::new(0),
            inner: SpinMutex::new(user_data),
        }
    }

    /// Locks the mutex, simulating Priority Inheritance management.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        // --- PRIORITY INHERITANCE SIMULATION START ---
        // In a real implementation:
        // 1. Get the current task's priority (P_current).
        // 2. Lock the Mutex (inner.lock()).
        // 3. If contention occurs:
        //    a. Check if P_current > self.priority_ceiling.
        //    b. If so, raise the current task's effective priority to the higher ceiling.

        let guard = self.inner.lock();

        // For demonstration, we simply update the ceiling placeholder on lock acquisition.
        unsafe {
            // Assume the current task has a priority to inherit, e.g., 255 (highest).
            // This is where priority tracking logic would be fully integrated.
            *self.priority_ceiling.get() = 255;
        }
        // --- PRIORITY INHERITANCE SIMULATION END ---

        MutexGuard { mutex: self, guard }
    }

    // Other Mutex methods (try_lock, etc.) omitted for brevity.
}

/// A Guard structure holding the lock and implementing Deref/DerefMut.
pub struct MutexGuard<'a, T: ?Sized> {
    mutex: &'a Mutex<T>,
    guard: SpinMutexGuard<'a, T>,
}

impl<'a, T: ?Sized> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        // --- PRIORITY INHERITANCE SIMULATION START ---
        // When the lock is released:
        // 1. Reset the owner's priority back to its original value.
        // 2. Clear the `priority_ceiling` field.
        unsafe {
            *self.mutex.priority_ceiling.get() = 0;
        }
        // --- PRIORITY INHERITANCE SIMULATION END ---
    }
}

impl<'a, T: ?Sized> Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<'a, T: ?Sized> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

//! Synchronization primitives for the kernel.
//!
//! This module contains synchronization types essential for thread safety and real-time guarantees.

use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicUsize, Ordering},
};
use spin::{Mutex as SpinMutex, MutexGuard as SpinMutexGuard};

use crate::context::{self, ContextRef};

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

/// A Mutex wrapper implementing the Priority Inheritance Protocol (PIP).
pub struct Mutex<T: ?Sized> {
    /// The actual mutex protecting the data.
    /// The ID of the context currently holding the lock.
    owner_id: AtomicUsize,
    /// A wait queue for contexts trying to acquire this mutex.
    wait_queue: WaitQueue<ContextRef>,
    /// The actual mutex protecting the data.
    inner: SpinMutex<T>,
}

impl<T> Mutex<T> {
    pub const fn new(user_data: T) -> Self {
        Mutex {
            inner: SpinMutex::new(user_data),
            owner_id: AtomicUsize::new(0), // 0 indicates no owner
            wait_queue: WaitQueue::new(),
        }
    }

    /// Locks the mutex, implementing Priority Inheritance.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        // We are constructing a token here, which is unsafe if other locks are held.
        // Assuming Mutex::lock is the entry point for locking and respects hierarchy (or context level is base).
        let mut token = unsafe { CleanLockToken::new() };
        let current_context_ref = context::current();
        let current_context_id = current_context_ref.read(token.token()).id();

        loop {
            // Try to acquire the lock
            if let Some(guard) = self.inner.try_lock() {
                self.owner_id.store(current_context_id, Ordering::Release);
                return MutexGuard { mutex: self, guard };
            }

            // Lock is held by another context. Implement Priority Inheritance.
            let owner_id = self.owner_id.load(Ordering::Acquire);
            if owner_id != 0 {
                if let Some(owner_context_ref) = context::contexts().read().get(&owner_id).cloned()
                {
                    let current_priority = current_context_ref
                        .read(token.token())
                        .priority
                        .effective_priority();
                    let mut owner_context = owner_context_ref.write(token.token());

                    // If current context has higher priority than owner, boost owner's priority
                    if current_priority < owner_context.priority.effective_priority() {
                        owner_context.priority.inherit_priority(current_priority);
                    }
                }
            }

            // Block the current context and add it to the wait queue
            // Token is already available
            current_context_ref
                .write(token.token())
                .block("Mutex::lock");
            self.wait_queue
                .send(current_context_ref.clone(), &mut token);
            // Switch to another context
            unsafe { crate::context::switch(&mut token) };
        }
    }

    // Other Mutex methods (try_lock, etc.) can be added as needed.
}

impl<T: ?Sized + core::fmt::Debug> core::fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.inner.try_lock() {
            Some(guard) => write!(f, "Mutex {{ data: {:?} }}", &*guard),
            None => write!(f, "Mutex {{ <locked> }}"),
        }
    }
}

/// A Guard structure holding the lock and implementing Deref/DerefMut.
pub struct MutexGuard<'a, T: ?Sized> {
    mutex: &'a Mutex<T>,
    guard: SpinMutexGuard<'a, T>,
}

impl<'a, T: ?Sized> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        let mut token = unsafe { CleanLockToken::new() };
        let owner_id = self.mutex.owner_id.swap(0, Ordering::Acquire); // Clear owner
        let owner_context_ref = context::contexts().read().get(&owner_id).cloned();

        // Restore owner's priority if it was boosted
        if let Some(owner_ctx_ref) = owner_context_ref {
            owner_ctx_ref
                .write(token.token())
                .priority
                .restore_base_priority();
        }

        // Unblock the highest-priority waiting task
        // Token reused
        if let Ok(next_waiter_ref) = self
            .mutex
            .wait_queue
            .receive(false, "Mutex::drop", &mut token)
        {
            next_waiter_ref.write(token.token()).unblock();
        }
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

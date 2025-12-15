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
    check_no_locks, CleanLockToken, Level, LockToken, Lower, Mutex as OrderedMutex,
    MutexGuard as OrderedMutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard, L0, L1, L2,
};

// Re-export wait queue types
pub use wait_condition::WaitCondition;
pub use wait_queue::{WaitQueue, Waitable};

// Re-export lock-free and optimized types
pub use lockfree_queue::LockFreeQueue;
pub use optimized_wait_queue::OptimizedWaitQueue;
pub use priority::{IpcCriticalGuard, Priority, PriorityTracker};

/// A Mutex wrapper implementing the Priority Inheritance Protocol (PIP).
pub struct Mutex<T: ?Sized> {
    /// The ID of the context currently holding the lock.
    owner_id: AtomicUsize,
    /// A wait queue for contexts trying to acquire this mutex, ordered by priority.
    wait_queue: WaitQueue<ContextRef>,
    /// The actual spinlock protecting the data.
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
        let mut token = unsafe { CleanLockToken::new() };
        let current_context_ref = context::current();
        let current_context_id = current_context_ref.read(token.token()).id();
        let current_priority = current_context_ref.read(token.token()).priority.effective_priority();

        // Fast path: try to acquire the lock without blocking.
        if self.owner_id.compare_exchange(0, current_context_id, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            return MutexGuard {
                mutex: self,
                guard: self.inner.lock(),
            };
        }

        loop {
            // The lock is held, so we need to wait.
            // Inherit priority to the lock owner.
            let owner_id = self.owner_id.load(Ordering::Relaxed);
            if owner_id != 0 {
                if let Some(owner_context_ref) = context::contexts().read().get(&owner_id).cloned() {
                    let mut owner_context = owner_context_ref.write(token.token());
                    // The key for priority inheritance is the address of the mutex.
                    owner_context.priority.inherit_priority(self as *const _ as usize, current_priority);
                }
            }

            // Add ourselves to the wait queue. The queue should be priority-ordered.
            self.wait_queue.send(current_context_ref.clone(), &mut token);

            // Block and wait to be woken up.
            context::current().write(token.token()).block("Mutex::lock");
            unsafe { context::switch(&mut token) };

            // When we wake up, try to acquire the lock again.
            if self.owner_id.compare_exchange(0, current_context_id, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                return MutexGuard {
                    mutex: self,
                    guard: self.inner.lock(),
                };
            }
        }
    }
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
        let current_context_ref = context::current();
        let mut current_context = current_context_ref.write(token.token());

        // Restore original priority.
        current_context.priority.restore_priority(self.mutex as *const _ as usize);

        // Release the lock.
        self.mutex.owner_id.store(0, Ordering::Release);

        // Wake up the highest-priority waiting task.
        if let Ok(next_waiter_ref) = self.mutex.wait_queue.receive(false, "Mutex::drop", &mut token) {
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

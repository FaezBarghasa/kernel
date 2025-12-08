// This code was adapted from MIT licensed https://github.com/antialize/ordered-locks
// We cannot use that library directly as it is wrapping std::sync types

#![allow(dead_code)]

//! This create implement compiletime ordering of locks into levels, [`L1`], [`L2`], [`L3`], [`L4`] and [`L5`].
//! In order to acquire a lock at level `i` only locks at level `i-1` or below may be held.
//!
//! If locks are alwayes acquired in level order on all threads, then one cannot have a deadlock
//! involving only acquireng locks.
//!
//! At some point in time we would want Level to be replaced by usize, however
//! with current cont generics (rust 1.55), we cannot compare const generic arguments
//! so we are left with this mess.
use alloc::{sync::Arc, vec::Vec};
use core::{marker::PhantomData, sync::atomic::Ordering};

use crate::context::{self, ContextRef};

/// Lock level of a mutex
///
/// While a mutex of L1 is locked on a thread, only mutexes of L2 or higher may be locked.
/// This lock hierarchy prevents deadlocks from occurring. For a dead lock to occour
/// We need some thread TA to hold a resource RA, and request a resource RB, while
/// another thread TB holds RB, and requests RA. This is not possible with a lock
/// hierarchy either RA or RB must be on a level that the other.
pub trait Level {}

/// Indicate that the implementor is lower that the level O
pub trait Lower<O: Level>: Level {}

/// Lowest locking level, no locks can be on this level
#[derive(Debug)]
pub struct L0 {}

#[derive(Debug)]
pub struct L1 {}

#[derive(Debug)]
pub struct L2 {}

#[derive(Debug)]
pub struct L3 {}

#[derive(Debug)]
pub struct L4 {}

#[derive(Debug)]
pub struct L5 {}

impl Level for L0 {}
impl Level for L1 {}
impl Level for L2 {}
impl Level for L3 {}
impl Level for L4 {}
impl Level for L5 {}

impl Lower<L1> for L0 {}
impl Lower<L2> for L0 {}
impl Lower<L3> for L0 {}
impl Lower<L4> for L0 {}
impl Lower<L5> for L0 {}

impl Lower<L2> for L1 {}
impl Lower<L3> for L1 {}
impl Lower<L4> for L1 {}
impl Lower<L5> for L1 {}

impl Lower<L3> for L2 {}
impl Lower<L4> for L2 {}
impl Lower<L5> for L2 {}

impl Lower<L4> for L3 {}
impl Lower<L5> for L3 {}

impl Lower<L5> for L4 {}

/// Indicate that the implementor is higher that the level O
pub trait Higher<O: Level>: Level {}
impl<L1: Level, L2: Level> Higher<L2> for L1 where L2: Lower<L1> {}

/// While this exists only locks with a level higher than L, may be locked.
/// These tokens are carried around the call stack to indicate tho current locking level.
/// They have no size and should disappear at runtime.
pub struct LockToken<'a, L: Level>(PhantomData<&'a mut L>);

impl<'a, L: Level> LockToken<'a, L> {
    /// Create a borrowed copy of self
    pub fn token(&mut self) -> LockToken<'_, L> {
        LockToken(Default::default())
    }

    /// Create a borrowed copy of self, on a higher level
    pub fn downgrade<LC: Higher<L>>(&mut self) -> LockToken<'_, LC> {
        LockToken(Default::default())
    }

    pub fn downgraded<LP: Lower<L>>(_: LockToken<'a, LP>) -> Self {
        LockToken(Default::default())
    }
}

/// Token indicating that there are no acquired locks while not borrowed.
pub struct CleanLockToken(());

impl CleanLockToken {
    /// Create a borrowed copy of self
    pub fn token(&mut self) -> LockToken<'_, L0> {
        LockToken(Default::default())
    }

    /// Create a borrowed copy of self, on a higher level
    pub fn downgrade<L: Level>(&mut self) -> LockToken<'_, L> {
        LockToken(Default::default())
    }

    /// Create a new instance
    ///
    /// # Safety
    ///
    /// This is safe to call as long as there are no currently acquired locks
    /// in the thread/task, and as long as there are no other CleanLockToken
    /// in the thread/task.
    ///
    /// A CleanLockToken
    pub unsafe fn new() -> Self {
        CleanLockToken(())
    }
}

/// A mutual exclusion primitive useful for protecting shared data
///
/// This mutex will block threads waiting for the lock to become available. The
/// mutex can also be statically initialized or created via a `new`
/// constructor. Each mutex has a type parameter which represents the data that
/// it is protecting. The data can only be accessed through the RAII guards
/// returned from `lock` and `try_lock`, which guarantees that the data is only
/// ever accessed when the mutex is locked.
#[derive(Debug)]
pub struct Mutex<L: Level, T: ?Sized> {
    _phantom: PhantomData<L>,
    /// The context currently holding the lock, for priority inheritance
    holder: spin::Mutex<Option<ContextRef>>,
    inner: spin::Mutex<T>,
}

impl<L: Level, T: Default> Default for Mutex<L, T> {
    fn default() -> Self {
        Self {
            _phantom: Default::default(),
            holder: spin::Mutex::new(None),
            inner: Default::default(),
        }
    }
}

impl<L: Level, T> Mutex<L, T> {
    /// Creates a new mutex in an unlocked state ready for use
    pub const fn new(val: T) -> Self {
        Self {
            _phantom: PhantomData,
            holder: spin::Mutex::new(None),
            inner: spin::Mutex::new(val),
        }
    }

    /// Acquires a mutex, blocking the current thread until it is able to do so.
    ///
    /// This function will block the local thread until it is available to acquire the mutex.
    /// Upon returning, the thread is the only thread with the mutex held.
    /// An RAII guard is returned to allow scoped unlock of the lock. When the guard goes out of scope, the mutex will be unlocked.
    pub fn lock<'a, LP: Lower<L> + 'a>(
        &'a self,
        mut lock_token: LockToken<'a, LP>,
    ) -> MutexGuard<'a, L, T> {
        let current_context_ref = context::current();

        loop {
            // Try to acquire the lock
            if let Some(guard) = self.inner.try_lock() {
                // Successfully acquired the lock
                *self.holder.lock() = Some(current_context_ref.clone());
                return MutexGuard {
                    inner: guard,
                    lock_token: LockToken::downgraded(lock_token),
                    mutex: self,
                };
            }

            // Lock is held, check for priority inversion
            let holder_context_ref_opt = self.holder.lock().clone();
            if let Some(holder_context_ref) = holder_context_ref_opt {
                let mut clean = unsafe { CleanLockToken::new() };
                let current_priority = current_context_ref
                    .read(clean.token())
                    .priority
                    .effective_priority();
                let holder_priority = holder_context_ref
                    .read(clean.token())
                    .priority
                    .effective_priority();

                if current_priority < holder_priority {
                    // Current thread has higher priority than holder, perform priority inheritance
                    holder_context_ref
                        .read(clean.token())
                        .priority
                        .inherit_priority(current_priority);
                }
            }

            // Block the current thread until the lock is available
            // TODO: Use a proper wait queue for mutexes
            // For now, just yield
            unsafe { crate::context::switch(&mut CleanLockToken::new()) };
        }
    }

    /// Attempts to acquire this lock.
    ///
    /// If the lock could not be acquired at this time, then `None` is returned.
    /// Otherwise, an RAII guard is returned. The lock will be unlocked when the
    /// guard is dropped.
    ///
    /// This function does not block.
    pub fn try_lock<'a, LP: Lower<L> + 'a>(
        &'a self,
        mut lock_token: LockToken<'a, LP>,
    ) -> Option<MutexGuard<'a, L, T>> {
        let current_context_ref = context::current();

        if let Some(guard) = self.inner.try_lock() {
            *self.holder.lock() = Some(current_context_ref.clone());
            Some(MutexGuard {
                inner: guard,
                lock_token: LockToken::downgraded(lock_token),
                mutex: self,
            })
        } else {
            // Lock is held, check for priority inversion
            let holder_context_ref_opt = self.holder.lock().clone();
            if let Some(holder_context_ref) = holder_context_ref_opt {
                let mut clean = unsafe { CleanLockToken::new() };
                let current_priority = current_context_ref
                    .read(clean.token())
                    .priority
                    .effective_priority();
                let holder_priority = holder_context_ref
                    .read(clean.token())
                    .priority
                    .effective_priority();

                if current_priority < holder_priority {
                    // Current thread has higher priority than holder, perform priority inheritance
                    holder_context_ref
                        .read(clean.token())
                        .priority
                        .inherit_priority(current_priority);
                }
            }
            None
        }
    }

    /// Consumes this Mutex, returning the underlying data.
    pub fn into_inner(self) -> T {
        self.inner.into_inner()
    }
}

/// An RAII implementation of a "scoped lock" of a mutex. When this structure is
/// dropped (falls out of scope), the lock will be unlocked.
///
/// The data protected by the mutex can be accessed through this guard via its
/// `Deref` and `DerefMut` implementations.
pub struct MutexGuard<'a, L: Level, T: ?Sized + 'a> {
    inner: spin::MutexGuard<'a, T>,
    lock_token: LockToken<'a, L>,
    mutex: &'a Mutex<L, T>,
}

impl<'a, L: Level, T: ?Sized + 'a> MutexGuard<'a, L, T> {
    /// Split the guard into two parts, the first a mutable reference to the held content
    /// the second a [`LockToken`] that can be used for further locking
    pub fn token_split(&mut self) -> (&mut T, LockToken<'_, L>) {
        (&mut self.inner, self.lock_token.token())
    }
}

impl<'a, L: Level, T: ?Sized + 'a> core::ops::Deref for MutexGuard<'a, L, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}
impl<'a, L: Level, T: ?Sized + 'a> core::ops::DerefMut for MutexGuard<'a, L, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.deref_mut()
    }
}

impl<'a, L: Level, T: ?Sized + 'a> Drop for MutexGuard<'a, L, T> {
    fn drop(&mut self) {
        // Clear the holder and restore priority
        *self.mutex.holder.lock() = None;
        // The priority of the thread holding the lock will be restored
        // when its boost_deadline expires, or explicitly by other means.
        // For a true PI mutex, we might need to explicitly restore here
        // to the highest priority of any other locks it holds, or its base.
        // For now, rely on expiration or explicit restore.
        // TODO: Implement proper priority restoration for nested locks.
        context::current()
            .inner
            .read()
            .priority
            .restore_base_priority();
    }
}

#[derive(Debug)]
pub struct RwLock<L: Level, T: ?Sized> {
    _phantom: PhantomData<L>,
    /// The context currently holding the write lock, for priority inheritance
    writer_holder: spin::Mutex<Option<ContextRef>>,
    /// The contexts currently holding read locks, for priority inheritance
    reader_holders: spin::Mutex<Vec<ContextRef>>,
    inner: spin::RwLock<T>,
}

impl<L: Level, T: Default> Default for RwLock<L, T> {
    fn default() -> Self {
        Self {
            _phantom: Default::default(),
            writer_holder: spin::Mutex::new(None),
            reader_holders: spin::Mutex::new(Vec::new()),
            inner: Default::default(),
        }
    }
}

/// A reader-writer lock
///
/// This type of lock allows a number of readers or at most one writer at any point in time.
/// The write portion of this lock typically allows modification of the underlying data (exclusive access)
/// and the read portion of this lock typically allows for read-only access (shared access).
///
/// The type parameter T represents the data that this lock protects. It is required that T satisfies
/// Send to be shared across threads and Sync to allow concurrent access through readers.
/// The RAII guards returned from the locking methods implement Deref (and DerefMut for the write methods)
/// to allow access to the contained of the lock.
impl<L: Level, T> RwLock<L, T> {
    /// Creates a new instance of an RwLock<T> which is unlocked.
    pub const fn new(val: T) -> Self {
        Self {
            inner: spin::RwLock::new(val),
            _phantom: PhantomData,
            writer_holder: spin::Mutex::new(None),
            reader_holders: spin::Mutex::new(Vec::new()),
        }
    }

    /// Consumes this RwLock, returning the underlying data.
    pub fn into_inner(self) -> T {
        self.inner.into_inner()
    }

    /// Locks this RwLock with exclusive write access, blocking the current thread until it can be acquired.
    /// This function will not return while other writers or other readers currently have access to the lock.
    /// Returns an RAII guard which will drop the write access of this RwLock when dropped.
    pub fn write<'a, LP: Lower<L> + 'a>(
        &'a self,
        mut lock_token: LockToken<'a, LP>,
    ) -> RwLockWriteGuard<'a, L, T> {
        let current_context_ref = context::current();

        loop {
            if let Some(guard) = self.inner.try_write() {
                *self.writer_holder.lock() = Some(current_context_ref.clone());
                return RwLockWriteGuard {
                    inner: guard,
                    lock_token: LockToken::downgraded(lock_token),
                    rwlock: self,
                };
            }

            // Lock is held, check for priority inversion
            let writer_context_ref_opt = self.writer_holder.lock().clone();
            if let Some(writer_context_ref) = writer_context_ref_opt {
                let mut clean = unsafe { CleanLockToken::new() };
                let current_priority = current_context_ref
                    .read(clean.token())
                    .priority
                    .effective_priority();
                let writer_priority = writer_context_ref
                    .read(clean.token())
                    .priority
                    .effective_priority();

                if current_priority < writer_priority {
                    writer_context_ref
                        .read(clean.token())
                        .priority
                        .inherit_priority(current_priority);
                }
            }
            // Also check readers
            for reader_context_ref in self.reader_holders.lock().iter() {
                let mut clean = unsafe { CleanLockToken::new() };
                let current_priority = current_context_ref
                    .read(clean.token())
                    .priority
                    .effective_priority();
                let reader_priority = reader_context_ref
                    .read(clean.token())
                    .priority
                    .effective_priority();

                if current_priority < reader_priority {
                    reader_context_ref
                        .read(clean.token())
                        .priority
                        .inherit_priority(current_priority);
                }
            }

            unsafe { crate::context::switch(&mut CleanLockToken::new()) };
        }
    }

    /// Locks this RwLock with shared read access, blocking the current thread until it can be acquired.
    ///
    /// The calling thread will be blocked until there are no more writers which hold the lock.
    /// There may be other readers currently inside the lock when this method returns.
    ///
    /// Note that attempts to recursively acquire a read lock on a RwLock when the current thread
    /// already holds one may result in a deadlock.
    ///
    /// Returns an RAII guard which will release this thread’s shared access once it is dropped.
    pub fn read<'a, LP: Lower<L> + 'a>(
        &'a self,
        mut lock_token: LockToken<'a, LP>,
    ) -> RwLockReadGuard<'a, L, T> {
        let current_context_ref = context::current();

        loop {
            if let Some(guard) = self.inner.try_read() {
                self.reader_holders.lock().push(current_context_ref.clone());
                return RwLockReadGuard {
                    inner: guard,
                    lock_token: LockToken::downgraded(lock_token),
                    rwlock: self,
                };
            }

            // Lock is held, check for priority inversion
            let writer_context_ref_opt = self.writer_holder.lock().clone();
            if let Some(writer_context_ref) = writer_context_ref_opt {
                let mut clean = unsafe { CleanLockToken::new() };
                let current_priority = current_context_ref
                    .read(clean.token())
                    .priority
                    .effective_priority();
                let writer_priority = writer_context_ref
                    .read(clean.token())
                    .priority
                    .effective_priority();

                if current_priority < writer_priority {
                    writer_context_ref
                        .read(clean.token())
                        .priority
                        .inherit_priority(current_priority);
                }
            }

            unsafe { crate::context::switch(&mut CleanLockToken::new()) };
        }
    }

    // Unsafe due to not using token, currently required by context::switch
    pub unsafe fn write_arc(self: &Arc<Self>) -> ArcRwLockWriteGuard<L, T> {
        let current_context_ref = context::current();
        loop {
            if let Some(guard) = self.inner.try_write() {
                *self.writer_holder.lock() = Some(current_context_ref.clone());
                core::mem::forget(guard); // Manually manage guard
                return ArcRwLockWriteGuard {
                    rwlock: self.clone(),
                };
            }

            let writer_context_ref_opt = self.writer_holder.lock().clone();
            if let Some(writer_context_ref) = writer_context_ref_opt {
                let current_priority = current_context_ref
                    .inner
                    .read()
                    .priority
                    .effective_priority();
                let writer_priority = writer_context_ref
                    .inner
                    .read()
                    .priority
                    .effective_priority();

                if current_priority < writer_priority {
                    writer_context_ref
                        .inner
                        .read()
                        .priority
                        .inherit_priority(current_priority);
                }
            }
            for reader_context_ref in self.reader_holders.lock().iter() {
                let current_priority = current_context_ref
                    .inner
                    .read()
                    .priority
                    .effective_priority();
                let reader_priority = reader_context_ref
                    .inner
                    .read()
                    .priority
                    .effective_priority();

                if current_priority < reader_priority {
                    reader_context_ref
                        .inner
                        .read()
                        .priority
                        .inherit_priority(current_priority);
                }
            }
            unsafe { crate::context::switch(&mut CleanLockToken::new()) };
        }
    }
}

/// RAII structure used to release the exclusive write access of a lock when dropped
pub struct RwLockWriteGuard<'a, L: Level, T> {
    inner: spin::RwLockWriteGuard<'a, T>,
    lock_token: LockToken<'a, L>,
    rwlock: &'a RwLock<L, T>,
}

impl<L: Level, T> RwLockWriteGuard<'_, L, T> {
    /// Split the guard into two parts, the first a mutable reference to the held content
    /// the second a [`LockToken`] that can be used for further locking
    pub fn token_split(&mut self) -> (&mut T, LockToken<'_, L>) {
        (&mut self.inner, self.lock_token.token())
    }
}

impl<L: Level, T> core::ops::Deref for RwLockWriteGuard<'_, L, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}

impl<L: Level, T> core::ops::DerefMut for RwLockWriteGuard<'_, L, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.deref_mut()
    }
}

impl<'a, L: Level, T> Drop for RwLockWriteGuard<'a, L, T> {
    fn drop(&mut self) {
        *self.rwlock.writer_holder.lock() = None;
        context::current()
            .inner
            .read()
            .priority
            .restore_base_priority();
    }
}

/// RAII structure used to release the shared read access of a lock when dropped.
pub struct RwLockReadGuard<'a, L: Level, T> {
    inner: spin::RwLockReadGuard<'a, T>,
    lock_token: LockToken<'a, L>,
    rwlock: &'a RwLock<L, T>,
}

impl<L: Level, T> RwLockReadGuard<'_, L, T> {
    /// Split the guard into two parts, the first a reference to the held content
    /// the second a [`LockToken`] that can be used for further locking
    pub fn token_split(&mut self) -> (&T, LockToken<'_, L>) {
        (&self.inner, self.lock_token.token())
    }
}

impl<L: Level, T> core::ops::Deref for RwLockReadGuard<'_, L, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}

impl<'a, L: Level, T> Drop for RwLockReadGuard<'a, L, T> {
    fn drop(&mut self) {
        let current_context_ref = context::current();
        self.rwlock
            .reader_holders
            .lock()
            .retain(|ctx| !Arc::ptr_eq(ctx, &current_context_ref));
        context::current()
            .inner
            .read()
            .priority
            .restore_base_priority();
    }
}

pub struct ArcRwLockWriteGuard<L: Level + 'static, T> {
    rwlock: Arc<RwLock<L, T>>,
}

impl<L: Level, T> ArcRwLockWriteGuard<L, T> {
    pub fn rwlock(s: &Self) -> &Arc<RwLock<L, T>> {
        &s.rwlock
    }
}

impl<L: Level, T> core::ops::Deref for ArcRwLockWriteGuard<L, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.rwlock.inner.as_mut_ptr() }
    }
}

impl<L: Level, T> core::ops::DerefMut for ArcRwLockWriteGuard<L, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.rwlock.inner.as_mut_ptr() }
    }
}

impl<L: Level, T> Drop for ArcRwLockWriteGuard<L, T> {
    #[inline]
    fn drop(&mut self) {
        *self.rwlock.writer_holder.lock() = None;
        context::current()
            .inner
            .read()
            .priority
            .restore_base_priority();
        unsafe {
            self.rwlock.inner.force_write_unlock();
        }
    }
}

/// This function can only be called if no lock is held by the calling thread/task
#[inline]
pub fn check_no_locks(_: LockToken<'_, L0>) {}

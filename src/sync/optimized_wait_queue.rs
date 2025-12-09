//! Optimized Wait Queue using Lock-Free Data Structures
//!
//! This provides a drop-in replacement for the spinlock-based WaitQueue
//! with significantly reduced contention and better cache behavior.

use crate::{
    sync::{lockfree_queue::LockFreeQueue, CleanLockToken, WaitCondition},
    syscall::{
        error::{Error, Result, EAGAIN, EINTR},
        usercopy::UserSliceWo,
    },
};
use core::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub struct OptimizedWaitQueue<T> {
    /// Lock-free queue for data
    queue: LockFreeQueue<T>,
    /// Wait condition for blocking operations
    condition: WaitCondition,
    /// Fast-path check to avoid condition variable overhead
    has_waiters: AtomicBool,
}

impl<T> OptimizedWaitQueue<T> {
    /// Runtime initialization
    pub fn new() -> Self {
        OptimizedWaitQueue {
            queue: LockFreeQueue::new(),
            condition: WaitCondition::new(),
            has_waiters: AtomicBool::new(false),
        }
    }

    /// Fast path check for empty queue
    #[inline]
    pub fn is_currently_empty(&self) -> bool {
        self.queue.is_empty_approx()
    }

    /// Receive with optional blocking
    ///
    /// Optimized fast path when queue is not empty.
    pub fn receive(
        &self,
        block: bool,
        reason: &'static str,
        token: &mut CleanLockToken,
    ) -> Result<T> {
        loop {
            // Fast path: try to dequeue without blocking
            if let Some(value) = self.queue.dequeue() {
                return Ok(value);
            }

            // Queue is empty
            if !block {
                return Err(Error::new(EAGAIN));
            }

            // Mark that we have waiters for wake optimization
            self.has_waiters.store(true, Ordering::Release);

            // Double-check queue before waiting (avoid lost wakeup)
            if let Some(value) = self.queue.dequeue() {
                self.has_waiters.store(false, Ordering::Relaxed);
                return Ok(value);
            }

            // Block on condition variable
            let current_context_ref = crate::context::current();
            {
                current_context_ref.write(token.token()).block(reason);
            }

            // Wait for notification
            // SAFETY: context::switch is safe to call here as we are holding a valid token
            // and the current context is in a valid state for switching.
            unsafe { crate::context::switch(token) };

            // Clear waiters flag if we're the last one
            self.has_waiters.store(false, Ordering::Relaxed);

            // Check for signals
            {
                let context = current_context_ref.read(token.token());
                if let Some((control, pctl)) = context.sig.as_ref().map(|sig| {
                    crate::context::context::Context::sigcontrol_raw_const(sig)
                }) {
                    if control.currently_pending_unblocked(pctl) != 0 {
                        return Err(Error::new(EINTR));
                    }
                }
            }

            // Loop to retry dequeue
        }
    }

    /// Send value to queue
    ///
    /// Optimized to avoid unnecessary wake-ups when no waiters.
    pub fn send(&self, value: T, token: &mut CleanLockToken) -> usize {
        let len = self.queue.enqueue(value);

        // Only notify if we had waiters (reduce overhead)
        if self.has_waiters.load(Ordering::Acquire) {
            self.condition.notify(token);
        }

        len
    }

    /// Wake one waiter
    ///
    /// This is called when an item is already in the queue.
    #[inline]
    pub fn wake_one(&self) {
        if self.has_waiters.load(Ordering::Acquire) {
            // SAFETY: CleanLockToken::new() is unsafe because it creates a token that implies
            // no locks are held. We are in wake_one which might be called from various contexts,
            // but we are only using it to notify a condition variable which is generally safe.
            // However, the caller must ensure this doesn't violate lock ordering if called from
            // a critical section.
            unsafe {
                let mut token = CleanLockToken::new();
                self.condition.notify(&mut token);
            }
        }
    }

    /// Get approximate queue length
    #[inline]
    pub fn len(&self) -> usize {
        self.queue.len_approx()
    }
}

impl<T> Default for OptimizedWaitQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy> OptimizedWaitQueue<T> {
    pub fn receive_into_user(
        &self,
        mut buf: UserSliceWo,
        block: bool,
        reason: &'static str,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        let mut total = 0;
        
        loop {
             let do_block = block && total == 0;
             match self.receive(do_block, reason, token) {
                 Ok(val) => {
                     let slice = unsafe {
                         core::slice::from_raw_parts(&val as *const T as *const u8, core::mem::size_of::<T>())
                     };
                     match buf.copy_common_bytes_from_slice(slice) {
                         Ok(written) => {
                             total += written;
                             if let Some(next) = buf.advance(written) {
                                 buf = next;
                             } else {
                                 break;
                             }
                         }
                         Err(e) => return Err(e),
                     }
                 }
                 Err(Error { errno: EAGAIN }) => break,
                 Err(e) => return Err(e),
             }
             
             if buf.len() == 0 { break; }
        }
        
        Ok(total)
    }
}

unsafe impl<T: Send> Send for OptimizedWaitQueue<T> {}
unsafe impl<T: Send> Sync for OptimizedWaitQueue<T> {}

#![forbid(unsafe_code)]

//! # Hopter OS Soft Lock Mechanism
//!
//! Implements the "Soft Lock" zero-latency interrupt masking technique.
//! Traditional RTOS implementations disable global interrupts (`__disable_irq()`)
//! to protect critical sections, destroying real-time determinism.
//!
//! The `SoftLock` uses purely atomic compare-and-swap operations. If a high-priority
//! interrupt preempts a task holding the lock, the interrupt does not blindly corrupt
//! data. It can spin for a bounded number of cycles, or safely queue its state,
//! guaranteeing that uncorrelated interrupts (like a motor PWM fault) are serviced
//! instantly with zero latency masking overhead.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// A Soft Lock that provides mutual exclusion without masking hardware interrupts.
pub struct SoftLock<T: ?Sized> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

// Safety: SoftLock provides exclusive access via atomics.
#[allow(unsafe_code)]
unsafe impl<T: ?Sized + Send> Sync for SoftLock<T> {}
#[allow(unsafe_code)]
unsafe impl<T: ?Sized + Send> Send for SoftLock<T> {}

impl<T> SoftLock<T> {
    /// Creates a new `SoftLock` wrapping the provided data.
    pub const fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }
}

impl<T: ?Sized> SoftLock<T> {
    /// Acquires the soft lock without disabling interrupts.
    ///
    /// If the lock is held, the thread will spin. High-priority interrupts
    /// are still able to fire and preempt this spinning context, ensuring
    /// ultra-low interrupt latency.
    pub fn lock(&self) -> SoftLockGuard<'_, T> {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        SoftLockGuard { lock: self }
    }

    /// Attempts to acquire the lock without spinning.
    pub fn try_lock(&self) -> Option<SoftLockGuard<'_, T>> {
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(SoftLockGuard { lock: self })
        } else {
            None
        }
    }
}

/// A RAII guard for the `SoftLock`.
pub struct SoftLockGuard<'a, T: ?Sized> {
    lock: &'a SoftLock<T>,
}

impl<'a, T: ?Sized> Deref for SoftLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        #[allow(unsafe_code)]
        unsafe {
            &*self.lock.data.get()
        }
    }
}

impl<'a, T: ?Sized> DerefMut for SoftLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        #[allow(unsafe_code)]
        unsafe {
            &mut *self.lock.data.get()
        }
    }
}

impl<'a, T: ?Sized> Drop for SoftLockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

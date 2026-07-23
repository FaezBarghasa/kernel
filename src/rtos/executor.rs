#![forbid(unsafe_code)]

//! # Embassy-Style Static Async Executor
//!
//! Provides a zero-allocation, static async task executor suitable for
//! running background I/O tasks inside the RTIC idle loop.

use core::cell::UnsafeCell;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

/// A statically allocated Task slot.
pub struct StaticTask<F: Future<Output = ()> + 'static> {
    future: UnsafeCell<Option<F>>,
}

// Safety: Access to StaticTask is guarded by the executor state.
#[allow(unsafe_code)]
unsafe impl<F: Future<Output = ()> + 'static> Sync for StaticTask<F> {}

impl<F: Future<Output = ()> + 'static> StaticTask<F> {
    /// Creates an empty static task slot.
    pub const fn new() -> Self {
        Self {
            future: UnsafeCell::new(None),
        }
    }

    /// Initializes the static task slot with a future.
    pub fn init(&self, future: F) {
        #[allow(unsafe_code)]
        unsafe {
            *self.future.get() = Some(future);
        }
    }

    /// Polls the inner future if present.
    pub fn poll(&self, cx: &mut Context<'_>) -> Poll<()> {
        #[allow(unsafe_code)]
        unsafe {
            if let Some(fut) = &mut *self.future.get() {
                // Pin the future stored in static memory
                let pinned = Pin::new_unchecked(fut);
                match pinned.poll(cx) {
                    Poll::Ready(()) => {
                        *self.future.get() = None;
                        Poll::Ready(())
                    }
                    Poll::Pending => Poll::Pending,
                }
            } else {
                Poll::Ready(())
            }
        }
    }
}

/// A static async task executor with fixed task capacity.
pub struct StaticExecutor<const MAX_TASKS: usize> {
    active_mask: AtomicUsize,
}

impl<const MAX_TASKS: usize> StaticExecutor<MAX_TASKS> {
    /// Creates a new `StaticExecutor`.
    pub const fn new() -> Self {
        Self {
            active_mask: AtomicUsize::new(0),
        }
    }

    /// Marks a static task index as active/pending execution.
    pub fn mark_ready(&self, task_idx: usize) -> bool {
        if task_idx >= MAX_TASKS || task_idx >= (usize::BITS as usize) {
            return false;
        }
        self.active_mask.fetch_or(1 << task_idx, Ordering::Release);
        true
    }

    /// Checks if there are pending tasks in the executor queue.
    pub fn has_pending(&self) -> bool {
        self.active_mask.load(Ordering::Acquire) != 0
    }

    /// Creates a lightweight static waker.
    fn create_waker() -> Waker {
        fn dummy_clone(_: *const ()) -> RawWaker {
            dummy_raw_waker()
        }
        fn dummy_wake(_: *const ()) {}
        fn dummy_wake_by_ref(_: *const ()) {}
        fn dummy_drop(_: *const ()) {}

        static VTABLE: RawWakerVTable = RawWakerVTable::new(
            dummy_clone,
            dummy_wake,
            dummy_wake_by_ref,
            dummy_drop,
        );

        fn dummy_raw_waker() -> RawWaker {
            RawWaker::new(core::ptr::null(), &VTABLE)
        }

        #[allow(unsafe_code)]
        unsafe { Waker::from_raw(dummy_raw_waker()) }
    }
}

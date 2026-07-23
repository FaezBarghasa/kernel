#![forbid(unsafe_code)]

//! # R3-Style Static Kernel Object Model
//!
//! Implements the purely static allocation primitives required for the RTOS core.
//! In severely memory-constrained targets (like the ESP32/Nano targets), dynamic heap
//! allocations (`alloc::alloc::alloc`) are completely avoided.
//! All primitives are defined statically at compile-time using `const fn` and fixed-size arrays.

pub mod executor;
pub mod stack_guard;

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};

/// A statically allocated, fixed-capacity FIFO queue for lock-free communication.
pub struct StaticQueue<T, const CAPACITY: usize> {
    head: AtomicUsize,
    tail: AtomicUsize,
    buffer: [UnsafeCell<MaybeUninit<T>>; CAPACITY],
}

#[allow(unsafe_code)]
unsafe impl<T: Send, const CAPACITY: usize> Sync for StaticQueue<T, CAPACITY> {}

impl<T, const CAPACITY: usize> StaticQueue<T, CAPACITY> {
    /// Creates a new `StaticQueue`.
    pub const fn new() -> Self {
        Self {
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            buffer: [const { UnsafeCell::new(MaybeUninit::uninit()) }; CAPACITY],
        }
    }

    /// Enqueues an item if there is space.
    pub fn enqueue(&self, item: T) -> bool {
        let tail = self.tail.load(Ordering::Relaxed);
        let next_tail = (tail + 1) % CAPACITY;
        
        if next_tail == self.head.load(Ordering::Acquire) {
            return false; // Queue is full
        }
        
        #[allow(unsafe_code)]
        unsafe {
            (*self.buffer[tail].get()).write(item);
        }
        self.tail.store(next_tail, Ordering::Release);
        true
    }

    /// Dequeues an item if available.
    pub fn dequeue(&self) -> Option<T> {
        let head = self.head.load(Ordering::Relaxed);
        
        if head == self.tail.load(Ordering::Acquire) {
            return None; // Queue is empty
        }
        
        #[allow(unsafe_code)]
        let item = unsafe { (*self.buffer[head].get()).assume_init_read() };
        self.head.store((head + 1) % CAPACITY, Ordering::Release);
        Some(item)
    }
}

/// A statically allocated Semaphore.
pub struct StaticSemaphore {
    count: AtomicUsize,
}

impl StaticSemaphore {
    /// Creates a new `StaticSemaphore` with a given initial count.
    pub const fn new(initial: usize) -> Self {
        Self {
            count: AtomicUsize::new(initial),
        }
    }

    /// Acquires a permit.
    pub fn acquire(&self) {
        loop {
            let current = self.count.load(Ordering::Acquire);
            if current > 0 {
                if self.count.compare_exchange_weak(current, current - 1, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                    break;
                }
            } else {
                core::hint::spin_loop(); // For true RTOS this would suspend the task
            }
        }
    }

    /// Releases a permit.
    pub fn release(&self) {
        self.count.fetch_add(1, Ordering::Release);
    }
}

/// A trait summarizing the kernel functionality for a specific composition.
pub trait SystemTraits {
    type Timer;
    type Scheduler;
    type MemoryIsolation;
}

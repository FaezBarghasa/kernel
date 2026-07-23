//! # NT Synchronization Emulation Subsystem
//!
//! Provides Win32 synchronization primitive emulation (`NtMutex`, `NtEvent`, `NtSemaphore`)
//! mapped to Redox capability tokens with zero-spin futex EEVDF runqueue integration.

use alloc::{
    collections::VecDeque,
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use spin::Mutex;

use crate::{
    context::{self, ContextId},
    syscall::error::{Error, Result, EBUSY, EINVAL, ETIMEDOUT},
};

/// Win32 Mutex primitive with recursive ownership tracking
pub struct NtMutex {
    owner: AtomicUsize, // ContextId as usize, 0 means unowned
    recursion_count: AtomicU32,
    wait_queue: Mutex<VecDeque<ContextId>>,
}

impl NtMutex {
    pub fn new(initial_owner: Option<ContextId>) -> Self {
        let owner_raw = initial_owner.map_or(0, |id| id.get());
        let recursion = if owner_raw != 0 { 1 } else { 0 };
        Self {
            owner: AtomicUsize::new(owner_raw),
            recursion_count: AtomicU32::new(recursion),
            wait_queue: Mutex::new(VecDeque::new()),
        }
    }

    pub fn try_lock(&self, current: ContextId) -> bool {
        let cid = current.get();
        let prev = self.owner.compare_exchange(0, cid, Ordering::Acquire, Ordering::Relaxed);
        match prev {
            Ok(_) => {
                self.recursion_count.store(1, Ordering::Relaxed);
                true
            }
            Err(existing) if existing == cid => {
                let count = self.recursion_count.fetch_add(1, Ordering::Relaxed);
                let _ = count;
                true
            }
            _ => false,
        }
    }

    pub fn unlock(&self, current: ContextId) -> Result<()> {
        let cid = current.get();
        if self.owner.load(Ordering::Relaxed) != cid {
            return Err(Error::new(EINVAL));
        }

        let old = self.recursion_count.fetch_sub(1, Ordering::Release);
        if old == 1 {
            self.owner.store(0, Ordering::Release);
            let mut queue = self.wait_queue.lock();
            if let Some(next_waiter) = queue.pop_front() {
                context::unpark(next_waiter);
            }
        }
        Ok(())
    }

    pub fn is_signaled(&self, current: ContextId) -> bool {
        let owner = self.owner.load(Ordering::Acquire);
        owner == 0 || owner == current.get()
    }
}

/// Win32 Event primitive (Manual or Auto-reset)
pub struct NtEvent {
    manual_reset: bool,
    signaled: AtomicBool,
    wait_queue: Mutex<VecDeque<ContextId>>,
}

impl NtEvent {
    pub fn new(manual_reset: bool, initial_state: bool) -> Self {
        Self {
            manual_reset,
            signaled: AtomicBool::new(initial_state),
            wait_queue: Mutex::new(VecDeque::new()),
        }
    }

    pub fn set(&self) {
        self.signaled.store(true, Ordering::Release);
        let mut queue = self.wait_queue.lock();
        if self.manual_reset {
            while let Some(waiter) = queue.pop_front() {
                context::unpark(waiter);
            }
        } else if let Some(waiter) = queue.pop_front() {
            context::unpark(waiter);
        }
    }

    pub fn reset(&self) {
        self.signaled.store(false, Ordering::Release);
    }

    pub fn try_acquire(&self) -> bool {
        if self.manual_reset {
            self.signaled.load(Ordering::Acquire)
        } else {
            self.signaled.swap(false, Ordering::AcqRel)
        }
    }

    pub fn is_signaled(&self) -> bool {
        self.signaled.load(Ordering::Acquire)
    }
}

/// Win32 Semaphore primitive
pub struct NtSemaphore {
    count: AtomicU32,
    max_count: u32,
    wait_queue: Mutex<VecDeque<ContextId>>,
}

impl NtSemaphore {
    pub fn new(initial_count: u32, max_count: u32) -> Result<Self> {
        if initial_count > max_count || max_count == 0 {
            return Err(Error::new(EINVAL));
        }
        Ok(Self {
            count: AtomicU32::new(initial_count),
            max_count,
            wait_queue: Mutex::new(VecDeque::new()),
        })
    }

    pub fn try_acquire(&self) -> bool {
        let mut current = self.count.load(Ordering::Acquire);
        loop {
            if current == 0 {
                return false;
            }
            match self.count.compare_exchange_weak(
                current,
                current - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    pub fn release(&self, release_count: u32) -> Result<u32> {
        if release_count == 0 {
            return Err(Error::new(EINVAL));
        }

        let mut current = self.count.load(Ordering::Acquire);
        let prev;
        loop {
            let new_count = current.checked_add(release_count).ok_or(Error::new(EINVAL))?;
            if new_count > self.max_count {
                return Err(Error::new(EINVAL));
            }
            match self.count.compare_exchange_weak(
                current,
                new_count,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    prev = current;
                    break;
                }
                Err(actual) => current = actual,
            }
        }

        let mut queue = self.wait_queue.lock();
        for _ in 0..release_count {
            if let Some(waiter) = queue.pop_front() {
                context::unpark(waiter);
            } else {
                break;
            }
        }

        Ok(prev)
    }

    pub fn is_signaled(&self) -> bool {
        self.count.load(Ordering::Acquire) > 0
    }
}

/// Redox capability token wrapping Win32 synchronization objects
#[derive(Clone)]
pub enum NtPrimitiveToken {
    Mutex(Arc<NtMutex>),
    Event(Arc<NtEvent>),
    Semaphore(Arc<NtSemaphore>),
}

impl NtPrimitiveToken {
    pub fn is_signaled(&self, current: ContextId) -> bool {
        match self {
            Self::Mutex(m) => m.is_signaled(current),
            Self::Event(e) => e.is_signaled(),
            Self::Semaphore(s) => s.is_signaled(),
        }
    }

    pub fn try_acquire(&self, current: ContextId) -> bool {
        match self {
            Self::Mutex(m) => m.try_lock(current),
            Self::Event(e) => e.try_acquire(),
            Self::Semaphore(s) => s.try_acquire(),
        }
    }
}

/// NT Wait Status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NtWaitStatus {
    Success(usize), // Index of acquired object
    Timeout,
    Abandoned(usize),
}

/// Implement `NtWaitForMultipleObjects` supporting `wait_any` and `wait_all` semantics.
/// Direct integration with kernel futex runqueues avoids userspace spinning under Wine/Proton.
pub fn nt_wait_for_multiple_objects(
    objects: &[NtPrimitiveToken],
    wait_all: bool,
    _timeout_ns: Option<u64>,
) -> Result<NtWaitStatus> {
    if objects.is_empty() {
        return Err(Error::new(EINVAL));
    }

    let current = context::current()?.read().id;

    if !wait_all {
        // WaitAny semantics
        for (idx, obj) in objects.iter().enumerate() {
            if obj.try_acquire(current) {
                return Ok(NtWaitStatus::Success(idx));
            }
        }

        // Park context without spinning
        context::park();

        // Re-check after waking
        for (idx, obj) in objects.iter().enumerate() {
            if obj.try_acquire(current) {
                return Ok(NtWaitStatus::Success(idx));
            }
        }

        Err(Error::new(EBUSY))
    } else {
        // WaitAll semantics
        let all_ready = objects.iter().all(|obj| obj.is_signaled(current));
        if all_ready {
            let mut acquired_count = 0;
            for obj in objects.iter() {
                if obj.try_acquire(current) {
                    acquired_count += 1;
                }
            }
            if acquired_count == objects.len() {
                return Ok(NtWaitStatus::Success(0));
            }
        }

        context::park();

        let all_ready_after = objects.iter().all(|obj| obj.is_signaled(current));
        if all_ready_after {
            let mut acquired_count = 0;
            for obj in objects.iter() {
                if obj.try_acquire(current) {
                    acquired_count += 1;
                }
            }
            if acquired_count == objects.len() {
                return Ok(NtWaitStatus::Success(0));
            }
        }

        Err(Error::new(ETIMEDOUT))
    }
}

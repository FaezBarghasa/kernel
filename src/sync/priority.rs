//! Priority Inheritance and Dynamic Priority Management
//!
//! This module implements priority inheritance protocol and dynamic priority boosting
//! for IPC completion, similar to RCU boost in monolithic kernels.

use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

/// Priority levels for contexts
///
/// Lower numeric value = higher priority (Unix/POSIX convention)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Priority {
    /// Real-time critical (IPC servers, interrupt handlers)
    Realtime = 0,
    /// High priority (IPC completion boost)
    High = 64,
    /// Normal priority (default)
    Normal = 100,
    /// Low priority (background tasks)
    Low = 139,
}

impl Priority {
    #[inline]
    pub fn from_u8(val: u8) -> Self {
        match val {
            0..=63 => Priority::Realtime,
            64..=99 => Priority::High,
            100..=138 => Priority::Normal,
            _ => Priority::Low,
        }
    }

    #[inline]
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Per-context priority tracking
#[derive(Debug)]
pub struct PriorityTracker {
    /// Base priority (never changes unless explicitly set)
    base_priority: AtomicU8,
    /// Effective priority (may be boosted)
    effective_priority: AtomicU8,
    /// List of inherited priorities from other contexts, with a key to identify the source.
    inherited_priorities: VecDeque<(usize, u8)>,
    /// Priority boost deadline (TSC timestamp, 0 = no boost)
    boost_deadline: AtomicU64,
    /// Count of critical IPC operations in progress
    ipc_critical_count: AtomicU8,
}

impl PriorityTracker {
    pub const fn new(priority: Priority) -> Self {
        let prio = priority as u8;
        PriorityTracker {
            base_priority: AtomicU8::new(prio),
            effective_priority: AtomicU8::new(prio),
            inherited_priorities: VecDeque::new(),
            boost_deadline: AtomicU64::new(0),
            ipc_critical_count: AtomicU8::new(0),
        }
    }

    /// Get current effective priority
    #[inline]
    pub fn effective_priority(&self) -> u8 {
        self.effective_priority.load(Ordering::Relaxed)
    }

    /// Check if a priority boost has expired and recalculate if it has.
    pub fn check_boost_expired(&mut self) {
        let deadline = self.boost_deadline.load(Ordering::Relaxed);

        if deadline != 0 {
            #[cfg(target_arch = "x86_64")]
            {
                let current_tsc = unsafe { core::arch::x86_64::_rdtsc() };
                if current_tsc > deadline {
                    // Boost expired, clear deadline and recalculate priority
                    self.boost_deadline.store(0, Ordering::Relaxed);
                    self.recalculate_effective_priority();
                }
            }
        }
    }

    /// Recalculates the effective priority based on base priority and inherited priorities.
    fn recalculate_effective_priority(&mut self) {
        let base = self.base_priority.load(Ordering::Relaxed);
        // Find the highest priority (lowest value) among inherited priorities.
        let highest_inherited = self.inherited_priorities.iter().map(|(_, p)| *p).min().unwrap_or(base);
        let new_effective = core::cmp::min(base, highest_inherited);
        self.effective_priority.store(new_effective, Ordering::Release);
    }

    /// Boost priority for IPC completion
    ///
    /// This gives the thread higher priority for a short duration
    /// to complete IPC operations quickly.
    #[inline]
    pub fn boost_for_ipc(&self, duration_cycles: u64) {
        #[cfg(target_arch = "x86_64")]
        {
            let current_tsc = unsafe { core::arch::x86_64::_rdtsc() };
            let deadline = current_tsc + duration_cycles;

            // Set boosted priority
            self.effective_priority
                .store(Priority::High as u8, Ordering::Release);
            self.boost_deadline.store(deadline, Ordering::Release);
        }
    }

    /// Mark IPC critical section entry
    ///
    /// Prevents priority decay while handling critical IPC
    #[inline]
    pub fn enter_ipc_critical(&self) {
        let old_count = self.ipc_critical_count.fetch_add(1, Ordering::Acquire);

        // If this is the first critical section, boost immediately
        if old_count == 0 {
            // Default boost: ~10 microseconds at 3GHz
            self.boost_for_ipc(30_000);
        }
    }

    /// Mark IPC critical section exit
    #[inline]
    pub fn exit_ipc_critical(&self) {
        self.ipc_critical_count.fetch_sub(1, Ordering::Release);
        // Note: We don't immediately remove boost to allow completion
        // The boost will naturally expire
    }

    /// Check if in IPC critical section
    #[inline]
    pub fn is_ipc_critical(&self) -> bool {
        self.ipc_critical_count.load(Ordering::Relaxed) > 0
    }

    /// Inherit a priority from a synchronization object.
    /// The key should uniquely identify the synchronization object.
    pub fn inherit_priority(&mut self, key: usize, donor_priority: u8) {
        // If a priority from this key is already present, remove it before adding the new one.
        if let Some(pos) = self.inherited_priorities.iter().position(|(k, _)| *k == key) {
            self.inherited_priorities.remove(pos);
        }
        self.inherited_priorities.push_back((key, donor_priority));
        self.recalculate_effective_priority();
    }

    /// Restore priority after releasing a resource.
    /// The key should be the same as the one used in `inherit_priority`.
    pub fn restore_priority(&mut self, key: usize) {
        if let Some(pos) = self.inherited_priorities.iter().position(|(k, _)| *k == key) {
            self.inherited_priorities.remove(pos);
        }
        self.recalculate_effective_priority();
    }

    /// Set base priority
    pub fn set_base_priority(&mut self, priority: Priority) {
        let prio = priority.as_u8();
        self.base_priority.store(prio, Ordering::Relaxed);
        self.recalculate_effective_priority();
    }
}

impl Default for PriorityTracker {
    fn default() -> Self {
        Self::new(Priority::Normal)
    }
}

/// RAII guard for IPC critical section
///
/// Automatically manages priority boosting for the duration of critical IPC operations
pub struct IpcCriticalGuard<'a> {
    tracker: &'a PriorityTracker,
}

impl<'a> IpcCriticalGuard<'a> {
    pub fn new(tracker: &'a PriorityTracker) -> Self {
        tracker.enter_ipc_critical();
        IpcCriticalGuard { tracker }
    }
}

impl<'a> Drop for IpcCriticalGuard<'a> {
    fn drop(&mut self) {
        self.tracker.exit_ipc_critical();
    }
}

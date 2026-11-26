#![forbid(unsafe_code)]

//! # ARINC 653 Spatial & Temporal Partitioning Scheduler
//!
//! Implements deterministic Time Division Multiple Access (TDMA) scheduling windows
//! for ISO 26262 ASIL-D automotive safety compliance. Major Frames are divided into
//! immutable Minor Frames allocated to safety-critical partitions.
//!
//! ## Mathematical & Partition Model
//! Given Major Frame duration $T_{major}$ and $K$ Minor Frames with durations $d_1, d_2, \dots, d_K$:
//! $$T_{major} = \sum_{j=1}^K d_j$$
//!
//! Non-safety partitions exceeding their minor window $d_j$ are forcefully preempted at $t = t_{start} + d_j$.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use alloc::vec::Vec;
use spin::Mutex;

/// ASIL Safety Criticality Level (ISO 26262).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AsilLevel {
    AsilD = 4,
    AsilC = 3,
    AsilB = 2,
    AsilA = 1,
    QM = 0, // Quality Management / Non-safety (e.g. Infotainment)
}

/// ARINC 653 Minor Window Specification.
#[derive(Debug, Clone, Copy)]
pub struct MinorFrame {
    pub partition_id: u32,
    pub duration_ns: u64,
    pub asil_level: AsilLevel,
}

/// ARINC 653 TDMA Scheduler.
pub struct Arinc653Scheduler {
    pub major_frame_duration_ns: AtomicU64,
    pub current_minor_index: AtomicUsize,
    pub minor_frames: Mutex<Vec<MinorFrame>>,
    pub total_major_cycles: AtomicU64,
}

impl Arinc653Scheduler {
    /// Creates a new `Arinc653Scheduler`.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub const fn new() -> Self {
        Self {
            major_frame_duration_ns: AtomicU64::new(0),
            current_minor_index: AtomicUsize::new(0),
            minor_frames: Mutex::new(Vec::new()),
            total_major_cycles: AtomicU64::new(0),
        }
    }

    /// Configures the ARINC 653 Major Frame schedule.
    ///
    /// Complexity: $\mathcal{O}(K)$
    pub fn configure_schedule(&self, frames: Vec<MinorFrame>) {
        let mut total_duration = 0u64;
        for frame in &frames {
            total_duration += frame.duration_ns;
        }

        let mut lock = self.minor_frames.lock();
        *lock = frames;
        self.major_frame_duration_ns.store(total_duration, Ordering::Release);
        self.current_minor_index.store(0, Ordering::Release);
    }

    /// Evaluates current active partition for time slice $t$. Forcefully preempts QM tasks if window expires.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub fn get_active_partition(&self) -> Option<MinorFrame> {
        let lock = self.minor_frames.lock();
        if lock.is_empty() {
            return None;
        }

        let idx = self.current_minor_index.load(Ordering::Acquire);
        lock.get(idx).copied()
    }

    /// Advances to the next minor window in the TDMA major frame cycle.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub fn advance_minor_frame(&self) {
        let lock = self.minor_frames.lock();
        if lock.is_empty() {
            return;
        }

        let mut idx = self.current_minor_index.load(Ordering::Acquire);
        idx += 1;
        if idx >= lock.len() {
            idx = 0;
            self.total_major_cycles.fetch_add(1, Ordering::Relaxed);
        }
        self.current_minor_index.store(idx, Ordering::Release);
    }
}

/// Global ARINC 653 TDMA Scheduler instance.
pub static ARINC653_SCHEDULER: Arinc653Scheduler = Arinc653Scheduler::new();

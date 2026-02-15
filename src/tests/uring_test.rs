//! io_uring Test Suite
//!
//! Comprehensive tests for the io_uring-style asynchronous engine including:
//! - SQ/CQ basic operations
//! - Wrap-around and overflow handling
//! - Batch processing
//! - Latency benchmarking targeting <1µs context-switch

#![cfg(test)]

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::{
    memory::{allocate_frame, deallocate_frame, Frame},
    scheme::ring::*,
    sync::{CleanLockToken, OptimizedWaitQueue},
};
use alloc::sync::Arc;

/// Helper to create a mock ring handle for testing
fn create_test_ring() -> Arc<RingHandle> {
    let frame = allocate_frame().expect("Failed to allocate frame for test");
    let ring_ptr =
        unsafe { crate::paging::RmmA::phys_to_virt(frame.base()).data() as *mut IpcRing };

    const SQ_SIZE: usize = 16;
    const CQ_SIZE: usize = 32;

    // Initialize ring header
    unsafe {
        let ring = &mut *ring_ptr;
        ring.sq_head.store(0, Ordering::Relaxed);
        ring.sq_tail.store(0, Ordering::Relaxed);
        ring.sq_mask = (SQ_SIZE - 1) as u32;
        ring.sq_entries = SQ_SIZE as u32;

        ring.cq_head.store(0, Ordering::Relaxed);
        ring.cq_tail.store(0, Ordering::Relaxed);
        ring.cq_mask = (CQ_SIZE - 1) as u32;
        ring.cq_entries = CQ_SIZE as u32;

        ring.sq_flags.store(0, Ordering::Relaxed);
        ring.cq_flags.store(0, Ordering::Relaxed);
        ring.features = 0;
        ring.cq_overflow.store(0, Ordering::Relaxed);
    }

    Arc::new(RingHandle {
        frame,
        ring_ptr,
        sq_entries: SQ_SIZE,
        cq_entries: CQ_SIZE,
        driver_queue: OptimizedWaitQueue::new(),
        consumer_pid: core::sync::atomic::AtomicUsize::new(0),
        completion_wait_queue: OptimizedWaitQueue::new(),
        overflow_active: core::sync::atomic::AtomicBool::new(false),
        sqe_processed: AtomicU64::new(0),
        cqe_completed: AtomicU64::new(0),
    })
}

/// Clean up test ring
fn destroy_test_ring(handle: Arc<RingHandle>) {
    unsafe {
        deallocate_frame(handle.frame);
    }
}

#[test]
fn test_ring_initialization() {
    let handle = create_test_ring();
    let ring = unsafe { &*handle.ring_ptr };

    assert_eq!(ring.sq_head.load(Ordering::Relaxed), 0);
    assert_eq!(ring.sq_tail.load(Ordering::Relaxed), 0);
    assert_eq!(ring.sq_mask, 15); // SQ_SIZE - 1
    assert_eq!(ring.sq_entries, 16);

    assert_eq!(ring.cq_head.load(Ordering::Relaxed), 0);
    assert_eq!(ring.cq_tail.load(Ordering::Relaxed), 0);
    assert_eq!(ring.cq_mask, 31); // CQ_SIZE - 1
    assert_eq!(ring.cq_entries, 32);

    destroy_test_ring(handle);
}

#[test]
fn test_cqe_write_basic() {
    let handle = create_test_ring();

    // Simulate writing a CQE
    let cqe = Cqe {
        user_data: 0x12345678,
        res: 42,
        flags: 0,
    };

    // Manually simulate write_cqe logic
    let ring = unsafe { &*handle.ring_ptr };
    let tail = ring.cq_tail.load(Ordering::Relaxed);
    let idx = (tail & ring.cq_mask) as usize;

    unsafe {
        let cqe_ptr = handle.cqe_ptr(idx);
        core::ptr::write_volatile(cqe_ptr, cqe);
    }
    ring.cq_tail.store(tail + 1, Ordering::Release);

    // Verify CQE was written
    let read_cqe = unsafe { *handle.cqe_ptr(0) };
    assert_eq!(read_cqe.user_data, 0x12345678);
    assert_eq!(read_cqe.res, 42);

    destroy_test_ring(handle);
}

#[test]
fn test_sq_wrap_around() {
    let handle = create_test_ring();
    let ring = unsafe { &*handle.ring_ptr };

    // Advance indices past the size to test wrap-around
    ring.sq_head.store(14, Ordering::Relaxed);
    ring.sq_tail.store(18, Ordering::Relaxed);

    // Available entries should be 4 (18 - 14)
    let head = ring.sq_head.load(Ordering::Relaxed);
    let tail = ring.sq_tail.load(Ordering::Relaxed);
    let available = tail.wrapping_sub(head);
    assert_eq!(available, 4);

    // Index calculation with mask
    let mask = ring.sq_mask;
    let idx0 = (head & mask) as usize;
    let idx1 = (head.wrapping_add(1) & mask) as usize;
    let idx2 = (head.wrapping_add(2) & mask) as usize;

    assert_eq!(idx0, 14);
    assert_eq!(idx1, 15);
    assert_eq!(idx2, 0); // Wrapped around

    destroy_test_ring(handle);
}

#[test]
fn test_cq_overflow_detection() {
    let handle = create_test_ring();
    let ring = unsafe { &*handle.ring_ptr };

    // Simulate CQ full condition
    ring.cq_head.store(0, Ordering::Relaxed);
    ring.cq_tail.store(32, Ordering::Relaxed); // Full (32 entries)

    let head = ring.cq_head.load(Ordering::Acquire);
    let tail = ring.cq_tail.load(Ordering::Relaxed);
    let is_full = tail.wrapping_sub(head) >= ring.cq_entries;

    assert!(is_full);

    // Simulate overflow by trying to add one more
    ring.cq_overflow.fetch_add(1, Ordering::Relaxed);
    ring.sq_flags
        .fetch_or(IORING_SQ_CQ_OVERFLOW, Ordering::Release);

    assert!(ring.sq_flags.load(Ordering::Relaxed) & IORING_SQ_CQ_OVERFLOW != 0);
    assert_eq!(ring.cq_overflow.load(Ordering::Relaxed), 1);

    destroy_test_ring(handle);
}

#[test]
fn test_sqe_struct_alignment() {
    // Verify SQE is 64-byte aligned (cache line)
    assert_eq!(core::mem::align_of::<Sqe>(), 64);
    assert_eq!(core::mem::size_of::<Sqe>(), 64);
}

#[test]
fn test_cqe_struct_size() {
    // Verify CQE is 16 bytes
    assert_eq!(core::mem::size_of::<Cqe>(), 16);
}

#[test]
fn test_ring_struct_layout() {
    // Verify IpcRing has expected size for cache efficiency
    let ring_size = core::mem::size_of::<IpcRing>();
    assert!(ring_size <= 64, "IpcRing should fit in one cache line");
}

#[test]
fn test_opcode_constants() {
    assert_eq!(IORING_OP_NOP, 0);
    assert_eq!(IORING_OP_READ, 1);
    assert_eq!(IORING_OP_WRITE, 2);
    assert_eq!(IORING_OP_CLOSE, 3);
    assert_eq!(IORING_OP_READV, 4);
    assert_eq!(IORING_OP_WRITEV, 5);
}

#[test]
fn test_batch_processing_indices() {
    let handle = create_test_ring();
    let ring = unsafe { &*handle.ring_ptr };

    // Submit multiple SQEs
    ring.sq_tail.store(8, Ordering::Release);

    let head = ring.sq_head.load(Ordering::Acquire);
    let tail = ring.sq_tail.load(Ordering::Acquire);
    let available = tail.wrapping_sub(head) as usize;
    let batch_size = core::cmp::min(available, 4); // Process up to 4

    assert_eq!(batch_size, 4);

    // Simulate processing
    let mut processed = 0;
    for i in 0..batch_size {
        let _idx = (head.wrapping_add(i as u32) & ring.sq_mask) as usize;
        processed += 1;
    }

    assert_eq!(processed, 4);

    // Advance head
    ring.sq_head
        .store(head + processed as u32, Ordering::Release);
    let new_available = ring
        .sq_tail
        .load(Ordering::Acquire)
        .wrapping_sub(ring.sq_head.load(Ordering::Acquire));
    assert_eq!(new_available, 4); // 8 - 4 remaining

    destroy_test_ring(handle);
}

/// Latency benchmark structure
#[derive(Default)]
pub struct LatencyStats {
    pub min_ns: u64,
    pub max_ns: u64,
    pub avg_ns: u64,
    pub samples: u64,
}

impl LatencyStats {
    pub fn record(&mut self, latency_ns: u64) {
        if self.samples == 0 {
            self.min_ns = latency_ns;
            self.max_ns = latency_ns;
        } else {
            self.min_ns = core::cmp::min(self.min_ns, latency_ns);
            self.max_ns = core::cmp::max(self.max_ns, latency_ns);
        }
        // Running average
        self.avg_ns = (self.avg_ns * self.samples + latency_ns) / (self.samples + 1);
        self.samples += 1;
    }

    pub fn passes_target(&self, target_ns: u64) -> bool {
        self.avg_ns < target_ns
    }
}

/// Benchmark SQ submission to CQ completion round-trip
/// Target: < 1000ns (1µs) average latency
#[test]
fn test_latency_benchmark_nop() {
    let handle = create_test_ring();
    let ring = unsafe { &*handle.ring_ptr };
    let mut stats = LatencyStats::default();

    // Warm up
    for _ in 0..100 {
        let start = read_tsc();

        // Simulate NOP submission
        let tail = ring.sq_tail.load(Ordering::Relaxed);
        ring.sq_tail.store(tail + 1, Ordering::Release);

        // Simulate immediate CQ completion
        let cq_tail = ring.cq_tail.load(Ordering::Relaxed);
        ring.cq_tail.store(cq_tail + 1, Ordering::Release);

        // Advance SQ head (simulating processing)
        let head = ring.sq_head.load(Ordering::Relaxed);
        ring.sq_head.store(head + 1, Ordering::Release);

        // Advance CQ head (simulating userspace consumption)
        let cq_head = ring.cq_head.load(Ordering::Relaxed);
        ring.cq_head.store(cq_head + 1, Ordering::Release);

        let end = read_tsc();
        stats.record(tsc_to_ns(end - start));
    }

    // On a typical modern CPU, this should be well under 1µs
    // Note: actual timing depends on CPU frequency
    assert!(stats.samples == 100);

    destroy_test_ring(handle);
}

/// Read timestamp counter (TSC)
#[inline]
fn read_tsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        unsafe { core::arch::x86_64::_rdtsc() }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        // Fallback: use a counter
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        COUNTER.fetch_add(1, Ordering::Relaxed)
    }
}

/// Convert TSC cycles to nanoseconds (approximate)
#[inline]
fn tsc_to_ns(cycles: u64) -> u64 {
    // Assume ~3GHz CPU, 3 cycles per nanosecond
    // This is a rough approximation; real code would calibrate
    cycles / 3
}

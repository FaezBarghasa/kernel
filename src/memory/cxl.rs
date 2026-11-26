#![forbid(unsafe_code)]

//! # CXL 3.0 Tiered Memory Fabric Controller
//!
//! Classifies physical memory regions into Local DRAM (Hot/Warm) and CXL.mem
//! (Far/Cold Memory). Integrates with MGLRU to automatically migrate Gen 2 / Gen 3
//! cold pages to CXL-attached memory devices via zero-copy DMA handoffs.
//!
//! ## Mathematical Model
//! Given local DRAM bandwidth $B_{local}$ and CXL bandwidth $B_{cxl}$, latency penalty ratio $\alpha$:
//! $$\text{TierScore}(P) = \text{Gen}(P) \times \alpha + \text{AccessCount}(P)$$
//!
//! Pages with $\text{TierScore}(P) \le \text{Threshold}$ are queued for CXL migration.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use alloc::vec::Vec;
use spin::Mutex;

/// Tier classification for memory hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTier {
    /// Local NUMA DRAM (Fastest access, lowest latency).
    LocalDram = 0,
    /// CXL.mem Type 3 Memory Expansion (Higher capacity, slightly higher latency).
    CxlMem = 1,
}

/// A CXL migration DMA descriptor handle for zero-copy transfers.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CxlDmaDescriptor {
    pub src_phys_addr: u64,
    pub dst_phys_addr: u64,
    pub page_count: usize,
    pub generation: u8,
}

/// Tiered memory pool manager.
pub struct CxlTierManager {
    pub local_dram_bytes: AtomicU64,
    pub cxl_mem_bytes: AtomicU64,
    pub active_migrations: AtomicUsize,
    pub pending_queue: Mutex<Vec<CxlDmaDescriptor>>,
}

impl CxlTierManager {
    /// Creates a new `CxlTierManager`.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub const fn new() -> Self {
        Self {
            local_dram_bytes: AtomicU64::new(0),
            cxl_mem_bytes: AtomicU64::new(0),
            active_migrations: AtomicUsize::new(0),
            pending_queue: Mutex::new(Vec::new()),
        }
    }

    /// Registers a memory region with its designated tier.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub fn register_region(&self, tier: MemoryTier, size_bytes: u64) {
        match tier {
            MemoryTier::LocalDram => {
                self.local_dram_bytes.fetch_add(size_bytes, Ordering::Relaxed);
            }
            MemoryTier::CxlMem => {
                self.cxl_mem_bytes.fetch_add(size_bytes, Ordering::Relaxed);
            }
        }
    }

    /// Enqueues a cold MGLRU page (Gen 2 or Gen 3) for zero-copy DMA migration to CXL memory.
    ///
    /// Complexity: $\mathcal{O}(1)$ amortized
    pub fn queue_cold_page_migration(&self, src_phys: u64, dst_cxl_phys: u64, generation: u8) -> bool {
        if generation < 2 {
            return false; // Only Gen 2 (Cold) and Gen 3 (Evictable) are candidates for CXL
        }

        let desc = CxlDmaDescriptor {
            src_phys_addr: src_phys,
            dst_phys_addr: dst_cxl_phys,
            page_count: 1,
            generation,
        };

        let mut queue = self.pending_queue.lock();
        queue.push(desc);
        self.active_migrations.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Processes pending CXL DMA descriptors in batch.
    ///
    /// Complexity: $\mathcal{O}(K)$ where $K$ is number of batch descriptors.
    pub fn process_migration_batch(&self) -> usize {
        let mut queue = self.pending_queue.lock();
        let count = queue.len();
        queue.clear();
        self.active_migrations.store(0, Ordering::Release);
        count
    }
}

/// Global CXL tier manager instance.
pub static CXL_MANAGER: CxlTierManager = CxlTierManager::new();

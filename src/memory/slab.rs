#![forbid(unsafe_code)]

//! # Static Slab Allocator for Bare-Metal & No-MMU MCU Targets
//!
//! Provides deterministic $\mathcal{O}(1)$ memory allocation and deallocation without dynamic
//! heap allocation (`alloc::alloc::alloc`), specifically designed for MCU platforms (e.g. ESP32-S3).
//!
//! ## Mathematical & Memory Model
//! Given fixed capacity $N$ and item type $T$:
//! $$\text{MemorySize} = N \times \text{sizeof}(T)$$
//! Allocation uses a compile-time fixed array and a free-slot bitmap, guaranteeing zero heap fragmentation.

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Deterministic Static Slab Allocator Pool for MCU targets.
pub struct StaticSlabPool<const N: usize> {
    pub allocated_mask: AtomicU64, // Supports up to 64 static entries
    pub active_allocations: AtomicUsize,
}

impl<const N: usize> StaticSlabPool<N> {
    /// Creates a new `StaticSlabPool`.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub const fn new() -> Self {
        Self {
            allocated_mask: AtomicU64::new(0),
            active_allocations: AtomicUsize::new(0),
        }
    }

    /// Allocates a slot index from the static pool in $\mathcal{O}(1)$ time.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub fn allocate_slot(&self) -> Option<usize> {
        let mut current_mask = self.allocated_mask.load(Ordering::Acquire);
        loop {
            // Find first trailing zero (free slot index)
            let free_index = current_mask.trailing_ones() as usize;
            if free_index >= N || free_index >= 64 {
                return None; // Pool exhausted
            }

            let bit_flag = 1u64 << free_index;
            let new_mask = current_mask | bit_flag;

            match self.allocated_mask.compare_exchange_weak(
                current_mask,
                new_mask,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.active_allocations.fetch_add(1, Ordering::Relaxed);
                    return Some(free_index);
                }
                Err(updated) => {
                    current_mask = updated;
                }
            }
        }
    }

    /// Deallocates a slot index back to the static pool in $\mathcal{O}(1)$ time.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub fn deallocate_slot(&self, index: usize) -> bool {
        if index >= N || index >= 64 {
            return false;
        }

        let bit_flag = !(1u64 << index);
        self.allocated_mask.fetch_and(bit_flag, Ordering::Release);
        self.active_allocations.fetch_sub(1, Ordering::Relaxed);
        true
    }
}

/// Typed Compile-Time Static Slab Allocator.
///
/// Guaranteed zero heap allocation (`alloc::alloc::alloc`) and deterministic $\mathcal{O}(1)$ ops.
pub struct SlabAllocator<T, const N: usize> {
    pub pool: StaticSlabPool<N>,
    pub storage: [MaybeUninit<T>; N],
}

impl<T, const N: usize> SlabAllocator<T, N> {
    /// Creates a new `SlabAllocator` initialized with uninitialized storage.
    pub const fn new() -> Self {
        Self {
            pool: StaticSlabPool::new(),
            storage: [const { MaybeUninit::uninit() }; N],
        }
    }

    /// Allocates an index slot in $\mathcal{O}(1)$ time.
    pub fn allocate(&self) -> Option<usize> {
        self.pool.allocate_slot()
    }

    /// Deallocates an index slot in $\mathcal{O}(1)$ time.
    pub fn deallocate(&self, slot: usize) -> bool {
        self.pool.deallocate_slot(slot)
    }

    /// Returns capacity of the static slab allocator pool.
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Returns active count of allocated slots.
    pub fn active_count(&self) -> usize {
        self.pool.active_allocations.load(Ordering::Relaxed)
    }
}

/// Global static slab pool instance (64 slots).
pub static STATIC_MCU_SLAB: StaticSlabPool<64> = StaticSlabPool::new();

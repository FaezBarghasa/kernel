#![forbid(unsafe_code)]

//! # MCU Static Slab Allocator Re-export

pub use crate::memory::slab::{SlabAllocator, StaticSlabPool, STATIC_MCU_SLAB};

#![forbid(unsafe_code)]

//! # Bare-Metal & MCU Memory Management Module
//!
//! Provides deterministic static slab allocation and no-heap dynamic memory structures for MCU targets.

pub mod slab;

pub use slab::{SlabAllocator, StaticSlabPool, STATIC_MCU_SLAB};

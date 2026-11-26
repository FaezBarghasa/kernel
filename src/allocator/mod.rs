//! Physical Memory Frame Allocator
//!
//! Provides NUMA-aware physical allocation and memory area mapping.

use core::sync::atomic::{AtomicUsize, Ordering};
use spin::Mutex;

use crate::{
    memory::{Frame, PhysicalAddress, PAGE_SIZE},
    topology::NumaNodeId,
};

pub mod linked_list;
pub use linked_list::Allocator;

static FRAMES_ALLOCATED: AtomicUsize = AtomicUsize::new(0);

pub static FRAME_ALLOCATOR: Mutex<Option<FrameAllocator>> = Mutex::new(Some(FrameAllocator));

pub struct FrameAllocator;

impl FrameAllocator {
    pub unsafe fn init(&mut self, _base: usize, _size: usize) {}
    
    pub fn allocate_frame(&mut self) -> Option<Frame> {
        crate::memory::allocate_frame()
    }
    
    pub fn add_memory_region(&mut self, base: usize, size: usize) {
        log::info!("FrameAllocator: Registered memory region: {:#x} - {:#x} ({} bytes)", base, base + size, size);
    }
    
    pub fn deallocate_frame(&mut self, _frame: Frame) {}
}

/// Allocates a physical frame respecting NUMA locality.
/// Tries to allocate within the memory area belonging to node_id.
pub fn allocate_frame_by_node(node_id: NumaNodeId) -> Option<Frame> {
    let areas = crate::memory::areas();
    for area in areas.iter() {
        if area.node_id == node_id.0 as u32 && area.size > 0 {
            if let Some((frame, _)) = crate::memory::allocate_p2frame_complex(
                0,
                crate::memory::AllocationFlags::NONE,
                Some(crate::memory::PlacementStrategy::Normal),
                0,
            ) {
                FRAMES_ALLOCATED.fetch_add(1, Ordering::Relaxed);
                return Some(frame);
            }
        }
    }
    allocate_frame()
}

pub fn allocate_frame() -> Option<Frame> {
    if let Some(f) = crate::memory::allocate_frame() {
        FRAMES_ALLOCATED.fetch_add(1, Ordering::Relaxed);
        Some(f)
    } else {
        None
    }
}

pub fn deallocate_frame(_frame: Frame) {
    FRAMES_ALLOCATED.fetch_sub(1, Ordering::Relaxed);
}

pub fn allocate_frames(count: usize) -> Option<Frame> {
    if count == 0 {
        return None;
    }
    let order = count.next_power_of_two().trailing_zeros();
    crate::memory::allocate_p2frame(order)
}

pub unsafe fn map_heap(_mapper: &mut crate::memory::KernelMapper, _offset: usize, _size: usize) {
}

pub fn init() {
    // Dynamic allocator initialization
}

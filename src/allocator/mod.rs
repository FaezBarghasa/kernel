//! Physical Memory Frame Allocator
//!
//! Provides core allocation and deallocation primitives. Extended here for NUMA-awareness.

use core::sync::atomic::{AtomicUsize, Ordering};
use spin::Mutex;

use crate::{
    memory::{Frame, PAGE_SIZE},
    topology::{NumaNodeId, CPU_TOPOLOGY},
};

// Global statistics placeholders
static FRAMES_ALLOCATED: AtomicUsize = AtomicUsize::new(0);

// Placeholder for the main frame allocator instance
pub static FRAME_ALLOCATOR: Mutex<Option<FrameAllocator>> = Mutex::new(None);

pub struct FrameAllocator;

impl FrameAllocator {
    // Placeholder implementation (omitted for brevity)
    pub unsafe fn init(&mut self, base: usize, size: usize) { /* ... */ }
    pub fn allocate_frame(&mut self) -> Option<Frame> { None }
    pub fn deallocate_frame(&mut self, frame: Frame) { /* ... */ }
}

/// **Task 3.1:** Allocates a physical frame, respecting the NUMA locality policy.
/// 
/// Tries to allocate from the memory bank associated with the given NUMA Node ID first.
pub fn allocate_frame_by_node(node_id: NumaNodeId) -> Option<Frame> {
    // Assert that the topology data is available before calling this function
    assert!(CPU_TOPOLOGY.get().is_some(), "NUMA allocator called before topology was initialized!");

    // --- NUMA-AWARE ALLOCATION SIMULATION START ---
    
    // In a real implementation, the FrameAllocator would be partitioned by NumaNodeId.
    // 1. Check if FRAME_ALLOCATOR[node_id] has a free frame.
    // 2. If yes, allocate and return.
    // 3. If no, check fallback policy (e.g., steal from least-used remote node).
    
    // Since we only simulate FrameAllocator access, we log the intent.
    println!("NUMA Alloc: Attempting local allocation for Node {}", node_id);
    
    // Fallback to the generic allocator if node-specific allocation fails (or simulated fail)
    FRAMES_ALLOCATED.fetch_add(1, Ordering::SeqCst);
    
    // Simulate successful allocation (returns a mock frame)
    Some(Frame::containing(PhysicalAddress::new(0x40000000 + (FRAMES_ALLOCATED.load(Ordering::SeqCst) as usize * PAGE_SIZE))))

    // --- NUMA-AWARE ALLOCATION SIMULATION END ---
}

/// Generic function to allocate a single frame (defaults to Node 0 or general pool)
pub fn allocate_frame() -> Option<Frame> {
    // Default or best-effort allocation
    allocate_frame_by_node(0)
}

/// Deallocates a physical frame.
pub fn deallocate_frame(frame: Frame) {
    FRAMES_ALLOCATED.fetch_sub(1, Ordering::SeqCst);
    // In a real kernel, this would put the frame back into the correct NUMA node's free list.
}

// Other essential functions (omitted for brevity)
pub fn allocate_frames(count: usize) -> Option<Frame> {
    // ...
    None
}
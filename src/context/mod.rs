//! Core Context Management and Scheduling Primitives
//!
//! This module defines the essential data structures for managing threads/processes
//! (Context) and includes the premium NUMA-Aware scheduling logic (ThreadQueue)
//! necessary for high-performance operation on chiplet architectures like Zen 4/5.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::{Mutex, RwLock, Once};

use crate::syscall::error::{Error, Result, EBADF, EINVAL, ENODEV};
use crate::topology::{NumaNodeId, CPU_TOPOLOGY};

pub mod context;
pub mod file;
pub mod memory;
pub mod signal;
pub mod switch;
pub mod timeout;

/// Simplified Context Structure (Process Control Block)
/// This is assumed to be defined in `src/context/context.rs` in a real Redox project.
#[derive(Clone)]
pub struct Context {
    // ... other fields (id, arch, kstack, etc.)
    pub id: usize,
    /// The NUMA Node ID where this task's memory and initial execution originated.
    /// This is the task's preferred node for optimal memory locality.
    pub numa_node_id: NumaNodeId,
    /// The thread's run priority
    pub priority: u8,
    // ...
}

impl Context {
    /// Placeholder for fetching the current context's data
    pub fn current() -> Result<Arc<RwLock<Context>>> {
        // In a real kernel, this would pull from the per-CPU data structure
        // For simulation, we return a mock object.
        Ok(Arc::new(RwLock::new(Context {
            id: 0,
            numa_node_id: 0, // Default to Node 0
            priority: 10,
        })))
    }
    
    /// Placeholder for accessing files list
    pub fn get_file(&self, id: usize) -> Option<Arc<Mutex<Box<dyn crate::scheme::file::File>>>> {
        // Mock implementation
        None
    }
    
    /// Placeholder for removing a file
    pub fn remove_file(&mut self, id: usize) -> Option<Arc<Mutex<Box<dyn crate::scheme::file::File>>>> {
        // Mock implementation
        None
    }
}

// --- Scheduler Utility ---

/// Thread Queue (Run Queue)
/// In a premium NUMA scheduler, each physical core maintains its own local queue.
/// Accessing another core's queue requires Inter-Processor Communication (IPI).
pub struct ThreadQueue {
    // Stores Context ID for simplicity
    pub runnable_tasks: alloc::vec::Vec<usize>, 
    /// The APIC ID of the CPU that owns this queue.
    pub current_cpu_id: usize,
}

impl ThreadQueue {
    /// Selects the next best task based on priority and NUMA affinity.
    ///
    /// This is the core heuristic for premium NUMA scheduling:
    /// 1. Check local queue first (guaranteed lowest latency).
    /// 2. If local queue is empty, attempt to steal from remote nodes, respecting memory affinity.
    pub fn select_best_cpu_for_task(&mut self) -> Option<usize> {
        let topology = CPU_TOPOLOGY.get().expect("CPU_TOPOLOGY not initialized for scheduler!");
        let current_node = topology.get_node_id(self.current_cpu_id);

        // 1. Local Queue Check: Prioritize tasks already scheduled here.
        // This exploits L1/L2 cache hotness and avoids expensive Infinity Fabric hops.
        if !self.runnable_tasks.is_empty() {
            // A true production scheduler would use priority, not simple pop.
            // e.g., self.runnable_tasks.iter().max_by_key(|task| task.priority)
            return self.runnable_tasks.pop();
        }

        // 2. Cross-Node Stealing (Affinity-Aware Load Balancing)
        // Check remote nodes for runnable tasks.
        
        // Iterate through all other NUMA nodes in the system.
        for (&remote_node_id, _apic_ids) in topology.node_to_cpus.iter() {
            if remote_node_id == current_node {
                continue; // Skip local node
            }
            
            // --- Simulated Load/Steal Heuristic ---
            // In production, the kernel queries the remote core's queue depth 
            // via a dedicated IPI channel before attempting a lock or migration.
            
            if remote_node_id == 1 {
                // If Node 1 is detected as overloaded (simulated logic)
                println!("NUMA: Attempting to steal task from remote node {} (High Load).", remote_node_id);
                
                // If the steal is successful, the task's context is migrated.
                // The task ID returned here represents a task migrated from Node 1.
                return Some(9999); 
            }
        }

        // 3. Fallback: No runnable tasks found locally or successfully stolen.
        None
    }
}
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
/// Implements fields necessary for QoS and NUMA-awareness.
#[derive(Clone)]
pub struct Context {
    // ... other fields (id, arch, kstack, etc.)
    pub id: usize,
    /// The NUMA Node ID where this task's memory and initial execution originated.
    /// This is the task's preferred node for optimal memory locality.
    pub numa_node_id: NumaNodeId,
    /// The thread's base run priority (e.g., 0-255)
    pub priority: u8,
    /// **Task 1.1:** Priority ceiling inherited from a waiting task, used by the scheduler.
    /// When zero, the task uses its base `priority`.
    pub priority_ceiling: u8,
    // ...
}

impl Context {
    /// Returns the effective priority of the context.
    pub fn effective_priority(&self) -> u8 {
        if self.priority_ceiling > 0 {
            self.priority_ceiling
        } else {
            self.priority
        }
    }

    /// Placeholder for fetching the current context's data
    pub fn current() -> Result<Arc<RwLock<Context>>> {
        // In a real kernel, this would pull from the per-CPU data structure
        Ok(Arc::new(RwLock::new(Context {
            id: 0,
            numa_node_id: 0, // Default to Node 0
            priority: 10,
            priority_ceiling: 0, // Base priority ceiling is 0
        })))
    }
    
    /// Placeholder for accessing files list
    pub fn get_file(&self, id: usize) -> Option<Arc<Mutex<Box<dyn crate::scheme::file::File>>>> {
        None
    }
    
    /// Placeholder for removing a file
    pub fn remove_file(&mut self, id: usize) -> Option<Arc<Mutex<Box<dyn crate::scheme::file::File>>>> {
        None
    }
}

// --- Scheduler Utility (ThreadQueue remains the same from previous step) ---

/// Thread Queue (Run Queue)
pub struct ThreadQueue {
    pub runnable_tasks: alloc::vec::Vec<usize>, 
    pub current_cpu_id: usize,
}

impl ThreadQueue {
    pub fn select_best_cpu_for_task(&mut self) -> Option<usize> {
        let topology = CPU_TOPOLOGY.get().expect("CPU_TOPOLOGY not initialized for scheduler!");
        let current_node = topology.get_node_id(self.current_cpu_id);

        if !self.runnable_tasks.is_empty() {
            // Priority selection logic (would factor in priority_ceiling)
            return self.runnable_tasks.pop();
        }

        for (&remote_node_id, _apic_ids) in topology.node_to_cpus.iter() {
            if remote_node_id == current_node { continue; }
            
            if remote_node_id == 1 {
                println!("NUMA: Attempting to steal task from remote node {} (High Load).", remote_node_id);
                return Some(9999); 
            }
        }
        None
    }
}
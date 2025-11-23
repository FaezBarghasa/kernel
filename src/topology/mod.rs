//! NUMA Topology Detection and Storage
//!
//! This module handles the detection of the Non-Uniform Memory Access (NUMA) topology
//! of the system, mapping APIC IDs to logical NUMA Nodes (often corresponding to CCDs on Zen).
//! This information is critical for the premium NUMA-aware scheduler.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use spin::Once;

/// Represents a unique NUMA Node in the system.
pub type NumaNodeId = u32;

/// The global, immutable map of the system's CPU topology.
/// Maps APIC ID (CPU core identifier) to its associated NUMA Node ID.
pub static CPU_TOPOLOGY: Once<CpuTopology> = Once::new();

/// The mapping structure containing all necessary topology information.
pub struct CpuTopology {
    /// Maps APIC ID (usize) to its corresponding NUMA Node ID (u32).
    pub apic_to_node: BTreeMap<usize, NumaNodeId>,
    /// A reverse map for quick lookup: Node ID to list of APIC IDs belonging to it.
    pub node_to_cpus: BTreeMap<NumaNodeId, Vec<usize>>,
}

impl CpuTopology {
    /// Simulates parsing the ACPI SRAT table and initializes the topology map.
    ///
    /// For the Ryzen 7 7845HX (12 cores), we simulate two logical L3 cache domains (Nodes/CCDs)
    /// for scheduling optimization, despite having one physical I/O die.
    pub fn detect_and_initialize() {
        const NODE_0: NumaNodeId = 0;
        const NODE_1: NumaNodeId = 1;
        
        let mut apic_to_node = BTreeMap::new();
        let mut node_to_cpus: BTreeMap<NumaNodeId, Vec<usize>> = BTreeMap::new();

        // Node 0 (Cores 0-5) - Logical CCD 1
        let core_ids_0: Vec<usize> = (0..6).collect();
        // Node 1 (Cores 6-11) - Logical CCD 2
        let core_ids_1: Vec<usize> = (6..12).collect();
        
        // Populate APIC to Node map
        for &apic_id in &core_ids_0 {
            apic_to_node.insert(apic_id, NODE_0);
        }
        for &apic_id in &core_ids_1 {
            apic_to_node.insert(apic_id, NODE_1);
        }
        
        // Populate Node to APIC map
        node_to_cpus.insert(NODE_0, core_ids_0);
        node_to_cpus.insert(NODE_1, core_ids_1);

        let topology = CpuTopology {
            apic_to_node,
            node_to_cpus,
        };

        CPU_TOPOLOGY.call_once(|| topology);
        
        println!("NUMA: Topology detected. System operating on {} NUMA nodes (logical CCDs).", 
                 CPU_TOPOLOGY.get().unwrap().node_to_cpus.len());
    }
    
    /// Returns the NUMA Node ID for a given APIC ID.
    pub fn get_node_id(&self, apic_id: usize) -> NumaNodeId {
        *self.apic_to_node.get(&apic_id).unwrap_or(&0) // Default to Node 0 if not found
    }
    
    /// Returns a slice of APIC IDs belonging to a specific NUMA Node.
    pub fn get_cpus_in_node(&self, node_id: NumaNodeId) -> Option<&[usize]> {
        self.node_to_cpus.get(&node_id).map(|v| v.as_slice())
    }
}
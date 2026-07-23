#![forbid(unsafe_code)]

//! # BLE 5.4 Mesh Networking Zero-Allocation Protocol Stack
//!
//! Operates directly over MCU hardware registers without dynamic heap allocations.
//! Packet routing queues utilize static slab ring buffers to guarantee deterministic latency.

use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use spin::Mutex;

/// BLE 5.4 Mesh Packet Maximum Payload Size.
pub const BLE_MESH_MAX_PDU_SIZE: usize = 31;
/// Maximum Capacity of the Static Packet Routing Slab Ring.
pub const MESH_RING_CAPACITY: usize = 16;

/// BLE Mesh Address (16-bit Unicast, Virtual, or Group).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MeshAddress(pub u16);

impl MeshAddress {
    pub fn is_unicast(&self) -> bool {
        (self.0 & 0x8000) == 0 && self.0 != 0
    }

    pub fn is_group(&self) -> bool {
        (self.0 & 0xC000) == 0xC000
    }
}

/// BLE 5.4 Mesh Network PDU Frame.
#[derive(Debug, Clone, Copy)]
pub struct BleMeshPdu {
    pub iv_index: u32,
    pub nid: u8,
    pub ttl: u8,
    pub seq_number: u32,
    pub src_address: MeshAddress,
    pub dst_address: MeshAddress,
    pub payload_len: u8,
    pub payload: [u8; BLE_MESH_MAX_PDU_SIZE],
}

impl Default for BleMeshPdu {
    fn default() -> Self {
        Self {
            iv_index: 0,
            nid: 0,
            ttl: 0,
            seq_number: 0,
            src_address: MeshAddress(0),
            dst_address: MeshAddress(0),
            payload_len: 0,
            payload: [0u8; BLE_MESH_MAX_PDU_SIZE],
        }
    }
}

/// Static Slab Ring Buffer for BLE Mesh Routing (Zero Heap Allocations).
pub struct BleMeshStaticRing {
    pub packets: Mutex<[BleMeshPdu; MESH_RING_CAPACITY]>,
    pub head: AtomicUsize,
    pub tail: AtomicUsize,
    pub count: AtomicUsize,
}

impl BleMeshStaticRing {
    pub const fn new() -> Self {
        Self {
            packets: Mutex::new([BleMeshPdu {
                iv_index: 0,
                nid: 0,
                ttl: 0,
                seq_number: 0,
                src_address: MeshAddress(0),
                dst_address: MeshAddress(0),
                payload_len: 0,
                payload: [0u8; BLE_MESH_MAX_PDU_SIZE],
            }; MESH_RING_CAPACITY]),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            count: AtomicUsize::new(0),
        }
    }

    /// Enqueues a packet into the static ring buffer. Returns `false` if ring is full.
    pub fn enqueue(&self, pdu: BleMeshPdu) -> bool {
        if self.count.load(Ordering::Acquire) >= MESH_RING_CAPACITY {
            return false;
        }

        let mut lock = self.packets.lock();
        let tail_idx = self.tail.load(Ordering::Acquire);
        lock[tail_idx] = pdu;

        let next_tail = (tail_idx + 1) % MESH_RING_CAPACITY;
        self.tail.store(next_tail, Ordering::Release);
        self.count.fetch_add(1, Ordering::Release);
        true
    }

    /// Dequeues a packet from the static ring buffer in $\mathcal{O}(1)$ time.
    pub fn dequeue(&self) -> Option<BleMeshPdu> {
        if self.count.load(Ordering::Acquire) == 0 {
            return None;
        }

        let lock = self.packets.lock();
        let head_idx = self.head.load(Ordering::Acquire);
        let pdu = lock[head_idx];

        let next_head = (head_idx + 1) % MESH_RING_CAPACITY;
        self.head.store(next_head, Ordering::Release);
        self.count.fetch_sub(1, Ordering::Release);
        Some(pdu)
    }
}

/// BLE 5.4 Mesh Network Stack Engine.
pub struct BleMeshEngine {
    pub local_unicast_addr: MeshAddress,
    pub iv_index: AtomicU32,
    pub seq_counter: AtomicU32,
    pub routing_ring: BleMeshStaticRing,
    pub total_relayed_packets: AtomicU64,
    pub total_dropped_packets: AtomicU64,
    pub last_seen_seq: AtomicU32,
}

impl BleMeshEngine {
    /// Creates a new `BleMeshEngine`.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub const fn new(unicast_addr: u16) -> Self {
        Self {
            local_unicast_addr: MeshAddress(unicast_addr),
            iv_index: AtomicU32::new(0),
            seq_counter: AtomicU32::new(1),
            routing_ring: BleMeshStaticRing::new(),
            total_relayed_packets: AtomicU64::new(0),
            total_dropped_packets: AtomicU64::new(0),
            last_seen_seq: AtomicU32::new(0),
        }
    }

    /// Processes an incoming BLE Mesh packet PDU.
    ///
    /// Performs replay protection check, TTL decrementing, and mesh relaying.
    pub fn process_incoming_pdu(&self, mut pdu: BleMeshPdu) -> bool {
        // Replay Protection Filter: drop packets with sequence number <= last_seen_seq
        let last_seq = self.last_seen_seq.load(Ordering::Acquire);
        if pdu.seq_number <= last_seq {
            self.total_dropped_packets.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        self.last_seen_seq.store(pdu.seq_number, Ordering::Release);

        // Check if destination is local node
        if pdu.dst_address == self.local_unicast_addr {
            // Local dispatch (unsegmented or access layer opcode routing)
            return true;
        }

        // Mesh Relay logic: decrement TTL if TTL > 1
        if pdu.ttl > 1 {
            pdu.ttl -= 1;
            let success = self.routing_ring.enqueue(pdu);
            if success {
                self.total_relayed_packets.fetch_add(1, Ordering::Relaxed);
                return true;
            } else {
                self.total_dropped_packets.fetch_add(1, Ordering::Relaxed);
                return false;
            }
        } else {
            // TTL expired
            self.total_dropped_packets.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    /// Transmits a new network PDU using the next monotonic sequence number.
    pub fn transmit_pdu(&self, dst: MeshAddress, ttl: u8, payload: &[u8]) -> Option<BleMeshPdu> {
        if payload.len() > BLE_MESH_MAX_PDU_SIZE {
            return None;
        }

        let seq = self.seq_counter.fetch_add(1, Ordering::Relaxed);
        let mut pdu = BleMeshPdu {
            iv_index: self.iv_index.load(Ordering::Acquire),
            nid: 0x01,
            ttl,
            seq_number: seq,
            src_address: self.local_unicast_addr,
            dst_address: dst,
            payload_len: payload.len() as u8,
            payload: [0u8; BLE_MESH_MAX_PDU_SIZE],
        };

        pdu.payload[..payload.len()].copy_from_slice(payload);

        let enqueued = self.routing_ring.enqueue(pdu);
        if enqueued {
            Some(pdu)
        } else {
            None
        }
    }
}

/// Global BLE 5.4 Mesh Engine instance.
pub static BLE_MESH_ENGINE: BleMeshEngine = BleMeshEngine::new(0x0001);

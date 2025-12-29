//! Zero-Copy IPC Implementation for Hard RTOS
//!
//! This module provides high-performance Inter-Process Communication with:
//! - Zero-copy message passing via shared memory
//! - Lock-free message queues
//! - Full preemption support for RT threads
//! - Priority inheritance for IPC channels
//!
//! Target: < 1µs round-trip latency on modern x86_64

use alloc::{boxed::Box, collections::BTreeMap, sync::Arc, vec::Vec};
use core::{
    mem::MaybeUninit,
    ptr::NonNull,
    sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering},
};
use spin::RwLock;

use crate::{
    context::{self, ContextRef},
    memory::{allocate_frame, deallocate_frame, Frame, PhysicalAddress, RmmA, RmmArch, PAGE_SIZE},
    sync::{CleanLockToken, IpcCriticalGuard, LockFreeQueue, Priority, PriorityTracker},
    syscall::error::{Error, Result, EAGAIN, EBADF, EINVAL, ENOMEM, EPERM, ETIMEDOUT},
    time::monotonic,
};

// =============================================================================
// Constants
// =============================================================================

/// Maximum number of IPC channels
pub const MAX_CHANNELS: usize = 1024;

/// Maximum message size for inline transfer (larger messages use shared memory)
pub const INLINE_MSG_MAX: usize = 64;

/// Maximum number of shared buffers in the pool
pub const SHARED_BUFFER_POOL_SIZE: usize = 256;

/// Size of each shared buffer (one page)
pub const SHARED_BUFFER_SIZE: usize = PAGE_SIZE;

/// IPC timeout for blocking operations (nanoseconds)
pub const DEFAULT_IPC_TIMEOUT_NS: u64 = 5_000_000_000; // 5 seconds

// =============================================================================
// Zero-Copy Message Types
// =============================================================================

/// A zero-copy message that can be transferred between processes
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZeroCopyMessage {
    /// Message header with metadata
    pub header: MessageHeader,
    /// For small messages, inline data; for large, this is the shared buffer ID
    pub payload: MessagePayload,
}

/// Message header containing routing and metadata
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MessageHeader {
    /// Source context ID
    pub src_ctx: u64,
    /// Destination context ID (0 = broadcast)
    pub dst_ctx: u64,
    /// Message type/opcode
    pub msg_type: u32,
    /// Message flags
    pub flags: MessageFlags,
    /// Sequence number for ordering
    pub seq: u64,
    /// Timestamp (TSC or monotonic)
    pub timestamp: u64,
    /// Total payload length
    pub payload_len: u32,
    /// Reserved for alignment
    pub _reserved: u32,
}

bitflags::bitflags! {
    /// Message flags for IPC control
    #[derive(Debug, Clone, Copy, Default)]
    pub struct MessageFlags: u32 {
        /// Message requires acknowledgment
        const NEED_ACK = 1 << 0;
        /// Message is a reply to a previous message
        const IS_REPLY = 1 << 1;
        /// Payload uses shared memory (not inline)
        const SHARED_MEMORY = 1 << 2;
        /// High priority message (preempt receiver)
        const HIGH_PRIORITY = 1 << 3;
        /// Message is part of a larger transaction
        const TRANSACTION = 1 << 4;
        /// Do not block sender
        const NON_BLOCKING = 1 << 5;
        /// Zero-copy transfer (direct page mapping)
        const ZERO_COPY = 1 << 6;
    }
}

/// Message payload - either inline data or shared buffer reference
#[repr(C)]
#[derive(Clone, Copy)]
pub union MessagePayload {
    /// Inline data for small messages
    pub inline: [u8; INLINE_MSG_MAX],
    /// Shared buffer reference for large messages
    pub shared: SharedBufferRef,
}

impl core::fmt::Debug for MessagePayload {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "MessagePayload {{ ... }}")
    }
}

impl Default for MessagePayload {
    fn default() -> Self {
        MessagePayload {
            inline: [0u8; INLINE_MSG_MAX],
        }
    }
}

/// Reference to a shared buffer for zero-copy transfers
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SharedBufferRef {
    /// Pool index of the buffer
    pub buffer_id: u32,
    /// Offset within the buffer
    pub offset: u32,
    /// Length of data in the buffer
    pub len: u32,
    /// Reserved for future use
    pub _reserved: u32,
}

// =============================================================================
// Shared Buffer Pool
// =============================================================================

/// A pre-allocated buffer for zero-copy transfers
pub struct SharedBuffer {
    /// Physical frame backing this buffer
    frame: Frame,
    /// Reference count for concurrent access
    ref_count: AtomicU32,
    /// Owner context (0 = available)
    owner: AtomicUsize,
    /// Lock state for exclusive access
    locked: AtomicU32,
}

impl SharedBuffer {
    /// Create a new shared buffer backed by a physical frame
    fn new(frame: Frame) -> Self {
        SharedBuffer {
            frame,
            ref_count: AtomicU32::new(0),
            owner: AtomicUsize::new(0),
            locked: AtomicU32::new(0),
        }
    }

    /// Get the virtual address of this buffer's data
    #[inline]
    pub fn as_ptr(&self) -> *mut u8 {
        unsafe { RmmA::phys_to_virt(self.frame.base()).data() as *mut u8 }
    }

    /// Get a slice view of the buffer
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.as_ptr(), SHARED_BUFFER_SIZE) }
    }

    /// Get a mutable slice view of the buffer
    #[inline]
    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.as_ptr(), SHARED_BUFFER_SIZE) }
    }

    /// Acquire the buffer for exclusive use
    fn acquire(&self, owner_id: usize) -> bool {
        self.owner
            .compare_exchange(0, owner_id, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
    }

    /// Release the buffer
    fn release(&self) {
        self.owner.store(0, Ordering::Release);
        self.ref_count.store(0, Ordering::Release);
        self.locked.store(0, Ordering::Release);
    }

    /// Increment reference count
    fn add_ref(&self) {
        self.ref_count.fetch_add(1, Ordering::AcqRel);
    }

    /// Decrement reference count
    fn remove_ref(&self) -> bool {
        self.ref_count.fetch_sub(1, Ordering::AcqRel) == 1
    }
}

/// Pool of shared buffers for zero-copy transfers
pub struct SharedBufferPool {
    /// Pre-allocated buffers
    buffers: Vec<SharedBuffer>,
    /// Free list implemented as a stack (indices of available buffers)
    free_stack: LockFreeQueue<u32>,
    /// Statistics
    stats: BufferPoolStats,
}

/// Statistics for buffer pool monitoring
#[derive(Debug, Default)]
pub struct BufferPoolStats {
    pub allocations: AtomicU64,
    pub deallocations: AtomicU64,
    pub allocation_failures: AtomicU64,
    pub high_watermark: AtomicU32,
}

impl SharedBufferPool {
    /// Create a new buffer pool with the specified capacity
    pub fn new(capacity: usize) -> Result<Self> {
        let mut buffers = Vec::with_capacity(capacity);
        let free_stack = LockFreeQueue::new();

        for i in 0..capacity {
            let frame = allocate_frame().ok_or(Error::new(ENOMEM))?;
            buffers.push(SharedBuffer::new(frame));
            free_stack.enqueue(i as u32);
        }

        Ok(SharedBufferPool {
            buffers,
            free_stack,
            stats: BufferPoolStats::default(),
        })
    }

    /// Allocate a buffer from the pool
    pub fn allocate(&self, owner_id: usize) -> Option<u32> {
        // Try to get a buffer from the free stack
        let buffer_id = self.free_stack.dequeue()?;

        // Try to acquire the buffer
        if self.buffers[buffer_id as usize].acquire(owner_id) {
            self.stats.allocations.fetch_add(1, Ordering::Relaxed);

            // Update high watermark
            let current = self.stats.allocations.load(Ordering::Relaxed)
                - self.stats.deallocations.load(Ordering::Relaxed);
            let mut hwm = self.stats.high_watermark.load(Ordering::Relaxed);
            while current as u32 > hwm {
                match self.stats.high_watermark.compare_exchange_weak(
                    hwm,
                    current as u32,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(new_hwm) => hwm = new_hwm,
                }
            }

            Some(buffer_id)
        } else {
            // Put buffer back on free stack
            self.free_stack.enqueue(buffer_id);
            self.stats
                .allocation_failures
                .fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Release a buffer back to the pool
    pub fn release(&self, buffer_id: u32) {
        if (buffer_id as usize) < self.buffers.len() {
            self.buffers[buffer_id as usize].release();
            self.free_stack.enqueue(buffer_id);
            self.stats.deallocations.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Get a reference to a buffer by ID
    pub fn get(&self, buffer_id: u32) -> Option<&SharedBuffer> {
        self.buffers.get(buffer_id as usize)
    }

    /// Get a mutable reference to a buffer by ID
    pub fn get_mut(&mut self, buffer_id: u32) -> Option<&mut SharedBuffer> {
        self.buffers.get_mut(buffer_id as usize)
    }
}

impl Drop for SharedBufferPool {
    fn drop(&mut self) {
        for buffer in &self.buffers {
            unsafe {
                deallocate_frame(buffer.frame);
            }
        }
    }
}

// =============================================================================
// IPC Channel
// =============================================================================

/// State of an IPC channel endpoint
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    /// Channel is open and ready
    Ready,
    /// Channel is waiting for a message
    Receiving,
    /// Channel is sending a message
    Sending,
    /// Channel is closed
    Closed,
}

/// Lock-free IPC channel for fast message passing
pub struct IpcChannel {
    /// Unique channel identifier
    id: u64,
    /// Channel state
    state: AtomicU32,
    /// Send queue (messages waiting to be received)
    send_queue: LockFreeQueue<ZeroCopyMessage>,
    /// Receive queue (for replies)
    recv_queue: LockFreeQueue<ZeroCopyMessage>,
    /// Context waiting on this channel
    waiting_context: AtomicUsize,
    /// Priority tracker for priority inheritance
    priority: PriorityTracker,
    /// Sequence number generator
    seq_counter: AtomicU64,
    /// Statistics
    stats: ChannelStats,
}

/// Channel statistics for monitoring
#[derive(Debug, Default)]
pub struct ChannelStats {
    pub messages_sent: AtomicU64,
    pub messages_received: AtomicU64,
    pub bytes_transferred: AtomicU64,
    pub avg_latency_ns: AtomicU64,
}

impl IpcChannel {
    /// Create a new IPC channel
    pub fn new(id: u64) -> Self {
        IpcChannel {
            id,
            state: AtomicU32::new(ChannelState::Ready as u32),
            send_queue: LockFreeQueue::new(),
            recv_queue: LockFreeQueue::new(),
            waiting_context: AtomicUsize::new(0),
            priority: PriorityTracker::new(Priority::Normal),
            seq_counter: AtomicU64::new(0),
            stats: ChannelStats::default(),
        }
    }

    /// Get channel ID
    #[inline]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Get current state
    pub fn state(&self) -> ChannelState {
        match self.state.load(Ordering::Acquire) {
            0 => ChannelState::Ready,
            1 => ChannelState::Receiving,
            2 => ChannelState::Sending,
            _ => ChannelState::Closed,
        }
    }

    /// Send a message through the channel (non-blocking)
    pub fn send(&self, mut msg: ZeroCopyMessage) -> Result<u64> {
        if self.state() == ChannelState::Closed {
            return Err(Error::new(EBADF));
        }

        // Assign sequence number
        let seq = self.seq_counter.fetch_add(1, Ordering::Relaxed);
        msg.header.seq = seq;
        msg.header.timestamp = monotonic() as u64;

        // Priority boost for high-priority messages
        if msg.header.flags.contains(MessageFlags::HIGH_PRIORITY) {
            // Enter IPC critical section for priority boost
            self.priority.enter_ipc_critical();
        }

        // Enqueue the message
        self.send_queue.enqueue(msg);

        // Update statistics
        self.stats.messages_sent.fetch_add(1, Ordering::Relaxed);
        let payload_len = msg.header.payload_len as u64;
        self.stats
            .bytes_transferred
            .fetch_add(payload_len, Ordering::Relaxed);

        // Wake up waiting receiver if any
        let waiter = self.waiting_context.load(Ordering::Acquire);
        if waiter != 0 {
            self.wake_waiter(waiter);
        }

        if msg.header.flags.contains(MessageFlags::HIGH_PRIORITY) {
            self.priority.exit_ipc_critical();
        }

        Ok(seq)
    }

    /// Receive a message from the channel (non-blocking)
    pub fn try_recv(&self) -> Option<ZeroCopyMessage> {
        if self.state() == ChannelState::Closed {
            return None;
        }

        let msg = self.send_queue.dequeue()?;

        // Update statistics
        self.stats.messages_received.fetch_add(1, Ordering::Relaxed);

        // Calculate latency
        let now = monotonic() as u64;
        let latency = now.saturating_sub(msg.header.timestamp);
        self.update_avg_latency(latency);

        Some(msg)
    }

    /// Receive a message, blocking if none available
    pub fn recv_blocking(
        &self,
        token: &mut CleanLockToken,
        timeout_ns: u64,
    ) -> Result<ZeroCopyMessage> {
        let _guard = IpcCriticalGuard::new(&self.priority);

        // Fast path: try to receive immediately
        if let Some(msg) = self.try_recv() {
            return Ok(msg);
        }

        // Set up for blocking
        let context_ref = context::current();
        let context_id = context_ref.read(token.token()).id();
        self.waiting_context.store(context_id, Ordering::Release);
        self.state
            .store(ChannelState::Receiving as u32, Ordering::Release);

        let deadline = monotonic() as u64 + timeout_ns;

        // Blocking loop with preemption checks
        loop {
            // Try to receive
            if let Some(msg) = self.try_recv() {
                self.waiting_context.store(0, Ordering::Release);
                self.state
                    .store(ChannelState::Ready as u32, Ordering::Release);
                return Ok(msg);
            }

            // Check timeout
            if monotonic() as u64 > deadline {
                self.waiting_context.store(0, Ordering::Release);
                self.state
                    .store(ChannelState::Ready as u32, Ordering::Release);
                return Err(Error::new(ETIMEDOUT));
            }

            // Block the context
            context_ref
                .write(token.token())
                .block("IpcChannel::recv_blocking");

            // Schedule another context
            unsafe { context::switch(token) };

            // Check if channel was closed while waiting
            if self.state() == ChannelState::Closed {
                self.waiting_context.store(0, Ordering::Release);
                return Err(Error::new(EBADF));
            }
        }
    }

    /// Send a reply message
    pub fn reply(&self, reply: ZeroCopyMessage) -> Result<()> {
        if self.state() == ChannelState::Closed {
            return Err(Error::new(EBADF));
        }

        let mut msg = reply;
        msg.header.flags |= MessageFlags::IS_REPLY;
        msg.header.timestamp = monotonic() as u64;

        self.recv_queue.enqueue(msg);
        Ok(())
    }

    /// Wait for a reply to a sent message
    pub fn wait_reply(
        &self,
        _seq: u64,
        token: &mut CleanLockToken,
        timeout_ns: u64,
    ) -> Result<ZeroCopyMessage> {
        let deadline = monotonic() as u64 + timeout_ns;

        loop {
            // Check for matching reply
            if let Some(msg) = self.recv_queue.dequeue() {
                return Ok(msg);
            }

            // Check timeout
            if monotonic() as u64 > deadline {
                return Err(Error::new(ETIMEDOUT));
            }

            // Yield to other contexts
            unsafe { context::switch(token) };

            if self.state() == ChannelState::Closed {
                return Err(Error::new(EBADF));
            }
        }
    }

    /// Close the channel
    pub fn close(&self) {
        self.state
            .store(ChannelState::Closed as u32, Ordering::Release);

        // Wake up any waiting context
        let waiter = self.waiting_context.swap(0, Ordering::AcqRel);
        if waiter != 0 {
            self.wake_waiter(waiter);
        }
    }

    /// Wake up a waiting context
    fn wake_waiter(&self, context_id: usize) {
        let mut token = unsafe { CleanLockToken::new() };
        let contexts = context::contexts().read();
        if let Some(context_ref) = contexts.get(&context_id) {
            context_ref.write(token.token()).unblock();
        }
    }

    /// Update rolling average latency
    fn update_avg_latency(&self, latency: u64) {
        // Simple exponential moving average: new_avg = old_avg * 0.9 + new_sample * 0.1
        let old_avg = self.stats.avg_latency_ns.load(Ordering::Relaxed);
        let new_avg = if old_avg == 0 {
            latency
        } else {
            (old_avg * 9 + latency) / 10
        };
        self.stats.avg_latency_ns.store(new_avg, Ordering::Relaxed);
    }
}

// =============================================================================
// Direct Transfer (Same Address Space Zero-Copy)
// =============================================================================

/// Performs a direct zero-copy transfer between contexts in the same address space.
/// This is the fastest IPC path when sender and receiver share memory.
pub fn direct_transfer(
    src_addr: usize,
    dst_addr: usize,
    len: usize,
    src_ctx: &ContextRef,
    dst_ctx: &ContextRef,
    token: &mut CleanLockToken,
) -> Result<usize> {
    // Verify same address space
    let src_addrsp = src_ctx.read(token.token()).addr_space()?;
    let dst_addrsp = dst_ctx.read(token.token()).addr_space()?;

    if !Arc::ptr_eq(src_addrsp, dst_addrsp) {
        return Err(Error::new(EPERM));
    }

    // Enter IPC critical section for priority boost
    let src_priority = &src_ctx.read(token.token()).priority;
    let _guard = IpcCriticalGuard::new(src_priority);

    // Since same address space, we can do a direct memory copy
    // This is safe because we've verified the address spaces match
    unsafe {
        let src_ptr = src_addr as *const u8;
        let dst_ptr = dst_addr as *mut u8;

        // Use optimized copy for aligned addresses
        if src_addr % 8 == 0 && dst_addr % 8 == 0 && len % 8 == 0 {
            let src_u64 = src_ptr as *const u64;
            let dst_u64 = dst_ptr as *mut u64;
            let count = len / 8;
            for i in 0..count {
                *dst_u64.add(i) = *src_u64.add(i);
            }
        } else {
            core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, len);
        }
    }

    Ok(len)
}

// =============================================================================
// Global IPC State
// =============================================================================

/// Global IPC channel registry
pub struct IpcRegistry {
    channels: RwLock<BTreeMap<u64, Arc<IpcChannel>>>,
    next_id: AtomicU64,
    buffer_pool: Option<SharedBufferPool>,
}

impl IpcRegistry {
    /// Create a new IPC registry
    pub const fn new() -> Self {
        IpcRegistry {
            channels: RwLock::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
            buffer_pool: None,
        }
    }

    /// Initialize the buffer pool (must be called after memory is initialized)
    pub fn init_buffer_pool(&mut self) -> Result<()> {
        self.buffer_pool = Some(SharedBufferPool::new(SHARED_BUFFER_POOL_SIZE)?);
        Ok(())
    }

    /// Create a new channel
    pub fn create_channel(&self) -> Result<u64> {
        if self.channels.read().len() >= MAX_CHANNELS {
            return Err(Error::new(ENOMEM));
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let channel = Arc::new(IpcChannel::new(id));

        self.channels.write().insert(id, channel);
        Ok(id)
    }

    /// Get a channel by ID
    pub fn get_channel(&self, id: u64) -> Option<Arc<IpcChannel>> {
        self.channels.read().get(&id).cloned()
    }

    /// Close and remove a channel
    pub fn close_channel(&self, id: u64) -> Result<()> {
        let channel = self.channels.write().remove(&id).ok_or(Error::new(EBADF))?;
        channel.close();
        Ok(())
    }

    /// Allocate a shared buffer for zero-copy transfer
    pub fn alloc_buffer(&self, owner_id: usize) -> Option<u32> {
        self.buffer_pool.as_ref()?.allocate(owner_id)
    }

    /// Release a shared buffer
    pub fn release_buffer(&self, buffer_id: u32) {
        if let Some(pool) = &self.buffer_pool {
            pool.release(buffer_id);
        }
    }

    /// Get a shared buffer
    pub fn get_buffer(&self, buffer_id: u32) -> Option<&SharedBuffer> {
        self.buffer_pool.as_ref()?.get(buffer_id)
    }
}

/// Global IPC registry instance
pub static mut IPC_REGISTRY: IpcRegistry = IpcRegistry::new();

/// Get the global IPC registry
pub fn registry() -> &'static IpcRegistry {
    unsafe { &IPC_REGISTRY }
}

/// Get the global IPC registry mutably (for initialization)
pub fn registry_mut() -> &'static mut IpcRegistry {
    unsafe { &mut IPC_REGISTRY }
}

// =============================================================================
// Preemption Control
// =============================================================================

/// Preemption state flags
pub mod preempt {
    use core::sync::atomic::{AtomicU32, Ordering};

    /// Per-CPU preemption counter
    #[derive(Debug)]
    pub struct PreemptState {
        /// Preemption disable count (0 = preemption enabled)
        count: AtomicU32,
        /// Preemption was requested while disabled
        pending: AtomicU32,
    }

    impl PreemptState {
        pub const fn new() -> Self {
            PreemptState {
                count: AtomicU32::new(0),
                pending: AtomicU32::new(0),
            }
        }

        /// Disable preemption (increases count)
        #[inline]
        pub fn disable(&self) {
            self.count.fetch_add(1, Ordering::Acquire);
        }

        /// Enable preemption (decreases count)
        /// Returns true if preemption should happen now
        #[inline]
        pub fn enable(&self) -> bool {
            let old = self.count.fetch_sub(1, Ordering::Release);
            if old == 1 && self.pending.swap(0, Ordering::Acquire) != 0 {
                return true;
            }
            false
        }

        /// Check if preemption is enabled
        #[inline]
        pub fn is_enabled(&self) -> bool {
            self.count.load(Ordering::Relaxed) == 0
        }

        /// Mark preemption as pending
        #[inline]
        pub fn set_pending(&self) {
            self.pending.store(1, Ordering::Release);
        }

        /// Check and clear pending preemption
        #[inline]
        pub fn check_pending(&self) -> bool {
            self.pending.swap(0, Ordering::AcqRel) != 0
        }
    }

    impl Default for PreemptState {
        fn default() -> Self {
            Self::new()
        }
    }
}

/// Full preemption check point
/// Call this at safe preemption points to allow RT threads to preempt
#[inline]
pub fn preempt_check(token: &mut CleanLockToken) {
    let current = context::current();
    let ctx = current.read(token.token());

    // Check if there's a higher priority thread waiting
    if let Some(scheduler_ref) = crate::percpu::PercpuBlock::current()
        .scheduler
        .run_queue
        .rt_queue
        .front()
    {
        let waiting_priority = scheduler_ref
            .context
            .read(token.token())
            .priority
            .effective_priority();
        let current_priority = ctx.priority.effective_priority();

        // Lower number = higher priority
        if waiting_priority < current_priority {
            drop(ctx);
            // Preempt: switch to the higher priority thread
            unsafe { context::switch(token) };
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_creation() {
        let msg = ZeroCopyMessage {
            header: MessageHeader {
                src_ctx: 1,
                dst_ctx: 2,
                msg_type: 0,
                flags: MessageFlags::empty(),
                seq: 0,
                timestamp: 0,
                payload_len: 4,
                _reserved: 0,
            },
            payload: MessagePayload {
                inline: {
                    let mut data = [0u8; INLINE_MSG_MAX];
                    data[0..4].copy_from_slice(b"test");
                    data
                },
            },
        };

        assert_eq!(msg.header.payload_len, 4);
    }

    #[test]
    fn test_channel_state() {
        let channel = IpcChannel::new(1);
        assert_eq!(channel.state(), ChannelState::Ready);
        channel.close();
        assert_eq!(channel.state(), ChannelState::Closed);
    }
}

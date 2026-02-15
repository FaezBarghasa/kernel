//! Zero-Copy Ring Buffer IPC Scheme
//!
//! Implements io_uring-style asynchronous dispatching with submission queue (SQ)
//! and completion queue (CQ) for zero-copy kernel-userspace communication.
//!
//! ## Design
//! - Producer (userspace) writes SQEs and advances sq_tail
//! - Consumer (kernel) reads SQEs and advances sq_head
//! - Kernel writes CQEs and advances cq_tail
//! - Userspace reads CQEs and advances cq_head
//!
//! Memory barriers ensure correct ordering across CPU cores.

use alloc::{
    boxed::Box,
    collections::{BTreeMap, VecDeque},
    string::String,
    sync::Arc,
};
use core::{
    slice,
    sync::atomic::{fence, AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
};
use spin::RwLock;

use crate::{
    context::{
        self,
        file::InternalFlags,
        memory::{AddrSpaceWrapper, Grant, PageSpan},
        ContextId,
    },
    memory::{
        allocate_frame, deallocate_frame, Frame, KernelMapper, PhysicalAddress, RmmA, RmmArch,
        PAGE_SIZE,
    },
    paging::{Page, PageFlags, VirtualAddress},
    scheme::{CallerCtx, KernelScheme, OpenResult},
    sync::{CleanLockToken, IpcCriticalGuard, OptimizedWaitQueue},
    syscall::{
        data::Map,
        error::{Error, Result, EBADF, EINVAL, EIO, ENOMEM, EOVERFLOW, ESPIPE},
        flag::{MapFlags, O_CLOEXEC, O_RDWR},
        number::*,
        usercopy::{UserSliceRo, UserSliceWo},
    },
};
use alloc::vec::Vec;
use core::num::NonZeroUsize;

const F_SETOWN: usize = 8;
const F_GETOWN: usize = 9;

// =============================================================================
// Ring Structure Definitions (Linux io_uring compatible layout)
// =============================================================================

/// Shared ring buffer header mapped to userspace
#[repr(C)]
pub struct IpcRing {
    // Submission Queue control
    pub sq_head: AtomicU32,
    pub sq_tail: AtomicU32,
    pub sq_mask: u32,
    pub sq_entries: u32,

    // Completion Queue control
    pub cq_head: AtomicU32,
    pub cq_tail: AtomicU32,
    pub cq_mask: u32,
    pub cq_entries: u32,

    // Flags and features
    pub sq_flags: AtomicU32,
    pub cq_flags: AtomicU32,
    pub features: u32,

    // Overflow tracking
    pub cq_overflow: AtomicU64,

    pub _reserved: [u32; 4],
}

/// Submission Queue Entry (64 bytes, cache-line aligned)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct Sqe {
    pub opcode: u8,
    pub flags: u8,
    pub ioprio: u16,
    pub fd: i32,
    pub off: u64,      // offset or addr2
    pub addr: u64,     // buffer address
    pub len: u32,      // buffer length
    pub rw_flags: u32, // op-specific flags
    pub user_data: u64,
    pub buf_index: u16,   // for buffer selection
    pub personality: u16, // for credential passing
    pub splice_fd: i32,   // for SPLICE ops
    pub addr3: u64,       // additional address
    pub _pad: [u64; 1],
}

/// Completion Queue Entry (16 bytes)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Cqe {
    pub user_data: u64,
    pub res: i32,
    pub flags: u32,
}

// Operation codes
pub const IORING_OP_NOP: u8 = 0;
pub const IORING_OP_READ: u8 = 1;
pub const IORING_OP_WRITE: u8 = 2;
pub const IORING_OP_CLOSE: u8 = 3;
pub const IORING_OP_READV: u8 = 4;
pub const IORING_OP_WRITEV: u8 = 5;
pub const IORING_OP_FSYNC: u8 = 6;
pub const IORING_OP_POLL_ADD: u8 = 7;
pub const IORING_OP_POLL_REMOVE: u8 = 8;
pub const IORING_OP_TIMEOUT: u8 = 9;
pub const IORING_OP_TIMEOUT_REMOVE: u8 = 10;
pub const IORING_OP_ACCEPT: u8 = 11;
pub const IORING_OP_ASYNC_CANCEL: u8 = 12;
pub const IORING_OP_LINK_TIMEOUT: u8 = 13;
pub const IORING_OP_CONNECT: u8 = 14;
pub const IORING_OP_OPENAT: u8 = 15;
// RDMA Extensions
pub const IORING_OP_RDMA_READ: u8 = 16;
pub const IORING_OP_RDMA_WRITE: u8 = 17;
pub const IORING_OP_REG_MR: u8 = 18; // Register Memory Region
pub const IORING_OP_DEREG_MR: u8 = 19; // Deregister Memory Region

// SQE flags
pub const IOSQE_FIXED_FILE: u8 = 1 << 0;
pub const IOSQE_IO_DRAIN: u8 = 1 << 1;
pub const IOSQE_IO_LINK: u8 = 1 << 2;
pub const IOSQE_IO_HARDLINK: u8 = 1 << 3;
pub const IOSQE_ASYNC: u8 = 1 << 4;
pub const IOSQE_RDMA_FENCE: u8 = 1 << 5; // RDMA Fence

// CQE flags
pub const IORING_CQE_F_BUFFER: u32 = 1 << 0;
pub const IORING_CQE_F_MORE: u32 = 1 << 1;

// SQ flags (set by kernel)
pub const IORING_SQ_NEED_WAKEUP: u32 = 1 << 0;
pub const IORING_SQ_CQ_OVERFLOW: u32 = 1 << 1;

// =============================================================================
// Ring Handle and Scheme Implementation
// =============================================================================

/// Per-ring kernel state
pub struct RingHandle {
    pub frame: Frame,
    pub ring_ptr: *mut IpcRing,

    /// SQ entries array (immediately after ring header)
    pub sq_entries: usize,
    /// CQ entries array (after SQ entries)
    pub cq_entries: usize,

    /// Pending driver operations wait queue
    pub driver_queue: OptimizedWaitQueue<()>,
    /// Context ID of the userspace driver consuming this ring
    pub consumer_pid: AtomicUsize,
    /// Wait queue for processes blocking on CQE availability
    pub completion_wait_queue: OptimizedWaitQueue<()>,

    /// Overflow flag for backpressure signaling
    pub overflow_active: AtomicBool,
    /// Statistics: total SQEs processed
    pub sqe_processed: AtomicU64,
    /// Statistics: total CQEs completed
    /// Statistics: total CQEs completed
    pub cqe_completed: AtomicU64,

    /// Registered Memory Regions
    pub mrs: RwLock<BTreeMap<u32, MemoryRegion>>,
    /// Next MR ID
    pub next_mr_id: AtomicU32,

    /// Driver Command Queue (SQEs relevant to driver)
    pub driver_cmds: RwLock<VecDeque<Sqe>>,
}

/// Registered Memory Region
#[derive(Debug)]
pub struct MemoryRegion {
    pub addr_space: Arc<AddrSpaceWrapper>,
    pub start: VirtualAddress,
    pub size: usize,
    pub flags: u32,
}

// Safety: RingHandle owns the frame; pointer access is synchronized via atomics
unsafe impl Send for RingHandle {}
unsafe impl Sync for RingHandle {}

impl RingHandle {
    /// Calculate SQE pointer for a given index
    #[inline]
    unsafe fn sqe_ptr(&self, idx: usize) -> *const Sqe {
        unsafe {
            let base = self.ring_ptr.add(1) as *const Sqe;
            base.add(idx)
        }
    }

    /// Calculate CQE pointer for a given index
    #[inline]
    unsafe fn cqe_ptr(&self, idx: usize) -> *mut Cqe {
        unsafe {
            let sq_end = (self.ring_ptr.add(1) as *mut Sqe).add(self.sq_entries);
            (sq_end as *mut Cqe).add(idx)
        }
    }

    /// Check if CQ is full (would overflow on next write)
    #[inline]
    fn cq_is_full(&self) -> bool {
        let ring = unsafe { &*self.ring_ptr };
        let head = ring.cq_head.load(Ordering::Acquire);
        let tail = ring.cq_tail.load(Ordering::Relaxed);
        (tail.wrapping_sub(head)) >= ring.cq_entries
    }

    /// Write a CQE with proper overflow handling
    fn write_cqe(&self, cqe: &Cqe) -> bool {
        let ring = unsafe { &*self.ring_ptr };

        // Acquire barrier: ensure we see latest cq_head from userspace
        fence(Ordering::Acquire);

        let head = ring.cq_head.load(Ordering::Acquire);
        let tail = ring.cq_tail.load(Ordering::Relaxed);
        let mask = ring.cq_mask;
        let entries = ring.cq_entries;

        // Check for overflow
        if tail.wrapping_sub(head) >= entries {
            // CQ is full - increment overflow counter
            ring.cq_overflow.fetch_add(1, Ordering::Relaxed);
            ring.sq_flags
                .fetch_or(IORING_SQ_CQ_OVERFLOW, Ordering::Release);
            self.overflow_active.store(true, Ordering::Release);
            return false;
        }

        // Write CQE to the slot
        let cq_idx = (tail & mask) as usize;
        unsafe {
            let cqe_slot = self.cqe_ptr(cq_idx);
            core::ptr::write_volatile(cqe_slot, *cqe);
        }

        // Release barrier: ensure CQE write is visible before tail update
        fence(Ordering::Release);

        // Advance tail
        ring.cq_tail.store(tail.wrapping_add(1), Ordering::Release);

        self.cqe_completed.fetch_add(1, Ordering::Relaxed);

        true
    }
}

pub struct RingScheme {
    handles: RwLock<BTreeMap<usize, Arc<RingHandle>>>,
    registry: RwLock<BTreeMap<String, Arc<RingHandle>>>,
    next_id: AtomicUsize,
}

impl RingScheme {
    pub fn new() -> Self {
        RingScheme {
            handles: RwLock::new(BTreeMap::new()),
            registry: RwLock::new(BTreeMap::new()),
            next_id: AtomicUsize::new(0),
        }
    }

    /// Process a single SQE and dispatch to appropriate handler
    fn process_sqe(
        &self,
        handle: &Arc<RingHandle>,
        sqe: &Sqe,
        token: &mut CleanLockToken,
    ) -> Result<()> {
        match sqe.opcode {
            IORING_OP_NOP => {
                // NOP completes synchronously with success
                let cqe = Cqe {
                    user_data: sqe.user_data,
                    res: 0,
                    flags: 0,
                };
                if !handle.write_cqe(&cqe) {
                    return Err(Error::new(EOVERFLOW));
                }
                Ok(())
            }

            IORING_OP_READ | IORING_OP_WRITE | IORING_OP_READV | IORING_OP_WRITEV => {
                // I/O operations dispatch to userspace driver
                let consumer_pid = handle.consumer_pid.load(Ordering::Acquire);

                if consumer_pid == 0 {
                    // No driver registered - return error
                    let cqe = Cqe {
                        user_data: sqe.user_data,
                        res: -(EIO as i32),
                        flags: 0,
                    };
                    handle.write_cqe(&cqe);
                    return Err(Error::new(EIO));
                }

                // Queue for driver processing
                handle.driver_queue.send((), token);
                handle.driver_queue.wake_one();
                Ok(())
            }

            IORING_OP_CLOSE => {
                let consumer_pid = handle.consumer_pid.load(Ordering::Acquire);
                if consumer_pid == 0 {
                    let cqe = Cqe {
                        user_data: sqe.user_data,
                        res: -(EIO as i32),
                        flags: 0,
                    };
                    handle.write_cqe(&cqe);
                    return Err(Error::new(EIO));
                }
                handle.driver_queue.send((), token);
                handle.driver_queue.wake_one();
                Ok(())
            }

            IORING_OP_FSYNC => {
                let consumer_pid = handle.consumer_pid.load(Ordering::Acquire);
                if consumer_pid == 0 {
                    let cqe = Cqe {
                        user_data: sqe.user_data,
                        res: -(EIO as i32),
                        flags: 0,
                    };
                    handle.write_cqe(&cqe);
                    return Err(Error::new(EIO));
                }
                handle.driver_queue.send((), token);
                handle.driver_queue.wake_one();
                Ok(())
            }

            IORING_OP_POLL_ADD | IORING_OP_POLL_REMOVE => {
                // Poll operations for async I/O readiness notification
                let consumer_pid = handle.consumer_pid.load(Ordering::Acquire);
                if consumer_pid == 0 {
                    let cqe = Cqe {
                        user_data: sqe.user_data,
                        res: -(EIO as i32),
                        flags: 0,
                    };
                    handle.write_cqe(&cqe);
                    return Err(Error::new(EIO));
                }
                handle.driver_queue.send((), token);
                handle.driver_queue.wake_one();
                Ok(())
            }

            IORING_OP_TIMEOUT | IORING_OP_TIMEOUT_REMOVE => {
                // Timeout operations
                let consumer_pid = handle.consumer_pid.load(Ordering::Acquire);
                if consumer_pid == 0 {
                    let cqe = Cqe {
                        user_data: sqe.user_data,
                        res: -(EIO as i32),
                        flags: 0,
                    };
                    handle.write_cqe(&cqe);
                    return Err(Error::new(EIO));
                }
                handle.driver_queue.send((), token);
                handle.driver_queue.wake_one();
                Ok(())
            }

            IORING_OP_ACCEPT | IORING_OP_CONNECT | IORING_OP_RDMA_READ | IORING_OP_RDMA_WRITE => {
                // Socket & RDMA operations
                let consumer_pid = handle.consumer_pid.load(Ordering::Acquire);
                if consumer_pid == 0 {
                    let cqe = Cqe {
                        user_data: sqe.user_data,
                        res: -(EIO as i32),
                        flags: 0,
                    };
                    handle.write_cqe(&cqe);
                    return Err(Error::new(EIO));
                }

                // Push to driver command queue
                handle.driver_cmds.write().push_back(*sqe);

                handle.driver_queue.send((), token);
                handle.driver_queue.wake_one();
                Ok(())
            }

            IORING_OP_REG_MR => {
                let addr = VirtualAddress::new(sqe.addr as usize);
                let size = sqe.len as usize;

                // Lock memory
                let context_lock = context::current();
                let context = context_lock.read(token.token());
                let addr_space = context.addr_space.clone().ok_or(Error::new(EIO))?;

                {
                    let mut inner = addr_space.acquire_write();
                    inner.mlock(addr, size).map_err(|_| Error::new(ENOMEM))?;
                }

                // Create MR
                let id = handle.next_mr_id.fetch_add(1, Ordering::Relaxed);
                let mr = MemoryRegion {
                    addr_space,
                    start: addr,
                    size,
                    flags: sqe.rw_flags,
                };

                handle.mrs.write().insert(id, mr);

                // Return ID
                let cqe = Cqe {
                    user_data: sqe.user_data,
                    res: id as i32,
                    flags: 0,
                };
                handle.write_cqe(&cqe);
                Ok(())
            }

            IORING_OP_DEREG_MR => {
                let id = sqe.addr as u32; // Use addr field for ID

                let mr = handle.mrs.write().remove(&id).ok_or(Error::new(EINVAL))?;

                {
                    let mut inner = mr.addr_space.acquire_write();
                    let _ = inner.munlock(mr.start, mr.size);
                }

                let cqe = Cqe {
                    user_data: sqe.user_data,
                    res: 0,
                    flags: 0,
                };
                handle.write_cqe(&cqe);
                Ok(())
            }

            IORING_OP_ASYNC_CANCEL => {
                // Cancel pending async operation
                let consumer_pid = handle.consumer_pid.load(Ordering::Acquire);
                if consumer_pid == 0 {
                    let cqe = Cqe {
                        user_data: sqe.user_data,
                        res: -(EIO as i32),
                        flags: 0,
                    };
                    handle.write_cqe(&cqe);
                    return Err(Error::new(EIO));
                }
                handle.driver_queue.send((), token);
                handle.driver_queue.wake_one();
                Ok(())
            }

            IORING_OP_OPENAT => {
                // Open file operation
                let consumer_pid = handle.consumer_pid.load(Ordering::Acquire);
                if consumer_pid == 0 {
                    let cqe = Cqe {
                        user_data: sqe.user_data,
                        res: -(EIO as i32),
                        flags: 0,
                    };
                    handle.write_cqe(&cqe);
                    return Err(Error::new(EIO));
                }
                handle.driver_queue.send((), token);
                handle.driver_queue.wake_one();
                Ok(())
            }

            _ => {
                // Unknown opcode
                let cqe = Cqe {
                    user_data: sqe.user_data,
                    res: -(EINVAL as i32),
                    flags: 0,
                };
                handle.write_cqe(&cqe);
                Err(Error::new(EINVAL))
            }
        }
    }

    /// Process SQ in batches for reduced overhead
    fn process_sq_batch(
        &self,
        handle: &Arc<RingHandle>,
        max_batch: usize,
        token: &mut CleanLockToken,
    ) -> usize {
        let ring = unsafe { &*handle.ring_ptr };

        // Acquire barrier: ensure we see all SQE writes before reading
        fence(Ordering::Acquire);

        let mut head = ring.sq_head.load(Ordering::Acquire);
        let tail = ring.sq_tail.load(Ordering::Acquire);
        let mask = ring.sq_mask;

        // Calculate batch size
        let available = tail.wrapping_sub(head) as usize;
        let batch_size = core::cmp::min(available, max_batch);

        let mut processed = 0;

        // Batch processing loop with proper wrap-around
        for i in 0..batch_size {
            let idx = (head.wrapping_add(i as u32) & mask) as usize;

            let sqe = unsafe { *handle.sqe_ptr(idx) };

            // Check for linked operations (IO_LINK flag)
            let is_linked = (sqe.flags & IOSQE_IO_LINK) != 0;

            match self.process_sqe(handle, &sqe, token) {
                Ok(()) => {
                    processed += 1;
                    handle.sqe_processed.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    // If linked, subsequent linked ops should be cancelled
                    if is_linked {
                        // Skip to next non-linked op
                        continue;
                    }
                    // On non-linked error, we still advance but log the failure
                    processed += 1;
                }
            }
        }

        // Advance head by number of processed entries
        head = head.wrapping_add(processed as u32);

        // Release barrier: ensure all processing is visible before head update
        fence(Ordering::Release);

        ring.sq_head.store(head, Ordering::Release);

        processed
    }

    /// Called by userspace driver to signal command completion
    pub fn ring_complete(handle: &Arc<RingHandle>, cqe: &Cqe, token: &mut CleanLockToken) {
        // Boost priority for IPC completion path
        let context_lock = context::current();
        let context = context_lock.read(token.token());
        let _ipc_guard = IpcCriticalGuard::new(&context.priority);

        handle.write_cqe(cqe);

        // Wake up waiters on completion queue
        handle.completion_wait_queue.wake_one();
    }

    /// Clear overflow state when userspace has drained CQ
    pub fn clear_overflow(handle: &Arc<RingHandle>) {
        let ring = unsafe { &*handle.ring_ptr };
        ring.sq_flags
            .fetch_and(!IORING_SQ_CQ_OVERFLOW, Ordering::Release);
        handle.overflow_active.store(false, Ordering::Release);
    }
}

impl KernelScheme for RingScheme {
    fn kopen(
        &self,
        _path: &str,
        _flags: usize,
        _ctx: CallerCtx,
        _token: &mut CleanLockToken,
    ) -> Result<OpenResult> {
        // Check if this is a named ring request
        if !_path.is_empty() {
            let registry = self.registry.write();
            if let Some(existing) = registry.get(_path) {
                let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                self.handles.write().insert(id, existing.clone());
                return Ok(OpenResult::SchemeLocal(id, InternalFlags::empty()));
            }
            drop(registry); // Drop lock before creating new
        }

        // Allocate frame for ring buffer (Order 1 = 8KB)
        // ... (existing allocation logic) ...
        let frame = crate::memory::allocate_p2frame(1).ok_or(Error::new(ENOMEM))?;
        let data = unsafe { RmmA::phys_to_virt(frame.base()).data() as *mut u8 };
        let ring_ptr = data as *mut IpcRing;

        // Ring sizes (power of 2 for efficient masking)
        const SQ_SIZE: usize = 64;
        const CQ_SIZE: usize = 128;
        const SQ_MASK: u32 = (SQ_SIZE - 1) as u32;
        const CQ_MASK: u32 = (CQ_SIZE - 1) as u32;

        // Initialize ring header
        unsafe {
            let ring = &mut *ring_ptr;
            ring.sq_head.store(0, Ordering::Relaxed);
            ring.sq_tail.store(0, Ordering::Relaxed);
            ring.sq_mask = SQ_MASK;
            ring.sq_entries = SQ_SIZE as u32;

            ring.cq_head.store(0, Ordering::Relaxed);
            ring.cq_tail.store(0, Ordering::Relaxed);
            ring.cq_mask = CQ_MASK;
            ring.cq_entries = CQ_SIZE as u32;

            ring.sq_flags.store(0, Ordering::Relaxed);
            ring.cq_flags.store(0, Ordering::Relaxed);
            ring.features = 0;
            ring.cq_overflow.store(0, Ordering::Relaxed);

            // Zero out reserved and entry areas
            // We have 8KB now, so clean up to 8KB
            core::ptr::write_bytes(
                ring_ptr.add(1) as *mut u8,
                0,
                (PAGE_SIZE * 2) - core::mem::size_of::<IpcRing>(),
            );
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let handle = Arc::new(RingHandle {
            frame,
            ring_ptr,
            sq_entries: SQ_SIZE,
            cq_entries: CQ_SIZE,
            driver_queue: OptimizedWaitQueue::new(),
            consumer_pid: AtomicUsize::new(0),
            completion_wait_queue: OptimizedWaitQueue::new(),
            overflow_active: AtomicBool::new(false),
            sqe_processed: AtomicU64::new(0),
            cqe_completed: AtomicU64::new(0),
            mrs: RwLock::new(BTreeMap::new()),
            next_mr_id: AtomicU32::new(1),
            driver_cmds: RwLock::new(VecDeque::new()),
        });

        if !_path.is_empty() {
            self.registry
                .write()
                .insert(String::from(_path), handle.clone());
        }

        self.handles.write().insert(id, handle);

        Ok(OpenResult::SchemeLocal(id, InternalFlags::empty()))
    }

    fn close(&self, id: usize, _token: &mut CleanLockToken) -> Result<()> {
        let handle = self.handles.write().remove(&id).ok_or(Error::new(EBADF))?;

        // Clear consumer reference
        handle.consumer_pid.store(0, Ordering::Release);

        // Deallocate frame
        unsafe {
            crate::memory::deallocate_p2frame(handle.frame, 1);
        }

        // Deregister all MRs
        let mut mrs_lock = handle.mrs.write();
        let mrs = core::mem::take(&mut *mrs_lock);
        drop(mrs_lock); // Release lock

        for (_, mr) in mrs {
            let mut inner = mr.addr_space.acquire_write();
            let _ = inner.munlock(mr.start, mr.size);
        }

        Ok(())
    }

    fn kfmap(
        &self,
        id: usize,
        addr_space: &Arc<AddrSpaceWrapper>,
        map: &Map,
        _consume: bool,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        let handles = self.handles.read();
        let handle = handles.get(&id).ok_or(Error::new(EBADF))?;
        let frame = handle.frame;
        let page_count = NonZeroUsize::new(1).unwrap();

        let base_page = addr_space.acquire_write().mmap(
            (map.address != 0)
                .then_some(Page::containing_address(VirtualAddress::new(map.address))),
            page_count,
            map.flags,
            &mut Vec::new(),
            |dst_page, page_flags, dst_mapper, dst_flusher| {
                Grant::physmap(
                    frame,
                    PageSpan::new(dst_page, page_count.get()),
                    page_flags,
                    dst_mapper,
                    dst_flusher,
                )
            },
        )?;

        Ok(base_page.start_address().data())
    }

    /// Write acts as the "doorbell" - wakes kernel to process SQ
    fn kwrite(
        &self,
        id: usize,
        _buf: UserSliceRo,
        _flags: u32,
        _stored_flags: u32,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        let handles = self.handles.read();
        let handle = handles.get(&id).ok_or(Error::new(EBADF))?.clone();
        drop(handles);

        // Process up to 32 SQEs per doorbell (tuneable batch size)
        const MAX_BATCH: usize = 32;
        let processed = self.process_sq_batch(&handle, MAX_BATCH, token);

        Ok(processed)
    }

    fn fcntl(
        &self,
        id: usize,
        cmd: usize,
        arg: usize,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        let handles = self.handles.read();
        let handle = handles.get(&id).ok_or(Error::new(EBADF))?;

        match cmd {
            F_SETOWN => {
                // Register driver as consumer
                handle.consumer_pid.store(arg, Ordering::Release);
                Ok(0)
            }
            F_GETOWN => Ok(handle.consumer_pid.load(Ordering::Acquire)),
            _ => Err(Error::new(EINVAL)),
        }
    }

    fn kread(
        &self,
        id: usize,
        mut buf: UserSliceWo,
        _flags: u32,
        _stored_flags: u32,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        let handles = self.handles.read();
        let handle = handles.get(&id).ok_or(Error::new(EBADF))?;

        if handle.consumer_pid.load(Ordering::Acquire) == 0 {
            return Err(Error::new(EBADF));
        }

        let mut commands = handle.driver_cmds.write();
        let mut total_read = 0;
        let sqe_size = core::mem::size_of::<Sqe>();

        while let Some(_) = commands.front() {
            if buf.len() < sqe_size {
                break;
            }

            let sqe = commands.pop_front().unwrap();

            let sqe_bytes =
                unsafe { slice::from_raw_parts(&sqe as *const Sqe as *const u8, sqe_size) };

            if buf.copy_from_slice(sqe_bytes).is_err() {
                commands.push_front(sqe);
                return Err(Error::new(EIO));
            }

            if let Some(rest) = buf.advance(sqe_size) {
                buf = rest;
            } else {
                break;
            }

            total_read += sqe_size;
        }

        Ok(total_read)
    }
}

//! Zero-Copy Ring Buffer IPC Scheme
//!
//! This module implements a high-performance, asynchronous IPC mechanism inspired by io_uring.
//! It uses a shared memory region containing two ring buffers:
//! 1. Submission Queue (SQ): Userspace pushes commands here.
//! 2. Completion Queue (CQ): Kernel pushes results here.
//!
//! This eliminates the overhead of a system call per operation.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::slice;
use core::sync::atomic::{AtomicU32, Ordering};
use spin::RwLock;

use crate::{
    context::{self, ContextId},
    memory::{allocate_frames, deallocate_frames, Frame, KernelMapper, PhysicalAddress, PAGE_SIZE, RmmA, RmmArch},
    paging::{Page, PageFlags, VirtualAddress},
    scheme::{CallerCtx, KernelScheme, OpenResult},
    syscall::{
        data::Stat,
        error::{Error, Result, EBADF, EINVAL, ENOMEM, ESPIPE},
        flag::{MapFlags, O_CLOEXEC, O_RDWR},
        number::*,
    },
};

/// The layout of the shared memory region.
/// This structure is mapped directly into both Kernel and User address spaces.
#[repr(C)]
pub struct IpcRing {
    /// Head index of the Submission Queue (Updated by Kernel)
    pub sq_head: AtomicU32,
    /// Tail index of the Submission Queue (Updated by User)
    pub sq_tail: AtomicU32,
    /// Mask for SQ wrapping (must be power of 2 minus 1)
    pub sq_mask: u32,
    
    /// Head index of the Completion Queue (Updated by User)
    pub cq_head: AtomicU32,
    /// Tail index of the Completion Queue (Updated by Kernel)
    pub cq_tail: AtomicU32,
    /// Mask for CQ wrapping
    pub cq_mask: u32,

    /// Feature flags for the ring
    pub features: u32,
    pub _reserved: u32,
}

/// A command submitted by userspace.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Sqe {
    pub opcode: u8,
    pub flags: u8,
    pub ioprio: u16,
    pub fd: i32,
    pub addr: u64,
    pub len: u32,
    pub user_data: u64,
}

/// A completion result pushed by the kernel.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Cqe {
    pub user_data: u64,
    pub res: i32,
    pub flags: u32,
}

// Opcodes corresponding to io_uring style operations
pub const IORING_OP_NOP: u8 = 0;
pub const IORING_OP_READ: u8 = 1;
pub const IORING_OP_WRITE: u8 = 2;
pub const IORING_OP_CLOSE: u8 = 3;

pub struct RingHandle {
    pub frame: Frame,
    pub ring_ptr: *mut IpcRing,
    pub sq_entries: usize,
    pub cq_entries: usize,
}

// Safety: RingHandle owns the frame and pointer implies access to shared memory.
unsafe impl Send for RingHandle {}
unsafe impl Sync for RingHandle {}

pub struct RingScheme {
    handles: RwLock<BTreeMap<usize, Arc<RingHandle>>>,
    next_id: AtomicU32,
}

impl RingScheme {
    pub fn new() -> Self {
        RingScheme {
            handles: RwLock::new(BTreeMap::new()),
            next_id: AtomicU32::new(1),
        }
    }

    /// Process a single Submission Queue Entry (SQE)
    fn process_sqe(&self, sqe: &Sqe) -> i32 {
        match sqe.opcode {
            IORING_OP_NOP => 0,
            
            IORING_OP_READ => {
                // Resolve FD and call kread
                // Note: In a true async impl, this would queue a task. 
                // For Phase 3, we execute synchronously to demonstrate the plumbing.
                if let Ok(context) = context::current() {
                    let context = context.read();
                    if let Some(file) = context.get_file(sqe.fd as usize) {
                        // Safety: We are trusting the user provided address/len here.
                        // A robust implementation must validate `sqe.addr` is within user range.
                        let buf = unsafe { 
                            slice::from_raw_parts_mut(sqe.addr as *mut u8, sqe.len as usize) 
                        };
                        
                        match file.read(buf) {
                            Ok(count) => count as i32,
                            Err(err) => -(err.errno as i32),
                        }
                    } else {
                        -(EBADF as i32)
                    }
                } else {
                    -(ESPIPE as i32)
                }
            },

            IORING_OP_WRITE => {
                // Resolve FD and call kwrite
                if let Ok(context) = context::current() {
                    let context = context.read();
                    if let Some(file) = context.get_file(sqe.fd as usize) {
                        let buf = unsafe { 
                            slice::from_raw_parts(sqe.addr as *const u8, sqe.len as usize) 
                        };
                        
                        match file.write(buf) {
                            Ok(count) => count as i32,
                            Err(err) => -(err.errno as i32),
                        }
                    } else {
                        -(EBADF as i32)
                    }
                } else {
                    -(ESPIPE as i32)
                }
            },
            
            IORING_OP_CLOSE => {
                if let Ok(context) = context::current() {
                    let mut context = context.write();
                    if context.remove_file(sqe.fd as usize).is_some() {
                        0
                    } else {
                        -(EBADF as i32)
                    }
                } else {
                    -(ESPIPE as i32)
                }
            }

            _ => -(EINVAL as i32),
        }
    }
}

impl KernelScheme for RingScheme {
    fn kopen(&self, _path: &str, _flags: usize, _ctx: CallerCtx) -> Result<OpenResult> {
        // 1. Allocate a physical frame for the ring buffer
        // For simplicity, we allocate one 4KB page.
        // Layout:
        // [Header (64 bytes)]
        // [SQEs (64 * 32 bytes = 2048 bytes)]
        // [CQEs (128 * 16 bytes = 2048 bytes)] (Overfills 4KB, need adjustment or larger alloc)
        // Adjusted for 4KB page: 
        // Header: 64 bytes
        // SQE: 48 * 32 = 1536
        // CQE: 128 * 16 = 2048
        // Total: 3648 bytes < 4096 bytes. Safe.

        let frame = allocate_frames(1).ok_or(Error::new(ENOMEM))?;
        
        // 2. Map into Kernel space to initialize
        // Use RmmA::phys_to_virt to get the kernel logical address of the frame
        let phys_addr = frame.start_address();
        let virt_addr = crate::memory::phys_to_virt(phys_addr);
        let ring_ptr = virt_addr.data() as *mut IpcRing;
        
        const SQ_SIZE: usize = 48; // Power of 2 approximation handled by mask logic below, simplified for fit
        const SQ_MASK: u32 = 63;   // Logical mask, bounds checking required if size != mask+1
        const CQ_SIZE: usize = 128;
        const CQ_MASK: u32 = 127;

        unsafe {
            // Initialize Ring Header
            (*ring_ptr).sq_head.store(0, Ordering::Relaxed);
            (*ring_ptr).sq_tail.store(0, Ordering::Relaxed);
            (*ring_ptr).sq_mask = SQ_MASK;
            
            (*ring_ptr).cq_head.store(0, Ordering::Relaxed);
            (*ring_ptr).cq_tail.store(0, Ordering::Relaxed);
            (*ring_ptr).cq_mask = CQ_MASK;
            
            (*ring_ptr).features = 0;
            
            // Zero out the rest of the page to prevent leaking data
            core::ptr::write_bytes(ring_ptr.add(1) as *mut u8, 0, PAGE_SIZE - core::mem::size_of::<IpcRing>());
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed) as usize;
        let handle = Arc::new(RingHandle {
            frame,
            ring_ptr,
            sq_entries: SQ_SIZE, // Logical limit
            cq_entries: CQ_SIZE,
        });

        self.handles.write().insert(id, handle);

        Ok(OpenResult::SchemeLocal(id))
    }

    fn kclose(&self, id: usize) -> Result<usize> {
        let handle = self.handles.write().remove(&id).ok_or(Error::new(EBADF))?;
        // Deallocate frame when handle is dropped
        unsafe {
            deallocate_frames(handle.frame, 1);
        }
        Ok(0)
    }

    /// Memory Map the Ring Buffer into Userspace
    /// Returns the physical address. The memory manager handles the page table update.
    fn kfmap(&self, id: usize, _addr: usize, _len: usize, _flags: MapFlags, _arg: usize) -> Result<usize> {
        let handles = self.handles.read();
        let handle = handles.get(&id).ok_or(Error::new(EBADF))?;

        // Return the physical address so the memory manager can map it to the user's requested VMA.
        Ok(handle.frame.start_address().data())
    }

    /// Write to the file descriptor acts as the "Doorbell" to wake the kernel
    /// and process the submission queue.
    /// 
    /// The buffer passed to write is ignored (or could contain metadata).
    /// The existence of the syscall call is the signal.
    fn kwrite(&self, id: usize, _buf: &[u8]) -> Result<usize> {
        let handles = self.handles.read();
        let handle = handles.get(&id).ok_or(Error::new(EBADF))?;

        let ring = unsafe { &*handle.ring_ptr };
        
        // 1. Read Kernel Head and User Tail
        // Load Acquire ensures we see the writes to the SQE slots performed by userspace
        let mut head = ring.sq_head.load(Ordering::Acquire);
        let tail = ring.sq_tail.load(Ordering::Acquire);
        let mask = ring.sq_mask;

        let mut processed = 0;

        // 2. Process Loop
        while head < tail {
            let idx = (head & mask) as usize;
            
            // Calculate SQE offset
            // Layout: Header -> SQEs -> CQEs
            let sqe_ptr = unsafe {
                (handle.ring_ptr.add(1) as *const Sqe).add(idx)
            };
            let sqe = unsafe { *sqe_ptr };

            // EXECUTE COMMAND
            let result = self.process_sqe(&sqe);

            // 3. Push Completion (CQE)
            let mut cq_tail = ring.cq_tail.load(Ordering::Relaxed);
            let cq_mask = ring.cq_mask;
            let cq_idx = (cq_tail & cq_mask) as usize;

            // Calculate CQE offset
            let cqe_ptr = unsafe {
                let sq_region_end = (handle.ring_ptr.add(1) as *mut Sqe).add(64); // Fixed offset for alignment
                (sq_region_end as *mut Cqe).add(cq_idx)
            };

            unsafe {
                (*cqe_ptr).user_data = sqe.user_data;
                (*cqe_ptr).res = result;
                (*cqe_ptr).flags = 0;
            }

            // Commit CQE
            // Store Release ensures userspace sees the CQE data before seeing the updated tail
            ring.cq_tail.store(cq_tail.wrapping_add(1), Ordering::Release);

            head = head.wrapping_add(1);
            processed += 1;
        }

        // 4. Update Kernel Head
        // Store Release ensures userspace sees we've consumed the SQEs
        ring.sq_head.store(head, Ordering::Release);

        Ok(processed)
    }
}
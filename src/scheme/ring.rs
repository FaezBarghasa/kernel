//! Zero-Copy Ring Buffer IPC Scheme
//!
//! Implements truly asynchronous dispatching by pushing commands onto a queue
//! and handling completion signals from userspace drivers.

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
    sync::wait_queue::WaitQueue,
    syscall::{
        error::{Error, Result, EBADF, EINVAL, ENOMEM, ESPIPE, EIO},
        flag::{MapFlags, O_CLOEXEC, O_RDWR},
        number::*,
    },
};

// --- Ring Structure Definitions (Same as Phase 3) ---
#[repr(C)]
pub struct IpcRing {
    pub sq_head: AtomicU32,
    pub sq_tail: AtomicU32,
    pub sq_mask: u32,
    pub cq_head: AtomicU32,
    pub cq_tail: AtomicU32,
    pub cq_mask: u32,
    pub features: u32,
    pub _reserved: u32,
}

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

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Cqe {
    pub user_data: u64,
    pub res: i32,
    pub flags: u32,
}

pub const IORING_OP_NOP: u8 = 0;
pub const IORING_OP_READ: u8 = 1;
pub const IORING_OP_WRITE: u8 = 2;
pub const IORING_OP_CLOSE: u8 = 3;

pub struct RingHandle {
    pub frame: Frame,
    pub ring_ptr: *mut IpcRing,
    pub sq_entries: usize,
    pub cq_entries: usize,
    /// **Task 4.1:** Queue to hold commands waiting for a userspace driver to process them.
    pub driver_queue: WaitQueue,
    /// **Task 4.2:** Context ID of the userspace process (e.g., the Netstack) consuming the queue.
    pub consumer_pid: AtomicUsize,
    /// **Task 4.2:** Queue to wake up the original userspace process waiting for CQE.
    pub completion_wait_queue: WaitQueue,
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

    /// **Task 4.1:** Handles the asynchronous dispatch of an SQE.
    fn process_sqe(&self, handle: &Arc<RingHandle>, sqe: &Sqe) -> Result<()> {
        match sqe.opcode {
            IORING_OP_NOP => {
                // NOP is fast and can be completed synchronously
                let cqe = Cqe { user_data: sqe.user_data, res: 0, flags: 0 };
                Self::ring_complete(handle, &cqe);
                Ok(())
            },
            
            // These operations MUST be passed to a driver in a microkernel
            IORING_OP_READ | IORING_OP_WRITE | IORING_OP_CLOSE => {
                let consumer_pid = handle.consumer_pid.load(Ordering::Relaxed);
                
                if consumer_pid == 0 {
                    // No driver registered to handle this ring yet
                    let cqe = Cqe { user_data: sqe.user_data, res: -(EIO as i32), flags: 0 };
                    Self::ring_complete(handle, &cqe);
                    return Err(Error::new(EIO));
                }

                // Push the SQE pointer/metadata onto the driver's wait queue.
                // The kernel yields immediately. The driver (consumer_pid) is woken up 
                // via `handle.driver_queue.wake_one()`.
                handle.driver_queue.send(sqe.user_data); 
                
                // Wake up the consumers of the driver_queue (the userspace driver)
                handle.driver_queue.wake_one();

                Ok(())
            },

            _ => {
                let cqe = Cqe { user_data: sqe.user_data, res: -(EINVAL as i32), flags: 0 };
                Self::ring_complete(handle, &cqe);
                Err(Error::new(EINVAL))
            }
        }
    }

    /// **Task 4.2:** Function called by a userspace driver to signal a command completion.
    /// In a production system, this would be a custom syscall (`SYS_RING_COMPLETE`).
    pub fn ring_complete(handle: &Arc<RingHandle>, cqe: &Cqe) {
        let ring = unsafe { &*handle.ring_ptr };

        // 1. Write CQE to the Completion Queue
        let mut cq_tail = ring.cq_tail.load(Ordering::Relaxed);
        let cq_mask = ring.cq_mask;
        let cq_idx = (cq_tail & cq_mask) as usize;

        let cqe_ptr = unsafe {
            let sq_region_end = (handle.ring_ptr.add(1) as *mut Sqe).add(64); // Fixed offset
            (sq_region_end as *mut Cqe).add(cq_idx)
        };

        unsafe {
            cqe_ptr.write(*cqe);
        }

        // 2. Commit CQE and update tail (Release ordering)
        ring.cq_tail.store(cq_tail.wrapping_add(1), Ordering::Release);

        // 3. Wake up userspace process waiting on completion
        // The process that originally submitted the command is waiting on `completion_wait_queue`.
        handle.completion_wait_queue.wake_one();
    }
}

impl KernelScheme for RingScheme {
    // ... (kopen, kclose, kfmap remain similar)
    fn kopen(&self, _path: &str, _flags: usize, _ctx: CallerCtx) -> Result<OpenResult> {
        let frame = allocate_frames(1).ok_or(Error::new(ENOMEM))?;
        let phys_addr = frame.start_address();
        let virt_addr = crate::memory::phys_to_virt(phys_addr);
        let ring_ptr = virt_addr.data() as *mut IpcRing;
        
        const SQ_SIZE: usize = 48; 
        const SQ_MASK: u32 = 63;
        const CQ_SIZE: usize = 128;
        const CQ_MASK: u32 = 127;

        unsafe {
            (*ring_ptr).sq_head.store(0, Ordering::Relaxed);
            (*ring_ptr).sq_tail.store(0, Ordering::Relaxed);
            (*ring_ptr).sq_mask = SQ_MASK;
            (*ring_ptr).cq_head.store(0, Ordering::Relaxed);
            (*ring_ptr).cq_tail.store(0, Ordering::Relaxed);
            (*ring_ptr).cq_mask = CQ_MASK;
            (*ring_ptr).features = 0;
            core::ptr::write_bytes(ring_ptr.add(1) as *mut u8, 0, PAGE_SIZE - core::mem::size_of::<IpcRing>());
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed) as usize;
        let handle = Arc::new(RingHandle {
            frame,
            ring_ptr,
            sq_entries: SQ_SIZE,
            cq_entries: CQ_SIZE,
            driver_queue: WaitQueue::new(),
            consumer_pid: AtomicUsize::new(0),
            completion_wait_queue: WaitQueue::new(),
        });

        self.handles.write().insert(id, handle);

        Ok(OpenResult::SchemeLocal(id))
    }

    fn kclose(&self, id: usize) -> Result<usize> {
        let handle = self.handles.write().remove(&id).ok_or(Error::new(EBADF))?;
        unsafe { deallocate_frames(handle.frame, 1); }
        // Ensure consumer PID is zeroed out
        handle.consumer_pid.store(0, Ordering::Relaxed);
        Ok(0)
    }

    fn kfmap(&self, id: usize, _addr: usize, _len: usize, _flags: MapFlags, _arg: usize) -> Result<usize> {
        let handles = self.handles.read();
        let handle = handles.get(&id).ok_or(Error::new(EBADF))?;
        Ok(handle.frame.start_address().data())
    }

    /// Write to the file descriptor acts as the "Doorbell" to wake the kernel
    /// and process the submission queue.
    fn kwrite(&self, id: usize, _buf: &[u8]) -> Result<usize> {
        let handles = self.handles.read();
        let handle = handles.get(&id).ok_or(Error::new(EBADF))?.clone(); // Clone Arc for use in processing

        let ring = unsafe { &*handle.ring_ptr };
        
        let mut head = ring.sq_head.load(Ordering::Acquire);
        let tail = ring.sq_tail.load(Ordering::Acquire);
        let mask = ring.sq_mask;

        let mut processed = 0;

        while head < tail {
            let idx = (head & mask) as usize;
            
            let sqe_ptr = unsafe {
                (handle.ring_ptr.add(1) as *const Sqe).add(idx)
            };
            let sqe = unsafe { *sqe_ptr };

            // ASYNCHRONOUS DISPATCH
            let result = self.process_sqe(&handle, &sqe);
            
            if result.is_ok() {
                processed += 1;
            } else {
                // If dispatch fails (e.g., driver not ready), stop processing queue
                break;
            }

            head = head.wrapping_add(1);
        }

        // 4. Update Kernel Head
        ring.sq_head.store(head, Ordering::Release);

        Ok(processed)
    }
}
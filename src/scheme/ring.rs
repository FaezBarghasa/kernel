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
    context::{self, ContextId, memory::AddrSpaceWrapper},
    memory::{allocate_frames, deallocate_frames, Frame, KernelMapper, PhysicalAddress, PAGE_SIZE, RmmA, RmmArch},
    paging::{Page, PageFlags, VirtualAddress},
    scheme::{CallerCtx, KernelScheme, OpenResult},
    sync::CleanLockToken,
    syscall::{
        data::Stat,
        error::{Error, Result, EBADF, EINVAL, ENOMEM, ESPIPE},
        flag::{MapFlags, O_CLOEXEC, O_RDWR},
        number::*,
        usercopy::{UserSliceRo, UserSliceWo},
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
    fn process_sqe(&self, sqe: &Sqe, token: &mut CleanLockToken) -> i32 {
        match sqe.opcode {
            IORING_OP_NOP => 0,
            
            IORING_OP_READ => {
                // Resolve FD and call kread
                let (scheme, number) = {
                    let context_lock = context::current();
                    let context = context_lock.read(token.token());
                    if let Some(file) = context.get_file(sqe.fd as usize) {
                         let scheme: Arc<dyn KernelScheme> = {
                             let schemes = crate::scheme::schemes(token.token());
                             match schemes.get(file.description.read().scheme) {
                                 Some(scheme) => alloc::sync::Arc::clone(scheme),
                                 None => return -(EBADF as i32),
                             }
                        };
                        (scheme, file.description.read().number)
                    } else {
                        return -(EBADF as i32);
                    }
                };
                // We dropped context lock, now we can call scheme.kread
                // Safety: We are trusting the user provided address/len here.
                // A robust implementation must validate `sqe.addr` is within user range.
                if let Ok(buf) = UserSliceWo::new(sqe.addr as usize, sqe.len as usize) {
                     match scheme.kread(number, buf, 0, 0, token) {
                         Ok(count) => count as i32,
                         Err(err) => -(err.errno as i32),
                     }
                } else {
                    -(EINVAL as i32)
                }
            },

            IORING_OP_WRITE => {
                // Resolve FD and call kwrite
                let (scheme, number) = {
                    let context_lock = context::current();
                    let context = context_lock.read(token.token());
                    if let Some(file) = context.get_file(sqe.fd as usize) {
                        let scheme: Arc<dyn KernelScheme> = {
                             let schemes = crate::scheme::schemes(token.token());
                             match schemes.get(file.description.read().scheme) {
                                 Some(scheme) => alloc::sync::Arc::clone(scheme),
                                 None => return -(EBADF as i32),
                             }
                        };
                        (scheme, file.description.read().number)
                    } else {
                        return -(EBADF as i32);
                    }
                };

                if let Ok(buf) = UserSliceRo::new(sqe.addr as usize, sqe.len as usize) {
                    match scheme.kwrite(number, buf, 0, 0, token) {
                        Ok(count) => count as i32,
                        Err(err) => -(err.errno as i32),
                    }
                } else {
                    -(EINVAL as i32)
                }
            },
            
            IORING_OP_CLOSE => {
                let context_lock = context::current();
                // We need write access to remove file
                let mut context = context_lock.write(token.token());
                if context.remove_file(context::file::FileHandle::from(sqe.fd as usize)).is_some() {
                    0
                } else {
                    -(EBADF as i32)
                }
            }

            _ => -(EINVAL as i32),
        }
    }
}

impl KernelScheme for RingScheme {
    fn kopen(&self, _path: &str, _flags: usize, _ctx: CallerCtx, _token: &mut CleanLockToken) -> Result<OpenResult> {
        // 1. Allocate a physical frame for the ring buffer
        let frame = allocate_frames(1).ok_or(Error::new(ENOMEM))?;
        
        // 2. Map into Kernel space to initialize
        let phys_addr = frame.base();
        let virt_addr = crate::memory::phys_to_virt(phys_addr);
        let ring_ptr = virt_addr.data() as *mut IpcRing;
        
        const SQ_SIZE: usize = 48; 
        const SQ_MASK: u32 = 63;  
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
        
        Ok(OpenResult::SchemeLocal(id, crate::context::file::InternalFlags::empty()))
    }

    fn close(&self, id: usize, _token: &mut CleanLockToken) -> Result<()> {
        let handle = self.handles.write().remove(&id).ok_or(Error::new(EBADF))?;
        unsafe {
            deallocate_frames(handle.frame, 1);
        }
        Ok(())
    }

    fn kfmap(&self, id: usize, _addr: &Arc<AddrSpaceWrapper>, _map: &Map, _consume: bool, _token: &mut CleanLockToken) -> Result<usize> {
        let handles = self.handles.read();
        let handle = handles.get(&id).ok_or(Error::new(EBADF))?;

        // Return the physical address so the memory manager can map it to the user's requested VMA.
        Ok(handle.frame.base().data())
    }

    fn kwrite(&self, id: usize, _buf: UserSliceRo, _flags: u32, _stored_flags: u32, token: &mut CleanLockToken) -> Result<usize> {
        let handle = {
            let handles = self.handles.read();
            handles.get(&id).cloned().ok_or(Error::new(EBADF))?
        };

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

            // EXECUTE COMMAND
            let result = self.process_sqe(&sqe, token);

            // 3. Push Completion (CQE)
            let cq_tail = ring.cq_tail.load(Ordering::Relaxed);
            let cq_mask = ring.cq_mask;
            let cq_idx = (cq_tail & cq_mask) as usize;

            let cqe_ptr = unsafe {
                let sq_region_end = (handle.ring_ptr.add(1) as *mut Sqe).add(64); // Fixed offset for alignment
                (sq_region_end as *mut Cqe).add(cq_idx)
            };

            unsafe {
                (*cqe_ptr).user_data = sqe.user_data;
                (*cqe_ptr).res = result;
                (*cqe_ptr).flags = 0;
            }

            ring.cq_tail.store(cq_tail.wrapping_add(1), Ordering::Release);

            head = head.wrapping_add(1);
            processed += 1;
        }

        ring.sq_head.store(head, Ordering::Release);

        Ok(processed)
    }
}

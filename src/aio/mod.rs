//! # POSIX AIO to io_uring Bridge
//!
//! Maps standard POSIX AIO functions (`aio_read`, `aio_write`, `aio_error`, `aio_return`, `aio_cancel`, `aio_fsync`)
//! directly onto Redox's high-performance native `io_uring` ring-buffer primitives (`kernel::scheme::ring`).
//! Translates legacy callback threads into zero-copy kernel rings.

use alloc::{
    collections::BTreeMap,
    sync::Arc,
};
use core::sync::atomic::{AtomicI32, AtomicIsize, AtomicU32, Ordering};
use spin::Mutex;

use crate::{
    scheme::ring::{Cqe, Sqe, IpcRing},
    syscall::error::{Error, Result, EAGAIN, EBADF, EINVAL, EINPROGRESS, EIO, ECANCELED},
};

pub const AIO_READ: i32 = 1;
pub const AIO_WRITE: i32 = 2;
pub const AIO_FSYNC: i32 = 3;

pub const IORING_OP_READ: u8 = 1;
pub const IORING_OP_WRITE: u8 = 2;
pub const IORING_OP_FSYNC: u8 = 3;

/// POSIX Asynchronous I/O Control Block
#[repr(C)]
pub struct Aiocb {
    pub aio_fildes: i32,
    pub aio_offset: u64,
    pub aio_buf: usize,
    pub aio_nbytes: usize,
    pub aio_reqprio: i32,
    pub aio_lio_opcode: i32,
    pub internal_error: AtomicI32,
    pub internal_return: AtomicIsize,
    pub internal_id: u64,
}

impl Aiocb {
    pub fn new(fd: i32, buf: usize, nbytes: usize, offset: u64, opcode: i32) -> Self {
        Self {
            aio_fildes: fd,
            aio_offset: offset,
            aio_buf: buf,
            aio_nbytes: nbytes,
            aio_reqprio: 0,
            aio_lio_opcode: opcode,
            internal_error: AtomicI32::new(EINPROGRESS as i32),
            internal_return: AtomicIsize::new(-1),
            internal_id: 0,
        }
    }
}

pub struct PosixAioBridge {
    pending_requests: Mutex<BTreeMap<u64, Arc<Aiocb>>>,
    next_request_id: AtomicU32,
}

impl PosixAioBridge {
    pub fn new() -> Self {
        Self {
            pending_requests: Mutex::new(BTreeMap::new()),
            next_request_id: AtomicU32::new(1),
        }
    }

    /// Submit a POSIX AIO control block to the kernel io_uring ring
    pub fn submit(&self, aiocb: Arc<Aiocb>, ring: &mut IpcRing, sqes: &mut [Sqe]) -> Result<()> {
        let req_id = self.next_request_id.fetch_add(1, Ordering::Relaxed) as u64;
        
        let opcode = match aiocb.aio_lio_opcode {
            AIO_READ => IORING_OP_READ,
            AIO_WRITE => IORING_OP_WRITE,
            AIO_FSYNC => IORING_OP_FSYNC,
            _ => return Err(Error::new(EINVAL)),
        };

        let sqe = Sqe {
            opcode,
            flags: 0,
            ioprio: aiocb.aio_reqprio as u16,
            fd: aiocb.aio_fildes,
            off: aiocb.aio_offset,
            addr: aiocb.aio_buf as u64,
            len: aiocb.aio_nbytes as u32,
            rw_flags: 0,
            user_data: req_id,
            buf_index: 0,
            personality: 0,
            splice_fd: -1,
            addr3: 0,
            __pad2: [0; 1],
        };

        let tail = ring.sq_tail.load(Ordering::Relaxed);
        let index = (tail & ring.sq_mask) as usize;
        if index < sqes.len() {
            sqes[index] = sqe;
            ring.sq_tail.store(tail.wrapping_add(1), Ordering::Release);
            self.pending_requests.lock().insert(req_id, aiocb);
            Ok(())
        } else {
            Err(Error::new(EAGAIN))
        }
    }

    /// Process completion queue entries from io_uring and update POSIX AIO statuses zero-copy
    pub fn process_completions(&self, cqe: &Cqe) {
        let req_id = cqe.user_data;
        if let Some(aiocb) = self.pending_requests.lock().remove(&req_id) {
            if cqe.res < 0 {
                aiocb.internal_error.store(-cqe.res, Ordering::Release);
                aiocb.internal_return.store(-1, Ordering::Release);
            } else {
                aiocb.internal_error.store(0, Ordering::Release);
                aiocb.internal_return.store(cqe.res as isize, Ordering::Release);
            }
        }
    }
}

/// Standard POSIX AIO interface functions
pub fn aio_read(aiocb: &Aiocb, bridge: &PosixAioBridge, ring: &mut IpcRing, sqes: &mut [Sqe]) -> Result<()> {
    let mut cb = Aiocb::new(aiocb.aio_fildes, aiocb.aio_buf, aiocb.aio_nbytes, aiocb.aio_offset, AIO_READ);
    cb.aio_reqprio = aiocb.aio_reqprio;
    bridge.submit(Arc::new(cb), ring, sqes)
}

pub fn aio_write(aiocb: &Aiocb, bridge: &PosixAioBridge, ring: &mut IpcRing, sqes: &mut [Sqe]) -> Result<()> {
    let mut cb = Aiocb::new(aiocb.aio_fildes, aiocb.aio_buf, aiocb.aio_nbytes, aiocb.aio_offset, AIO_WRITE);
    cb.aio_reqprio = aiocb.aio_reqprio;
    bridge.submit(Arc::new(cb), ring, sqes)
}

pub fn aio_error(aiocb: &Aiocb) -> i32 {
    aiocb.internal_error.load(Ordering::Acquire)
}

pub fn aio_return(aiocb: &Aiocb) -> isize {
    aiocb.internal_return.load(Ordering::Acquire)
}

pub fn aio_cancel(fd: i32, aiocb: Option<&Aiocb>, bridge: &PosixAioBridge) -> i32 {
    let mut pending = bridge.pending_requests.lock();
    if let Some(cb) = aiocb {
        if pending.remove(&cb.internal_id).is_some() {
            cb.internal_error.store(ECANCELED as i32, Ordering::Release);
            return 0; // AIO_CANCELED
        }
    } else {
        pending.retain(|_, cb| {
            if cb.aio_fildes == fd {
                cb.internal_error.store(ECANCELED as i32, Ordering::Release);
                false
            } else {
                true
            }
        });
    }
    0
}

pub fn aio_fsync(op: i32, aiocb: &Aiocb, bridge: &PosixAioBridge, ring: &mut IpcRing, sqes: &mut [Sqe]) -> Result<()> {
    let mut cb = Aiocb::new(aiocb.aio_fildes, 0, 0, 0, AIO_FSYNC);
    cb.aio_reqprio = op;
    bridge.submit(Arc::new(cb), ring, sqes)
}

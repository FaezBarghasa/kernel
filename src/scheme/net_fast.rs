use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::RwLock;

use crate::{
    context::{file::InternalFlags, memory::AddrSpaceWrapper},
    scheme::{CallerCtx, KernelScheme, OpenResult},
    sync::CleanLockToken,
    syscall::{
        data::Map,
        error::{Error, Result, EBADF, ENOTTY},
    },
};

// IOCTL Definitions
pub const NET_FAST_IOCTL_SUBMIT: usize = 0x4E01;

// Ring Structures (must match userspace layout)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PacketDescriptor {
    pub offset: u32,
    pub len: u32,
    pub flags: u32, // 1 = owned by kernel, 0 = owned by user
}

#[repr(C)]
pub struct PacketRing {
    pub head: AtomicUsize,
    pub tail: AtomicUsize,
    pub descriptors: [PacketDescriptor; 256],
}

pub struct NetFastInterface {
    pub id: usize,
    pub ring: Arc<RwLock<PacketRing>>,
}

pub struct NetFastScheme {
    next_id: AtomicUsize,
    interfaces: RwLock<Vec<Arc<NetFastInterface>>>,
}

impl NetFastScheme {
    pub fn new() -> Self {
        Self {
            next_id: AtomicUsize::new(0),
            interfaces: RwLock::new(Vec::new()),
        }
    }
}

impl KernelScheme for NetFastScheme {
    fn kopen(
        &self,
        _path: &str,
        _flags: usize,
        _ctx: CallerCtx,
        _token: &mut CleanLockToken,
    ) -> Result<OpenResult> {
        let fd = self.next_id.fetch_add(1, Ordering::SeqCst);
        let interface = Arc::new(NetFastInterface {
            id: fd,
            ring: Arc::new(RwLock::new(PacketRing {
                head: AtomicUsize::new(0),
                tail: AtomicUsize::new(0),
                descriptors: [PacketDescriptor {
                    offset: 0,
                    len: 0,
                    flags: 0,
                }; 256],
            })),
        });
        self.interfaces.write().push(interface);
        Ok(OpenResult::SchemeLocal(fd, InternalFlags::empty()))
    }

    fn kfmap(
        &self,
        file: usize,
        _addr_space: &Arc<AddrSpaceWrapper>,
        _map: &Map,
        _consume: bool,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        let interfaces = self.interfaces.read();
        let _interface = interfaces
            .iter()
            .find(|i| i.id == file)
            .ok_or(Error::new(EBADF))?;

        // In a real implementation, we would return the physical address of the ring buffer.
        // For this phase (Titan), we demonstrate the interface.
        Ok(0)
    }

    fn fcntl(
        &self,
        file: usize,
        cmd: usize,
        _arg: usize,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        let interfaces = self.interfaces.read();
        let _interface = interfaces
            .iter()
            .find(|i| i.id == file)
            .ok_or(Error::new(EBADF))?;

        match cmd {
            NET_FAST_IOCTL_SUBMIT => {
                // Handle submission
                Ok(0)
            }
            _ => Err(Error::new(ENOTTY)),
        }
    }
}

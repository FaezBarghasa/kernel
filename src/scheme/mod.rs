//! Kernel Schemes
//!
//! Schemes are the primary abstraction in Redox (like "everything is a file" in Unix).
//! This module manages the registry of built-in kernel schemes.

use alloc::{boxed::Box, collections::BTreeMap, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::RwLock;

use crate::{
    context::{
        file::{FileDescription, InternalFlags},
        memory::AddrSpaceWrapper,
    },
    sync::CleanLockToken,
    syscall::{
        data::{Map, Stat},
        error::{Error, Result, ENODEV, ENOSYS},
        flag::{CallFlags, EventFlags, MapFlags, MunmapFlags},
        usercopy::{UserSliceRo, UserSliceRw, UserSliceWo},
    },
};

#[cfg(feature = "acpi")]
pub mod acpi;
pub mod debug;
#[cfg(dtb)]
pub mod dtb;
pub mod event;
pub mod irq;
pub mod memory;
pub mod pipe;
pub mod proc;
pub mod ring;
pub mod ring_bench;
pub mod root;
pub mod serio;
pub mod sys;
pub mod time;
pub mod user;

pub use self::ring::RingScheme;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct SchemeId(usize);
impl SchemeId {
    pub fn new(id: usize) -> Self {
        Self(id)
    }
    pub fn get(&self) -> usize {
        self.0
    }
}
impl From<usize> for SchemeId {
    fn from(id: usize) -> Self {
        Self(id)
    }
}
impl From<SchemeId> for usize {
    fn from(id: SchemeId) -> Self {
        id.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct FileHandle(usize);
impl FileHandle {
    pub fn get(&self) -> usize {
        self.0
    }
}
impl From<usize> for FileHandle {
    fn from(id: usize) -> Self {
        Self(id)
    }
}
impl From<FileHandle> for usize {
    fn from(h: FileHandle) -> Self {
        h.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct SchemeNamespace(usize);
impl SchemeNamespace {
    pub fn get(&self) -> usize {
        self.0
    }
}
impl From<usize> for SchemeNamespace {
    fn from(id: usize) -> Self {
        Self(id)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CallerCtx {
    pub uid: u32,
    pub gid: u32,
    pub pid: usize,
}

pub enum OpenResult {
    SchemeLocal(usize, crate::context::file::InternalFlags),
    External(Arc<RwLock<FileDescription>>),
}

pub enum StrOrBytes<'a> {
    Str(&'a str),
    Bytes(&'a [u8]),
}

impl<'a> StrOrBytes<'a> {
    pub fn as_bytes(&self) -> &'a [u8] {
        match self {
            Self::Str(s) => s.as_bytes(),
            Self::Bytes(b) => b,
        }
    }
    pub fn as_str(&self) -> Result<&'a str, Error> {
        match self {
            Self::Str(s) => Ok(s),
            Self::Bytes(_) => Err(Error::new(ENODEV)),
        }
    }
}
impl<'a> From<&'a str> for StrOrBytes<'a> {
    fn from(s: &'a str) -> Self {
        Self::Str(s)
    }
}
impl<'a> From<&'a [u8]> for StrOrBytes<'a> {
    fn from(b: &'a [u8]) -> Self {
        Self::Bytes(b)
    }
}

/// Kernel scheme trait
pub trait KernelScheme: Send + Sync {
    fn kopen(
        &self,
        path: &str,
        flags: usize,
        ctx: CallerCtx,
        token: &mut CleanLockToken,
    ) -> Result<OpenResult>;
    fn kopenat(
        &self,
        _file: usize,
        _path: StrOrBytes,
        _flags: usize,
        _fcntl_flags: u32,
        _ctx: CallerCtx,
        _token: &mut CleanLockToken,
    ) -> Result<OpenResult> {
        Err(Error::new(ENOSYS))
    }
    fn rmdir(&self, _path: &str, _ctx: CallerCtx, _token: &mut CleanLockToken) -> Result<()> {
        Err(Error::new(ENOSYS))
    }
    fn unlink(&self, _path: &str, _ctx: CallerCtx, _token: &mut CleanLockToken) -> Result<()> {
        Err(Error::new(ENOSYS))
    }
    fn close(&self, _file: usize, _token: &mut CleanLockToken) -> Result<()> {
        Err(Error::new(ENOSYS))
    }
    fn kread(
        &self,
        _file: usize,
        _buf: UserSliceWo,
        _flags: u32,
        _stored_flags: u32,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        Err(Error::new(ENOSYS))
    }
    fn kreadoff(
        &self,
        _file: usize,
        _buf: UserSliceWo,
        _offset: u64,
        _flags: u32,
        _stored_flags: u32,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        Err(Error::new(ENOSYS))
    }
    fn kwrite(
        &self,
        _file: usize,
        _buf: UserSliceRo,
        _flags: u32,
        _stored_flags: u32,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        Err(Error::new(ENOSYS))
    }
    fn kwriteoff(
        &self,
        _file: usize,
        _buf: UserSliceRo,
        _offset: u64,
        _flags: u32,
        _stored_flags: u32,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        Err(Error::new(ENOSYS))
    }
    fn legacy_seek(
        &self,
        _file: usize,
        _pos: isize,
        _whence: usize,
        _token: &mut CleanLockToken,
    ) -> Option<Result<usize>> {
        None
    }
    fn kfmap(
        &self,
        _file: usize,
        _addr_space: &Arc<AddrSpaceWrapper>,
        _map: &Map,
        _consume: bool,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        Err(Error::new(ENOSYS))
    }
    fn kfunmap(
        &self,
        _number: usize,
        _offset: usize,
        _size: usize,
        _flags: MunmapFlags,
        _token: &mut CleanLockToken,
    ) -> Result<()> {
        Err(Error::new(ENOSYS))
    }
    fn kfstat(&self, _file: usize, _stat: UserSliceWo, _token: &mut CleanLockToken) -> Result<()> {
        Err(Error::new(ENOSYS))
    }
    fn kfstatvfs(
        &self,
        _file: usize,
        _stat: UserSliceWo,
        _token: &mut CleanLockToken,
    ) -> Result<()> {
        Err(Error::new(ENOSYS))
    }
    fn kfpath(
        &self,
        _file: usize,
        _buf: UserSliceWo,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        Err(Error::new(ENOSYS))
    }
    fn kfutimens(
        &self,
        _file: usize,
        _buf: UserSliceRo,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        Err(Error::new(ENOSYS))
    }
    fn fsize(&self, _file: usize, _token: &mut CleanLockToken) -> Result<u64> {
        Err(Error::new(ENOSYS))
    }
    fn fchmod(&self, _file: usize, _mode: u16, _token: &mut CleanLockToken) -> Result<()> {
        Err(Error::new(ENOSYS))
    }
    fn fchown(
        &self,
        _file: usize,
        _uid: u32,
        _gid: u32,
        _token: &mut CleanLockToken,
    ) -> Result<()> {
        Err(Error::new(ENOSYS))
    }
    fn fcntl(
        &self,
        _file: usize,
        _cmd: usize,
        _arg: usize,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        Err(Error::new(ENOSYS))
    }
    fn fevent(
        &self,
        _file: usize,
        _flags: EventFlags,
        _token: &mut CleanLockToken,
    ) -> Result<EventFlags> {
        Err(Error::new(ENOSYS))
    }
    fn fsync(&self, _file: usize, _token: &mut CleanLockToken) -> Result<()> {
        Err(Error::new(ENOSYS))
    }
    fn ftruncate(&self, _file: usize, _len: usize, _token: &mut CleanLockToken) -> Result<()> {
        Err(Error::new(ENOSYS))
    }
    fn flink(
        &self,
        _file: usize,
        _path: &str,
        _ctx: CallerCtx,
        _token: &mut CleanLockToken,
    ) -> Result<()> {
        Err(Error::new(ENOSYS))
    }
    fn frename(
        &self,
        _file: usize,
        _path: &str,
        _ctx: CallerCtx,
        _token: &mut CleanLockToken,
    ) -> Result<()> {
        Err(Error::new(ENOSYS))
    }
    fn kdup(
        &self,
        _file: usize,
        _buf: UserSliceRo,
        _ctx: CallerCtx,
        _token: &mut CleanLockToken,
    ) -> Result<OpenResult> {
        Err(Error::new(ENOSYS))
    }
    fn kcall(
        &self,
        _file: usize,
        _payload: UserSliceRw,
        _flags: CallFlags,
        _metadata: &[u64],
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        Err(Error::new(ENOSYS))
    }
    fn kfdwrite(
        &self,
        _file: usize,
        _descs: Vec<Arc<RwLock<FileDescription>>>,
        _flags: CallFlags,
        _arg: u64,
        _metadata: &[u64],
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        Err(Error::new(ENOSYS))
    }
    fn kfdread(
        &self,
        _file: usize,
        _payload: UserSliceRw,
        _flags: CallFlags,
        _metadata: &[u64],
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        Err(Error::new(ENOSYS))
    }
    fn getdents(
        &self,
        _file: usize,
        _buf: UserSliceWo,
        _header_size: u16,
        _opaque: u64,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        Err(Error::new(ENOSYS))
    }
}

#[derive(Clone)]
pub enum GlobalSchemes {
    Debug,
    Event,
    Memory,
    Pipe,
    Proc,
    Ring(Arc<RingScheme>),
    Serio,
    Irq,
    Time,
    Sys,
    #[cfg(feature = "acpi")]
    Acpi,
    #[cfg(dtb)]
    Dtb,
    Root(Arc<root::RootScheme>),
}

impl GlobalSchemes {
    pub fn scheme_id(&self) -> SchemeId {
        let name = match self {
            Self::Debug => "debug",
            Self::Event => "event",
            Self::Memory => "memory",
            Self::Pipe => "pipe",
            Self::Proc => "proc",
            Self::Ring(_) => "ring",
            Self::Serio => "serio",
            Self::Irq => "irq",
            Self::Time => "time",
            Self::Sys => "sys",
            #[cfg(feature = "acpi")]
            Self::Acpi => "acpi",
            #[cfg(dtb)]
            Self::Dtb => "dtb",
            Self::Root(_) => "root",
        };
        SCHEMES.read().get_id(name).unwrap_or(SchemeId(0))
    }
}

macro_rules! forward_scheme {
    ($self:ident, |$s:ident| $expr:expr) => {
        match $self {
            GlobalSchemes::Debug => {
                let $s = &debug::DebugScheme;
                $expr
            }
            GlobalSchemes::Event => {
                let $s = &event::EventScheme;
                $expr
            }
            GlobalSchemes::Memory => {
                let $s = &memory::MemoryScheme;
                $expr
            }
            GlobalSchemes::Pipe => {
                let $s = &pipe::PipeScheme;
                $expr
            }
            GlobalSchemes::Proc => {
                let $s = &proc::ProcScheme;
                $expr
            }
            GlobalSchemes::Ring(s) => {
                let $s = s;
                $expr
            }
            GlobalSchemes::Serio => {
                let $s = &serio::SerioScheme;
                $expr
            }
            GlobalSchemes::Irq => {
                let $s = &irq::IrqScheme;
                $expr
            }
            GlobalSchemes::Time => {
                let $s = &time::TimeScheme;
                $expr
            }
            GlobalSchemes::Sys => {
                let $s = &sys::SysScheme;
                $expr
            }

            #[cfg(feature = "acpi")]
            GlobalSchemes::Acpi => {
                let $s = &acpi::AcpiScheme;
                $expr
            }
            #[cfg(dtb)]
            GlobalSchemes::Dtb => {
                let $s = &dtb::DtbScheme;
                $expr
            }
            GlobalSchemes::Root(s) => {
                let $s = s;
                $expr
            }
        }
    };
}

impl KernelScheme for GlobalSchemes {
    fn kopen(
        &self,
        path: &str,
        flags: usize,
        ctx: CallerCtx,
        token: &mut CleanLockToken,
    ) -> Result<OpenResult> {
        forward_scheme!(self, |s| s.kopen(path, flags, ctx, token))
    }
    fn kopenat(
        &self,
        file: usize,
        path: StrOrBytes,
        flags: usize,
        fcntl: u32,
        ctx: CallerCtx,
        token: &mut CleanLockToken,
    ) -> Result<OpenResult> {
        forward_scheme!(self, |s| s.kopenat(file, path, flags, fcntl, ctx, token))
    }
    fn rmdir(&self, path: &str, ctx: CallerCtx, token: &mut CleanLockToken) -> Result<()> {
        forward_scheme!(self, |s| s.rmdir(path, ctx, token))
    }
    fn unlink(&self, path: &str, ctx: CallerCtx, token: &mut CleanLockToken) -> Result<()> {
        forward_scheme!(self, |s| s.unlink(path, ctx, token))
    }
    fn close(&self, file: usize, token: &mut CleanLockToken) -> Result<()> {
        forward_scheme!(self, |s| s.close(file, token))
    }
    fn kread(
        &self,
        file: usize,
        buf: UserSliceWo,
        flags: u32,
        stored_flags: u32,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        forward_scheme!(self, |s| s.kread(file, buf, flags, stored_flags, token))
    }
    fn kreadoff(
        &self,
        file: usize,
        buf: UserSliceWo,
        offset: u64,
        flags: u32,
        stored_flags: u32,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        forward_scheme!(self, |s| s.kreadoff(
            file,
            buf,
            offset,
            flags,
            stored_flags,
            token
        ))
    }
    fn kwrite(
        &self,
        file: usize,
        buf: UserSliceRo,
        flags: u32,
        stored_flags: u32,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        forward_scheme!(self, |s| s.kwrite(file, buf, flags, stored_flags, token))
    }
    fn kwriteoff(
        &self,
        file: usize,
        buf: UserSliceRo,
        offset: u64,
        flags: u32,
        stored_flags: u32,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        forward_scheme!(self, |s| s.kwriteoff(
            file,
            buf,
            offset,
            flags,
            stored_flags,
            token
        ))
    }
    fn legacy_seek(
        &self,
        file: usize,
        pos: isize,
        whence: usize,
        token: &mut CleanLockToken,
    ) -> Option<Result<usize>> {
        forward_scheme!(self, |s| s.legacy_seek(file, pos, whence, token))
    }
    fn kfmap(
        &self,
        file: usize,
        addr_space: &Arc<AddrSpaceWrapper>,
        map: &Map,
        consume: bool,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        forward_scheme!(self, |s| s.kfmap(file, addr_space, map, consume, token))
    }
    fn kfunmap(
        &self,
        number: usize,
        offset: usize,
        size: usize,
        flags: MunmapFlags,
        token: &mut CleanLockToken,
    ) -> Result<()> {
        forward_scheme!(self, |s| s.kfunmap(number, offset, size, flags, token))
    }
    fn kfstat(&self, file: usize, stat: UserSliceWo, token: &mut CleanLockToken) -> Result<()> {
        forward_scheme!(self, |s| s.kfstat(file, stat, token))
    }
    fn kfstatvfs(&self, file: usize, stat: UserSliceWo, token: &mut CleanLockToken) -> Result<()> {
        forward_scheme!(self, |s| s.kfstatvfs(file, stat, token))
    }
    fn kfpath(&self, file: usize, buf: UserSliceWo, token: &mut CleanLockToken) -> Result<usize> {
        forward_scheme!(self, |s| s.kfpath(file, buf, token))
    }
    fn kfutimens(
        &self,
        file: usize,
        buf: UserSliceRo,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        forward_scheme!(self, |s| s.kfutimens(file, buf, token))
    }
    fn fsize(&self, file: usize, token: &mut CleanLockToken) -> Result<u64> {
        forward_scheme!(self, |s| s.fsize(file, token))
    }
    fn fchmod(&self, file: usize, mode: u16, token: &mut CleanLockToken) -> Result<()> {
        forward_scheme!(self, |s| s.fchmod(file, mode, token))
    }
    fn fchown(&self, file: usize, uid: u32, gid: u32, token: &mut CleanLockToken) -> Result<()> {
        forward_scheme!(self, |s| s.fchown(file, uid, gid, token))
    }
    fn fcntl(
        &self,
        file: usize,
        cmd: usize,
        arg: usize,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        forward_scheme!(self, |s| s.fcntl(file, cmd, arg, token))
    }
    fn fevent(
        &self,
        file: usize,
        flags: EventFlags,
        token: &mut CleanLockToken,
    ) -> Result<EventFlags> {
        forward_scheme!(self, |s| s.fevent(file, flags, token))
    }
    fn fsync(&self, file: usize, token: &mut CleanLockToken) -> Result<()> {
        forward_scheme!(self, |s| s.fsync(file, token))
    }
    fn ftruncate(&self, file: usize, len: usize, token: &mut CleanLockToken) -> Result<()> {
        forward_scheme!(self, |s| s.ftruncate(file, len, token))
    }
    fn flink(
        &self,
        file: usize,
        path: &str,
        ctx: CallerCtx,
        token: &mut CleanLockToken,
    ) -> Result<()> {
        forward_scheme!(self, |s| s.flink(file, path, ctx, token))
    }
    fn frename(
        &self,
        file: usize,
        path: &str,
        ctx: CallerCtx,
        token: &mut CleanLockToken,
    ) -> Result<()> {
        forward_scheme!(self, |s| s.frename(file, path, ctx, token))
    }
    fn kdup(
        &self,
        file: usize,
        buf: UserSliceRo,
        ctx: CallerCtx,
        token: &mut CleanLockToken,
    ) -> Result<OpenResult> {
        forward_scheme!(self, |s| s.kdup(file, buf, ctx, token))
    }
    fn kcall(
        &self,
        file: usize,
        payload: UserSliceRw,
        flags: CallFlags,
        metadata: &[u64],
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        forward_scheme!(self, |s| s.kcall(file, payload, flags, metadata, token))
    }
    fn kfdwrite(
        &self,
        file: usize,
        descs: Vec<Arc<RwLock<FileDescription>>>,
        flags: CallFlags,
        arg: u64,
        metadata: &[u64],
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        forward_scheme!(self, |s| s
            .kfdwrite(file, descs, flags, arg, metadata, token))
    }
    fn kfdread(
        &self,
        file: usize,
        payload: UserSliceRw,
        flags: CallFlags,
        metadata: &[u64],
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        forward_scheme!(self, |s| s.kfdread(file, payload, flags, metadata, token))
    }
    fn getdents(
        &self,
        file: usize,
        buf: UserSliceWo,
        header_size: u16,
        opaque: u64,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        forward_scheme!(self, |s| s.getdents(file, buf, header_size, opaque, token))
    }
}

#[derive(Clone)]
pub enum KernelSchemes {
    Global(GlobalSchemes),
    User(user::UserScheme),
}

impl KernelScheme for KernelSchemes {
    // Forward all methods to Global or User
    fn kopen(
        &self,
        path: &str,
        flags: usize,
        ctx: CallerCtx,
        token: &mut CleanLockToken,
    ) -> Result<OpenResult> {
        match self {
            Self::Global(s) => s.kopen(path, flags, ctx, token),
            Self::User(s) => s.kopen(path, flags, ctx, token),
        }
    }
    fn kopenat(
        &self,
        file: usize,
        path: StrOrBytes,
        flags: usize,
        fcntl: u32,
        ctx: CallerCtx,
        token: &mut CleanLockToken,
    ) -> Result<OpenResult> {
        match self {
            Self::Global(s) => s.kopenat(file, path, flags, fcntl, ctx, token),
            Self::User(s) => s.kopenat(file, path, flags, fcntl, ctx, token),
        }
    }
    fn rmdir(&self, path: &str, ctx: CallerCtx, token: &mut CleanLockToken) -> Result<()> {
        match self {
            Self::Global(s) => s.rmdir(path, ctx, token),
            Self::User(s) => s.rmdir(path, ctx, token),
        }
    }
    fn unlink(&self, path: &str, ctx: CallerCtx, token: &mut CleanLockToken) -> Result<()> {
        match self {
            Self::Global(s) => s.unlink(path, ctx, token),
            Self::User(s) => s.unlink(path, ctx, token),
        }
    }
    fn close(&self, file: usize, token: &mut CleanLockToken) -> Result<()> {
        match self {
            Self::Global(s) => s.close(file, token),
            Self::User(s) => s.close(file, token),
        }
    }
    fn kread(
        &self,
        file: usize,
        buf: UserSliceWo,
        flags: u32,
        stored_flags: u32,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        match self {
            Self::Global(s) => s.kread(file, buf, flags, stored_flags, token),
            Self::User(s) => s.kread(file, buf, flags, stored_flags, token),
        }
    }
    fn kreadoff(
        &self,
        file: usize,
        buf: UserSliceWo,
        offset: u64,
        flags: u32,
        stored_flags: u32,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        match self {
            Self::Global(s) => s.kreadoff(file, buf, offset, flags, stored_flags, token),
            Self::User(s) => s.kreadoff(file, buf, offset, flags, stored_flags, token),
        }
    }
    fn kwrite(
        &self,
        file: usize,
        buf: UserSliceRo,
        flags: u32,
        stored_flags: u32,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        match self {
            Self::Global(s) => s.kwrite(file, buf, flags, stored_flags, token),
            Self::User(s) => s.kwrite(file, buf, flags, stored_flags, token),
        }
    }
    fn kwriteoff(
        &self,
        file: usize,
        buf: UserSliceRo,
        offset: u64,
        flags: u32,
        stored_flags: u32,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        match self {
            Self::Global(s) => s.kwriteoff(file, buf, offset, flags, stored_flags, token),
            Self::User(s) => s.kwriteoff(file, buf, offset, flags, stored_flags, token),
        }
    }
    fn legacy_seek(
        &self,
        file: usize,
        pos: isize,
        whence: usize,
        token: &mut CleanLockToken,
    ) -> Option<Result<usize>> {
        match self {
            Self::Global(s) => s.legacy_seek(file, pos, whence, token),
            Self::User(s) => s.legacy_seek(file, pos, whence, token),
        }
    }
    fn kfmap(
        &self,
        file: usize,
        addr_space: &Arc<AddrSpaceWrapper>,
        map: &Map,
        consume: bool,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        match self {
            Self::Global(s) => s.kfmap(file, addr_space, map, consume, token),
            Self::User(s) => s.kfmap(file, addr_space, map, consume, token),
        }
    }
    fn kfunmap(
        &self,
        number: usize,
        offset: usize,
        size: usize,
        flags: MunmapFlags,
        token: &mut CleanLockToken,
    ) -> Result<()> {
        match self {
            Self::Global(s) => s.kfunmap(number, offset, size, flags, token),
            Self::User(s) => s.kfunmap(number, offset, size, flags, token),
        }
    }
    fn kfstat(&self, file: usize, stat: UserSliceWo, token: &mut CleanLockToken) -> Result<()> {
        match self {
            Self::Global(s) => s.kfstat(file, stat, token),
            Self::User(s) => s.kfstat(file, stat, token),
        }
    }
    fn kfstatvfs(&self, file: usize, stat: UserSliceWo, token: &mut CleanLockToken) -> Result<()> {
        match self {
            Self::Global(s) => s.kfstatvfs(file, stat, token),
            Self::User(s) => s.kfstatvfs(file, stat, token),
        }
    }
    fn kfpath(&self, file: usize, buf: UserSliceWo, token: &mut CleanLockToken) -> Result<usize> {
        match self {
            Self::Global(s) => s.kfpath(file, buf, token),
            Self::User(s) => s.kfpath(file, buf, token),
        }
    }
    fn kfutimens(
        &self,
        file: usize,
        buf: UserSliceRo,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        match self {
            Self::Global(s) => s.kfutimens(file, buf, token),
            Self::User(s) => s.kfutimens(file, buf, token),
        }
    }
    fn fsize(&self, file: usize, token: &mut CleanLockToken) -> Result<u64> {
        match self {
            Self::Global(s) => s.fsize(file, token),
            Self::User(s) => s.fsize(file, token),
        }
    }
    fn fchmod(&self, file: usize, mode: u16, token: &mut CleanLockToken) -> Result<()> {
        match self {
            Self::Global(s) => s.fchmod(file, mode, token),
            Self::User(s) => s.fchmod(file, mode, token),
        }
    }
    fn fchown(&self, file: usize, uid: u32, gid: u32, token: &mut CleanLockToken) -> Result<()> {
        match self {
            Self::Global(s) => s.fchown(file, uid, gid, token),
            Self::User(s) => s.fchown(file, uid, gid, token),
        }
    }
    fn fcntl(
        &self,
        file: usize,
        cmd: usize,
        arg: usize,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        match self {
            Self::Global(s) => s.fcntl(file, cmd, arg, token),
            Self::User(s) => s.fcntl(file, cmd, arg, token),
        }
    }
    fn fevent(
        &self,
        file: usize,
        flags: EventFlags,
        token: &mut CleanLockToken,
    ) -> Result<EventFlags> {
        match self {
            Self::Global(s) => s.fevent(file, flags, token),
            Self::User(s) => s.fevent(file, flags, token),
        }
    }
    fn fsync(&self, file: usize, token: &mut CleanLockToken) -> Result<()> {
        match self {
            Self::Global(s) => s.fsync(file, token),
            Self::User(s) => s.fsync(file, token),
        }
    }
    fn ftruncate(&self, file: usize, len: usize, token: &mut CleanLockToken) -> Result<()> {
        match self {
            Self::Global(s) => s.ftruncate(file, len, token),
            Self::User(s) => s.ftruncate(file, len, token),
        }
    }
    fn flink(
        &self,
        file: usize,
        path: &str,
        ctx: CallerCtx,
        token: &mut CleanLockToken,
    ) -> Result<()> {
        match self {
            Self::Global(s) => s.flink(file, path, ctx, token),
            Self::User(s) => s.flink(file, path, ctx, token),
        }
    }
    fn frename(
        &self,
        file: usize,
        path: &str,
        ctx: CallerCtx,
        token: &mut CleanLockToken,
    ) -> Result<()> {
        match self {
            Self::Global(s) => s.frename(file, path, ctx, token),
            Self::User(s) => s.frename(file, path, ctx, token),
        }
    }
    fn kdup(
        &self,
        file: usize,
        buf: UserSliceRo,
        ctx: CallerCtx,
        token: &mut CleanLockToken,
    ) -> Result<OpenResult> {
        match self {
            Self::Global(s) => s.kdup(file, buf, ctx, token),
            Self::User(s) => s.kdup(file, buf, ctx, token),
        }
    }
    fn kcall(
        &self,
        file: usize,
        payload: UserSliceRw,
        flags: CallFlags,
        metadata: &[u64],
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        match self {
            Self::Global(s) => s.kcall(file, payload, flags, metadata, token),
            Self::User(s) => s.kcall(file, payload, flags, metadata, token),
        }
    }
    fn kfdwrite(
        &self,
        file: usize,
        descs: Vec<Arc<RwLock<FileDescription>>>,
        flags: CallFlags,
        arg: u64,
        metadata: &[u64],
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        match self {
            Self::Global(s) => s.kfdwrite(file, descs, flags, arg, metadata, token),
            Self::User(s) => s.kfdwrite(file, descs, flags, arg, metadata, token),
        }
    }
    fn kfdread(
        &self,
        file: usize,
        payload: UserSliceRw,
        flags: CallFlags,
        metadata: &[u64],
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        match self {
            Self::Global(s) => s.kfdread(file, payload, flags, metadata, token),
            Self::User(s) => s.kfdread(file, payload, flags, metadata, token),
        }
    }
    fn getdents(
        &self,
        file: usize,
        buf: UserSliceWo,
        header_size: u16,
        opaque: u64,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        match self {
            Self::Global(s) => s.getdents(file, buf, header_size, opaque, token),
            Self::User(s) => s.getdents(file, buf, header_size, opaque, token),
        }
    }
}

pub struct SchemeList {
    map: BTreeMap<SchemeId, Arc<KernelSchemes>>,
    names: BTreeMap<SchemeNamespace, BTreeMap<Box<str>, SchemeId>>,
    next_id: AtomicUsize,
}

impl SchemeList {
    pub fn new() -> Self {
        Self {
            map: BTreeMap::new(),
            names: BTreeMap::new(),
            next_id: AtomicUsize::new(1),
        }
    }
    pub fn get(&self, id: SchemeId) -> Option<&Arc<KernelSchemes>> {
        self.map.get(&id)
    }
    pub fn get_name<'a>(
        &'a self,
        ns: SchemeNamespace,
        name: &str,
    ) -> Option<(SchemeId, &'a Arc<KernelSchemes>)> {
        if let Some(ids) = self.names.get(&ns) {
            if let Some(&id) = ids.get(name) {
                if let Some(scheme) = self.map.get(&id) {
                    return Some((id, scheme));
                }
            }
        }
        None
    }
    pub fn get_id(&self, name: &str) -> Option<SchemeId> {
        self.names
            .get(&SchemeNamespace(0))
            .and_then(|m| m.get(name))
            .cloned()
    }
    pub fn iter_name(&self, ns: SchemeNamespace) -> impl Iterator<Item = (&Box<str>, &SchemeId)> {
        self.names.get(&ns).into_iter().flat_map(|m| m.iter())
    }
    pub fn insert(&mut self, name: Box<str>, scheme: KernelSchemes) -> SchemeId {
        let id = SchemeId(self.next_id.fetch_add(1, Ordering::Relaxed));
        self.map.insert(id, Arc::new(scheme));
        self.names
            .entry(SchemeNamespace(0))
            .or_default()
            .insert(name, id);
        id
    }
}

pub static SCHEMES: RwLock<SchemeList> = RwLock::new(SchemeList {
    map: BTreeMap::new(),
    names: BTreeMap::new(),
    next_id: AtomicUsize::new(1),
});

pub fn schemes<L: crate::sync::Level>(
    _token: &crate::sync::LockToken<'_, L>,
) -> spin::RwLockReadGuard<'static, SchemeList> {
    SCHEMES.read()
}

pub fn schemes_mut<L: crate::sync::Level>(
    _token: &crate::sync::LockToken<'_, L>,
) -> spin::RwLockWriteGuard<'static, SchemeList> {
    SCHEMES.write()
}

pub fn init_schemes() {
    // Run benchmark temporarily
    ring_bench::benchmark_ring();

    let mut schemes = SCHEMES.write();
    let ring = Arc::new(RingScheme::new());

    schemes.insert(
        Box::from("debug"),
        KernelSchemes::Global(GlobalSchemes::Debug),
    );
    schemes.insert(
        Box::from("event"),
        KernelSchemes::Global(GlobalSchemes::Event),
    );
    schemes.insert(
        Box::from("memory"),
        KernelSchemes::Global(GlobalSchemes::Memory),
    );
    schemes.insert(
        Box::from("pipe"),
        KernelSchemes::Global(GlobalSchemes::Pipe),
    );
    schemes.insert(
        Box::from("proc"),
        KernelSchemes::Global(GlobalSchemes::Proc),
    );
    schemes.insert(
        Box::from("ring"),
        KernelSchemes::Global(GlobalSchemes::Ring(ring)),
    );
    schemes.insert(
        Box::from("serio"),
        KernelSchemes::Global(GlobalSchemes::Serio),
    );
    schemes.insert(Box::from("irq"), KernelSchemes::Global(GlobalSchemes::Irq));
    schemes.insert(
        Box::from("time"),
        KernelSchemes::Global(GlobalSchemes::Time),
    );
    schemes.insert(Box::from("sys"), KernelSchemes::Global(GlobalSchemes::Sys));
    #[cfg(feature = "acpi")]
    schemes.insert(
        Box::from("acpi"),
        KernelSchemes::Global(GlobalSchemes::Acpi),
    );
    #[cfg(dtb)]
    schemes.insert(Box::from("dtb"), KernelSchemes::Global(GlobalSchemes::Dtb));

    // Manually insert root scheme to get the ID
    let root_id = SchemeId(schemes.next_id.fetch_add(1, Ordering::Relaxed));
    let root = Arc::new(root::RootScheme::new(SchemeNamespace(0), root_id));
    schemes.map.insert(
        root_id,
        Arc::new(KernelSchemes::Global(GlobalSchemes::Root(root))),
    );
    schemes
        .names
        .entry(SchemeNamespace(0))
        .or_default()
        .insert(Box::from("root"), root_id);
}

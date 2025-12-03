//! # Virtual Memory Management for Contexts

use alloc::{sync::Arc, vec::Vec};
use spin::RwLock;

use crate::{
    memory::{self, Enomem, Frame, RaiiFrame},
    paging::{Page, PageFlags, RmmA, VirtualAddress, PAGE_SIZE},
    sync::CleanLockToken,
    syscall::{
        self,
        error::{Error, Result as SysResult},
        flag::MapFlags,
    },
};
use alloc::collections::BTreeMap;

#[derive(Debug)]
pub enum PfError {
    Oom,
    Segv,
    RecursionLimitExceeded,
    NonfatalInternalError,
}

impl From<Enomem> for PfError {
    fn from(_: Enomem) -> Self {
        Self::Oom
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum AccessMode {
    Read,
    Write,
    InstrFetch,
}

#[derive(Debug)]
pub struct Grant {
    start: Page,
    end: Page,
    flags: PageFlags<RmmA>,
    phys: Option<RaiiFrame>,
    pub provider: Provider,
}

impl Grant {
    pub fn new(start: Page, end: Page, flags: PageFlags<RmmA>) -> Self {
        Self {
            start,
            end,
            flags,
            phys: None,
            provider: Provider::Allocated { flags },
        }
    }

    pub fn phys(&self) -> Option<Frame> {
        self.phys.as_ref().map(|f| f.get())
    }
    pub fn grant_flags(&self) -> MapFlags {
        // TODO: reconstruct MapFlags from PageFlags
        let mut flags = MapFlags::empty();
        if self.flags.has_write() {
            flags |= MapFlags::PROT_WRITE;
        }
        // if self.flags.has_execute() { flags |= MapFlags::PROT_EXEC; } // PageFlags might not have has_execute depending on arch
        flags |= MapFlags::PROT_READ; // Always readable?
        flags
    }
    pub fn file_ref(&self) -> Option<&GrantFileRef> {
        match &self.provider {
            Provider::FmapBorrowed { file_ref } => Some(file_ref),
            _ => None,
        }
    }

    pub fn start_address(&self) -> VirtualAddress {
        self.start.start_address()
    }

    pub fn zeroed_phys_contiguous(
        _span: PageSpan,
        _flags: PageFlags<RmmA>,
        _mapper: &mut Mapper,
        _flusher: &mut GenericFlusher,
    ) -> SysResult<Self> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn zeroed(
        _span: PageSpan,
        _flags: PageFlags<RmmA>,
        _mapper: &mut Mapper,
        _flusher: &mut GenericFlusher,
        _shared: bool,
    ) -> SysResult<Self> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn physmap(
        _phys: Frame,
        _span: PageSpan,
        _flags: PageFlags<RmmA>,
        _mapper: &mut Mapper,
        _flusher: &mut GenericFlusher,
    ) -> SysResult<Self> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn set_phys(&mut self, frame: Frame) {
        let raii = unsafe { RaiiFrame::new_unchecked(frame) };
        self.phys = Some(raii);
    }

    pub fn unmap(mut self) {
        drop(self.phys.take());
    }

    pub fn borrow_grant(
        _owner: Arc<AddrSpaceWrapper>,
        _src_addr_space: &mut AddrSpaceWrapper,
        _src_base: Page,
        _dst_base: Page,
        _count: usize,
        _flags: MapFlags,
        _mapper: &mut Mapper,
        _flusher: &mut GenericFlusher,
        _cow: bool,
        _shared: bool,
        _zeromap: bool,
    ) -> SysResult<Grant> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn allocated_shared_one_page(
        _frame: Frame,
        _page: Page,
        _flags: MapFlags,
        _mapper: &mut Mapper,
        _flusher: &mut GenericFlusher,
        _shared: bool,
    ) -> SysResult<Grant> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn allocated_one_page_nomap(_page: Page, _flags: PageFlags<RmmA>) -> Grant {
        Grant::new(_page, _page.next(), _flags)
    }

    pub fn flags(&self) -> PageFlags<RmmA> {
        self.flags
    }

    pub fn grant_flags(&self) -> MapFlags {
        let mut flags = MapFlags::empty();
        if self.flags.has_write() {
            flags |= MapFlags::PROT_WRITE;
        }
        // TODO: Execute?
        flags |= MapFlags::PROT_READ;
        flags
    }

    pub fn page_count(&self) -> usize {
        (self.end.start_address().data() - self.start.start_address().data()) / PAGE_SIZE
    }
}

pub fn try_correcting_page_tables(
    _faulting_page: Page,
    _access: AccessMode,
    _token: &mut CleanLockToken,
) -> Result<(), PfError> {
    Err(PfError::Segv)
}

// --- Added missing types ---

pub struct AddrSpaceWrapper {
    pub inner: RwLock<AddrSpaceInner>,
}

pub type AddrSpace = AddrSpaceWrapper;

#[derive(Debug)]
pub struct AddrSpaceInner {
    pub used_by: crate::cpu_set::LogicalCpuSet,
    pub table: TableWrapper,
    pub tlb_ack: core::sync::atomic::AtomicUsize,
    pub grants: BTreeMap<Page, Grant>,
    pub mmap_min: usize,
}

#[derive(Debug)]
pub struct TableWrapper {
    pub utable: UTableWrapper,
}

pub type Table = TableWrapper;

#[derive(Debug)]
pub struct UTableWrapper(
    pub rmm::PageMapper<crate::arch::x86_shared::CurrentRmmArch, crate::memory::TheFrameAllocator>,
);

impl UTableWrapper {
    pub unsafe fn make_current(&self) {
        unsafe {
            self.0.make_current();
        }
    }
    pub fn table(&self) -> &rmm::PageTable<crate::arch::x86_shared::CurrentRmmArch> {
        self.0.table()
    }
    pub fn translate(&self, addr: VirtualAddress) -> Option<crate::paging::PhysicalAddress> {
        self.0.translate(addr).map(|(addr, _)| addr)
    }
}

impl AddrSpaceWrapper {
    pub fn new() -> SysResult<Arc<Self>> {
        Ok(Arc::new(Self {
            inner: RwLock::new(AddrSpaceInner {
                used_by: crate::cpu_set::LogicalCpuSet::new(),
                table: TableWrapper {
                    utable: UTableWrapper(unsafe {
                        rmm::PageMapper::create(
                            rmm::TableKind::User,
                            crate::memory::TheFrameAllocator,
                        )
                        .ok_or(Error::new(crate::syscall::error::ENOMEM))?
                    }),
                },
                tlb_ack: core::sync::atomic::AtomicUsize::new(0),
                grants: BTreeMap::new(),
                mmap_min: PAGE_SIZE,
            }),
        }))
    }

    pub fn acquire_read(&self) -> spin::RwLockReadGuard<AddrSpaceInner> {
        self.inner.read()
    }

    pub fn acquire_write(&self) -> spin::RwLockWriteGuard<AddrSpaceInner> {
        self.inner.write()
    }

    pub fn current(token: &mut CleanLockToken) -> SysResult<Arc<Self>> {
        crate::context::context::Context::current()
            .read(token.token())
            .addr_space
            .clone()
            .ok_or(Error::new(crate::syscall::error::ESRCH))
    }

    pub fn try_clone(&self) -> SysResult<Arc<Self>> {
        Self::new()
    }

    pub fn munmap(&self, _span: PageSpan, _unpin: bool) -> SysResult<Vec<Grant>> {
        Ok(Vec::new())
    }

    pub fn mmap(
        &self,
        _addr_space: &Arc<AddrSpaceWrapper>,
        _base: Option<Page>,
        _count: core::num::NonZeroUsize,
        _flags: MapFlags,
        _vec: &mut Vec<GrantFileRef>,
        _func: impl FnOnce(
            Page,
            crate::paging::PageFlags<RmmA>,
            &mut Mapper,
            &mut GenericFlusher,
        ) -> SysResult<Grant>,
    ) -> SysResult<Grant> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn mmap_anywhere(
        &self,
        addr_space: &Arc<AddrSpaceWrapper>,
        count: core::num::NonZeroUsize,
        flags: MapFlags,
        func: impl FnOnce(
            Page,
            crate::paging::PageFlags<RmmA>,
            &mut Mapper,
            &mut GenericFlusher,
        ) -> SysResult<Grant>,
    ) -> SysResult<Grant> {
        self.mmap(addr_space, None, count, flags, &mut Vec::new(), func)
    }

    pub fn r#move(
        &self,
        _target: Option<&Arc<AddrSpaceWrapper>>,
        _src: PageSpan,
        _dst: Option<Page>,
        _count: usize,
        _flags: MapFlags,
        _vec: &mut Vec<GrantFileRef>,
    ) -> SysResult<Grant> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn borrow_frame_enforce_rw_allocated(
        &self,
        _base: Page,
        _token: &mut CleanLockToken,
    ) -> SysResult<RaiiFrame> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn mprotect(&self, _base: Page, _count: usize, _flags: MapFlags) -> SysResult<()> {
        Err(Error::new(crate::syscall::error::ENOSYS))
    }
}

#[derive(Debug, Clone)]
pub struct GrantFileRef {
    pub base_offset: usize,
    pub description: Arc<RwLock<crate::context::file::FileDescriptor>>,
}

#[derive(Debug, Clone, Copy)]
pub struct PageSpan {
    pub base: Page,
    pub count: usize,
}

impl PageSpan {
    pub fn new(base: Page, count: usize) -> Self {
        Self { base, count }
    }
    pub fn validate_nonempty(base: VirtualAddress, size: usize) -> Option<Self> {
        Some(Self {
            base: Page::containing_address(base),
            count: size / PAGE_SIZE,
        })
    }
}

pub struct GenericFlusher;
impl GenericFlusher {
    pub fn queue(&self, _frame: Frame, _page: Option<Page>, _action: TlbShootdownActions) {}
}

pub struct Mapper;
impl Mapper {
    pub fn map_phys(
        &mut self,
        _virt: VirtualAddress,
        _phys: crate::paging::PhysicalAddress,
        _flags: PageFlags<RmmA>,
    ) -> Option<crate::paging::PageFlush<()>> {
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TlbShootdownActions;
impl TlbShootdownActions {
    pub const NEW_MAPPING: Self = Self;
}

#[derive(Debug)]
pub enum Provider {
    Allocated { flags: PageFlags<RmmA> },
    PhysBorrowed { base: Frame },
    External { address: usize, size: usize },
    FmapBorrowed { file_ref: GrantFileRef },
}

#[derive(Debug)]
pub struct BorrowedFmapSource;

#[derive(Debug)]
pub enum MmapMode {
    Cow,
}

pub const DANGLING: usize = 0;

pub fn handle_notify_files(_files: Vec<GrantFileRef>, _token: &mut CleanLockToken) {}

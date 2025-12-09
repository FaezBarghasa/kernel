//! # Virtual Memory Management for Contexts

use alloc::{sync::Arc, vec::Vec};
use spin::RwLock;

pub use crate::paging::TlbShootdownActions;

use crate::{
    context::file::FileDescription,
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
    pub locked: bool, // Added field for memory locking
}

impl Grant {
    pub fn new(start: Page, end: Page, flags: PageFlags<RmmA>) -> Self {
        Self {
            start,
            end,
            flags,
            phys: None,
            provider: Provider::Allocated { flags },
            locked: false,
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
        _mapper: &mut crate::memory::KernelMapper,
        _flusher: &mut TlbShootdownActions,
    ) -> SysResult<Self> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn zeroed(
        _span: PageSpan,
        _flags: PageFlags<RmmA>,
        _mapper: &mut crate::memory::KernelMapper,
        _flusher: &mut TlbShootdownActions,
        _shared: bool,
    ) -> SysResult<Self> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn physmap(
        _phys: Frame,
        _span: PageSpan,
        _flags: PageFlags<RmmA>,
        _mapper: &mut crate::memory::KernelMapper,
        _flusher: &mut TlbShootdownActions,
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

    pub fn borrow(
        _src_base: Page,
        _dst_base: Page,
        _count: usize,
        _flags: MapFlags,
        _mapper: &mut crate::memory::KernelMapper,
        _flusher: &mut TlbShootdownActions,
        _cow: bool,
        _shared: bool,
        _zeromap: bool,
        _src_inner: Option<&mut AddrSpaceInner>,
    ) -> SysResult<Grant> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn borrow_grant(
        _owner: Arc<AddrSpaceWrapper>,
        _src_addr_space: &mut AddrSpaceInner,
        _src_base: Page,
        _dst_base: Page,
        _count: usize,
        _flags: MapFlags,
        _mapper: &mut crate::memory::KernelMapper,
        _flusher: &mut TlbShootdownActions,
        _cow: bool,
        _shared: bool,
        _zeromap: bool,
    ) -> SysResult<Grant> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn allocated_shared_one_page(
        _frame: Frame,
        _page: Page,
        _flags: PageFlags<RmmA>,
        _mapper: &mut crate::memory::KernelMapper,
        _flusher: &mut TlbShootdownActions,
        _shared: bool,
    ) -> SysResult<Grant> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn borrow_fmap(
        _span: PageSpan,
        _flags: PageFlags<RmmA>,
        _file_ref: GrantFileRef,
        _src: Option<BorrowedFmapSource>,
        _dst_addr_space: &Arc<AddrSpaceWrapper>,
        _mapper: &mut crate::memory::KernelMapper,
        _flusher: &mut TlbShootdownActions,
        _token: &mut CleanLockToken,
    ) -> SysResult<Grant> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn allocated_one_page_nomap(_page: Page, _flags: PageFlags<RmmA>) -> Grant {
        Grant::new(_page, _page.next(), _flags)
    }

    pub fn flags(&self) -> PageFlags<RmmA> {
        self.flags
    }

    pub fn page_count(&self) -> usize {
        (self.end.start_address().data() - self.start.start_address().data()) / PAGE_SIZE
    }
}

pub fn try_correcting_page_tables(
    faulting_page: Page,
    _access: AccessMode,
    token: &mut CleanLockToken,
) -> Result<(), PfError> {
    let current_context_ref = crate::context::current();
    let current_context_guard = current_context_ref.read(token.token());

    if current_context_guard.memory_locked_count > 0 {
        if let Some(addr_space) = current_context_guard.addr_space.as_ref() {
            let mut inner = addr_space.inner.write();
            if let Some(grant) = inner.grants.get_mut(&faulting_page) {
                if grant.locked {
                    // Page is locked and not present, so we need to make it present
                    if grant.phys.is_none() {
                        let frame = memory::allocate_frame().ok_or(PfError::Oom)?;
                        grant.set_phys(frame);
                        let mut kernel_mapper = crate::memory::KernelMapper::lock();
                        let mapper = kernel_mapper
                            .get_mut()
                            .expect("failed to lock kernel mapper");
                        unsafe {
                            mapper
                                .map_phys(faulting_page.start_address(), frame.base(), grant.flags)
                                .ok_or(PfError::Oom)?
                                .flush();
                        }
                        return Ok(());
                    }
                }
            }
        }
    }

    // Default behavior if not a locked page or not handled
    Err(PfError::Segv)
}

// --- Added missing types ---

#[derive(Debug)]
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
    pub fn table(&self) -> rmm::PageTable<crate::arch::x86_shared::CurrentRmmArch> {
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
        crate::context::current()
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
}

impl AddrSpaceInner {
    pub fn borrow_frame_enforce_rw_allocated(
        &mut self,
        _base: Page,
        _token: &mut CleanLockToken,
    ) -> SysResult<RaiiFrame> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }
    pub fn mprotect(&mut self, _base: Page, _count: usize, _flags: MapFlags) -> SysResult<()> {
        Err(Error::new(crate::syscall::error::ENOSYS))
    }
    pub fn munmap(&mut self, _span: PageSpan, _unpin: bool) -> SysResult<Vec<Grant>> {
        Ok(Vec::new())
    }

    pub fn mmap(
        &mut self,
        _base: Option<Page>,
        _count: core::num::NonZeroUsize,
        _flags: MapFlags,
        _vec: &mut Vec<GrantFileRef>,
        _func: impl FnOnce(
            Page,
            crate::paging::PageFlags<RmmA>,
            &mut crate::memory::KernelMapper,
            &mut TlbShootdownActions,
        ) -> SysResult<Grant>,
    ) -> SysResult<Grant> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn mmap_anywhere(
        &mut self,
        count: core::num::NonZeroUsize,
        flags: MapFlags,
        func: impl FnOnce(
            Page,
            crate::paging::PageFlags<RmmA>,
            &mut crate::memory::KernelMapper,
            &mut TlbShootdownActions,
        ) -> SysResult<Grant>,
    ) -> SysResult<Grant> {
        self.mmap(None, count, flags, &mut Vec::new(), func)
    }

    pub fn find_free_span(&self, min_address: usize, page_count: usize) -> Option<PageSpan> {
        let mut start = Page::containing_address(VirtualAddress::new(min_address));

        for grant in self.grants.values() {
            let grant_start = grant.start;
            let grant_end = grant.end;

            if grant_start.start_address().data() >= start.start_address().data() {
                let gap_size = grant_start.start_address().data() - start.start_address().data();
                let gap_pages = gap_size / PAGE_SIZE;
                if gap_pages >= page_count {
                    return Some(PageSpan::new(start, page_count));
                }
            }

            if grant_end.start_address().data() > start.start_address().data() {
                start = grant_end;
            }
        }

        // Check gap after last grant up to USER_END_OFFSET
        let user_end = crate::consts::USER_END_OFFSET;
        if user_end >= start.start_address().data() {
            let remaining = user_end - start.start_address().data();
            if remaining / PAGE_SIZE >= page_count {
                return Some(PageSpan::new(start, page_count));
            }
        }

        None
    }
}

impl AddrSpaceInner {
    pub fn r#move(
        &mut self,
        _target: Option<(&Arc<AddrSpaceWrapper>, &mut AddrSpaceInner)>,
        _src: PageSpan,
        _dst: Option<Page>,
        _count: usize,
        _flags: MapFlags,
        _vec: &mut Vec<GrantFileRef>,
    ) -> SysResult<Grant> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

}

impl AddrSpaceInner {
    pub fn mlock(&mut self, start: VirtualAddress, size: usize) -> SysResult<()> {
        let current_context_ref = crate::context::current();
        let mut token = unsafe { CleanLockToken::new() };
        let mut current_context =
            current_context_ref.write(token.token());

        let mut kernel_mapper = crate::memory::KernelMapper::lock();
        let mapper = kernel_mapper
            .get_mut()
            .expect("failed to lock kernel mapper");
        let mut flusher = TlbShootdownActions::new();

        let start_page = Page::containing_address(start);
        let end_page = Page::containing_address(VirtualAddress::new(start.data() + size - 1));

        for page in Page::range_inclusive(start_page, end_page) {
            if let Some(grant) = self.grants.get_mut(&page) {
                if !grant.locked {
                    // Ensure the page is present in physical memory
                    if grant.phys.is_none() {
                        let frame =
                            memory::allocate_frame().ok_or(Error::new(syscall::error::ENOMEM))?;
                        grant.set_phys(frame);
                        unsafe {
                            mapper
                                .map_phys(page.start_address(), frame.base(), grant.flags)
                                .ok_or(Error::new(syscall::error::ENOMEM))?
                                .flush();
                        }
                    }
                    grant.locked = true;
                    current_context.memory_locked_count += 1;
                }
            } else {
                return Err(Error::new(syscall::error::ENOMEM));
            }
        }
        Ok(())
    }

    pub fn munlock(&mut self, start: VirtualAddress, size: usize) -> SysResult<()> {
        let current_context_ref = crate::context::current();
        let mut token = unsafe { CleanLockToken::new() };
        let mut current_context =
            current_context_ref.write(token.token());

        let start_page = Page::containing_address(start);
        let end_page = Page::containing_address(VirtualAddress::new(start.data() + size - 1));

        for page in Page::range_inclusive(start_page, end_page) {
            if let Some(grant) = self.grants.get_mut(&page) {
                if grant.locked {
                    grant.locked = false;
                    current_context.memory_locked_count -= 1;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct GrantFileRef {
    pub base_offset: usize,
    pub description: Arc<RwLock<FileDescription>>,
}

impl GrantFileRef {
    pub fn unmap(&self, _token: &mut CleanLockToken) -> SysResult<()> {
        Ok(())
    }
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
    pub fn empty() -> Self {
        Self { base: Page::containing_address(VirtualAddress::new(0)), count: 0 }
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn validate_nonempty(base: VirtualAddress, size: usize) -> Option<Self> {
        Some(Self {
            base: Page::containing_address(base),
            count: size / PAGE_SIZE,
        })
    }
}


#[derive(Debug)]
pub enum Provider {
    Allocated { flags: PageFlags<RmmA> },
    PhysBorrowed { base: Frame },
    External { address: usize, size: usize },
    FmapBorrowed { file_ref: GrantFileRef },
}

#[derive(Debug)]
pub struct BorrowedFmapSource<'a> {
    pub src_base: Page,
    pub addr_space_lock: Arc<AddrSpaceWrapper>,
    pub addr_space_guard: spin::RwLockWriteGuard<'a, AddrSpaceInner>,
    pub mode: MmapMode,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum MmapMode {
    Cow,
    Shared,
}

pub const DANGLING: usize = 0;

pub fn handle_notify_files(_files: Vec<GrantFileRef>, _token: &mut CleanLockToken) {}

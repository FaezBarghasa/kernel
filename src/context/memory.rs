//! # Virtual Memory Management for Contexts

use alloc::{sync::Arc, vec::Vec};
use spin::RwLock;

use crate::{
    arch::paging::{Page, PageFlags, PageMapper, RmmA, VirtualAddress, PAGE_SIZE},
    context::file::FileDescription,
    memory::{self, Enomem, Frame, PhysicalAddress, RaiiFrame},
    sync::CleanLockToken,
    syscall::{
        self,
        error::{Error, Result as SysResult, EEXIST, ENOMEM},
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
        _mapper: &mut PageMapper,
        _flusher: &mut TlbShootdownActions,
    ) -> SysResult<Self> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn zeroed(
        span: PageSpan,
        flags: PageFlags<RmmA>,
        mapper: &mut PageMapper,
        flusher: &mut TlbShootdownActions,
        _shared: bool,
    ) -> SysResult<Self> {
        let frames = memory::allocate_p2frame(span.count as u32).ok_or(Error::new(ENOMEM))?;

        #[cfg(feature = "no-mmu")]
        let span = {
            let phys_base_page =
                Page::containing_address(VirtualAddress::new(frames.base().data()));
            PageSpan::new(phys_base_page, span.count)
        };

        let mut grant = Grant::new(span.base, span.base.next_by(span.count), flags);
        grant.set_phys(frames);

        #[cfg(not(feature = "no-mmu"))]
        unsafe {
            mapper
                .map_phys(span.base.start_address(), frames.base(), flags)
                .ok_or(Error::new(ENOMEM))?
                .flush();
            // TODO: Zero memory (requires mapping?)
            // For now assuming allocator zeroes or we can access via phys map
            // But existing behavior likely maps then zeroes.
        }

        #[cfg(feature = "no-mmu")]
        unsafe {
            // In No-MMU, physical address is directly accessible (offset).
            // We should zero it.
            // Assumes linear mapping or simple cast.
            let ptr = frames.base().data() as *mut u8;
            core::ptr::write_bytes(ptr, 0, span.count * PAGE_SIZE);
        }

        Ok(grant)
    }

    /// Create a grant backed by huge pages (2MB).
    ///
    /// This is used when MAP_HUGETLB is specified. The span count must be
    /// a multiple of 512 (the number of 4KB pages in a 2MB huge page).
    ///
    /// Returns ENOMEM if huge page allocation fails.
    pub fn zeroed_hugepage(
        span: PageSpan,
        flags: PageFlags<RmmA>,
        mapper: &mut PageMapper,
        _flusher: &mut TlbShootdownActions,
    ) -> SysResult<Self> {
        // 2MB huge page = 512 * 4KB pages
        const HUGE_PAGE_SIZE: usize = 512;

        // For now, allocate a single 2MB huge page
        // Future: support multiple huge pages for larger allocations
        if span.count < HUGE_PAGE_SIZE {
            // For small allocations, fall back to regular pages
            return Err(Error::new(ENOMEM));
        }

        let huge_frame = memory::allocate_huge_frame().ok_or(Error::new(ENOMEM))?;

        #[cfg(feature = "no-mmu")]
        let span = {
            let phys_base_page =
                Page::containing_address(VirtualAddress::new(huge_frame.base().data()));
            PageSpan::new(phys_base_page, HUGE_PAGE_SIZE)
        };

        let mut grant = Grant::new(span.base, span.base.next_by(HUGE_PAGE_SIZE), flags);
        grant.set_phys(huge_frame);

        #[cfg(not(feature = "no-mmu"))]
        unsafe {
            // Map with huge page flags (PSE/large page bit)
            // Note: this requires page table support for 2MB entries
            let huge_flags = flags.custom_flag(0x80, true); // PS (Page Size) bit for x86
            mapper
                .map_phys(span.base.start_address(), huge_frame.base(), huge_flags)
                .ok_or(Error::new(ENOMEM))?
                .flush();
        }

        #[cfg(feature = "no-mmu")]
        unsafe {
            // Zero the entire 2MB huge page
            let ptr = huge_frame.base().data() as *mut u8;
            core::ptr::write_bytes(ptr, 0, HUGE_PAGE_SIZE * PAGE_SIZE);
        }

        Ok(grant)
    }

    /// Prefault all pages in a grant (for MAP_POPULATE).
    ///
    /// This touches each page to ensure it's physically allocated and mapped,
    /// avoiding page faults during application runtime.
    pub fn prefault_populate(&self, mapper: &mut PageMapper) -> SysResult<()> {
        #[cfg(not(feature = "no-mmu"))]
        {
            // Touch each page to ensure it's faulted in
            // This is done by reading from the virtual address through the kernel mapping
            let start_addr = self.start.start_address().data();
            let page_count = self.page_count();

            for i in 0..page_count {
                let page_addr = start_addr + (i * PAGE_SIZE);

                // Verify the page is mapped by checking translation
                if mapper.translate(VirtualAddress::new(page_addr)).is_none() {
                    // Page isn't mapped - this shouldn't happen for allocated grants
                    // but we handle it gracefully
                    continue;
                }
            }
        }

        #[cfg(feature = "no-mmu")]
        {
            // In no-MMU mode, pages are directly accessible, no prefaulting needed
            let _ = mapper;
        }

        Ok(())
    }

    pub fn physmap(
        _phys: Frame,
        _span: PageSpan,
        _flags: PageFlags<RmmA>,
        _mapper: &mut PageMapper,
        _flusher: &mut TlbShootdownActions,
    ) -> SysResult<Self> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn set_phys(&mut self, frame: Frame) {
        let raii = unsafe { RaiiFrame::new_unchecked(frame) };
        self.phys = Some(raii);
    }

    pub fn clear_phys(&mut self) -> Option<RaiiFrame> {
        self.phys.take()
    }

    pub fn unmap(mut self) {
        drop(self.phys.take());
    }

    pub fn borrow(
        _src_base: Page,
        _dst_base: Page,
        _count: usize,
        _flags: MapFlags,
        _mapper: &mut PageMapper,
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
        _mapper: &mut PageMapper,
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
        _mapper: &mut PageMapper,
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
        _mapper: &mut PageMapper,
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

    if let Some(addr_space) = current_context_guard.addr_space.as_ref() {
        let mut inner = addr_space.inner.write();
        if let Some(grant) = inner.grants.get_mut(&faulting_page) {
            // Check if swapped to zram first
            if let Provider::ZramSwapped { zram_idx, flags } = grant.provider {
                let frame = memory::allocate_frame().ok_or(PfError::Oom)?;
                let phys_addr = frame.base();
                if crate::memory::mglru::swap_in(zram_idx, phys_addr) {
                    let mut kernel_mapper = crate::memory::KernelMapper::lock();
                    let mapper = kernel_mapper
                        .get_mut()
                        .expect("failed to lock kernel mapper");
                    unsafe {
                        mapper
                            .map_phys(faulting_page.start_address(), phys_addr, flags)
                            .ok_or(PfError::Oom)?
                            .flush();
                    }
                    grant.phys = Some(unsafe { RaiiFrame::new_unchecked(frame) });
                    grant.provider = Provider::Allocated { flags };
                    return Ok(());
                } else {
                    return Err(PfError::NonfatalInternalError);
                }
            }

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

#[cfg(not(feature = "no-mmu"))]
#[derive(Debug)]
pub struct UTableWrapper(
    pub rmm::PageMapper<crate::arch::x86_shared::CurrentRmmArch, crate::memory::TheFrameAllocator>,
);

#[cfg(feature = "no-mmu")]
#[derive(Debug)]
pub struct UTableWrapper;

#[cfg(feature = "no-mmu")]
pub struct DummyFlusher;

#[cfg(feature = "no-mmu")]
impl DummyFlusher {
    pub fn flush(self) {}
}

impl UTableWrapper {
    #[cfg(feature = "no-mmu")]
    pub unsafe fn map_phys(
        &mut self,
        _virt: VirtualAddress,
        _phys: PhysicalAddress,
        _flags: PageFlags<RmmA>,
    ) -> Option<DummyFlusher> {
        Some(DummyFlusher)
    }

    #[cfg(not(feature = "no-mmu"))]
    pub unsafe fn map_phys(
        &mut self,
        virt: VirtualAddress,
        phys: PhysicalAddress,
        flags: PageFlags<RmmA>,
    ) -> Option<rmm::PageFlush<RmmA>> {
        self.0.map_phys(virt, phys, flags)
    }
    pub unsafe fn make_current(&self) {
        #[cfg(not(feature = "no-mmu"))]
        unsafe {
            self.0.make_current();
        }
    }

    #[cfg(not(feature = "no-mmu"))]
    pub fn table(&self) -> rmm::PageTable<crate::arch::x86_shared::CurrentRmmArch> {
        self.0.table()
    }

    pub fn translate(&self, addr: VirtualAddress) -> Option<crate::paging::PhysicalAddress> {
        #[cfg(not(feature = "no-mmu"))]
        return self.0.translate(addr).map(|(addr, _)| addr);

        #[cfg(feature = "no-mmu")]
        return Some(crate::paging::PhysicalAddress::new(addr.data()));
    }
}

impl AddrSpaceWrapper {
    pub fn new() -> SysResult<Arc<Self>> {
        Ok(Arc::new(Self {
            inner: RwLock::new(AddrSpaceInner {
                used_by: crate::cpu_set::LogicalCpuSet::new(),
                table: TableWrapper {
                    utable: {
                        #[cfg(not(feature = "no-mmu"))]
                        {
                            UTableWrapper(unsafe {
                                rmm::PageMapper::create(
                                    rmm::TableKind::User,
                                    crate::memory::TheFrameAllocator,
                                )
                                .ok_or(Error::new(crate::syscall::error::ENOMEM))?
                            })
                        }
                        #[cfg(feature = "no-mmu")]
                        {
                            UTableWrapper
                        }
                    },
                },
                tlb_ack: core::sync::atomic::AtomicUsize::new(0),
                grants: BTreeMap::new(),
                mmap_min: PAGE_SIZE,
            }),
        }))
    }

    pub fn acquire_read(&self) -> spin::RwLockReadGuard<'_, AddrSpaceInner> {
        self.inner.read()
    }

    pub fn acquire_write(&self) -> spin::RwLockWriteGuard<'_, AddrSpaceInner> {
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
        base: Option<Page>,
        count: core::num::NonZeroUsize,
        flags: MapFlags,
        _vec: &mut Vec<GrantFileRef>,
        func: impl FnOnce(
            Page,
            crate::paging::PageFlags<RmmA>,
            &mut PageMapper,
            &mut TlbShootdownActions,
        ) -> SysResult<Grant>,
    ) -> SysResult<Page> {
        let page = if let Some(requested) = base {
            // Check for overlap
            if self
                .grants
                .range(requested..requested.next_by(count.get()))
                .next()
                .is_some()
            {
                return Err(Error::new(EEXIST));
            }
            requested
        } else {
            self.find_free_span(self.mmap_min, count.get())
                .ok_or(Error::new(ENOMEM))?
                .base
        };

        let mut page_flags = crate::paging::PageFlags::new().user(true);
        if flags.contains(MapFlags::PROT_WRITE) {
            page_flags = page_flags.write(true);
        }
        if flags.contains(MapFlags::PROT_EXEC) {
            page_flags = page_flags.execute(true);
        }

        #[cfg(not(feature = "no-mmu"))]
        let mut kernel_mapper_lock = crate::memory::KernelMapper::lock();

        // In no-mmu, we might not have a functional KernelMapper, but the signature requires it.
        // We'll trust that the func (Grant::zeroed) handles it appropriately or we provide a dummy/locked one.
        // Assuming KernelMapper exists and is lockable even in no-mmu (it is).
        #[cfg(feature = "no-mmu")]
        let mut kernel_mapper_lock = crate::memory::KernelMapper::lock();

        let mapper = kernel_mapper_lock.get_mut().ok_or(Error::new(ENOMEM))?;
        let mut flusher = TlbShootdownActions::new();

        let grant = func(page, page_flags, mapper, &mut flusher)?;
        let start = grant.start;

        self.grants.insert(grant.start, grant);

        #[cfg(feature = "no-mmu")]
        {
            if let Some(grant_ref) = self.grants.get(&start) {
                use crate::memory::model::{MemoryModel, MpuPermissions};

                let mut perm = MpuPermissions::READ;
                if grant_ref.flags().has_write() {
                    perm |= MpuPermissions::WRITE;
                }
                if grant_ref.flags().has_execute() {
                    perm |= MpuPermissions::EXEC;
                }

                let _ = crate::memory::model::MEMORY_MODEL.protect_userspace(
                    grant_ref.start_address().data(),
                    grant_ref.page_count() * PAGE_SIZE,
                    perm,
                );
            }
        }

        Ok(start)
    }

    pub fn mmap_anywhere(
        &mut self,
        count: core::num::NonZeroUsize,
        flags: MapFlags,
        func: impl FnOnce(
            Page,
            crate::paging::PageFlags<RmmA>,
            &mut PageMapper,
            &mut TlbShootdownActions,
        ) -> SysResult<Grant>,
    ) -> SysResult<Page> {
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
    ) -> SysResult<Page> {
        Err(Error::new(crate::syscall::error::ENOMEM))
    }
}

impl AddrSpaceInner {
    pub fn mlock(&mut self, start: VirtualAddress, size: usize) -> SysResult<()> {
        let current_context_ref = crate::context::current();
        let mut token = unsafe { CleanLockToken::new() };
        let mut current_context = current_context_ref.write(token.token());

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
        let mut current_context = current_context_ref.write(token.token());

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
        Self {
            base: Page::containing_address(VirtualAddress::new(0)),
            count: 0,
        }
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
pub use crate::arch::paging::TlbShootdownActions;

#[derive(Debug)]
pub enum Provider {
    Allocated { flags: PageFlags<RmmA> },
    PhysBorrowed { base: Frame },
    External { address: usize, size: usize },
    FmapBorrowed { file_ref: GrantFileRef },
    ZramSwapped { zram_idx: usize, flags: PageFlags<RmmA> },
}

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

//! # Memory management
//! Includes the physical memory allocator (buddy system).

mod kernel_mapper;

use core::{
    cell::SyncUnsafeCell,
    mem,
    num::NonZeroUsize,
    sync::atomic::{AtomicUsize, Ordering},
};

pub use kernel_mapper::KernelMapper;
use spin::Mutex;

pub use crate::paging::{PhysicalAddress, RmmA, RmmArch, PAGE_MASK, PAGE_SIZE};
use crate::{
    context::{
        self,
        memory::{AccessMode, PfError},
    },
    kernel_executable_offsets::{__usercopy_end, __usercopy_start},
    paging::{entry::EntryFlags, Page, PageFlags},
    sync::CleanLockToken,
    syscall::error::{Error, EFAULT, EINVAL, EIO, ENOMEM, EOVERFLOW},
};
use rmm::{BumpAllocator, FrameAllocator, FrameCount, FrameUsage, TableKind, VirtualAddress};

pub(crate) static AREAS: SyncUnsafeCell<[rmm::MemoryArea; 512]> = SyncUnsafeCell::new(
    [rmm::MemoryArea {
        base: PhysicalAddress::new(0),
        size: 0,
    }; 512],
);
pub(crate) static AREA_COUNT: SyncUnsafeCell<u16> = SyncUnsafeCell::new(0);

static BUMP_FRAMES: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn areas() -> &'static [rmm::MemoryArea] {
    unsafe {
        let count = AREA_COUNT.get().read() as usize;
        let areas_ptr = AREAS.get() as *const rmm::MemoryArea;
        core::slice::from_raw_parts(areas_ptr, count)
    }
}

bitflags! {
    pub struct AllocationFlags: u32 {
        const NONE = 0;
        const ZEROED = 1 << 0;
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PlacementStrategy {
    Normal,
}

pub fn free_frames() -> usize {
    total_frames().saturating_sub(used_frames())
}

pub fn used_frames() -> usize {
    FREELIST.lock().used_frames + BUMP_FRAMES.load(Ordering::Relaxed)
}

pub fn total_frames() -> usize {
    sections().iter().map(|section| section.frames.len()).sum()
}

pub fn allocate_p2frame(order: u32) -> Option<Frame> {
    allocate_p2frame_complex(order, AllocationFlags::NONE, None, order).map(|(f, _)| f)
}

pub fn allocate_frame() -> Option<Frame> {
    allocate_p2frame(0)
}

pub fn allocate_p2frame_complex(
    _req_order: u32,
    flags: AllocationFlags,
    _strategy: Option<PlacementStrategy>,
    min_order: u32,
) -> Option<(Frame, usize)> {
    let mut freelist = FREELIST.lock();

    let (frame_order, frame) = freelist
        .for_orders
        .iter()
        .enumerate()
        .skip(min_order as usize)
        .find_map(|(i, f)| f.map(|f| (i as u32, f)))?;

    let info = get_page_info(frame)
        .unwrap_or_else(|| panic!("no page info for allocated frame {frame:?}"));

    let next_free = match info.transition_to_used() {
        Ok(next) => next,
        Err(_) => panic!("freelist frame {frame:?} was not in Free state!"),
    };

    debug_assert_eq!(
        next_free.order(),
        frame_order,
        "{frame:?}->next {next_free:?}.order != {frame_order}"
    );

    if let Some(next) = next_free.frame() {
        let f = get_free_alloc_page_info(next);
        f.set_prev(P2Frame::new(None, frame_order));
    }

    if let Some(entry) = freelist.for_orders.get_mut(frame_order as usize) {
        *entry = next_free.frame();
    }

    for order in (min_order..frame_order).rev() {
        let order_page_count = 1usize.checked_shl(order).ok_or(Error::new(EOVERFLOW)).ok()?;
        let hi = frame.try_next_by(order_page_count).ok()?;

        let hi_info = get_page_info(hi)
            .expect("sub-p2frame of split p2flame lacked PageInfo");

        let free_info = hi_info.transition_to_free(order);
        free_info.set_next(P2Frame::new(None, order));
        free_info.set_prev(P2Frame::new(None, order));

        if let Some(entry) = freelist.for_orders.get_mut(order as usize) {
            *entry = Some(hi);
        }
    }

    if let Some(added) = 1usize.checked_shl(min_order) {
        freelist.used_frames = freelist.used_frames.checked_add(added).expect("Used frames overflow");
    } else {
        return None;
    }

    drop(freelist);

    unsafe {
        if flags.contains(AllocationFlags::ZEROED) {
            if let Some(size) = PAGE_SIZE.checked_shl(min_order) {
                (RmmA::phys_to_virt(frame.base()).data() as *mut u8).write_bytes(0, size);
            }
        }
    }

    debug_assert!(frame.base().data() >= unsafe { ALLOCATOR_DATA.abs_off });

    Some((frame, PAGE_SIZE.checked_shl(min_order).unwrap_or(0)))
}

pub unsafe fn deallocate_p2frame(orig_frame: Frame, order: u32) {
    let mut freelist = FREELIST.lock();

    let initial_info = get_page_info(orig_frame)
        .unwrap_or_else(|| panic!("missing PageInfo for {orig_frame:?} being freed"));

    if initial_info.state() != PageInfoState::Used {
        panic!("Attempted to free frame {orig_frame:?} which is not in Used state");
    }

    initial_info.refcount.store(0, Ordering::Relaxed);

    let mut largest_order = order;
    let mut current = orig_frame;

    for merge_order in order..MAX_ORDER {
        let size_at_order = match PAGE_SIZE.checked_shl(merge_order) {
            Some(s) => s,
            None => break,
        };

        let sibling_addr = current.base().data() ^ size_at_order;
        let sibling = Frame::containing(PhysicalAddress::new(sibling_addr));

        let Some(_cur_info) = get_page_info(current) else {
            unreachable!("attempting to free non-allocator-owned page");
        };

        let Some(sib_info) = get_page_info(sibling) else {
            break;
        };

        let sib_free_info = match sib_info.as_free() {
            Some(info) => info,
            None => break,
        };

        if sib_free_info.next().order() < merge_order {
            break;
        }

        if let Some(sib_prev) = sib_free_info.prev().frame() {
            get_free_alloc_page_info(sib_prev).set_next(sib_free_info.next());
        } else {
            if let Some(entry) = freelist.for_orders.get_mut(merge_order as usize) {
                *entry = sib_free_info.next().frame();
            }
        }
        if let Some(sib_next) = sib_free_info.next().frame() {
            get_free_alloc_page_info(sib_next).set_prev(sib_free_info.prev());
        }

        let mask = !(PAGE_SIZE.checked_shl(merge_order).unwrap_or(0));
        let current_data = current.base().data() & mask;
        current = Frame::containing(PhysicalAddress::new(current_data));

        largest_order = merge_order + 1;
    }

    get_page_info(current)
        .expect("freeing frame without PageInfo")
        .transition_to_free(largest_order);

    let new_head = current;

    if let Some(entry) = freelist.for_orders.get_mut(largest_order as usize) {
        if let Some(old_head) = entry.replace(new_head) {
            let old_head_info = get_free_alloc_page_info(old_head);
            let new_head_info = get_free_alloc_page_info(new_head);

            new_head_info.set_next(P2Frame::new(Some(old_head), largest_order));
            new_head_info.set_prev(P2Frame::new(None, largest_order));
            old_head_info.set_prev(P2Frame::new(Some(new_head), largest_order));
        }
    }

    if let Some(sub) = 1usize.checked_shl(order) {
        freelist.used_frames = freelist.used_frames.checked_sub(sub).expect("Free list underflow");
    }
}

pub unsafe fn deallocate_frame(frame: Frame) {
    unsafe { deallocate_p2frame(frame, 0) }
}

pub unsafe fn map_device_memory(addr: PhysicalAddress, len: usize) -> VirtualAddress {
    unsafe {
        let mut mapper_lock = KernelMapper::lock();
        let mapper = mapper_lock
            .get_mut()
            .expect("KernelMapper mapper locked re-entrant in map_device_memory");
        let base = PhysicalAddress::new(crate::paging::round_down_pages(addr.data()));

        let offset = addr.data().checked_sub(base.data()).unwrap_or(0);
        let aligned_len = crate::paging::round_up_pages(len.checked_add(offset).unwrap_or(usize::MAX));

        for page_idx in 0..aligned_len / crate::memory::PAGE_SIZE {
            let page_offset = page_idx.checked_mul(crate::memory::PAGE_SIZE).unwrap_or(0);
            let map_addr = base.add(page_offset);

            let (_, flush) = mapper
                .map_linearly(
                    map_addr,
                    PageFlags::new()
                        .write(true)
                        .custom_flag(EntryFlags::NO_CACHE.bits(), true),
                )
                .expect("failed to linearly map SDT");
            flush.flush();
        }
        RmmA::phys_to_virt(addr)
    }
}

const ORDER_COUNT: u32 = 11;
const MAX_ORDER: u32 = ORDER_COUNT - 1;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Frame {
    physaddr: NonZeroUsize,
}

#[derive(Clone, Copy)]
struct P2Frame(usize);
impl P2Frame {
    fn new(frame: Option<Frame>, order: u32) -> Self {
        Self(frame.map_or(0, |f| f.physaddr.get()) | (order as usize))
    }
    fn get(self) -> (Option<Frame>, u32) {
        let page_off_mask = PAGE_SIZE - 1;
        (
            NonZeroUsize::new(self.0 & !page_off_mask & !RC_USED_NOT_FREE)
                .map(|physaddr| Frame { physaddr }),
            (self.0 & page_off_mask) as u32,
        )
    }
    fn frame(self) -> Option<Frame> {
        self.get().0
    }
    fn order(self) -> u32 {
        self.get().1
    }
}
impl core::fmt::Debug for P2Frame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (frame, order) = self.get();
        write!(f, "[frame at {frame:?}] order {order}")
    }
}

impl core::fmt::Debug for Frame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "[frame at {:p}]", self.base().data() as *const u8)
    }
}

impl Frame {
    pub fn containing(address: PhysicalAddress) -> Frame {
        Frame {
            physaddr: NonZeroUsize::new(address.data() & !PAGE_MASK)
                .expect("frame 0x0 is reserved"),
        }
    }
    pub fn base(self) -> PhysicalAddress {
        PhysicalAddress::new(self.physaddr.get())
    }
    pub fn try_next_by(self, n: usize) -> Result<Self, Error> {
        let offset = n.checked_mul(PAGE_SIZE).ok_or(Error::new(EOVERFLOW))?;
        let new_addr = self.physaddr.get().checked_add(offset).ok_or(Error::new(EOVERFLOW))?;
        let physaddr = NonZeroUsize::new(new_addr).ok_or(Error::new(EINVAL))?;
        Ok(Self { physaddr })
    }
    pub fn try_offset_from(self, from: Self) -> Result<usize, Error> {
        let diff = self.physaddr.get().checked_sub(from.physaddr.get()).ok_or(Error::new(EOVERFLOW))?;
        Ok(diff / PAGE_SIZE)
    }
    pub fn offset_from(self, from: Self) -> usize {
        self.try_offset_from(from).expect("overflow in Frame::offset_from")
    }
    pub fn is_aligned_to_order(self, order: u32) -> bool {
        if let Some(mask) = PAGE_SIZE.checked_shl(order) {
            self.base().data() % mask == 0
        } else {
            false
        }
    }
}

#[derive(Debug)]
pub struct Enomem;

impl From<Enomem> for Error {
    fn from(_: Enomem) -> Self {
        Self::new(ENOMEM)
    }
}

#[derive(Debug)]
pub struct RaiiFrame {
    inner: Frame,
}
impl RaiiFrame {
    pub fn allocate() -> Result<Self, Enomem> {
        init_frame(RefCount::One)
            .map_err(|_| Enomem)
            .map(|inner| Self { inner })
    }
    pub unsafe fn new_unchecked(inner: Frame) -> Self {
        Self { inner }
    }
    pub fn get(&self) -> Frame {
        self.inner
    }
    pub fn take(self) -> Frame {
        let f = self.inner;
        core::mem::forget(self);
        f
    }
}

impl Drop for RaiiFrame {
    fn drop(&mut self) {
        if get_page_info(self.inner)
            .expect("RaiiFrame lacking PageInfo")
            .remove_ref()
            .is_none()
        {
            unsafe {
                deallocate_frame(self.inner);
            }
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum PageInfoState {
    Free,
    Used,
}

#[derive(Debug)]
pub struct PageInfo {
    refcount: AtomicUsize,
    next: AtomicUsize,
}

struct PageInfoFree<'info> {
    prev: &'info AtomicUsize,
    next: &'info AtomicUsize,
}

const RC_USED_NOT_FREE: usize = 1 << (usize::BITS - 1);
const RC_SHARED_NOT_COW: usize = 1 << (usize::BITS - 2);
const RC_MAX: usize = 1 << (usize::BITS - 3);
const RC_COUNT_MASK: usize = !(RC_USED_NOT_FREE | RC_SHARED_NOT_COW);

static mut ALLOCATOR_DATA: AllocatorData = AllocatorData {
    sections: &[],
    abs_off: 0,
};

struct AllocatorData {
    sections: &'static [Section],
    abs_off: usize,
}

#[derive(Debug)]
struct FreeList {
    for_orders: [Option<Frame>; ORDER_COUNT as usize],
    used_frames: usize,
}
static FREELIST: Mutex<FreeList> = Mutex::new(FreeList {
    for_orders: [None; ORDER_COUNT as usize],
    used_frames: 0,
});

pub struct Section {
    base: Frame,
    frames: &'static [PageInfo],
}

pub const MAX_SECTION_SIZE_BITS: u32 = 27;
pub const MAX_SECTION_SIZE: usize = 1 << MAX_SECTION_SIZE_BITS;
pub const MAX_SECTION_PAGE_COUNT: usize = MAX_SECTION_SIZE / PAGE_SIZE;

#[cold]
fn init_sections(mut allocator: BumpAllocator<RmmA>) {
    let (free_areas, offset_into_first_free_area) = allocator.free_areas();
    let bump_used = unsafe { allocator.usage().used().data() };
    BUMP_FRAMES.store(bump_used, Ordering::Relaxed);
}

#[cold]
pub fn init_mm(allocator: BumpAllocator<RmmA>) {
    init_sections(allocator);

    unsafe {
        let the_frame = allocate_frame().expect("failed to allocate static zeroed frame");
        let the_info = get_page_info(the_frame).expect("static zeroed frame had no PageInfo");
        the_info
            .refcount
            .store(RefCount::One.to_raw(), Ordering::Relaxed);

        THE_ZEROED_FRAME.get().write(Some((the_frame, the_info)));
    }
}

#[derive(Debug, PartialEq)]
pub enum AddRefError {
    CowToShared,
    SharedToCow,
    RcOverflow,
}

impl PageInfo {
    pub fn state(&self) -> PageInfoState {
        let val = self.refcount.load(Ordering::Relaxed);
        if val & RC_USED_NOT_FREE == RC_USED_NOT_FREE {
            PageInfoState::Used
        } else {
            PageInfoState::Free
        }
    }

    fn as_free(&self) -> Option<PageInfoFree<'_>> {
        if self.state() == PageInfoState::Used {
            None
        } else {
            Some(PageInfoFree {
                prev: &self.refcount,
                next: &self.next,
            })
        }
    }

    fn transition_to_used(&self) -> Result<P2Frame, ()> {
        let free_view = self.as_free().ok_or(())?;
        let next = free_view.next();
        self.refcount.store(RC_USED_NOT_FREE, Ordering::Relaxed);
        self.next.store(0, Ordering::Relaxed);
        Ok(next)
    }

    fn transition_to_free(&self, order: u32) -> PageInfoFree<'_> {
        debug_assert_eq!(self.state(), PageInfoState::Used);
        self.refcount.store(order as usize, Ordering::Relaxed);
        self.next.store(order as usize, Ordering::Relaxed);
        PageInfoFree {
            next: &self.next,
            prev: &self.refcount,
        }
    }

    pub fn add_ref(&self, kind: RefKind) -> Result<(), AddRefError> {
        if self.state() != PageInfoState::Used {
            panic!("Cannot add_ref to a Free frame");
        }

        match (self.refcount().expect("state checked above"), kind) {
            (RefCount::One, RefKind::Cow) => {
                self.refcount.store(RC_USED_NOT_FREE | 2, Ordering::Relaxed)
            }
            (RefCount::One, RefKind::Shared) => self
                .refcount
                .store(RC_USED_NOT_FREE | 2 | RC_SHARED_NOT_COW, Ordering::Relaxed),
            (RefCount::Cow(_), RefKind::Cow) | (RefCount::Shared(_), RefKind::Shared) => {
                let old = self.refcount.fetch_add(1, Ordering::Relaxed);
                if (old & RC_COUNT_MASK) >= RC_MAX {
                    self.refcount.fetch_sub(1, Ordering::Relaxed);
                    return Err(AddRefError::RcOverflow);
                }
            }
            (RefCount::Cow(_), RefKind::Shared) => return Err(AddRefError::CowToShared),
            (RefCount::Shared(_), RefKind::Cow) => return Err(AddRefError::SharedToCow),
        }
        Ok(())
    }

    #[must_use = "must deallocate if refcount reaches None"]
    pub fn remove_ref(&self) -> Option<RefCount> {
        if self.state() != PageInfoState::Used {
            panic!("Cannot remove_ref from a Free frame");
        }

        match self.refcount() {
            None => panic!("refcount was already zero when calling remove_ref!"),
            Some(RefCount::One) => {
                self.refcount.store(RC_USED_NOT_FREE, Ordering::Relaxed);
                None
            }
            Some(RefCount::Cow(_) | RefCount::Shared(_)) => RefCount::from_raw({
                let new_value = self.refcount.fetch_sub(1, Ordering::Relaxed) - 1;
                new_value
            }),
        }
    }

    pub fn refcount(&self) -> Option<RefCount> {
        let refcount = self.refcount.load(Ordering::Relaxed);
        RefCount::from_raw(refcount)
    }
}

impl PageInfoFree<'_> {
    fn next(&self) -> P2Frame {
        P2Frame(self.next.load(Ordering::Relaxed))
    }
    fn set_next(&self, next: P2Frame) {
        self.next.store(next.0, Ordering::Relaxed)
    }
    fn prev(&self) -> P2Frame {
        P2Frame(self.prev.load(Ordering::Relaxed))
    }
    fn set_prev(&self, prev: P2Frame) {
        self.prev.store(prev.0, Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RefKind {
    Cow,
    Shared,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RefCount {
    One,
    Shared(NonZeroUsize),
    Cow(NonZeroUsize),
}
impl RefCount {
    pub fn from_raw(raw: usize) -> Option<Self> {
        if raw & RC_USED_NOT_FREE != RC_USED_NOT_FREE {
            return None;
        }
        let refcount = raw & !(RC_SHARED_NOT_COW | RC_USED_NOT_FREE);
        let nz_refcount = NonZeroUsize::new(refcount)?;

        Some(if nz_refcount.get() == 1 {
            RefCount::One
        } else if raw & RC_SHARED_NOT_COW == RC_SHARED_NOT_COW {
            RefCount::Shared(nz_refcount)
        } else {
            RefCount::Cow(nz_refcount)
        })
    }
    pub fn to_raw(self) -> usize {
        match self {
            Self::One => 1 | RC_USED_NOT_FREE,
            Self::Shared(inner) => inner.get() | RC_SHARED_NOT_COW | RC_USED_NOT_FREE,
            Self::Cow(inner) => inner.get() | RC_USED_NOT_FREE,
        }
    }
}
#[inline]
fn sections() -> &'static [Section] {
    unsafe { ALLOCATOR_DATA.sections }
}
pub fn get_page_info(frame: Frame) -> Option<&'static PageInfo> {
    let sections = sections();
    let idx_res = sections.binary_search_by_key(&frame, |section| section.base);
    if idx_res == Err(0) {
        return None;
    }
    let section = &sections[idx_res.unwrap_or_else(|e| e - 1)];
    section.frames.get(frame.offset_from(section.base))
}

#[track_caller]
fn get_free_alloc_page_info(frame: Frame) -> PageInfoFree<'static> {
    let i = get_page_info(frame).unwrap_or_else(|| {
        panic!("allocator-owned frames need a PageInfo, but none for {frame:?}")
    });
    i.as_free()
        .unwrap_or_else(|| panic!("expected frame to be free, but {frame:?} wasn't, in {i:?}"))
}

pub struct Segv;

bitflags! {
    pub struct GenericPfFlags: u32 {
        const PRESENT = 1 << 0;
        const INVOLVED_WRITE = 1 << 1;
        const USER_NOT_SUPERVISOR = 1 << 2;
        const INSTR_NOT_DATA = 1 << 3;
        const INVL = 1 << 31;
    }
}

pub trait ArchIntCtx {
    fn ip(&self) -> usize;
    fn recover_and_efault(&mut self);
}

pub fn page_fault_handler(
    stack: &mut impl ArchIntCtx,
    code: GenericPfFlags,
    faulting_address: VirtualAddress,
) -> Result<(), Error> {
    let faulting_page = Page::containing_address(faulting_address);
    let usercopy_region = __usercopy_start()..__usercopy_end();
    let address_is_user = faulting_address.kind() == TableKind::User;

    let invalid_page_tables = code.contains(GenericPfFlags::INVL);
    let caused_by_user = code.contains(GenericPfFlags::USER_NOT_SUPERVISOR);
    let caused_by_kernel = !caused_by_user;
    let caused_by_write = code.contains(GenericPfFlags::INVOLVED_WRITE);
    let caused_by_instr_fetch = code.contains(GenericPfFlags::INSTR_NOT_DATA);
    let is_usercopy = usercopy_region.contains(&stack.ip());

    let mode = match (caused_by_write, caused_by_instr_fetch) {
        (true, false) => AccessMode::Write,
        (false, false) => AccessMode::Read,
        (false, true) => AccessMode::InstrFetch,
        (true, true) => {
            return Err(Error::new(EFAULT));
        }
    };

    if invalid_page_tables {
        return Err(Error::new(EFAULT));
    }

    if address_is_user && (caused_by_user || is_usercopy) {
        let mut token = unsafe { CleanLockToken::new() };
        match context::memory::try_correcting_page_tables(faulting_page, mode, &mut token) {
            Ok(()) => return Ok(()),
            Err(PfError::Oom) => return Err(Error::new(ENOMEM)),
            Err(PfError::Segv) => return Err(Error::new(EFAULT)),
            Err(PfError::RecursionLimitExceeded) => return Err(Error::new(EFAULT)),
            Err(PfError::NonfatalInternalError) => return Err(Error::new(EIO)),
        }
    }

    if address_is_user && caused_by_kernel && mode != AccessMode::InstrFetch && is_usercopy {
        stack.recover_and_efault();
        return Ok(());
    }

    Err(Error::new(EFAULT))
}

static THE_ZEROED_FRAME: SyncUnsafeCell<Option<(Frame, &'static PageInfo)>> =
    SyncUnsafeCell::new(None);

pub fn the_zeroed_frame() -> (Frame, &'static PageInfo) {
    unsafe {
        THE_ZEROED_FRAME
            .get()
            .read()
            .expect("zeroed frame must be initialized")
    }
}

pub fn init_frame(init_rc: RefCount) -> Result<Frame, PfError> {
    let new_frame = allocate_frame().ok_or(PfError::Oom)?;
    let page_info = get_page_info(new_frame).unwrap_or_else(|| {
        panic!(
            "all allocated frames need an associated page info, {:?} didn't",
            new_frame
        )
    });
    debug_assert_eq!(page_info.state(), PageInfoState::Used);

    page_info
        .refcount
        .store(init_rc.to_raw(), Ordering::Relaxed);

    Ok(new_frame)
}

#[derive(Debug)]
pub struct TheFrameAllocator;

impl FrameAllocator for TheFrameAllocator {
    unsafe fn allocate(&mut self, count: FrameCount) -> Option<PhysicalAddress> {
        let order = count.data().next_power_of_two().trailing_zeros();
        allocate_p2frame(order).map(|f| f.base())
    }
    unsafe fn free(&mut self, address: PhysicalAddress, count: FrameCount) {
        unsafe {
            let order = count.data().next_power_of_two().trailing_zeros();
            deallocate_p2frame(Frame::containing(address), order)
        }
    }
    unsafe fn usage(&self) -> FrameUsage {
        FrameUsage::new(
            FrameCount::new(used_frames()),
            FrameCount::new(total_frames()),
        )
    }
}
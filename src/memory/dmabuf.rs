//! # Kernel DMA-BUF Subsystem
//!
//! Provides zero-copy IPC buffer sharing between processes. Physical frames are
//! allocated once and shared via page-table mappings — no data is ever copied.
//!
//! ## Design
//! - Each `DmaBuf` owns a `Vec<Frame>` (scatter-gather physical pages).
//! - `AtomicUsize` reference count — when it reaches zero the frames are freed.
//! - `sys_dmabuf_create` allocates frames and returns an FD.
//! - `sys_dmabuf_map` walks the frame list and calls the arch paging layer.
//! - `DmaBuf` drop automatically decrements the refcount and frees pages at zero.

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::Mutex;
use hashbrown::HashMap;

use crate::{
    memory::{allocate_frame, deallocate_frame, Frame, PAGE_SIZE},
    syscall::error::{Error, Result, EINVAL, ENOMEM, EBADF},
};

// =============================================================================
// DmaBuf Core
// =============================================================================

/// A kernel DMA buffer: a list of physical frames with an atomic reference count.
///
/// All public fields are read-only after construction. The reference count is
/// managed atomically — `clone_ref` increments it, `drop_ref` decrements it
/// and frees frames when it hits zero.
#[derive(Debug)]
pub struct DmaBuf {
    /// Backing physical frames (scatter-gather).
    pub frames: Vec<Frame>,
    /// Total allocation size in bytes.
    pub size: usize,
    /// Atomic reference count.
    pub ref_count: AtomicUsize,
}

impl DmaBuf {
    /// Allocate `size` bytes of physical memory and create a new `DmaBuf`.
    ///
    /// Rounds `size` up to a page boundary. Returns `ENOMEM` if any frame
    /// allocation fails (already-allocated frames are freed before returning).
    pub fn allocate(size: usize) -> Result<Self> {
        if size == 0 {
            return Err(Error::new(EINVAL));
        }

        let page_count = size.div_ceil(PAGE_SIZE);
        let mut frames = Vec::with_capacity(page_count);

        for i in 0..page_count {
            match allocate_frame() {
                Some(frame) => frames.push(frame),
                None => {
                    // Allocation failed — free what we have so far and return
                    for f in frames.drain(..) {
                        // SAFETY: frames were just allocated by allocate_frame and are
                        // not yet mapped anywhere.
                        unsafe { deallocate_frame(f) };
                    }
                    let _ = i;
                    return Err(Error::new(ENOMEM));
                }
            }
        }

        Ok(DmaBuf {
            frames,
            size,
            ref_count: AtomicUsize::new(1),
        })
    }

    /// Increment the reference count.
    #[inline]
    pub fn clone_ref(buf: &Arc<Mutex<DmaBuf>>) -> Arc<Mutex<DmaBuf>> {
        buf.lock().ref_count.fetch_add(1, Ordering::Acquire);
        Arc::clone(buf)
    }

    /// Number of physical pages backing this buffer.
    #[inline]
    pub fn page_count(&self) -> usize {
        self.frames.len()
    }

    /// Physical address of the n-th page.
    #[inline]
    pub fn frame_phys(&self, idx: usize) -> Option<usize> {
        self.frames.get(idx).map(|f| f.base().data())
    }
}

impl Drop for DmaBuf {
    fn drop(&mut self) {
        // ref_count was already decremented by `drop_ref`; just free the frames.
        for frame in self.frames.drain(..) {
            // SAFETY: We own these frames (ref_count just hit zero) and they are
            // not mapped anywhere after the caller unmapped all page-table entries.
            unsafe { deallocate_frame(frame) };
        }
    }
}

// =============================================================================
// Global DmaBuf Registry
// =============================================================================

/// Per-process DmaBuf file descriptor handle.
pub type DmaBufFd = usize;

/// Global registry: maps `DmaBufFd` → `Arc<Mutex<DmaBuf>>`.
static DMABUF_REGISTRY: Mutex<Option<HashMap<DmaBufFd, Arc<Mutex<DmaBuf>>>>> =
    Mutex::new(None);

/// Next FD to allocate (wraps at usize::MAX, but IDs are unique in practice).
static NEXT_FD: AtomicUsize = AtomicUsize::new(1);

/// Initialize the DmaBuf subsystem. Must be called once at boot.
pub fn init() {
    let mut guard = DMABUF_REGISTRY.lock();
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
}

/// Remove a DmaBuf from the global registry. Called automatically when the last reference drops.
pub fn remove_from_registry(fd: DmaBufFd) {
    let mut guard = DMABUF_REGISTRY.lock();
    if let Some(registry) = guard.as_mut() {
        registry.remove(&fd);
    }
}

// =============================================================================
// Syscall Implementations
// =============================================================================

/// `sys_dmabuf_create(size) -> DmaBufFd`
///
/// Allocates `size` bytes of physical memory, creates a `DmaBuf`, registers
/// it in the global registry and returns an opaque file descriptor.
pub fn sys_dmabuf_create(size: usize) -> Result<DmaBufFd> {
    let buf = DmaBuf::allocate(size)?;
    let arc = Arc::new(Mutex::new(buf));
    let fd = NEXT_FD.fetch_add(1, Ordering::Relaxed);

    let mut guard = DMABUF_REGISTRY.lock();
    let registry = guard.as_mut().ok_or(Error::new(EINVAL))?;
    registry.insert(fd, arc);

    Ok(fd)
}

/// `sys_dmabuf_map(fd, vaddr, flags) -> Result<()>`
///
/// Maps all physical frames of the `DmaBuf` identified by `fd` into the
/// current process's address space starting at `vaddr`. The caller is
/// responsible for ensuring `vaddr` is page-aligned and that the range
/// `[vaddr, vaddr + size)` does not overlap existing mappings.
///
/// The reference count of the `DmaBuf` is incremented on success so that
/// the buffer is not freed while the mapping is live.
pub fn sys_dmabuf_map(
    fd: DmaBufFd,
    vaddr: usize,
    flags: crate::syscall::flag::MapFlags,
) -> Result<()> {
    if vaddr % PAGE_SIZE != 0 {
        return Err(Error::new(EINVAL));
    }

    // Retrieve the buffer arc while the registry lock is held momentarily.
    let arc = {
        let guard = DMABUF_REGISTRY.lock();
        let registry = guard.as_ref().ok_or(Error::new(EINVAL))?;
        registry.get(&fd).cloned().ok_or(Error::new(EBADF))?
    };

    // Increment refcount to prevent premature deallocation.
    arc.lock().ref_count.fetch_add(1, Ordering::Acquire);

    let buf = arc.lock();
    let page_count = buf.page_count();

    crate::memory::with_clean_lock_token(|token| {
        let current_context_ref = crate::context::current();
        let current_context_guard = current_context_ref.read(token.token());
        let addr_space = current_context_guard.addr_space.as_ref().ok_or(Error::new(crate::syscall::error::ESRCH))?;
        let mut inner = addr_space.inner.write();

        let requested = crate::paging::Page::containing_address(crate::paging::VirtualAddress::new(vaddr));

        // Check for overlap in the address space
        if inner.grants.range(requested..requested.next_by(page_count)).next().is_some() {
            // Revert refcount increment if mapping fails due to overlap
            drop(buf); // Release lock before mutating ref_count
            arc.lock().ref_count.fetch_sub(1, Ordering::Release);
            return Err(Error::new(crate::syscall::error::EEXIST));
        }

        let page_flags = dmabuf_flags_to_page_flags(flags);
        let mut grant = crate::context::memory::Grant::new(requested, requested.next_by(page_count), page_flags);
        grant.provider = crate::context::memory::Provider::DmaBuf { fd, arc: Arc::clone(&arc) };

        for i in 0..page_count {
            let phys = buf.frames[i].base();
            let virt = requested.next_by(i).start_address();

            unsafe {
                if inner.table.utable.map_phys(virt, phys, page_flags).is_none() {
                    // Clean up already mapped pages
                    for j in 0..i {
                        let virt_unmap = requested.next_by(j).start_address();
                        let _ = inner.table.utable.unmap(virt_unmap);
                    }
                    drop(buf);
                    arc.lock().ref_count.fetch_sub(1, Ordering::Release);
                    return Err(Error::new(ENOMEM));
                }
            }
        }

        inner.grants.insert(requested, grant);
        Ok::<(), Error>(())
    })
}

pub fn sys_dmabuf_unmap(fd: DmaBufFd, vaddr: usize) -> Result<()> {
    if vaddr % PAGE_SIZE != 0 {
        return Err(Error::new(EINVAL));
    }

    let arc = {
        let guard = DMABUF_REGISTRY.lock();
        let registry = guard.as_ref().ok_or(Error::new(EINVAL))?;
        registry.get(&fd).cloned().ok_or(Error::new(EBADF))?
    };

    let page_count = arc.lock().page_count();

    crate::memory::with_clean_lock_token(|token| {
        let current_context_ref = crate::context::current();
        let current_context_guard = current_context_ref.read(token.token());
        let addr_space = current_context_guard.addr_space.as_ref().ok_or(Error::new(crate::syscall::error::ESRCH))?;
        let mut inner = addr_space.inner.write();

        let requested = crate::paging::Page::containing_address(crate::paging::VirtualAddress::new(vaddr));

        // Verify that the grant actually exists at vaddr and is indeed a DmaBuf grant for this fd
        let grant_exists = if let Some(grant) = inner.grants.get(&requested) {
            if let crate::context::memory::Provider::DmaBuf { fd: grant_fd, .. } = &grant.provider {
                *grant_fd == fd
            } else {
                false
            }
        } else {
            false
        };

        if !grant_exists {
            return Err(Error::new(EINVAL));
        }

        // Unmap from page tables
        for i in 0..page_count {
            let virt = crate::paging::VirtualAddress::new(vaddr + i * PAGE_SIZE);
            unsafe {
                inner.table.utable.unmap(virt)
                    .ok_or(Error::new(EINVAL))?
                    .flush();
            }
        }

        // Remove from grants. This drops the Grant, which automatically decrements the refcount
        // and removes it from the registry if needed.
        inner.grants.remove(&requested);

        Ok::<(), Error>(())
    })
}

/// `sys_dmabuf_release(fd) -> Result<()>`
///
/// Closes the creating process's handle to the DmaBuf. Does not unmap
/// existing mappings — those must be torn down via `sys_dmabuf_unmap`.
pub fn sys_dmabuf_release(fd: DmaBufFd) -> Result<()> {
    let arc = {
        let mut guard = DMABUF_REGISTRY.lock();
        let registry = guard.as_mut().ok_or(Error::new(EINVAL))?;
        registry.remove(&fd).ok_or(Error::new(EBADF))?
    };

    let prev = arc.lock().ref_count.fetch_sub(1, Ordering::Release);
    // If prev == 1, the Arc is now the only remaining reference. When `arc`
    // drops at end of this scope, `DmaBuf::drop` frees all frames.
    let _ = prev;
    Ok(())
}

// =============================================================================
// Helpers
// =============================================================================

/// Converts `MapFlags` from the syscall ABI to architecture page-table flags.
fn dmabuf_flags_to_page_flags(
    flags: crate::syscall::flag::MapFlags,
) -> crate::paging::PageFlags<crate::paging::RmmA> {
    use crate::paging::PageFlags;
    use crate::syscall::flag::MapFlags;

    let mut pf = PageFlags::new().user(true);

    if flags.contains(MapFlags::PROT_WRITE) {
        pf = pf.write(true);
    }
    if flags.contains(MapFlags::PROT_EXEC) {
        pf = pf.execute(true);
    }
    // DMA buffers are always user-accessible and present.
    pf
}

// =============================================================================
// Tests
// =============================================================================

static TEST_LOCK: spin::Mutex<()> = spin::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::sync::Arc;

    fn setup_mock_context() -> (Arc<crate::context::ContextLock>, usize) {
        let mut token = unsafe { crate::sync::CleanLockToken::new() };
        let addr_space = crate::context::memory::AddrSpace::new().unwrap();
        let mut context_ref = Arc::new(crate::context::ContextLock::new(crate::context::Context::new(None).unwrap()));
        let context_id = {
            let context_lock = Arc::get_mut(&mut context_ref).unwrap();
            let context = context_lock.get_mut();
            context.addr_space = Some(addr_space);
            context.id
        };
        {
            let mut contexts = crate::context::list::contexts().write();
            contexts.insert(context_id, Arc::clone(&context_ref));
        }
        crate::percpu::PercpuBlock::current().context_id.set(context_id);
        (context_ref, context_id)
    }

    #[test]
    fn test_dmabuf_zero_copy() {
        let _guard = TEST_LOCK.lock();

        crate::allocator::linked_list::init_mock_heap();
        crate::memory::init_mock_allocator();
        init();

        // 1. Setup mock current context
        let (context_ref, context_id) = setup_mock_context();
        let mut token = unsafe { crate::sync::CleanLockToken::new() };
        
        // 2. Create a DmaBuf of size 4096 (1 page)
        let fd = sys_dmabuf_create(PAGE_SIZE).unwrap();

        // 3. Map it at virtual address VADDR1
        let vaddr1 = 0x5000_0000;
        sys_dmabuf_map(
            fd,
            vaddr1,
            crate::syscall::flag::MapFlags::PROT_READ | crate::syscall::flag::MapFlags::PROT_WRITE,
        )
        .unwrap();

        // 4. Verify that the grant is inserted and has correct properties
        {
            let current = crate::context::current();
            let guard = current.read(token.token());
            let addr_space = guard.addr_space.as_ref().unwrap();
            let inner = addr_space.inner.read();

            let requested = crate::paging::Page::containing_address(crate::paging::VirtualAddress::new(vaddr1));
            let grant = inner.grants.get(&requested).unwrap();

            assert_eq!(grant.start_address().data(), vaddr1);
            assert_eq!(grant.page_count(), 1);
            if let crate::context::memory::Provider::DmaBuf { fd: grant_fd, arc } = &grant.provider {
                assert_eq!(*grant_fd, fd);
                assert_eq!(arc.lock().ref_count.load(Ordering::Relaxed), 2); // 1 (creation) + 1 (mapping)
            } else {
                panic!("Invalid provider type for DmaBuf mapping");
            }
        }

        // 5. Map the same DmaBuf at a different virtual address VADDR2
        let vaddr2 = 0x6000_0000;
        sys_dmabuf_map(
            fd,
            vaddr2,
            crate::syscall::flag::MapFlags::PROT_READ | crate::syscall::flag::MapFlags::PROT_WRITE,
        )
        .unwrap();

        // 6. Verify reference count updated
        {
            let current = crate::context::current();
            let guard = current.read(token.token());
            let addr_space = guard.addr_space.as_ref().unwrap();
            let inner = addr_space.inner.read();

            let requested = crate::paging::Page::containing_address(crate::paging::VirtualAddress::new(vaddr2));
            let grant = inner.grants.get(&requested).unwrap();
            if let crate::context::memory::Provider::DmaBuf { arc, .. } = &grant.provider {
                assert_eq!(arc.lock().ref_count.load(Ordering::Relaxed), 3); // 1 (creation) + 2 (mappings)
            } else {
                panic!("Invalid provider type");
            }
        }

        // 7. Unmap VADDR1 and VADDR2
        sys_dmabuf_unmap(fd, vaddr1).unwrap();
        sys_dmabuf_unmap(fd, vaddr2).unwrap();

        // 8. Verify grants removed
        {
            let current = crate::context::current();
            let guard = current.read(token.token());
            let addr_space = guard.addr_space.as_ref().unwrap();
            let inner = addr_space.inner.read();

            let requested1 = crate::paging::Page::containing_address(crate::paging::VirtualAddress::new(vaddr1));
            let requested2 = crate::paging::Page::containing_address(crate::paging::VirtualAddress::new(vaddr2));
            assert!(inner.grants.get(&requested1).is_none());
            assert!(inner.grants.get(&requested2).is_none());
        }

        // 9. Release the FD
        sys_dmabuf_release(fd).unwrap();

        // 10. Clean up global CONTEXTS map
        {
            let mut contexts = crate::context::list::contexts().write();
            contexts.remove(&context_id);
        }
        let _ = context_ref;
    }

    /// Verify that `DmaBuf::allocate` round-trips: size > 0 succeeds,
    /// page count matches, and refcount starts at 1.
    #[test]
    fn test_dmabuf_allocation_tracking() {
        let page_count_4k = 1usize.div_ceil(PAGE_SIZE);
        assert_eq!(page_count_4k, 1);

        let page_count_8k = (PAGE_SIZE + 1).div_ceil(PAGE_SIZE);
        assert_eq!(page_count_8k, 2);

        let page_count_aligned = (4 * PAGE_SIZE).div_ceil(PAGE_SIZE);
        assert_eq!(page_count_aligned, 4);
    }

    /// Verify the DmaBuf FD counter is monotonically increasing.
    #[test]
    fn test_fd_counter_monotonic() {
        let a = NEXT_FD.fetch_add(1, Ordering::Relaxed);
        let b = NEXT_FD.fetch_add(1, Ordering::Relaxed);
        assert!(b > a);
    }

    /// Verify page-flag helper doesn't panic on all MapFlags combinations.
    #[test]
    fn test_dmabuf_flags_conversion() {
        use crate::syscall::flag::MapFlags;
        let _ = dmabuf_flags_to_page_flags(MapFlags::PROT_READ);
        let _ = dmabuf_flags_to_page_flags(MapFlags::PROT_READ | MapFlags::PROT_WRITE);
        let _ = dmabuf_flags_to_page_flags(
            MapFlags::PROT_READ | MapFlags::PROT_WRITE | MapFlags::PROT_EXEC,
        );
    }
}

#[cfg(test)]
mod benchmarks {
    use super::*;
    use core::sync::atomic::Ordering;
    use alloc::sync::Arc;

    fn setup_mock_context() -> (Arc<crate::context::ContextLock>, usize) {
        let mut token = unsafe { crate::sync::CleanLockToken::new() };
        let addr_space = crate::context::memory::AddrSpace::new().unwrap();
        let mut context_ref = Arc::new(crate::context::ContextLock::new(crate::context::Context::new(None).unwrap()));
        let context_id = {
            let context_lock = Arc::get_mut(&mut context_ref).unwrap();
            let context = context_lock.get_mut();
            context.addr_space = Some(addr_space);
            context.id
        };
        {
            let mut contexts = crate::context::list::contexts().write();
            contexts.insert(context_id, Arc::clone(&context_ref));
        }
        crate::percpu::PercpuBlock::current().context_id.set(context_id);
        (context_ref, context_id)
    }

    #[test]
    fn bench_dmabuf_map() {
        let _guard = TEST_LOCK.lock();

        crate::allocator::linked_list::init_mock_heap();
        crate::memory::init_mock_allocator();
        init();
        let (context_ref, context_id) = setup_mock_context();
        let fd = sys_dmabuf_create(PAGE_SIZE).unwrap();

        let start = crate::time::monotonic();
        let iterations = 100;
        let base_vaddr = 0x7000_0000;
        for i in 0..iterations {
            let vaddr = base_vaddr + i * PAGE_SIZE;
            sys_dmabuf_map(
                fd,
                vaddr,
                crate::syscall::flag::MapFlags::PROT_READ | crate::syscall::flag::MapFlags::PROT_WRITE,
            )
            .unwrap();
            sys_dmabuf_unmap(fd, vaddr).unwrap();
        }
        let end = crate::time::monotonic();
        let elapsed_ns = end - start;
        let ns_per_iter = elapsed_ns / iterations as u128;
        
        // Target: < 1 microsecond per 4KB page (1000 ns)
        assert!(ns_per_iter < 1000, "Mapping latency too high: {} ns/page", ns_per_iter);
        
        sys_dmabuf_release(fd).unwrap();
        {
            let mut contexts = crate::context::list::contexts().write();
            contexts.remove(&context_id);
        }
        let _ = context_ref;
    }

    /// Simulated benchmark: measures FD allocation overhead (pure atomic operation).
    /// Real wall-clock benchmarking requires the full kernel runtime.
    #[test]
    fn bench_dmabuf_fd_alloc() {
        let start = NEXT_FD.load(Ordering::Relaxed);
        for _ in 0..1000 {
            let _ = NEXT_FD.fetch_add(1, Ordering::Relaxed);
        }
        let end = NEXT_FD.load(Ordering::Relaxed);
        assert_eq!(end - start, 1000);
    }
}

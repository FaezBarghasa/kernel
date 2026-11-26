use core::sync::{
    atomic,
    atomic::{AtomicU32, AtomicUsize, Ordering},
};
use rmm::{Arch, PageFlags, PageMapper, TableKind, VirtualAddress};

const NO_PROCESSOR: u32 = !0;
static LOCK_OWNER: AtomicU32 = AtomicU32::new(NO_PROCESSOR);
static LOCK_COUNT: AtomicUsize = AtomicUsize::new(0);

/// A kernel page mapper.
pub struct KernelMapper {
    mapper: crate::paging::PageMapper,
    ro: bool,
}
impl KernelMapper {
    /// Locks the kernel page mapper.
    pub fn lock() -> Self {
        let mapper =
            unsafe { PageMapper::current(TableKind::Kernel, crate::memory::TheFrameAllocator) };

        let current_processor = crate::cpu_id();
        loop {
            match LOCK_OWNER.compare_exchange_weak(
                NO_PROCESSOR,
                current_processor.get(),
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                // already owned by this hardware thread
                Err(id) if id == current_processor.get() => break,
                // either CAS failed, or some other hardware thread holds the lock
                Err(_) => core::hint::spin_loop(),
            }
        }

        let prev_count = LOCK_COUNT.fetch_add(1, Ordering::Relaxed);
        atomic::compiler_fence(Ordering::Acquire);

        let ro = prev_count > 0;
        Self { mapper, ro }
    }
    /// Returns a mutable reference to the inner page mapper.
    pub fn get_mut(&mut self) -> Option<&mut crate::paging::PageMapper> {
        if self.ro {
            None
        } else {
            Some(&mut self.mapper)
        }
    }
}
impl core::ops::Deref for KernelMapper {
    type Target = crate::paging::PageMapper;

    fn deref(&self) -> &Self::Target {
        &self.mapper
    }
}
impl Drop for KernelMapper {
    fn drop(&mut self) {
        atomic::compiler_fence(Ordering::Release);

        let prev_count = LOCK_COUNT.fetch_sub(1, Ordering::Relaxed);

        if prev_count == 1 {
            LOCK_OWNER.store(NO_PROCESSOR, Ordering::Release);
        }
    }
}

/// Returns the page flags for a given virtual address.
pub unsafe fn page_flags<A: Arch>(virt: VirtualAddress) -> PageFlags<A> {
    use crate::kernel_executable_offsets::*;
    let virt_addr = virt.data();

    if virt_addr >= __text_start() && virt_addr < __text_end() {
        // Remap text read-only, execute
        PageFlags::new().execute(true)
    } else if virt_addr >= __rodata_start() && virt_addr < __rodata_end() {
        // Remap rodata read-only, no execute
        PageFlags::new()
    } else {
        // Remap everything else read-write, no execute
        PageFlags::new().write(true)
    }
}

use crate::memory::KernelMapper;
use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::NonNull,
};
use linked_list_allocator::Heap;
use spin::Mutex;

static HEAP: Mutex<Option<Heap>> = Mutex::new(None);

pub struct Allocator;

impl Allocator {
    pub unsafe fn init(offset: usize, size: usize) {
        unsafe {
            *HEAP.lock() = Some(Heap::new(offset as *mut u8, size));
        }
    }
}

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe {
            while let Some(ref mut heap) = *HEAP.lock() {
                match heap.allocate_first_fit(layout) {
                    Ok(ptr) => return ptr.as_ptr(),
                    Err(()) => {
                        let size = heap.size();
                        // Use KASLR-adjusted heap base for address randomization
                        super::map_heap(
                            &mut KernelMapper::lock(),
                            crate::kaslr::heap_base() + size,
                            crate::KERNEL_HEAP_SIZE,
                        );
                        heap.extend(crate::KERNEL_HEAP_SIZE);
                    }
                }
            }
            panic!("__rust_allocate: heap not initialized");
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            HEAP.lock()
                .as_mut()
                .expect("heap not initialized")
                .deallocate(NonNull::new_unchecked(ptr), layout)
        }
    }
}

#[cfg(test)]
pub fn init_mock_heap() {
    use core::cell::SyncUnsafeCell;
    struct ForceSync<T>(T);
    unsafe impl<T> Sync for ForceSync<T> {}
    static HEAP_BUFFER: ForceSync<SyncUnsafeCell<[u8; 2 * 1024 * 1024]>> = ForceSync(SyncUnsafeCell::new([0; 2 * 1024 * 1024]));
    unsafe {
        let mut guard = HEAP.lock();
        if guard.is_none() {
            let ptr = HEAP_BUFFER.0.get() as *mut u8;
            *guard = Some(linked_list_allocator::Heap::new(ptr, 2 * 1024 * 1024));
        }
    }
}

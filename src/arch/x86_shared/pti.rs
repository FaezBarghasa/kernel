#[cfg(feature = "pti")]
use core::ptr;

#[cfg(feature = "pti")]
use crate::memory::Frame;
#[cfg(feature = "pti")]
use crate::paging::entry::EntryFlags;
#[cfg(feature = "pti")]
use crate::paging::ActivePageTable;

#[cfg(feature = "pti")]
#[thread_local]
pub static mut PTI_CPU_STACK: [u8; 256] = [0; 256];

#[cfg(feature = "pti")]
#[thread_local]
pub static mut PTI_CONTEXT_STACK: usize = 0;

#[cfg(feature = "pti")]
#[inline(always)]
unsafe fn switch_stack(old: usize, new: usize) {
    let old_sp: usize;
    #[cfg(target_arch = "x86_64")]
    asm!("", out("rsp") old_sp);
    #[cfg(target_arch = "x86")]
    asm!("", out("esp") old_sp);

    let offset_sp = old - old_sp;

    let new_sp = new - offset_sp;

    ptr::copy_nonoverlapping(old_sp as *const u8, new_sp as *mut u8, offset_sp);

    #[cfg(target_arch = "x86_64")]
    asm!("", out("rsp") new_sp);
    #[cfg(target_arch = "x86")]
    asm!("", out("esp") new_sp);
}

/// Maps the kernel heap and switches to the per-context stack.
#[cfg(feature = "pti")]
#[inline(always)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn map() {
    // {
    //     let mut active_table = unsafe { ActivePageTable::new() };
    //
    //     // Map kernel heap
    //     let address = active_table.p4()[::KERNEL_HEAP_PML4].address();
    //     let frame = Frame::containing(address);
    //     let mut flags = active_table.p4()[::KERNEL_HEAP_PML4].flags();
    //     flags.remove(EntryFlags::PRESENT);
    //     active_table.p4_mut()[::KERNEL_HEAP_PML4].set(frame, flags);
    //
    //     // Reload page tables
    //     active_table.flush_all();
    // }

    // Switch to per-context stack
    switch_stack(
        PTI_CPU_STACK.as_ptr() as usize + PTI_CPU_STACK.len(),
        PTI_CONTEXT_STACK,
    );
}

/// Unmaps the kernel heap and switches to the per-CPU stack.
#[cfg(feature = "pti")]
#[inline(always)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unmap() {
    // Switch to per-CPU stack
    switch_stack(
        PTI_CONTEXT_STACK,
        PTI_CPU_STACK.as_ptr() as usize + PTI_CPU_STACK.len(),
    );

    // {
    //     let mut active_table = unsafe { ActivePageTable::new() };
    //
    //     // Unmap kernel heap
    //     let address = active_table.p4()[::KERNEL_HEAP_PML4].address();
    //     let frame = Frame::containing(address);
    //     let mut flags = active_table.p4()[::KERNEL_HEAP_PML4].flags();
    //     flags.insert(EntryFlags::PRESENT);
    //     active_table.p4_mut()[::KERNEL_HEAP_PML4].set(frame, flags);
    //
    //     // Reload page tables
    //     active_table.flush_all();
    // }
}

#[cfg(not(feature = "pti"))]
#[inline(always)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn map() {}

#[cfg(not(feature = "pti"))]
#[inline(always)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unmap() {}

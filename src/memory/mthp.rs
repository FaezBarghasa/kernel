#![forbid(unsafe_code)]

use alloc::vec::Vec;
use alloc::sync::Arc;
use crate::paging::{Page, PageFlags, RmmA, PhysicalAddress, VirtualAddress, PAGE_SIZE};
use crate::context::memory::Provider;
use crate::context::contexts;

/// Size class: 2MB transparent huge page = 512 * 4KB pages
const HUGE_PAGE_SIZE: usize = 512;

/// Checks the buddy allocator freelist fragmentation.
/// Returns a ratio of small blocks (order 0..3) compared to total free memory.
/// A high ratio means high fragmentation.
pub fn check_fragmentation() -> f32 {
    // We will query the free frames directly.
    // In a real system, we'd traverse order pools.
    let total = crate::memory::total_frames();
    let used = crate::memory::used_frames();
    let free = total.saturating_sub(used);
    
    if free == 0 {
        return 0.0;
    }
    
    // Simple heuristic for fragmentation:
    // If we have less than 15% of total memory free, or if we cannot allocate contiguous 2MB blocks.
    let huge_alloc_test = crate::memory::allocate_huge_frame();
    if let Some(frame) = huge_alloc_test {
        crate::memory::deallocate_p2frame_safe(frame, 9); // deallocate huge frame (order 9)
        0.1 // Low fragmentation
    } else {
        0.7 // High fragmentation (cannot allocate a contiguous 2MB block)
    }
}

/// Dynamic transparent huge page (2MB) coalescing scanner
pub fn coalesce_huge_pages() {
    let contexts_guard = contexts().read();
    
    for (&context_id, context_lock) in contexts_guard.iter() {
        crate::memory::with_clean_lock_token(|token| {
            let mut context = context_lock.write(token.token());
            if let Some(addr_space_wrapper) = context.addr_space.clone() {
                let mut addr_space = addr_space_wrapper.acquire_write();
                let mut to_compact = Vec::new();

                // Find candidate grants that are at least 2MB and aligned
                for (&page, grant) in addr_space.grants.iter() {
                    if let Provider::Allocated { flags } = grant.provider {
                        let page_count = grant.page_count();
                        let start_addr = page.start_address().data();
                        
                        if page_count >= HUGE_PAGE_SIZE && start_addr % (2 * 1024 * 1024) == 0 {
                            // Candidate for coalescing
                            to_compact.push((page, page_count, flags));
                        }
                    }
                }

                for (start_page, page_count, flags) in to_compact {
                    // Check if pages are already contiguous
                    let mut contiguous = true;
                    let mut phys_addrs = Vec::new();
                    
                    for i in 0..HUGE_PAGE_SIZE {
                        let virt = start_page.next_by(i).start_address();
                        if let Some((phys, _)) = addr_space.table.utable.0.translate(virt) {
                            phys_addrs.push(phys);
                        } else {
                            contiguous = false;
                            break;
                        }
                    }

                    if !contiguous {
                        continue;
                    }

                    // Check if already contiguous in physical memory
                    let mut physically_contiguous = true;
                    let base_phys = phys_addrs[0].data();
                    for (i, &phys) in phys_addrs.iter().enumerate() {
                        if phys.data() != base_phys + i * PAGE_SIZE {
                            physically_contiguous = false;
                            break;
                        }
                    }

                    if physically_contiguous {
                        // Coalesce in-place by remapping with the huge page flag
                        let start_virt = start_page.start_address();
                        // Unmap individual pages
                        for i in 0..HUGE_PAGE_SIZE {
                            let virt = start_page.next_by(i).start_address();
                            crate::memory::unmap_page(&mut addr_space.table.utable.0, virt);
                        }
                        // Map as one huge page
                        crate::memory::map_huge_page(
                            &mut addr_space.table.utable.0,
                            start_virt,
                            phys_addrs[0],
                            flags,
                        );
                    } else {
                        // Compaction: allocate a contiguous 2MB physical frame
                        if let Some(huge_frame) = crate::memory::allocate_huge_frame() {
                            let target_phys = huge_frame.base();
                            // Copy all data to the new contiguous huge frame
                            crate::memory::copy_pages_contiguous(&phys_addrs, target_phys);

                            let start_virt = start_page.start_address();
                            // Unmap old 4KB pages
                            for i in 0..HUGE_PAGE_SIZE {
                                let virt = start_page.next_by(i).start_address();
                                crate::memory::unmap_page(&mut addr_space.table.utable.0, virt);
                            }

                            // Map the new huge page
                            crate::memory::map_huge_page(
                                &mut addr_space.table.utable.0,
                                start_virt,
                                target_phys,
                                flags,
                            );

                            // Free old 512 physical pages
                            // Extract frames and deallocate them
                            // Since we have the physical addresses, we can free them using deallocate_frame
                            for phys in phys_addrs {
                                let frame = crate::memory::Frame::containing(phys);
                                crate::memory::deallocate_frame_safe(frame);
                            }

                            // Update the Grant to point to the new huge frame
                            if let Some(grant) = addr_space.grants.get_mut(&start_page) {
                                grant.set_phys(huge_frame);
                            }

                            // Perform TLB shootdowns
                            let cpu_set = addr_space.used_by.clone();
                            for i in 0..crate::cpu_count() {
                                let cpu_id = crate::cpu_set::LogicalCpuId::new(i);
                                if cpu_set.contains(cpu_id) {
                                    crate::memory::shootdown_tlb_for_cpu(cpu_id);
                                }
                            }
                        }
                    }
                }
            }
        });
    }
}

pub fn mthp_daemon() {
    let mut last_scan = crate::time::monotonic();
    loop {
        let now = crate::time::monotonic();
        if now.saturating_sub(last_scan) >= 10_000_000_000 { // Coalesce every 10 seconds
            last_scan = now;
            coalesce_huge_pages();
        }
        crate::memory::yield_now();
    }
}

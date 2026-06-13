#![forbid(unsafe_code)]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use alloc::sync::Arc;
use spin::Mutex;
use crate::paging::{Page, PageFlags, RmmA, PhysicalAddress, VirtualAddress, PAGE_SIZE};
use crate::context::memory::Provider;
use crate::context::contexts;

/// Zero-run compression for zram pages
fn compress_page(data: &[u8]) -> Vec<u8> {
    let mut compressed = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if data[i] == 0 {
            let mut zero_count = 0;
            while i < data.len() && data[i] == 0 && zero_count < 255 {
                zero_count += 1;
                i += 1;
            }
            compressed.push(0);
            compressed.push(zero_count);
        } else {
            compressed.push(data[i]);
            i += 1;
        }
    }
    compressed
}

/// Zero-run decompression for zram pages
fn decompress_page(compressed: &[u8], decompressed: &mut [u8]) {
    let mut i = 0;
    let mut j = 0;
    while i < compressed.len() && j < decompressed.len() {
        if compressed[i] == 0 {
            if i + 1 < compressed.len() {
                let zero_count = compressed[i + 1] as usize;
                for _ in 0..zero_count {
                    if j < decompressed.len() {
                        decompressed[j] = 0;
                        j += 1;
                    }
                }
                i += 2;
            } else {
                break;
            }
        } else {
            decompressed[j] = compressed[i];
            j += 1;
            i += 1;
        }
    }
}

pub struct ZramEntry {
    pub compressed_data: Vec<u8>,
}

pub struct ZramPool {
    pub entries: BTreeMap<usize, ZramEntry>,
    pub next_idx: usize,
}

pub static ZRAM_POOL: Mutex<ZramPool> = Mutex::new(ZramPool {
    entries: BTreeMap::new(),
    next_idx: 1,
});

/// Page generations tracking: maps (context_id, page_virtual_addr) to generation (0..3)
pub static PAGE_GENERATIONS: Mutex<BTreeMap<(usize, usize), u8>> = Mutex::new(BTreeMap::new());

/// Swap in a page from zram to the given physical address
pub fn swap_in(zram_idx: usize, phys_addr: PhysicalAddress) -> bool {
    let mut pool = ZRAM_POOL.lock();
    if let Some(entry) = pool.entries.remove(&zram_idx) {
        let mut temp_buf = [0u8; PAGE_SIZE];
        decompress_page(&entry.compressed_data, &mut temp_buf);
        crate::memory::copy_slice_to_page(&temp_buf, phys_addr);
        true
    } else {
        false
    }
}

/// Periodically run by the page scanner daemon
pub fn scan_and_evict() {
    let contexts_guard = contexts().read();
    
    // We will collect pages to evict/scan to avoid holding lock over contexts map
    for (&context_id, context_lock) in contexts_guard.iter() {
        crate::memory::with_clean_lock_token(|token| {
            let mut context = context_lock.write(token.token());
            if let Some(addr_space_wrapper) = context.addr_space.clone() {
                let mut addr_space_guard = addr_space_wrapper.acquire_write();
                let crate::context::memory::AddrSpaceInner { table, grants, used_by, .. } = &mut *addr_space_guard;
                let mut evicted_pages = Vec::new();

                for (&page, grant) in grants.iter_mut() {
                    // Only scan allocated pages
                    if let Provider::Allocated { flags } = grant.provider {
                        if let Some(frame) = grant.phys() {
                            let virt_addr = page.start_address();
                            // Check accessed bit (bit 5) in page tables via translation
                            let accessed = if let Some((_, check_flags)) = table.utable.0.translate(virt_addr) {
                                check_flags.has_flag(1 << 5)
                            } else {
                                false
                            };

                            let mut gen_map = PAGE_GENERATIONS.lock();
                            let key = (context_id, virt_addr.data());
                            let current_gen = gen_map.get(&key).copied().unwrap_or(0);

                            if accessed {
                                // Reset to Gen 0 and clear accessed bit in page table
                                gen_map.insert(key, 0);
                                let cleared_flags = flags.custom_flag(1 << 5, false);
                                crate::memory::remap_page_flags(&mut table.utable.0, virt_addr, cleared_flags);
                            } else {
                                let next_gen = current_gen.saturating_add(1);
                                if next_gen >= 3 {
                                    // Evict to zram!
                                    let mut temp_buf = [0u8; PAGE_SIZE];
                                    crate::memory::copy_page_to_slice(frame.base(), &mut temp_buf);
                                    let compressed = compress_page(&temp_buf);

                                    let mut pool = ZRAM_POOL.lock();
                                    let idx = pool.next_idx;
                                    pool.next_idx += 1;
                                    pool.entries.insert(idx, ZramEntry { compressed_data: compressed });

                                    // Unmap from page table
                                    crate::memory::unmap_page(&mut table.utable.0, virt_addr);

                                    // Free physical frame and update grant
                                    let _ = grant.clear_phys();
                                    grant.provider = Provider::ZramSwapped { zram_idx: idx, flags };

                                    gen_map.remove(&key);
                                    evicted_pages.push(page);
                                } else {
                                    gen_map.insert(key, next_gen);
                                }
                            }
                        }
                    }
                }

                // If we evicted pages, perform TLB shootdowns
                if !evicted_pages.is_empty() {
                    let cpu_set = used_by.clone();
                    for i in 0..crate::cpu_count() {
                        let cpu_id = crate::cpu_set::LogicalCpuId::new(i);
                        if cpu_set.contains(cpu_id) {
                            crate::memory::shootdown_tlb_for_cpu(cpu_id);
                        }
                    }
                }
            }
        });
    }
}

pub fn mglru_daemon() {
    let mut last_scan = crate::time::monotonic();
    loop {
        let now = crate::time::monotonic();
        if now.saturating_sub(last_scan) >= 5_000_000_000 { // Scan every 5 seconds
            last_scan = now;
            scan_and_evict();
        }
        crate::memory::yield_now();
    }
}

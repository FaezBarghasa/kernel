//! # CPU Sets

use core::mem;

pub const CPU_COUNT: usize = 128;

type Word = u64;
const WORD_BITS: usize = mem::size_of::<Word>() * 8;
const WORD_COUNT: usize = (CPU_COUNT + WORD_BITS - 1) / WORD_BITS;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct LogicalCpuId(pub u32);

impl core::fmt::Display for LogicalCpuId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl LogicalCpuId {
    pub const BSP: LogicalCpuId = LogicalCpuId(0);
    pub fn new(id: u32) -> Self {
        Self(id)
    }
    pub fn get(&self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuSet {
    mask: [Word; WORD_COUNT],
}

impl CpuSet {
    pub const fn new() -> Self {
        Self {
            mask: [0; WORD_COUNT],
        }
    }

    pub fn all() -> Self {
        let mut set = Self::new();
        for i in 0..crate::cpu_count() {
            set.add(LogicalCpuId::new(i));
        }
        set
    }

    pub fn contains(&self, id: LogicalCpuId) -> bool {
        let i = id.0 as usize;
        if i >= CPU_COUNT {
            return false;
        }
        let word_idx = i / WORD_BITS;
        let bit_idx = i % WORD_BITS;
        (self.mask[word_idx] & (1 << bit_idx)) != 0
    }

    pub fn add(&mut self, id: LogicalCpuId) {
        let i = id.0 as usize;
        if i < CPU_COUNT {
            let word_idx = i / WORD_BITS;
            let bit_idx = i % WORD_BITS;
            self.mask[word_idx] |= 1 << bit_idx;
        }
    }

    pub fn remove(&mut self, id: LogicalCpuId) {
        let i = id.0 as usize;
        if i < CPU_COUNT {
            let word_idx = i / WORD_BITS;
            let bit_idx = i % WORD_BITS;
            self.mask[word_idx] &= !(1 << bit_idx);
        }
    }
    pub fn atomic_set(&self, id: LogicalCpuId) {
        let i = id.0 as usize;
        if i < CPU_COUNT {
            let word_idx = i / WORD_BITS;
            let bit_idx = i % WORD_BITS;
            unsafe {
                let atom = &*((&self.mask[word_idx]) as *const u64
                    as *const core::sync::atomic::AtomicU64);
                atom.fetch_or(1 << bit_idx, core::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    pub fn atomic_clear(&self, id: LogicalCpuId) {
        let i = id.0 as usize;
        if i < CPU_COUNT {
            let word_idx = i / WORD_BITS;
            let bit_idx = i % WORD_BITS;
            unsafe {
                let atom = &*((&self.mask[word_idx]) as *const u64
                    as *const core::sync::atomic::AtomicU64);
                atom.fetch_and(!(1 << bit_idx), core::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    pub fn override_from(&mut self, mask: &RawMask) {
        self.mask = *mask;
    }

    pub fn to_raw(&self) -> RawMask {
        self.mask
    }
}

impl Default for CpuSet {
    fn default() -> Self {
        Self::new()
    }
}

pub type LogicalCpuSet = CpuSet;
pub const MAX_CPU_COUNT: usize = CPU_COUNT;
pub type RawMask = [u64; WORD_COUNT];

pub fn mask_as_bytes(mask: &RawMask) -> &[u8] {
    unsafe { core::slice::from_raw_parts(mask.as_ptr() as *const u8, mem::size_of::<RawMask>()) }
}

//! # CPU Sets

use core::mem;

pub const CPU_COUNT: usize = 128;

type Word = u64;
const WORD_BITS: usize = mem::size_of::<Word>() * 8;
const WORD_COUNT: usize = (CPU_COUNT + WORD_BITS - 1) / WORD_BITS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogicalCpuId(pub u32);

impl LogicalCpuId {
    pub const BSP: LogicalCpuId = LogicalCpuId(0);
    pub fn new(id: u32) -> Self { Self(id) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuSet {
    mask: [Word; WORD_COUNT],
}

impl CpuSet {
    pub const fn new() -> Self {
        Self { mask: [0; WORD_COUNT] }
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
        if i >= CPU_COUNT { return false; }
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
}

impl Default for CpuSet {
    fn default() -> Self { Self::new() }
}
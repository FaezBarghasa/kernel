use core::{
    fmt::Display,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::CPU_COUNT;

/// A unique number used internally by the kernel to identify CPUs.
///
/// This is usually but not necessarily the same as the APIC ID.
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
// TODO: NonMaxUsize?
// TODO: Optimize away this type if not cfg!(feature = "multi_core")
pub struct LogicalCpuId(u32);

impl LogicalCpuId {
    /// The logical CPU ID of the bootstrap processor.
    pub const BSP: Self = Self::new(0);

    /// Returns the next available logical CPU ID.
    pub fn next() -> Self {
        let id = CPU_COUNT.fetch_add(1, Ordering::Relaxed);
        assert!(id < MAX_CPU_COUNT);
        Self(id)
    }

    /// Creates a new logical CPU ID.
    pub const fn new(inner: u32) -> Self {
        Self(inner)
    }
    /// Returns the inner value of the logical CPU ID.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl core::fmt::Debug for LogicalCpuId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "[logical cpu #{}]", self.0)
    }
}
impl core::fmt::Display for LogicalCpuId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

#[cfg(target_pointer_width = "64")]
pub const MAX_CPU_COUNT: u32 = 128;

#[cfg(target_pointer_width = "32")]
pub const MAX_CPU_COUNT: u32 = 32;

const SET_WORDS: usize = (MAX_CPU_COUNT / usize::BITS) as usize;

// TODO: Support more than 128 CPUs.
// The maximum number of CPUs on Linux is configurable, and the type for LogicalCpuSet and
// LogicalCpuId may be optimized accordingly. In that case, box the mask if it's larger than some
// base size (probably 256 bytes).
/// A bitmask of logical CPU IDs.
#[derive(Debug)]
pub struct LogicalCpuSet([AtomicUsize; SET_WORDS]);

/// Returns the word and bit for a given logical CPU ID.
fn parts(id: LogicalCpuId) -> (usize, u32) {
    ((id.get() / usize::BITS) as usize, id.get() % usize::BITS)
}
impl LogicalCpuSet {
    /// Creates an empty CPU set.
    pub const fn empty() -> Self {
        Self([const { AtomicUsize::new(0) }; SET_WORDS])
    }
    /// Creates a CPU set with all CPUs.
    pub const fn all() -> Self {
        Self([const { AtomicUsize::new(!0) }; SET_WORDS])
    }
    /// Returns true if the CPU set contains the given logical CPU ID.
    pub fn contains(&mut self, id: LogicalCpuId) -> bool {
        let (word, bit) = parts(id);
        *self.0[word].get_mut() & (1 << bit) != 0
    }
    /// Atomically sets the bit for the given logical CPU ID.
    pub fn atomic_set(&self, id: LogicalCpuId) {
        let (word, bit) = parts(id);
        let _ = self.0[word].fetch_or(1 << bit, Ordering::Release);
    }
    /// Atomically clears the bit for the given logical CPU ID.
    pub fn atomic_clear(&self, id: LogicalCpuId) {
        let (word, bit) = parts(id);
        let _ = self.0[word].fetch_and(!(1 << bit), Ordering::Release);
    }

    /// Overrides the CPU set from a raw mask.
    pub fn override_from(&mut self, raw: &RawMask) {
        self.0 = raw.map(AtomicUsize::new);
    }
    /// Converts the CPU set to a raw mask.
    pub fn to_raw(&self) -> RawMask {
        self.0.each_ref().map(|w| w.load(Ordering::Acquire))
    }

    /// Returns an iterator over the logical CPU IDs in the set.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = LogicalCpuId> + '_ {
        // TODO: Will this be optimized away?
        self.0.iter_mut().enumerate().flat_map(move |(i, w)| {
            (0..usize::BITS).filter_map(move |b| {
                if *w.get_mut() & (1 << b) != 0 {
                    Some(LogicalCpuId::new(i as u32 * usize::BITS + b))
                } else {
                    None
                }
            })
        })
    }
}

impl Display for LogicalCpuSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let cpu_count = crate::cpu_count();

        let raw = self.to_raw();
        let words = raw.get(..(cpu_count / usize::BITS) as usize).unwrap_or(&[]);
        for (i, word) in words.iter().enumerate() {
            if i != 0 {
                write!(f, "_")?;
            }
            let word = if i == words.len() - 1 {
                *word & ((1_usize << (cpu_count % usize::BITS)) - 1)
            } else {
                *word
            };
            write!(f, "{word:x}")?;
        }
        Ok(())
    }
}

/// A raw bitmask of logical CPU IDs.
pub type RawMask = [usize; SET_WORDS];

/// Converts a raw mask to a byte slice.
pub fn mask_as_bytes(mask: &RawMask) -> &[u8] {
    unsafe { core::slice::from_raw_parts(mask.as_ptr().cast(), core::mem::size_of::<RawMask>()) }
}

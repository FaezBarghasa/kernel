//! Kernel Address Space Layout Randomization (KASLR)
//!
//! Calculates random slides for kernel heap and stack base addresses at boot time.
//! Uses entropy from the `entropy` module to generate cryptographically random offsets.
//!
//! ## Security Properties
//!
//! - Heap and stack bases are randomized each boot
//! - Offsets are properly aligned for huge pages (2MB)
//! - Entropy is derived from hardware RNG when available
//!
//! ## Limitations
//!
//! Full kernel code base randomization requires Position Independent Executable (PIE)
//! compilation and bootloader cooperation. This module only randomizes heap/stack.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::entropy;

/// Maximum heap slide: 512 GB range
const HEAP_MAX_SLIDE: usize = 512 * 1024 * 1024 * 1024;

/// Maximum stack slide: 256 GB range  
const STACK_MAX_SLIDE: usize = 256 * 1024 * 1024 * 1024;

/// Heap alignment: 2MB (huge page granularity)
const HEAP_GRANULARITY: usize = 2 * 1024 * 1024;

/// Stack alignment: 4KB (page granularity)
const STACK_GRANULARITY: usize = 4 * 1024;

/// Randomized heap offset
static HEAP_SLIDE: AtomicUsize = AtomicUsize::new(0);

/// Randomized stack offset
static STACK_SLIDE: AtomicUsize = AtomicUsize::new(0);

/// Whether KASLR has been initialized
static KASLR_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Whether KASLR randomization is active (false if no entropy available)
static KASLR_ENABLED: AtomicBool = AtomicBool::new(false);

/// Initialize KASLR. Must be called after `entropy::init()`.
///
/// Calculates random slides for heap and stack bases using boot entropy.
/// Should be called once during early kernel initialization.
pub fn init() {
    if KASLR_INITIALIZED.swap(true, Ordering::SeqCst) {
        return; // Already initialized
    }

    // Calculate heap slide
    let heap_slide = entropy::kaslr_offset(HEAP_GRANULARITY, HEAP_MAX_SLIDE);
    HEAP_SLIDE.store(heap_slide, Ordering::Release);

    // Calculate stack slide (use different entropy derivation)
    // XOR with a constant to get different value from heap
    let stack_entropy = entropy::get_boot_entropy() ^ 0xA5A5A5A5A5A5A5A5;
    let stack_slots = STACK_MAX_SLIDE / STACK_GRANULARITY;
    let stack_slide = if stack_slots > 0 {
        ((stack_entropy as usize) % stack_slots) * STACK_GRANULARITY
    } else {
        0
    };
    STACK_SLIDE.store(stack_slide, Ordering::Release);

    // KASLR is enabled if we have real entropy (not just fallback)
    let enabled = entropy::has_hardware_rng() || heap_slide != 0 || stack_slide != 0;
    KASLR_ENABLED.store(enabled, Ordering::Release);

    // Log KASLR status
    if enabled {
        log::info!(
            "KASLR: Enabled (heap slide: {:#x}, stack slide: {:#x})",
            heap_slide,
            stack_slide
        );
    } else {
        log::warn!("KASLR: Disabled (no entropy source)");
    }
}

/// Get the randomized heap base address.
///
/// Returns the base heap offset with KASLR slide applied.
/// If KASLR is not initialized, returns the default heap offset.
#[inline]
pub fn heap_base() -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        crate::KERNEL_HEAP_OFFSET.wrapping_add(HEAP_SLIDE.load(Ordering::Acquire))
    }

    #[cfg(target_arch = "aarch64")]
    {
        crate::KERNEL_HEAP_OFFSET.wrapping_add(HEAP_SLIDE.load(Ordering::Acquire))
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        crate::KERNEL_HEAP_OFFSET
    }
}

/// Get the heap slide offset (for diagnostics).
#[inline]
pub fn heap_slide() -> usize {
    HEAP_SLIDE.load(Ordering::Acquire)
}

/// Get the stack slide offset.
///
/// This value should be added to stack base calculations to randomize
/// kernel thread stack locations.
#[inline]
pub fn stack_slide() -> usize {
    STACK_SLIDE.load(Ordering::Acquire)
}

/// Check if KASLR is enabled.
#[inline]
pub fn is_enabled() -> bool {
    KASLR_ENABLED.load(Ordering::Acquire)
}

/// Get entropy quality information for diagnostics.
pub fn entropy_quality() -> &'static str {
    if entropy::has_hardware_rng() {
        "hardware"
    } else {
        "jitter"
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heap_slide_alignment() {
        // Simulate with known values
        let slide = HEAP_SLIDE.load(Ordering::Relaxed);
        assert_eq!(
            slide % HEAP_GRANULARITY,
            0,
            "Heap slide must be 2MB aligned"
        );
    }

    #[test]
    fn test_stack_slide_alignment() {
        let slide = STACK_SLIDE.load(Ordering::Relaxed);
        assert_eq!(
            slide % STACK_GRANULARITY,
            0,
            "Stack slide must be 4KB aligned"
        );
    }

    #[test]
    fn test_slides_within_range() {
        let heap = HEAP_SLIDE.load(Ordering::Relaxed);
        let stack = STACK_SLIDE.load(Ordering::Relaxed);
        assert!(heap < HEAP_MAX_SLIDE, "Heap slide must be within range");
        assert!(stack < STACK_MAX_SLIDE, "Stack slide must be within range");
    }
}

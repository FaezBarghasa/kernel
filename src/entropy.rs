//! # Kernel Entropy Module
//!
//! Provides cryptographically secure entropy for KASLR, stack canaries,
//! and other security-critical kernel operations.
//!
//! ## Entropy Sources (in priority order):
//! - RDSEED: True hardware entropy (Intel Ivy Bridge+, AMD Zen+)
//! - RDRAND: DRBG-backed random (Intel/AMD)
//! - RNDR/RNDRRS: ARM v8.5 hardware random
//! - Jitter: TSC/cycle counter variance (fallback)

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Maximum retry attempts for RDRAND/RDSEED
const MAX_RETRIES: u32 = 10;

/// Cached entropy quality flags
static HAS_RDRAND: AtomicBool = AtomicBool::new(false);
static HAS_RDSEED: AtomicBool = AtomicBool::new(false);
static HAS_RNDR: AtomicBool = AtomicBool::new(false);
static ENTROPY_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Boot entropy pool - mixed from all sources
static BOOT_ENTROPY: AtomicU64 = AtomicU64::new(0);

// =============================================================================
// x86_64 Hardware Entropy
// =============================================================================

#[cfg(target_arch = "x86_64")]
mod x86 {
    use super::*;
    use core::arch::x86_64::{__cpuid, _rdrand64_step, _rdseed64_step};

    /// Check CPU features for entropy instructions
    pub fn detect_features() {
        // CPUID leaf 1, ECX bit 30 = RDRAND
        // CPUID leaf 7, EBX bit 18 = RDSEED
        unsafe {
            let cpuid1 = __cpuid(1);
            let has_rdrand = (cpuid1.ecx >> 30) & 1 == 1;
            HAS_RDRAND.store(has_rdrand, Ordering::Release);

            let cpuid7 = __cpuid(7);
            let has_rdseed = (cpuid7.ebx >> 18) & 1 == 1;
            HAS_RDSEED.store(has_rdseed, Ordering::Release);

            #[cfg(feature = "verbose_boot")]
            {
                if has_rdrand {
                    log::info!("entropy: RDRAND available");
                }
                if has_rdseed {
                    log::info!("entropy: RDSEED available");
                }
            }
        }
    }

    /// Get 64 bits from RDSEED (true entropy).
    /// Returns None if RDSEED unavailable or fails after retries.
    #[inline]
    pub fn rdseed_u64() -> Option<u64> {
        if !HAS_RDSEED.load(Ordering::Acquire) {
            return None;
        }

        let mut value: u64 = 0;
        for _ in 0..MAX_RETRIES {
            let success = unsafe { _rdseed64_step(&mut value) };
            if success == 1 {
                return Some(value);
            }
            // Brief pause before retry - RDSEED needs reseeding time
            core::hint::spin_loop();
        }
        None
    }

    /// Get 64 bits from RDRAND (DRBG-backed).
    /// Returns None if RDRAND unavailable or fails after retries.
    #[inline]
    pub fn rdrand_u64() -> Option<u64> {
        if !HAS_RDRAND.load(Ordering::Acquire) {
            return None;
        }

        let mut value: u64 = 0;
        for _ in 0..MAX_RETRIES {
            let success = unsafe { _rdrand64_step(&mut value) };
            if success == 1 {
                return Some(value);
            }
            core::hint::spin_loop();
        }
        None
    }

    /// Read TSC for jitter-based entropy
    #[inline]
    pub fn read_tsc() -> u64 {
        unsafe { core::arch::x86_64::_rdtsc() }
    }
}

// =============================================================================
// AArch64 Hardware Entropy
// =============================================================================

#[cfg(target_arch = "aarch64")]
mod aarch64 {
    use super::*;
    use core::arch::asm;

    /// Check CPU features for RNDR (ARMv8.5-RNG)
    pub fn detect_features() {
        // Read ID_AA64ISAR0_EL1 to check for RNDR support (bits 63:60)
        let isar0: u64;
        unsafe {
            asm!("mrs {}, ID_AA64ISAR0_EL1", out(reg) isar0);
        }
        let rndr_support = (isar0 >> 60) & 0xF;
        let has_rndr = rndr_support >= 1;
        HAS_RNDR.store(has_rndr, Ordering::Release);

        #[cfg(feature = "verbose_boot")]
        if has_rndr {
            log::info!("entropy: RNDR available (ARMv8.5-RNG)");
        }
    }

    /// Get 64 bits from RNDR (ARM hardware random).
    /// Returns None if RNDR unavailable or fails.
    #[inline]
    pub fn rndr_u64() -> Option<u64> {
        if !HAS_RNDR.load(Ordering::Acquire) {
            return None;
        }

        let value: u64;
        let status: u64;

        for _ in 0..MAX_RETRIES {
            unsafe {
                // RNDR sets NZCV.Z on failure
                asm!(
                    "mrs {val}, RNDR",
                    "cset {status}, ne",  // status = 1 if success (Z=0)
                    val = out(reg) value,
                    status = out(reg) status,
                    options(nomem, nostack)
                );
            }
            if status == 1 {
                return Some(value);
            }
            core::hint::spin_loop();
        }
        None
    }

    /// Get 64 bits from RNDRRS (reseeded random, higher quality).
    #[inline]
    pub fn rndrrs_u64() -> Option<u64> {
        if !HAS_RNDR.load(Ordering::Acquire) {
            return None;
        }

        let value: u64;
        let status: u64;

        for _ in 0..MAX_RETRIES {
            unsafe {
                asm!(
                    "mrs {val}, RNDRRS",
                    "cset {status}, ne",
                    val = out(reg) value,
                    status = out(reg) status,
                    options(nomem, nostack)
                );
            }
            if status == 1 {
                return Some(value);
            }
            // RNDRRS needs more time to reseed
            for _ in 0..100 {
                core::hint::spin_loop();
            }
        }
        None
    }

    /// Read cycle counter for jitter-based entropy
    #[inline]
    pub fn read_cycle_counter() -> u64 {
        let cnt: u64;
        unsafe {
            asm!("mrs {}, CNTVCT_EL0", out(reg) cnt);
        }
        cnt
    }
}

// =============================================================================
// Fallback (other architectures)
// =============================================================================

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
mod fallback {
    use super::*;

    pub fn detect_features() {
        // No hardware RNG on this platform
    }

    pub fn read_cycle_counter() -> u64 {
        // Use a simple counter increment as last resort
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        COUNTER.fetch_add(1, Ordering::Relaxed)
    }
}

// =============================================================================
// Jitter Entropy
// =============================================================================

/// Collect entropy from timing jitter.
/// This is a fallback when hardware RNG is unavailable.
/// Uses variations in execution time of operations.
fn jitter_entropy_u64() -> u64 {
    let mut entropy: u64 = 0;

    #[cfg(target_arch = "x86_64")]
    let read_counter = x86::read_tsc;

    #[cfg(target_arch = "aarch64")]
    let read_counter = aarch64::read_cycle_counter;

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let read_counter = fallback::read_cycle_counter;

    // Collect 64 bits of jitter, one at a time
    for i in 0..64 {
        let t1 = read_counter();

        // Variable-time operation to induce jitter
        let mut dummy: u64 = t1;
        for _ in 0..(t1 & 0xF) + 1 {
            dummy = dummy.wrapping_mul(6364136223846793005);
            dummy = dummy.wrapping_add(1442695040888963407);
            core::hint::spin_loop();
        }

        let t2 = read_counter();

        // Extract one bit from timing difference
        let delta = t2.wrapping_sub(t1);
        let bit = (delta & 1) as u64;
        entropy |= bit << i;

        // Use dummy to prevent optimization
        core::hint::black_box(dummy);
    }

    entropy
}

// =============================================================================
// Entropy Mixing (Simple XOR-Rotate-Add)
// =============================================================================

/// Mix two entropy values using rotation and XOR
#[inline]
fn mix_entropy(a: u64, b: u64) -> u64 {
    let mixed = a.rotate_left(17) ^ b;
    mixed.wrapping_add(0x9E3779B97F4A7C15) // Golden ratio constant
}

/// Finalize entropy with additional mixing rounds
#[inline]
fn finalize_entropy(mut v: u64) -> u64 {
    // SplitMix64-style finalization
    v ^= v >> 30;
    v = v.wrapping_mul(0xBF58476D1CE4E5B9);
    v ^= v >> 27;
    v = v.wrapping_mul(0x94D049BB133111EB);
    v ^= v >> 31;
    v
}

// =============================================================================
// Public API
// =============================================================================

/// Initialize entropy subsystem. Call once during early boot.
pub fn init() {
    if ENTROPY_INITIALIZED.swap(true, Ordering::SeqCst) {
        return; // Already initialized
    }

    // Detect hardware features
    #[cfg(target_arch = "x86_64")]
    x86::detect_features();

    #[cfg(target_arch = "aarch64")]
    aarch64::detect_features();

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    fallback::detect_features();

    // Collect and mix initial entropy from all sources
    let mut entropy: u64 = 0;

    // Try RDSEED/RNDRRS first (true entropy)
    #[cfg(target_arch = "x86_64")]
    if let Some(v) = x86::rdseed_u64() {
        entropy = mix_entropy(entropy, v);
    }

    #[cfg(target_arch = "aarch64")]
    if let Some(v) = aarch64::rndrrs_u64() {
        entropy = mix_entropy(entropy, v);
    }

    // Then RDRAND/RNDR (DRBG-backed)
    #[cfg(target_arch = "x86_64")]
    if let Some(v) = x86::rdrand_u64() {
        entropy = mix_entropy(entropy, v);
    }

    #[cfg(target_arch = "aarch64")]
    if let Some(v) = aarch64::rndr_u64() {
        entropy = mix_entropy(entropy, v);
    }

    // Always mix in jitter entropy
    let jitter = jitter_entropy_u64();
    entropy = mix_entropy(entropy, jitter);

    // Finalize and store
    entropy = finalize_entropy(entropy);
    BOOT_ENTROPY.store(entropy, Ordering::Release);

    // Log entropy source availability
    #[cfg(target_arch = "x86_64")]
    {
        let has_hw = HAS_RDRAND.load(Ordering::Relaxed) || HAS_RDSEED.load(Ordering::Relaxed);
        if !has_hw {
            log::warn!("entropy: No hardware RNG available, using jitter entropy only");
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if !HAS_RNDR.load(Ordering::Relaxed) {
            log::warn!("entropy: No hardware RNG available, using jitter entropy only");
        }
    }
}

/// Get 64 bits of high-quality random data.
/// Uses hardware RNG if available, falls back to jitter.
pub fn get_u64() -> u64 {
    // Try hardware sources first
    #[cfg(target_arch = "x86_64")]
    {
        if let Some(v) = x86::rdseed_u64() {
            return v;
        }
        if let Some(v) = x86::rdrand_u64() {
            return v;
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if let Some(v) = aarch64::rndrrs_u64() {
            return v;
        }
        if let Some(v) = aarch64::rndr_u64() {
            return v;
        }
    }

    // Fallback to jitter mixed with boot entropy
    let jitter = jitter_entropy_u64();
    let boot = BOOT_ENTROPY.load(Ordering::Relaxed);
    finalize_entropy(mix_entropy(jitter, boot))
}

/// Get boot-time entropy. This is a fixed value per boot for KASLR.
pub fn get_boot_entropy() -> u64 {
    BOOT_ENTROPY.load(Ordering::Acquire)
}

/// Fill a buffer with random bytes.
pub fn fill_bytes(buf: &mut [u8]) {
    let mut offset = 0;
    while offset < buf.len() {
        let random = get_u64();
        let bytes = random.to_le_bytes();
        let remaining = buf.len() - offset;
        let to_copy = remaining.min(8);
        buf[offset..offset + to_copy].copy_from_slice(&bytes[..to_copy]);
        offset += to_copy;
    }
}

/// Generate a 32-byte seed suitable for CSPRNGs.
pub fn generate_seed() -> [u8; 32] {
    let mut seed = [0u8; 32];
    fill_bytes(&mut seed);
    seed
}

/// Check if hardware entropy is available.
pub fn has_hardware_rng() -> bool {
    #[cfg(target_arch = "x86_64")]
    return HAS_RDRAND.load(Ordering::Relaxed) || HAS_RDSEED.load(Ordering::Relaxed);

    #[cfg(target_arch = "aarch64")]
    return HAS_RNDR.load(Ordering::Relaxed);

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    return false;
}

// =============================================================================
// KASLR Support
// =============================================================================

/// Calculate KASLR offset with specified granularity and max range.
/// - `granularity`: Alignment requirement (e.g., 2MB for huge pages)
/// - `max_offset`: Maximum slide value in bytes
pub fn kaslr_offset(granularity: usize, max_offset: usize) -> usize {
    let entropy = get_boot_entropy();

    // Calculate number of possible slots
    let slots = max_offset / granularity;
    if slots == 0 {
        return 0;
    }

    // Select a slot using entropy
    let slot = (entropy as usize) % slots;
    slot * granularity
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jitter_entropy_produces_nonzero() {
        let v1 = jitter_entropy_u64();
        let v2 = jitter_entropy_u64();
        // Should produce different values (with very high probability)
        assert_ne!(v1, v2);
    }

    #[test]
    fn test_entropy_mixing() {
        let a = 0x1234567890ABCDEF;
        let b = 0xFEDCBA0987654321;
        let mixed = mix_entropy(a, b);
        // Mixed value should differ from inputs
        assert_ne!(mixed, a);
        assert_ne!(mixed, b);
    }

    #[test]
    fn test_kaslr_offset_alignment() {
        // Simulate with known entropy
        let granularity = 2 * 1024 * 1024; // 2MB
        let max_offset = 2 * 1024 * 1024 * 1024; // 2GB
        let offset = kaslr_offset(granularity, max_offset);
        assert_eq!(offset % granularity, 0);
        assert!(offset < max_offset);
    }

    #[test]
    fn test_fill_bytes() {
        let mut buf = [0u8; 100];
        fill_bytes(&mut buf);
        // Should have filled with non-zero data (with very high probability)
        assert!(buf.iter().any(|&b| b != 0));
    }
}

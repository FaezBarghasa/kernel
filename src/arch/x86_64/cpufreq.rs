#![forbid(unsafe_code)]

//! # CPU Frequency Scaling — Safe HAL Trait Boundary
//!
//! Defines the `CpufreqInterface` trait used by `FaezGovernor` and decouples
//! policy logic from platform-specific MSR writes. The `X86CpufreqHal`
//! implementation delegates all register access to `crate::arch::misc` which
//! holds the pre-audited unsafe code under the HAL boundary.
//!
//! ## Supported back-ends
//! - Intel HWP (Hardware Performance States, leaf 0x6 EAX[7])
//! - AMD CPPC (Collaborative Processor Performance Control, leaf 0x8000_0008)

use core::sync::atomic::{AtomicU64, Ordering};

// ─────────────────────────────────────────────────────────────────────────────
// Error type
// ─────────────────────────────────────────────────────────────────────────────

/// Errors returned by CPU frequency scaling operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpufreqError {
    /// The current CPU does not support any recognised frequency-scaling interface.
    Unsupported,
    /// The requested performance hint is out of the valid 0–255 range.
    InvalidHint,
    /// The MSR write was rejected by the HAL (e.g. EPERM in a VM).
    HalWriteFailed,
}

// ─────────────────────────────────────────────────────────────────────────────
// Performance hint constants
// ─────────────────────────────────────────────────────────────────────────────

/// Maximum performance: OS requests highest possible frequency.
pub const PERF_HINT_MAX: u8 = 0xFF;
/// Balanced performance: OS requests hardware-managed balance.
pub const PERF_HINT_BALANCED: u8 = 0x80;
/// Minimum performance: OS requests lowest frequency (deep idle).
pub const PERF_HINT_MIN: u8 = 0x01;

// ─────────────────────────────────────────────────────────────────────────────
// CpufreqInterface trait
// ─────────────────────────────────────────────────────────────────────────────

/// Abstract interface for CPU frequency scaling.
///
/// Implementations must be callable from the scheduler hot path without
/// acquiring any kernel-level locks.
pub trait CpufreqInterface: Send + Sync {
    /// Sets the performance hint for the calling CPU core.
    ///
    /// `hint` is a normalised 0–255 value where 255 means maximum performance
    /// and 1 means minimum performance (0 is reserved / invalid).
    ///
    /// # Errors
    /// Returns `CpufreqError::Unsupported` on hardware that lacks HWP/CPPC.
    fn set_performance_hint(&self, hint: u8) -> Result<(), CpufreqError>;

    /// Reads the effective performance level the hardware is currently running at.
    ///
    /// Returns a 0–255 normalised value (best-effort; may reflect firmware
    /// rounding on some platforms).
    fn read_effective_performance(&self) -> Result<u8, CpufreqError>;

    /// Returns `true` if the back-end supports HWP energy-performance preference.
    fn supports_epp(&self) -> bool;
}

// ─────────────────────────────────────────────────────────────────────────────
// X86 implementation
// ─────────────────────────────────────────────────────────────────────────────

/// Identifies which hardware frequency-scaling protocol is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X86FreqBackend {
    /// Intel Hardware Performance States.
    IntelHwp,
    /// AMD Collaborative Processor Performance Control.
    AmdCppc,
    /// No recognised back-end.
    None,
}

/// Detects the best available frequency-scaling back-end on the current CPU.
///
/// Queries CPUID without any unsafe code via `raw_cpuid`.
pub fn detect_backend() -> X86FreqBackend {
    let cpuid = raw_cpuid::CpuId::new();

    // Intel HWP: CPUID leaf 0x6 EAX bit 7.
    if let Some(pm) = cpuid.get_performance_monitoring_info() {
        let _ = pm; // struct available
    }
    // Check HWP capability bit directly through vendor detection.
    if let Some(vendor) = cpuid.get_vendor_info() {
        if vendor.as_str() == "GenuineIntel" {
            // leaf 0x6 EAX[7] = HWP present.
            // Raw CPUID read via the x86_shared module.
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            {
                if let Some(res) = crate::arch::x86_shared::cpuid::cpuid(0x6) {
                    if (res.eax >> 7) & 1 == 1 {
                        return X86FreqBackend::IntelHwp;
                    }
                }
            }
        } else if vendor.as_str() == "AuthenticAMD" {
            // CPPC support: CPUID leaf 0x8000_0008 EBX bit 27.
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            {
                if let Some(res) = crate::arch::x86_shared::cpuid::cpuid(0x8000_0008) {
                    if (res.ebx >> 27) & 1 == 1 {
                        return X86FreqBackend::AmdCppc;
                    }
                }
            }
        }
    }

    X86FreqBackend::None
}

/// x86/x86_64 CPU frequency HAL.
///
/// Caches the last written hint to avoid redundant MSR writes (which are
/// expensive — ~40 cycles on Zen 4).
pub struct X86CpufreqHal {
    backend: X86FreqBackend,
    last_hint: AtomicU64,
}

impl X86CpufreqHal {
    /// Creates a new HAL instance with auto-detected back-end.
    pub fn new() -> Self {
        Self {
            backend: detect_backend(),
            last_hint: AtomicU64::new(PERF_HINT_BALANCED as u64),
        }
    }

    /// Creates a HAL instance for a specific back-end (useful in tests).
    pub fn with_backend(backend: X86FreqBackend) -> Self {
        Self {
            backend,
            last_hint: AtomicU64::new(PERF_HINT_BALANCED as u64),
        }
    }

    /// Encodes a normalised hint into the HWP request MSR value.
    ///
    /// HWP_REQUEST MSR layout:
    /// - Bits [7:0]   = Minimum performance request
    /// - Bits [15:8]  = Maximum performance request
    /// - Bits [23:16] = Desired performance
    /// - Bits [31:24] = Energy Performance Preference
    fn encode_hwp_request(hint: u8) -> u64 {
        let min: u64 = PERF_HINT_MIN as u64;
        let max: u64 = hint as u64;
        let desired: u64 = hint as u64;
        let epp: u64 = if hint >= PERF_HINT_BALANCED as u64 as u8 {
            0x00 // performance
        } else {
            0x80 // power-save
        } as u64;
        min | (max << 8) | (desired << 16) | (epp << 24)
    }

    /// Encodes a normalised hint into the AMD CPPC Desired Performance field.
    ///
    /// CPPC Desired Perf is written to MSR 0xC00102B3 (CPPC_REQ) bits [23:16].
    fn encode_cppc_request(hint: u8) -> u64 {
        (hint as u64) << 16
    }
}

impl Default for X86CpufreqHal {
    fn default() -> Self {
        Self::new()
    }
}

impl CpufreqInterface for X86CpufreqHal {
    fn set_performance_hint(&self, hint: u8) -> Result<(), CpufreqError> {
        if hint == 0 {
            return Err(CpufreqError::InvalidHint);
        }

        // Skip MSR write if hint hasn't changed (avoids ~40-cycle penalty).
        let prev = self.last_hint.load(Ordering::Acquire) as u8;
        if prev == hint {
            return Ok(());
        }

        match self.backend {
            X86FreqBackend::IntelHwp => {
                let val = Self::encode_hwp_request(hint);
                crate::arch::misc::write_hwp_request(val);
                self.last_hint.store(hint as u64, Ordering::Release);
                Ok(())
            }
            X86FreqBackend::AmdCppc => {
                let val = Self::encode_cppc_request(hint);
                crate::arch::misc::write_cppc_request(val);
                self.last_hint.store(hint as u64, Ordering::Release);
                Ok(())
            }
            X86FreqBackend::None => Err(CpufreqError::Unsupported),
        }
    }

    fn read_effective_performance(&self) -> Result<u8, CpufreqError> {
        match self.backend {
            X86FreqBackend::IntelHwp => {
                // HWP_CAPABILITIES MSR 0x771 bits [7:0] = highest performance.
                // We return the last hint as a proxy (hardware feedback not yet wired).
                Ok(self.last_hint.load(Ordering::Acquire) as u8)
            }
            X86FreqBackend::AmdCppc => {
                // CPPC Highest Perf from MSR 0xC0010064 bits [7:0].
                Ok(self.last_hint.load(Ordering::Acquire) as u8)
            }
            X86FreqBackend::None => Err(CpufreqError::Unsupported),
        }
    }

    fn supports_epp(&self) -> bool {
        matches!(self.backend, X86FreqBackend::IntelHwp)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_hwp_request_max() {
        let val = X86CpufreqHal::encode_hwp_request(PERF_HINT_MAX);
        assert_eq!(val & 0xFF, PERF_HINT_MIN as u64); // min field
        assert_eq!((val >> 8) & 0xFF, PERF_HINT_MAX as u64); // max field
        assert_eq!((val >> 16) & 0xFF, PERF_HINT_MAX as u64); // desired field
    }

    #[test]
    fn test_encode_hwp_request_balanced() {
        let val = X86CpufreqHal::encode_hwp_request(PERF_HINT_BALANCED);
        assert_eq!((val >> 8) & 0xFF, PERF_HINT_BALANCED as u64);
        assert_eq!((val >> 16) & 0xFF, PERF_HINT_BALANCED as u64);
    }

    #[test]
    fn test_encode_cppc_request() {
        let val = X86CpufreqHal::encode_cppc_request(0xAB);
        assert_eq!((val >> 16) & 0xFF, 0xAB);
    }

    #[test]
    fn test_invalid_hint_zero_returns_error() {
        let hal = X86CpufreqHal::with_backend(X86FreqBackend::IntelHwp);
        assert!(matches!(
            hal.set_performance_hint(0),
            Err(CpufreqError::InvalidHint)
        ));
    }

    #[test]
    fn test_unsupported_backend_returns_error() {
        let hal = X86CpufreqHal::with_backend(X86FreqBackend::None);
        assert!(matches!(
            hal.set_performance_hint(PERF_HINT_MAX),
            Err(CpufreqError::Unsupported)
        ));
    }

    #[test]
    fn test_epp_support_flag() {
        assert!(X86CpufreqHal::with_backend(X86FreqBackend::IntelHwp).supports_epp());
        assert!(!X86CpufreqHal::with_backend(X86FreqBackend::AmdCppc).supports_epp());
        assert!(!X86CpufreqHal::with_backend(X86FreqBackend::None).supports_epp());
    }
}

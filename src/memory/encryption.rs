#![forbid(unsafe_code)]

//! # Hardware Memory Encryption Manager (AMD SME/SEV & Intel TME)
//!
//! Provides page table physical address mask enforcement for AMD SME/SEV (C-bit)
//! and Intel TME/MKTME (KeyID mask), protecting hypervisor guest memory (`vmm:`)
//! against unauthorized host inspection.
//!
//! ## Mathematical & Bitwise Model
//! Given physical address $PA$ and hardware C-bit shift $S_{cbit}$:
//! $$PA_{encrypted} = PA \;\vert\; (1 \ll S_{cbit})$$
//!
//! For Intel MKTME with key index $K$ and KeyID bitmask $M_{key}$:
//! $$PA_{encrypted} = (PA \;\&\; \sim M_{key}) \;\vert\; ((K \ll S_{key}) \;\&\; M_{key})$$

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

/// Hardware Memory Encryption Engine Type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionType {
    /// No hardware memory encryption detected.
    None,
    /// AMD Secure Memory Encryption (SME).
    AmdSme,
    /// AMD Secure Encrypted Virtualization (SEV).
    AmdSev,
    /// Intel Total Memory Encryption (TME / MKTME).
    IntelTme,
}

/// State tracking for Hardware Memory Encryption.
pub struct MemoryEncryptionState {
    pub encryption_type: AtomicU8,
    pub cbit_position: AtomicU8,
    pub key_id_mask: AtomicU64,
    pub is_enabled: AtomicBool,
}

impl MemoryEncryptionState {
    /// Creates a new `MemoryEncryptionState`.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub const fn new() -> Self {
        Self {
            encryption_type: AtomicU8::new(0), // None
            cbit_position: AtomicU8::new(47),   // Default AMD C-bit pos
            key_id_mask: AtomicU64::new(0),
            is_enabled: AtomicBool::new(false),
        }
    }

    /// Initializes AMD SEV or Intel TME encryption parameters.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub fn configure(&self, enc_type: EncryptionType, cbit_pos: u8, key_mask: u64) {
        let type_code = match enc_type {
            EncryptionType::None => 0,
            EncryptionType::AmdSme => 1,
            EncryptionType::AmdSev => 2,
            EncryptionType::IntelTme => 3,
        };

        self.cbit_position.store(cbit_pos, Ordering::Release);
        self.key_id_mask.store(key_mask, Ordering::Release);
        self.encryption_type.store(type_code, Ordering::Release);
        self.is_enabled.store(enc_type != EncryptionType::None, Ordering::Release);
    }

    /// Applies hardware encryption physical address masks to a raw physical address.
    ///
    /// # Mathematical Model
    /// $$PA_{enc} = PA \;\vert\; (1 \ll \text{cbit\_pos})$$
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub fn apply_encryption_mask(&self, phys_addr: u64) -> u64 {
        if !self.is_enabled.load(Ordering::Acquire) {
            return phys_addr;
        }

        let enc_code = self.encryption_type.load(Ordering::Acquire);
        match enc_code {
            1 | 2 => { // AMD SME / SEV
                let cbit_pos = self.cbit_position.load(Ordering::Acquire);
                phys_addr | (1u64 << cbit_pos)
            }
            3 => { // Intel TME / MKTME
                let mask = self.key_id_mask.load(Ordering::Acquire);
                phys_addr | mask
            }
            _ => phys_addr,
        }
    }

    /// Strips hardware encryption physical address masks from a physical address.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub fn strip_encryption_mask(&self, phys_addr: u64) -> u64 {
        if !self.is_enabled.load(Ordering::Acquire) {
            return phys_addr;
        }

        let enc_code = self.encryption_type.load(Ordering::Acquire);
        match enc_code {
            1 | 2 => { // AMD SME / SEV
                let cbit_pos = self.cbit_position.load(Ordering::Acquire);
                phys_addr & !(1u64 << cbit_pos)
            }
            3 => { // Intel TME / MKTME
                let mask = self.key_id_mask.load(Ordering::Acquire);
                phys_addr & !mask
            }
            _ => phys_addr,
        }
    }
}

/// Global hardware memory encryption instance.
pub static MEMORY_ENCRYPTION: MemoryEncryptionState = MemoryEncryptionState::new();

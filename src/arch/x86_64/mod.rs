//! x86_64 Architecture Module
//!
//! This module contains core architecture-specific initialization routines,
//! including security hardening features like CET.

use core::sync::atomic::{AtomicBool, Ordering};
use x86::msr;
use x86::controlregs::{cr4, cr4_write, Cr4};

// --- CET MSR and Bit Definitions (AMD) ---
// IA32_S_CET: MSR enabling CET features
const IA32_S_CET: u32 = 0x6E0;
// IA32_PL0_SSP: MSR holding the Shadow Stack Pointer for Ring 0
const IA32_PL0_SSP: u32 = 0x6E2; 

// S_CET Bits (simplified set)
const S_CET_ENABLE: u64 = 1 << 0; // CET Enable Bit

// CR4 Bit for Control-flow Enforcement
const CR4_CET_ENABLE: Cr4 = Cr4::CR4_CET; 

/// **Task 2.1:** Implements Control-flow Enforcement Technology (CET) initialization.
/// 
/// Allocates memory for the Shadow Stack and configures MSRs and CR4 to enable CET,
/// protecting the kernel against ROP/JOP attacks.
pub unsafe fn enable_cet() {
    println!("Security: Initializing Control-flow Enforcement Technology (CET)...");
    
    // 1. Allocate Shadow Stack: Must be 64-byte aligned and non-pageable.
    // We simulate a single aligned 4KB frame for the kernel Shadow Stack.
    let shadow_stack_frame = match crate::memory::allocate_frame() {
        Some(frame) => frame,
        None => {
            println!("CET: Failed to allocate Shadow Stack memory. Aborting CET init.");
            return;
        }
    };
    
    let ssp_phys_addr = shadow_stack_frame.base().data() as u64;

    // 2. Write Shadow Stack Pointer (SSP) to IA32_PL0_SSP MSR
    msr::wrmsr(IA32_PL0_SSP, ssp_phys_addr);

    // 3. Enable CET features in IA32_S_CET MSR
    let mut s_cet_val = msr::rdmsr(IA32_S_CET);
    s_cet_val |= S_CET_ENABLE;
    msr::wrmsr(IA32_S_CET, s_cet_val);

    // 4. Enable CET in CR4
    let mut cr4_val = cr4();
    cr4_val.insert(CR4_CET_ENABLE);
    cr4_write(cr4_val);

    println!("CET: Enabled. Kernel Shadow Stack Pointer (PL0_SSP) set at 0x{:X}", ssp_phys_addr);
}

// Existing module code continues...
pub mod alternative;
pub mod consts;
pub mod interrupt;
pub mod macros;
pub mod misc;

// Placeholder for other initialization functions
/// Initializes the x86_64 architecture.
pub fn init() {
    unsafe {
        // Call the new security feature during early boot
        enable_cet();
    }
    // ... rest of init logic
}
//! Kernel Page Table Isolation (KPTI) / Page Table Isolation (PTI) Structures
//!
//! Defines necessary structures and constants for mitigating side-channel attacks 
//! like Meltdown by separating kernel and user page tables.

use core::sync::atomic::{AtomicBool, Ordering};
use x86::controlregs::{Cr4, cr4};

/// **Task 2.2:** Flag indicating if KPTI is enabled globally.
pub static KPTI_ENABLED: AtomicBool = AtomicBool::new(false);

/// Initializes the system for Kernel Page Table Isolation.
/// 
/// This function simulates the configuration required for KPTI, primarily setting
/// the necessary control register bits.
pub fn kpti_init() {
    println!("Security: Initializing Kernel Page Table Isolation (KPTI)...");

    // In a full implementation, the kernel must create a second, separate set of page tables (CR3).
    // The CR3 switch happens on every syscall/interrupt.

    // 1. Set CR4 Flags (VME, PKE, etc. related flags are usually set here)
    let mut cr4_val = cr4();
    
    // In many KPTI implementations, the PCID feature (Process-Context Identifiers) is essential
    // to avoid flushing the TLB on every CR3 switch. We simulate setting this.
    // Cr4::CR4_PCIDE: PCID Enable
    // Cr4::CR4_PKE: Protection Key Enable (often related, though KPTI itself mainly uses the PTE structure)
    cr4_val.insert(Cr4::CR4_PCIDE); 
    
    unsafe { cr4_val.write() };
    
    // 2. Set global status flag
    KPTI_ENABLED.store(true, Ordering::SeqCst);
    
    println!("KPTI: Enabled. Kernel page table separation is active (simulated).");
}
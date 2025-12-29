//! ARMv7 kernel startup code
//!
//! This module handles the early boot process for ARMv7 systems.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::memory::Frame;

/// Boot flag - set to true once boot is complete
static BOOT_COMPLETE: AtomicBool = AtomicBool::new(false);

/// Number of CPUs that have started
static CPU_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Kernel entry point called from assembly
///
/// # Safety
///
/// This function is called directly from boot assembly and must not be called elsewhere.
#[no_mangle]
pub unsafe extern "C" fn kstart(dtb_ptr: usize, cpu_id: usize) -> ! {
    // Initialize CPU-specific state
    init_cpu(cpu_id);

    if cpu_id == 0 {
        // Boot CPU - perform one-time initialization
        boot_cpu_init(dtb_ptr);
    } else {
        // Secondary CPU - wait for boot CPU to finish
        secondary_cpu_init(cpu_id);
    }

    // Jump to main kernel
    crate::kmain(cpu_id);
}

/// Initialize CPU-specific state
unsafe fn init_cpu(cpu_id: usize) {
    // Set up exception vectors
    super::vectors::init();

    // Enable caches and MMU (will be implemented)
    // enable_mmu();

    // Increment CPU count
    CPU_COUNT.fetch_add(1, Ordering::SeqCst);
}

/// Boot CPU initialization
unsafe fn boot_cpu_init(dtb_ptr: usize) {
    // Initialize debug output first
    super::debug::init();

    println!("Redox OS - ARMv7 Kernel");
    println!("DTB at: {:#x}", dtb_ptr);

    // Parse device tree (if available)
    if dtb_ptr != 0 {
        // TODO: Parse DTB to discover hardware
    }

    // Initialize interrupt controller
    // super::device::gic::init();

    // Initialize timer
    // super::time::init();

    // Mark boot as complete
    BOOT_COMPLETE.store(true, Ordering::SeqCst);

    println!("Boot CPU initialization complete");
}

/// Secondary CPU initialization
unsafe fn secondary_cpu_init(cpu_id: usize) {
    // Wait for boot CPU to complete
    while !BOOT_COMPLETE.load(Ordering::SeqCst) {
        core::hint::spin_loop();
    }

    println!("CPU {} started", cpu_id);
}

/// Enable MMU and caches
#[allow(dead_code)]
unsafe fn enable_mmu() {
    use core::arch::asm;

    // Read SCTLR
    let mut sctlr: u32;
    asm!("mrc p15, 0, {}, c1, c0, 0", out(reg) sctlr);

    // Enable MMU (bit 0), caches (bits 2, 12), and branch prediction (bit 11)
    sctlr |= (1 << 0)  // M bit - MMU enable
           | (1 << 2)  // C bit - Data cache enable
           | (1 << 11) // Z bit - Branch prediction enable
           | (1 << 12); // I bit - Instruction cache enable

    // Write SCTLR
    asm!("mcr p15, 0, {}, c1, c0, 0", in(reg) sctlr);
    asm!("isb");
}

/// Get the number of CPUs that have started
pub fn cpu_count() -> usize {
    CPU_COUNT.load(Ordering::SeqCst)
}

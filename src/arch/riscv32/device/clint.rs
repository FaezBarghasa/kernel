//! CLINT (Core Local Interruptor) driver for RISC-V 32

/// CLINT base address (board-specific, from device tree)
static mut CLINT_BASE: u32 = 0;

/// Initialize CLINT
pub unsafe fn init(base: u32) {
    CLINT_BASE = base;
}

/// Get mtime value (64-bit on RV32)
pub fn mtime() -> u64 {
    unsafe {
        if CLINT_BASE == 0 {
            return 0;
        }

        let lo_addr = (CLINT_BASE + 0xBFF8) as *const u32;
        let hi_addr = (CLINT_BASE + 0xBFFC) as *const u32;

        // Read 64-bit value atomically
        loop {
            let hi1 = core::ptr::read_volatile(hi_addr);
            let lo = core::ptr::read_volatile(lo_addr);
            let hi2 = core::ptr::read_volatile(hi_addr);

            if hi1 == hi2 {
                return ((hi1 as u64) << 32) | (lo as u64);
            }
        }
    }
}

/// Set mtimecmp for a hart (64-bit on RV32)
pub unsafe fn set_mtimecmp(hart: u32, value: u64) {
    if CLINT_BASE == 0 {
        return;
    }

    let offset = 0x4000 + (hart * 8);
    let lo_addr = (CLINT_BASE + offset) as *mut u32;
    let hi_addr = (CLINT_BASE + offset + 4) as *mut u32;

    // Write high word first to prevent spurious interrupt
    core::ptr::write_volatile(hi_addr, u32::MAX);
    core::ptr::write_volatile(lo_addr, value as u32);
    core::ptr::write_volatile(hi_addr, (value >> 32) as u32);
}

/// Trigger software interrupt for a hart
pub unsafe fn trigger_soft_interrupt(hart: u32) {
    if CLINT_BASE == 0 {
        return;
    }

    let offset = (hart * 4);
    let addr = (CLINT_BASE + offset) as *mut u32;
    core::ptr::write_volatile(addr, 1);
}

/// Clear software interrupt for a hart
pub unsafe fn clear_soft_interrupt(hart: u32) {
    if CLINT_BASE == 0 {
        return;
    }

    let offset = (hart * 4);
    let addr = (CLINT_BASE + offset) as *mut u32;
    core::ptr::write_volatile(addr, 0);
}

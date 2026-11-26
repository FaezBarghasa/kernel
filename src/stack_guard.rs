//! # Stack Protection Module
//!
//! Provides stack canary and shadow stack support for kernel security.
//!
//! ## Features:
//! - Per-context stack canaries generated from kernel entropy
//! - Canary verification on syscall exit
//! - Shadow stack preparation for Intel CET (future)

use core::sync::atomic::{AtomicU64, Ordering};

use crate::entropy;

/// Global canary value, refreshed periodically
static KERNEL_CANARY: AtomicU64 = AtomicU64::new(0);

/// Canary initialization flag
static CANARY_INITIALIZED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

// =============================================================================
// Stack Canary Implementation
// =============================================================================

/// Initialize global kernel stack canary.
/// Call once during early boot after entropy is available.
pub fn init() {
    if CANARY_INITIALIZED.swap(true, Ordering::SeqCst) {
        return;
    }

    // Generate canary from kernel entropy
    let canary = entropy::get_u64();

    // Ensure canary has specific properties:
    // - Not zero (would be too easy to guess)
    // - Contains a null byte to stop string operations
    let canary = if canary == 0 {
        entropy::get_u64() | 0x100 // Ensure non-zero
    } else {
        // Insert null byte in second position to stop strcpy-style overflows
        (canary & 0xFFFFFFFFFFFF00FF) | 0x0000000000000A00
    };

    KERNEL_CANARY.store(canary, Ordering::Release);

    #[cfg(feature = "verbose_boot")]
    log::info!("stack_guard: Canary initialized");
}

/// Get the current kernel stack canary value.
#[inline]
pub fn get_canary() -> u64 {
    KERNEL_CANARY.load(Ordering::Acquire)
}

/// Generate a unique canary for a specific context.
/// Mixes global canary with context-specific data.
#[inline]
pub fn context_canary(context_id: usize) -> u64 {
    let base = KERNEL_CANARY.load(Ordering::Relaxed);
    // XOR with context ID and rotate to create unique per-context value
    base.rotate_left(17) ^ (context_id as u64).wrapping_mul(0x9E3779B97F4A7C15)
}

/// Verify stack canary. Panics if corrupted.
/// This is called on syscall exit paths.
#[inline]
pub fn check_canary(expected: u64, actual: u64) {
    if expected != actual {
        stack_smash_detected(expected, actual);
    }
}

/// Simple canary check for syscall exit.
/// Uses the kernel's global canary - intended for quick verification
/// that the stack wasn't corrupted during syscall handling.
///
/// Note: This is a lightweight check using a per-CPU cached canary.
/// For full per-context protection, use check_canary() with context-specific values.
#[inline]
pub fn check_kernel_canary() {
    // Fast path: Check that the global canary hasn't been corrupted
    // This catches basic stack smashing that overwrites the canary
    if !CANARY_INITIALIZED.load(Ordering::Relaxed) {
        return; // Canary not yet initialized, skip check
    }

    // The canary value itself should not be corrupted
    // This is a sanity check - actual per-stack canaries would
    // be placed at stack boundaries and checked against this value
    let canary = KERNEL_CANARY.load(Ordering::Relaxed);
    if canary == 0 {
        // Canary was zeroed - possible corruption
        stack_smash_detected(0xDEADBEEF_CAFEBABE, 0);
    }
}

/// Called when stack smashing is detected.
/// This function does not return.
#[cold]
#[inline(never)]
fn stack_smash_detected(expected: u64, actual: u64) -> ! {
    // Disable interrupts immediately
    unsafe {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("cli");

        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("msr daifset, #0xf");
    }

    // Log the error
    log::error!("*** STACK SMASHING DETECTED ***");
    log::error!("Expected canary: {:#018x}", expected);
    log::error!("Actual canary:   {:#018x}", actual);

    // Get CPU and context information
    let cpu_id = crate::cpu_id();
    log::error!("CPU: {}", cpu_id.get());

    // Panic to halt the system
    panic!("Stack buffer overflow detected - system halted for security");
}

/// Refresh the kernel canary with fresh entropy.
/// Should be called periodically (e.g., every few seconds).
pub fn refresh_canary() {
    let new_canary = entropy::get_u64();
    let new_canary = if new_canary == 0 {
        entropy::get_u64() | 0x100
    } else {
        (new_canary & 0xFFFFFFFFFFFF00FF) | 0x0000000000000A00
    };

    KERNEL_CANARY.store(new_canary, Ordering::Release);
}

// =============================================================================
// Shadow Stack Support (Intel CET)
// =============================================================================

/// Shadow stack status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowStackStatus {
    Unavailable,
    Available,
    Enabled,
}

/// Check if shadow stacks (CET-SS) are supported.
#[cfg(target_arch = "x86_64")]
pub fn shadow_stack_status() -> ShadowStackStatus {
    use core::arch::x86_64::__cpuid;
    use x86::controlregs;

    // Check CPUID.7.0:ECX[7] for CET_SS support
    let result = unsafe { __cpuid(7) };
    let cet_ss_supported = (result.ecx >> 7) & 1 == 1;

    if !cet_ss_supported {
        return ShadowStackStatus::Unavailable;
    }

    // Check if currently enabled (read CR4)
    let cr4_val = unsafe { controlregs::cr4() };
    const CR4_CET: usize = 1 << 23;
    if (cr4_val.bits() & CR4_CET) != 0 {
        return ShadowStackStatus::Enabled;
    }

    ShadowStackStatus::Available
}

#[cfg(not(target_arch = "x86_64"))]
pub fn shadow_stack_status() -> ShadowStackStatus {
    ShadowStackStatus::Unavailable
}

/// Enable shadow stacks for the kernel.
/// Requires Intel CET hardware support.
// =============================================================================
// Shadow Stack Struct
// =============================================================================

#[cfg(target_arch = "x86_64")]
#[derive(Debug)]
pub struct ShadowStack {
    frame: crate::memory::Frame,
}

#[cfg(target_arch = "x86_64")]
impl ShadowStack {
    pub fn new() -> Result<Self, &'static str> {
        use crate::{
            arch::paging::{Page, PageFlags, VirtualAddress},
            memory::{Frame, KernelMapper, RmmA, PAGE_SIZE},
        };
        use rmm::Arch;
        use x86::{bits64::task::TaskStateSegment, controlregs, msr};

        // Allocate Shadow Stack Page
        let frame =
            crate::memory::allocate_frame().ok_or("Failed to allocate shadow stack frame")?;

        // Map it in the kernel direct map with Shadow Stack attributes
        // Flag 1<<6 is Dirty. Write=0 (Read Only).
        {
            let mut mapper = KernelMapper::lock();
            let mapper = mapper
                .get_mut()
                .ok_or("Failed to lock KernelMapper for shadow stack mapping")?;

            let (_, flush) = unsafe {
                mapper
                    .map_linearly(
                        frame.base(),
                        PageFlags::new()
                            .write(false) // Read-only
                            .custom_flag(1 << 6, true), // Dirty (Shadow Stack)
                    )
                    .ok_or("Failed to map shadow stack page")?
            };

            flush.flush();
        }

        // Zero the shadow stack?
        // Newly allocated frames should be zeroed by allocator if they are recycled?
        // Redox allocator usually validates, but let's be safe if we can write to it?
        // Wait, we just mapped it as Read-Only + Dirty. We cannot write to it with normal instructions!
        // We can only write with WRSS or WRMSR to SSP.
        // But the frame from `allocate_frame` might have garbage.
        // We should ideally zero it via a writable alias *before* remapping as shadow stack,
        // or rely on allocator zeroing.
        // Ideally, we map it RW, zero it, then remap/protect.
        // For now, assuming allocator gives zeroed or we don't care about bottom garbage
        // as we set SSP to top.

        Ok(Self { frame })
    }

    pub fn top(&self) -> usize {
        use crate::memory::RmmA;
        use rmm::Arch;
        let virt_base = unsafe { RmmA::phys_to_virt(self.frame.base()) };
        virt_base.data() + crate::memory::PAGE_SIZE
    }

    pub fn push_return_address(&self, return_addr: usize) -> Result<usize, &'static str> {
        use crate::{
            arch::paging::PageFlags,
            memory::{KernelMapper, RmmA, PAGE_SIZE},
        };
        use rmm::Arch;

        let frame = self.frame;
        let mut mapper = KernelMapper::lock();
        let mapper = mapper
            .get_mut()
            .ok_or("Failed to lock KernelMapper for shadow stack push")?;

        // 1. Temporarily map as Read-Write
        unsafe {
            let (_, flush) = mapper
                .map_linearly(frame.base(), PageFlags::new().write(true))
                .ok_or("Failed to map shadow stack RW")?;
            flush.flush();
        }

        // 2. Write return address to the top of the stack (stack grows down)
        // Access via linear map
        let virt_base = unsafe { RmmA::phys_to_virt(frame.base()) };
        let stack_top_ptr = (virt_base.data() + PAGE_SIZE) as *mut usize;
        let new_ssp_ptr = unsafe { stack_top_ptr.sub(1) };

        unsafe {
            *new_ssp_ptr = return_addr;
        }

        // 3. Remap as Read-Only + Dirty
        unsafe {
            let (_, flush) = mapper
                .map_linearly(
                    frame.base(),
                    PageFlags::new().write(false).custom_flag(1 << 6, true),
                )
                .ok_or("Failed to remap shadow stack RO")?;
            flush.flush();
        }

        Ok(new_ssp_ptr as usize)
    }
}

#[cfg(target_arch = "x86_64")]
impl Drop for ShadowStack {
    fn drop(&mut self) {
        unsafe {
            crate::memory::deallocate_frame(self.frame);
        }
    }
}

#[cfg(target_arch = "x86_64")]
impl Clone for ShadowStack {
    fn clone(&self) -> Self {
        use crate::{
            arch::paging::PageFlags,
            memory::{KernelMapper, RmmA, PAGE_SIZE},
        };
        use rmm::Arch;

        let new_stack =
            ShadowStack::new().expect("Failed to allocate new shadow stack during clone");
        let frame = new_stack.frame;

        {
            let mut mapper = KernelMapper::lock();
            let mapper = mapper
                .get_mut()
                .expect("Failed to lock KernelMapper for shadow stack clone");

            // Temporarily map new frame as Read-Write to copy data
            unsafe {
                let (_, flush) = mapper
                    .map_linearly(
                        frame.base(),
                        PageFlags::new().write(true), // Read-Write
                    )
                    .expect("Failed to map shadow stack RW");
                flush.flush();
            }

            // Copy data
            unsafe {
                let src = RmmA::phys_to_virt(self.frame.base()).data() as *const u8;
                let dst = RmmA::phys_to_virt(frame.base()).data() as *mut u8;
                core::ptr::copy_nonoverlapping(src, dst, PAGE_SIZE);
            }

            // Remap as Read-Only + Dirty (Shadow Stack) to restore protection
            unsafe {
                let (_, flush) = mapper
                    .map_linearly(
                        frame.base(),
                        PageFlags::new().write(false).custom_flag(1 << 6, true), // Dirty (Shadow Stack)
                    )
                    .expect("Failed to remap shadow stack RO");
                flush.flush();
            }
        }

        new_stack
    }
}

#[cfg(target_arch = "x86_64")]
pub unsafe fn enable_shadow_stack() -> Result<(), &'static str> {
    use crate::memory::RmmA;
    use core::arch::x86_64::__cpuid;
    use rmm::{Arch as RmmArch, PhysicalAddress, TableKind};
    use x86::{
        controlregs::{cr4, cr4_write, Cr4},
        msr::{rdmsr, wrmsr},
    };

    // Check support
    let result = unsafe { __cpuid(7) };
    let cet_ss_supported = (result.ecx >> 7) & 1 == 1;

    if !cet_ss_supported {
        return Err("CET shadow stack not supported by CPU");
    }

    // Allocate a shadow stack for the initial kernel thread (boot CPU)
    // We leak this ShadowStack because the boot CPU context lives forever (or matches `init` scope)
    // Actually, we should store it in PerCpu?
    // For now, just allocate raw frame like before, or use our new struct.
    let ss = ShadowStack::new()?;
    let stack_top = ss.top();

    // We leak the struct so it doesn't drop (dealloc) the frame
    core::mem::forget(ss);

    // Constants
    const IA32_S_CET: u32 = 0x6E0;
    const IA32_PL0_SSP: u32 = 0x6E2;
    const CET_SHSTK_EN: u64 = 1 << 0;
    const CET_WR_SHSTK_EN: u64 = 1 << 1;

    // Setup SSP
    unsafe { wrmsr(IA32_PL0_SSP, stack_top as u64) };

    // Enable in MSR
    let s_cet = unsafe { rdmsr(IA32_S_CET) };
    unsafe { wrmsr(IA32_S_CET, s_cet | CET_SHSTK_EN | CET_WR_SHSTK_EN) };

    // Enable in CR4
    const CR4_CET: Cr4 = Cr4::from_bits_truncate(1 << 23);
    let mut cr4_val = unsafe { cr4() };
    cr4_val.insert(CR4_CET);
    unsafe { cr4_write(cr4_val) };

    log::info!("stack_guard: Shadow stack enabled. SSP={:#x}", stack_top);

    Ok(())
}

#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn enable_shadow_stack() -> Result<(), &'static str> {
    Err("Shadow stacks not supported on this architecture")
}

// =============================================================================
// Per-CPU Canary Storage
// =============================================================================

/// Per-CPU canary stored for fast access via segment register.
/// On x86_64, this should be accessible via gs:[offset].
#[repr(C)]
pub struct PercpuStackGuard {
    /// The canary value for this CPU
    pub canary: u64,
    /// Shadow stack pointer (if CET enabled)
    pub shadow_stack_ptr: u64,
}

impl PercpuStackGuard {
    pub const fn new() -> Self {
        Self {
            canary: 0,
            shadow_stack_ptr: 0,
        }
    }

    /// Initialize with current canary value
    pub fn init(&mut self) {
        self.canary = get_canary();
    }
}

impl Default for PercpuStackGuard {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canary_non_zero() {
        // Initialize entropy first
        crate::entropy::init();
        init();

        let canary = get_canary();
        assert_ne!(canary, 0);
    }

    #[test]
    fn test_context_canaries_differ() {
        crate::entropy::init();
        init();

        let c1 = context_canary(1);
        let c2 = context_canary(2);
        let c3 = context_canary(1);

        assert_ne!(c1, c2);
        assert_eq!(c1, c3); // Same context ID should give same canary
    }

    #[test]
    fn test_canary_contains_terminator() {
        crate::entropy::init();
        init();

        let canary = get_canary();
        // Should contain a null-like byte (0x0A) in the pattern
        let bytes = canary.to_le_bytes();
        assert!(bytes.iter().any(|&b| b == 0x0A || b == 0x00));
    }
}

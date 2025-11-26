use crate::sync::CleanLockToken;
use core::arch::asm;

/// Resets the system.
pub unsafe fn kreset() -> ! {
    unsafe {
        println!("kreset");

        asm!("hvc   #0",
             in("x0") 0x8400_0009_usize,
             options(noreturn),
        )
    }
}

/// Performs an emergency reset of the system.
pub unsafe fn emergency_reset() -> ! {
    unsafe {
        asm!("hvc   #0",
             in("x0")  0x8400_0009_usize,
             options(noreturn),
        )
    }
}

/// Stops the system.
pub unsafe fn kstop(token: &mut CleanLockToken) -> ! {
    unsafe {
        println!("kstop");

        asm!("hvc   #0",
             in("x0")  0x8400_0008_usize,
             options(noreturn),
        )
    }
}

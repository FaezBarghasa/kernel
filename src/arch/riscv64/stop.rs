use crate::sync::CleanLockToken;
use sbi_rt::{system_reset, ResetType, ResetReason};

/// Resets the system.
pub unsafe fn kreset() -> ! {
    println!("kreset");
    system_reset(ResetType::ColdReset, ResetReason::NoReason);
    loop {
        core::hint::spin_loop();
    }
}

/// Performs an emergency reset of the system.
pub unsafe fn emergency_reset() -> ! {
    system_reset(ResetType::ColdReset, ResetReason::SystemFailure);
    loop {
        core::hint::spin_loop();
    }
}

/// Stops the system.
pub unsafe fn kstop(token: &mut CleanLockToken) -> ! {
    println!("kstop");
    system_reset(ResetType::Shutdown, ResetReason::NoReason);
    loop {
        core::hint::spin_loop();
    }
}

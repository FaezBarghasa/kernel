use crate::sync::CleanLockToken;

/// Resets the system.
pub unsafe fn kreset() -> ! {
    println!("kreset");
    unimplemented!()
}

/// Performs an emergency reset of the system.
pub unsafe fn emergency_reset() -> ! {
    unimplemented!()
}

/// Stops the system.
pub unsafe fn kstop(token: &mut CleanLockToken) -> ! {
    println!("kstop");
    unimplemented!()
}

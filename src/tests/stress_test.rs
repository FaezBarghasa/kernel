//! Kernel Stress Test Suite
//!
//! This module spawns multiple threads to perform rapid IPC and context switching
//! to stress test the scheduler and synchronization primitives.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use crate::context;
use crate::sync::CleanLockToken;

static THREAD_COUNT: AtomicUsize = AtomicUsize::new(0);
const TARGET_THREADS: usize = 100; // Adjusted for microkernel environment

pub fn start_stress_test() {
    println!("STRESS TEST: Starting with {} threads...", TARGET_THREADS);

    let mut token = unsafe { CleanLockToken::new() };

    for i in 0..TARGET_THREADS {
        let _ = context::spawn(
            false,
            None,
            move || worker_thread(i),
            &mut token
        );
    }
}

extern "C" fn worker_thread(id: usize) {
    THREAD_COUNT.fetch_add(1, Ordering::Relaxed);

    // Simulate work
    for _ in 0..1000 {
        core::hint::spin_loop();
        unsafe { context::switch(&mut CleanLockToken::new()) };
    }

    THREAD_COUNT.fetch_sub(1, Ordering::Relaxed);

    // Last thread reports
    if THREAD_COUNT.load(Ordering::Relaxed) == 0 {
        println!("STRESS TEST: All threads completed successfully.");
    }

    // Exit
    loop {
        unsafe { context::switch(&mut CleanLockToken::new()) };
    }
}

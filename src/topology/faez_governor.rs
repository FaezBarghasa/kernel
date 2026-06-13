#![forbid(unsafe_code)]

use crate::{
    cpu_set::LogicalCpuId,
    scheduler::scheduler,
};

/// Maximum allowed jitter (average lag) in nanoseconds before scaling up frequency.
pub const JITTER_TOLERANCE_NS: u64 = 1_000_000; // 1ms

/// Monitor and adjust CPU core frequency based on the EEVDF scheduler lag.
pub fn monitor_and_scale(cpu_id: LogicalCpuId) {
    let scheduler = scheduler();
    let ring = &scheduler.run_queue.non_rt_ring;
    let count = ring.len();

    if count == 0 {
        // If the EEVDF queue is empty, trigger low-power idle state if on microcontroller
        #[cfg(target_arch = "xtensa")]
        {
            // ESP32-S3 Deep Sleep Mode emulation:
            // Disable primary PLL, leave low-frequency RTC timer active
        }
        return;
    }

    let vruntime_avg = ring.average_vruntime();
    
    // We compute the lag of the current running context
    let current_vdeadline = scheduler.current_virtual_deadline.load(core::sync::atomic::Ordering::Relaxed);
    let lag = if vruntime_avg > current_vdeadline {
        vruntime_avg.saturating_sub(current_vdeadline)
    } else {
        0
    };

    if lag > JITTER_TOLERANCE_NS {
        // Scale up CPU frequency immediately!
        // Writing 1 (max performance) to Intel HWP or AMD CPPC
        #[cfg(target_arch = "x86_64")]
        {
            crate::arch::misc::write_hwp_request(0x000000000000ff01); // Max perf hint
            crate::arch::misc::write_cppc_request(0x000000000000ff01);
        }
    } else {
        // Scale down or set to balance
        #[cfg(target_arch = "x86_64")]
        {
            crate::arch::misc::write_hwp_request(0x0000000000008001); // Balanced hint
            crate::arch::misc::write_cppc_request(0x0000000000008001);
        }
    }
}

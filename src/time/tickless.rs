#![forbid(unsafe_code)]

//! # Preemptive Tickless Core Engine
//!
//! Eliminates periodic timer ticks by dynamically scheduling one-shot hardware interrupts
//! based on the EEVDF scheduler's earliest virtual deadline $V_{min}$.
//!
//! ## Mathematical Model
//! Given target counter $C_{target}$ derived from $V_{min}$ and current counter $C_{curr}$:
//! $$\Delta t = C_{target} - C_{curr}$$
//!
//! When preemption occurs prior to expiration at time $C_{event}$, the elapsed duration
//! $\delta = C_{event} - C_{start}$ is credited to thread runtime $T_i$, and remaining
//! quantum $S_{rem} = S_{quantum} - \delta$ is used to reschedule hardware timers in $\mathcal{O}(1)$ time.

use core::sync::atomic::{AtomicU64, Ordering};
use crate::time::set_next_timer_event;

/// State for a tickless CPU core timer.
pub struct TicklessCoreState {
    /// Monotonic timestamp counter when current slice started (in nanoseconds).
    pub slice_start_ns: AtomicU64,
    /// Target deadline timestamp counter (in nanoseconds).
    pub scheduled_deadline_ns: AtomicU64,
    /// Remaining time quantum in nanoseconds.
    pub remaining_quantum_ns: AtomicU64,
    /// Total interrupts handled without tick overhead.
    pub tickless_events_count: AtomicU64,
}

impl TicklessCoreState {
    /// Creates a new `TicklessCoreState` instance.
    pub const fn new() -> Self {
        Self {
            slice_start_ns: AtomicU64::new(0),
            scheduled_deadline_ns: AtomicU64::new(0),
            remaining_quantum_ns: AtomicU64::new(0),
            tickless_events_count: AtomicU64::new(0),
        }
    }

    /// One-Shot Hardware Programming:
    /// Calculates $\Delta t = \text{TargetCounter} - \text{CurrentCounter}$ from $V_{min}$
    /// and programs the hardware timer (LAPIC / GIC) in one-shot mode.
    pub fn program_one_shot_timer(&self, v_min_ns: u64, current_counter: u64) -> u64 {
        let delta_t = v_min_ns.saturating_sub(current_counter).max(1);
        let target_counter = current_counter.saturating_add(delta_t);

        self.slice_start_ns.store(current_counter, Ordering::Release);
        self.scheduled_deadline_ns.store(target_counter, Ordering::Release);

        set_next_timer_event(target_counter);
        self.tickless_events_count.fetch_add(1, Ordering::Relaxed);

        delta_t
    }

    /// Schedules a one-shot timer interrupt for the earliest virtual deadline.
    pub fn schedule_one_shot(&self, v_min_ns: u64, current_ns: u64, quantum_ns: u64) -> u64 {
        self.remaining_quantum_ns.store(quantum_ns, Ordering::Release);
        self.program_one_shot_timer(v_min_ns, current_ns)
    }

    /// Preemption Handling:
    /// If an asynchronous peripheral interrupt fires before the scheduled timer,
    /// recalculate elapsed execution time for the current thread, adjust its $T_i$ runtime,
    /// and reprogram the timer for the remaining duration seamlessly.
    pub fn handle_preemption_interrupt(&self, current_ns: u64) -> (u64, u64) {
        let start_ns = self.slice_start_ns.load(Ordering::Acquire);
        let elapsed_ns = current_ns.saturating_sub(start_ns);

        let initial_quantum = self.remaining_quantum_ns.load(Ordering::Acquire);
        let remaining_ns = initial_quantum.saturating_sub(elapsed_ns);

        self.remaining_quantum_ns.store(remaining_ns, Ordering::Release);

        if remaining_ns > 0 {
            let next_deadline = current_ns.saturating_add(remaining_ns);
            self.scheduled_deadline_ns.store(next_deadline, Ordering::Release);
            set_next_timer_event(next_deadline);
        }

        (elapsed_ns, remaining_ns)
    }
}

/// Global per-cpu tickless state instances.
pub static TICKLESS_ENGINE: TicklessCoreState = TicklessCoreState::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tickless_preemption_reprogram() {
        let state = TicklessCoreState::new();
        state.schedule_one_shot(10_000, 1_000, 5_000);
        // Preemption fires at 3_000 ns (elapsed = 2_000 ns, remaining = 3_000 ns)
        let (elapsed, remaining) = state.handle_preemption_interrupt(3_000);
        assert_eq!(elapsed, 2_000);
        assert_eq!(remaining, 3_000);
    }
}

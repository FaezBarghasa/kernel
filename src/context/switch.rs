//! # Context Switching

use crate::{
    context::{contexts, Context},
    percpu::PercpuBlock,
    scheduler,
    sync::CleanLockToken,
    time,
};
use alloc::sync::Arc;
use core::ops::Bound;

pub enum SwitchResult {
    Switched,
    AllContextsIdle,
}

#[derive(Debug, Default)]
pub struct ContextSwitchPercpu {}

// Removed the `tick` function as it's no longer needed in a tickless system.

pub unsafe fn switch(token: &mut CleanLockToken) -> SwitchResult {
    let cpu_id = crate::cpu_id();
    let current_context_id = PercpuBlock::current().context_id.get();

    let next_context_ref_opt = scheduler::schedule_next(token);

    if let Some(next_context_ref) = next_context_ref_opt {
        let next_context_id = next_context_ref.read(token.token()).id();

        if next_context_id == current_context_id {
            // If the same context is scheduled, just ensure the timer is set for its next event
            let next_wake_time = next_context_ref.read(token.token()).wake;
            if let Some(wake_time) = next_wake_time {
                time::set_next_timer_event(wake_time as u64);
            }
            return SwitchResult::Switched;
        }

        let prev_context_lock = if current_context_id != 0 {
            let contexts_guard = contexts().read();
            Some(Arc::clone(
                contexts_guard
                    .get(&current_context_id)
                    .expect("current context disappeared"),
            ))
        } else {
            None
        };

        let mut next_guard = next_context_ref.write(token.token());
        next_guard.cpu_id = Some(cpu_id);

        // Set the timer for the next context's wake time
        let next_wake_time = next_guard.wake;
        if let Some(wake_time) = next_wake_time {
            time::set_next_timer_event(wake_time as u64);
        }

        if let Some(prev_lock) = prev_context_lock {
            let mut prev_guard = prev_lock.write(token.token());

            PercpuBlock::current().context_id.set(next_context_id);

            crate::arch::switch_to(&mut *prev_guard, &mut *next_guard);
        } else {
            // This case handles the initial switch from an idle state or kmain
            // where there isn't a "previous" user context to save.
            PercpuBlock::current().context_id.set(next_context_id);
            crate::arch::switch_to_first(&mut *next_guard);
        }

        SwitchResult::Switched
    } else {
        // All contexts are idle. Program the timer for the earliest wake time among all contexts.
        let mut earliest_wake: Option<u128> = None;
        let contexts_guard = contexts().read();
        for (_id, context_lock) in contexts_guard.iter() {
            let context = context_lock.read(token.token());
            if let Some(wake_time) = context.wake {
                if earliest_wake.is_none() || wake_time < earliest_wake.unwrap() {
                    earliest_wake = Some(wake_time);
                }
            }
        }

        if let Some(wake_time) = earliest_wake {
            time::set_next_timer_event(wake_time as u64);
        } else {
            // If no contexts are set to wake up, set a default idle timeout
            time::set_next_timer_event(time::monotonic() as u64 + 1_000_000_000); // 1 second
        }

        SwitchResult::AllContextsIdle
    }
}

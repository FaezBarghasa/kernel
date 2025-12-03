//! # Context Switching

use crate::{
    context::{contexts, Context},
    percpu::PercpuBlock,
    sync::CleanLockToken,
};
use alloc::sync::Arc;
use core::ops::Bound;

pub enum SwitchResult {
    Switched,
    AllContextsIdle,
}

#[derive(Debug, Default)]
pub struct ContextSwitchPercpu {}

pub fn tick(_token: &mut CleanLockToken) {
    // TODO: Time slicing
}

pub unsafe fn switch(_token: &mut CleanLockToken) -> SwitchResult {
    let cpu_id = crate::cpu_id();
    let switch_result = {
        let contexts = contexts().read();
        let current_context_id = PercpuBlock::current().context_id.get();

        // Find next runnable context
        let mut next_context_id = None;

        // Helper to check if context is runnable on this CPU
        let is_runnable = |context: &Context| -> bool {
            context.status.is_runnable() && context.sched_affinity.contains(cpu_id)
        };

        // Iterate from current_id + 1
        // If current_context_id is 0, start from 1 (first valid ID)
        let start = if current_context_id == 0 {
            1
        } else {
            current_context_id + 1
        };

        for (&id, context_lock) in contexts.range((Bound::Included(start), Bound::Unbounded)) {
            if let Some(context) = context_lock.try_read() {
                if is_runnable(&context) {
                    next_context_id = Some(id);
                    break;
                }
            }
        }

        if next_context_id.is_none() {
            // Wrap around
            for (&id, context_lock) in contexts.range((Bound::Unbounded, Bound::Excluded(start))) {
                if let Some(context) = context_lock.try_read() {
                    if is_runnable(&context) {
                        next_context_id = Some(id);
                        break;
                    }
                }
            }
        }

        // If still none, and current is runnable, keep current?
        // But we want to switch if possible.
        // If only current is runnable, we stay.

        if next_context_id.is_none() {
            if current_context_id != 0 {
                if let Some(context_lock) = contexts.get(&current_context_id) {
                    if let Some(context) = context_lock.try_read() {
                        if is_runnable(&context) {
                            next_context_id = Some(current_context_id);
                        }
                    }
                }
            }
        }

        if let Some(next_id) = next_context_id {
            if next_id == current_context_id {
                SwitchResult::Switched
            } else {
                // Perform switch
                let next_context_lock =
                    Arc::clone(contexts.get(&next_id).expect("context disappeared"));
                let prev_context_lock = if current_context_id != 0 {
                    Some(Arc::clone(
                        contexts
                            .get(&current_context_id)
                            .expect("current context disappeared"),
                    ))
                } else {
                    None
                };

                drop(contexts); // Release read lock on list

                let mut next_guard = next_context_lock.write();
                next_guard.cpu_id = Some(cpu_id);

                if let Some(prev_lock) = prev_context_lock {
                    let mut prev_guard = prev_lock.write();

                    PercpuBlock::current().context_id.set(next_id);

                    unsafe {
                        crate::arch::switch_to(&mut *prev_guard, &mut *next_guard);
                    }
                } else {
                    // No previous context (e.g. initial switch from kmain/idle?)
                    // If we are here, we must have a way to switch.
                    // But switch_to requires prev.
                    // If current_context_id is 0, we are likely in early boot or idle.
                    // We should have a context for idle?
                    // For now, panic if no prev context, as switch_to requires it.
                    panic!("Switching from context 0 (no context)");
                }

                SwitchResult::Switched
            }
        } else {
            SwitchResult::AllContextsIdle
        }
    };

    switch_result
}

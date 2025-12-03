//! # Context Switching

use crate::{
    context::{contexts, Context},
    percpu::PercpuBlock,
    scheduler,
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

pub unsafe fn switch(token: &mut CleanLockToken) -> SwitchResult {
    let cpu_id = crate::cpu_id();
    let current_context_id = PercpuBlock::current().context_id.get();

    let next_context_ref_opt = scheduler::schedule_next(token);

    if let Some(next_context_ref) = next_context_ref_opt {
        let next_context_id = next_context_ref.read(token.token()).id();

        if next_context_id == current_context_id {
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
        SwitchResult::AllContextsIdle
    }
}

//! # Context Switching

use core::sync::atomic::Ordering;
use crate::context::{contexts, Context, Status};
use crate::cpu_set::LogicalCpuId;
use crate::percpu::PercpuBlock;
use crate::sync::CleanLockToken;

pub enum SwitchResult {
    Switched,
    AllContextsIdle,
}

pub unsafe fn switch(token: &mut CleanLockToken) -> SwitchResult {
    let cpu_id = crate::cpu_id();
    let cpu_id_usize = cpu_id.0 as usize;

    let mut contexts = contexts();
    let current_context_id = PercpuBlock::current().context_id;

    if let Some(current_id) = current_context_id {
        if let Some(current) = contexts.get_mut(current_id) {
            match current.status {
                Status::Runnable => {}
                _ => {}
            }
            current.cpu_id = Some(cpu_id_usize);
        }
    }

    let start_index = current_context_id.map_or(0, |id| id + 1);
    let len = contexts.len();
    let mut next_context_id = None;

    for i in 0..len {
        let index = (start_index + i) % len;

        let (id, context) = match contexts.get_index(index) {
            Some(entry) => (*entry.0, entry.1),
            None => continue,
        };

        if core::intrinsics::unlikely(context.status == Status::Runnable) {
            if core::intrinsics::likely(context.affinity.contains(cpu_id)) {
                if context.cpu_id == None || context.cpu_id == Some(cpu_id_usize) {
                    next_context_id = Some(id);
                    break;
                }
            }
        }
    }

    if let Some(next_id) = next_context_id {
        if next_id == current_context_id.unwrap_or(usize::MAX) {
            return SwitchResult::Switched;
        }

        PercpuBlock::current().context_id = Some(next_id);
        let next_context = contexts.get_mut(next_id).expect("context disappeared in switch");

        let prev_context_ptr = if let Some(curr_id) = current_context_id {
            contexts.get_mut(curr_id).map(|c| c as *mut Context).unwrap_or(core::ptr::null_mut())
        } else {
            core::ptr::null_mut()
        };

        let next_context_ptr = next_context as *mut Context;
        drop(contexts);

        unsafe {
            crate::arch::switch_to(prev_context_ptr, next_context_ptr);
        }

        SwitchResult::Switched
    } else {
        SwitchResult::AllContextsIdle
    }
}
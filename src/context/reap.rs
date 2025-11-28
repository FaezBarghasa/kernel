use crate::{context::memory::Grant, sync::CleanLockToken};
use alloc::collections::VecDeque;
use spin::Mutex;

pub static REAP_QUEUE: Mutex<VecDeque<Grant>> = Mutex::new(VecDeque::new());

pub fn reap_grants() {
    let mut token = unsafe { CleanLockToken::new() };
    let mut grants = REAP_QUEUE.lock();
    while let Some(mut grant) = grants.pop_front() {
        let res = grant.unmap(
            &mut unsafe { &mut *crate::memory::KernelMapper::get().get_mut() },
            &mut crate::context::memory::NopFlusher,
        );
        let _ = res.unmap(&mut token);
    }
}

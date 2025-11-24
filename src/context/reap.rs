use alloc::collections::VecDeque;
use spin::Mutex;
use crate::context::memory::Grant;
use crate::sync::CleanLockToken;

pub static REAP_QUEUE: Mutex<VecDeque<Grant>> = Mutex::new(VecDeque::new());

pub fn reap_grants() {
    let mut token = unsafe { CleanLockToken::new() };
    let mut grants = REAP_QUEUE.lock();
    while let Some(mut grant) = grants.pop_front() {
        let res = grant.unmap(&mut unsafe { &mut *crate::paging::KernelMapper::get().get_mut() }, &mut crate::context::memory::NopFlusher);
        let _ = res.unmap(&mut token);
    }
}

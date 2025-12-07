use alloc::{
    collections::BTreeMap,
    sync::{Arc, Weak},
    vec::Vec,
};

use crate::{
    context::{self, ContextLock},
    sync::{CleanLockToken, OrderedMutex, L1},
};

#[derive(Debug)]
pub struct WaitCondition {
    // Store contexts in a BTreeMap, ordered by effective priority (lower value = higher priority)
    pub(crate) contexts: OrderedMutex<L1, BTreeMap<u8, Vec<Weak<ContextLock>>>>,
}

impl WaitCondition {
    pub const fn new() -> WaitCondition {
        WaitCondition {
            contexts: OrderedMutex::new(BTreeMap::new()),
        }
    }

    // Notify all waiters
    pub fn notify(&self, token: &mut CleanLockToken) -> usize {
        let mut contexts_map = self.contexts.lock(token.token());
        let (contexts_map, mut token) = contexts_map.token_split();
        let mut notified_count = 0;

        // Iterate through priorities from highest (lowest u8 value) to lowest
        let mut priorities_to_remove = Vec::new();
        for (priority, contexts) in contexts_map.iter_mut() {
            for context_weak in contexts.drain(..) {
                if let Some(context_ref) = context_weak.upgrade() {
                    context_ref.write(token.token()).unblock();
                    notified_count += 1;
                }
            }
            priorities_to_remove.push(*priority);
        }

        // Remove empty priority vectors
        for priority in priorities_to_remove {
            contexts_map.remove(&priority);
        }

        notified_count
    }

    // Notify all waiters and boost their priority
    pub fn notify_boosted(&self, token: &mut CleanLockToken) -> usize {
        let mut contexts_map = self.contexts.lock(token.token());
        let (contexts_map, mut token) = contexts_map.token_split();
        let mut notified_count = 0;

        let mut priorities_to_remove = Vec::new();
        for (priority, contexts) in contexts_map.iter_mut() {
            for context_weak in contexts.drain(..) {
                if let Some(context_ref) = context_weak.upgrade() {
                    let mut context = context_ref.write(token.token());
                    context.unblock();
                    // Boost priority for IPC completion (approx 10k cycles)
                    context.priority.boost_for_ipc(10000);
                    notified_count += 1;
                }
            }
            priorities_to_remove.push(*priority);
        }

        // Remove empty priority vectors
        for priority in priorities_to_remove {
            contexts_map.remove(&priority);
        }

        notified_count
    }

    // Notify as though a signal woke the waiters
    pub unsafe fn notify_signal(&self, token: &mut CleanLockToken) -> usize {
        let mut contexts_map = self.contexts.lock(token.token());
        let (contexts_map, mut token) = contexts_map.token_split();
        let mut notified_count = 0;

        let mut priorities_to_remove = Vec::new();
        for (priority, contexts) in contexts_map.iter_mut() {
            for context_weak in contexts.drain(..) {
                if let Some(context_ref) = context_weak.upgrade() {
                    context_ref.write(token.token()).unblock();
                    notified_count += 1;
                }
            }
            priorities_to_remove.push(*priority);
        }

        // Remove empty priority vectors
        for priority in priorities_to_remove {
            contexts_map.remove(&priority);
        }

        notified_count
    }

    // Wait until notified. Unlocks guard when blocking is ready. Returns false if resumed by a signal or the notify_signal function
    pub fn wait<T>(&self, guard: T, reason: &'static str, token: &mut CleanLockToken) -> bool {
        let current_context_ref = context::current();
        {
            {
                let mut context = current_context_ref.write(token.token());
                if let Some((control, pctl, _)) = context.sigcontrol()
                    && control.currently_pending_unblocked(pctl) != 0
                {
                    return false;
                }
                context.block(reason);
            }

            // Get the effective priority of the current context
            let effective_priority = current_context_ref.read(token.token()).priority.effective_priority();

            self.contexts
                .lock(token.token())
                .entry(effective_priority)
                .or_default()
                .push(Arc::downgrade(&current_context_ref));

            drop(guard);
        }

        unsafe { context::switch(token) };

        let mut waited = true;

        // Remove the current context from the wait queue if it was not woken by notify
        let effective_priority = current_context_ref.read(token.token()).priority.effective_priority(); // Re-read priority, it might have changed
        let mut contexts_map = self.contexts.lock(token.token());

        if let Some(contexts_at_priority) = contexts_map.get_mut(&effective_priority) {
            contexts_at_priority.retain(|w| {
                if let Some(s) = w.upgrade() {
                    if Arc::ptr_eq(&s, &current_context_ref) {
                        waited = false;
                        false
                    } else {
                        true
                    }
                } else {
                    false
                }
            });
            if contexts_at_priority.is_empty() {
                contexts_map.remove(&effective_priority);
            }
        }

        waited
    }
}

impl Drop for WaitCondition {
    fn drop(&mut self) {
        //TODO: drop violates lock tokens
        unsafe {
            let mut token = CleanLockToken::new();
            self.notify_signal(&mut token);
        };
    }
}

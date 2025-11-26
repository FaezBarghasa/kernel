use alloc::collections::VecDeque;
use spin::Once;

use crate::{
    event,
    scheme::SchemeId,
    sync::{CleanLockToken, LockToken, OrderedMutex, OrderedMutexGuard, L0, L1},
    syscall::{
        data::TimeSpec,
        flag::{CLOCK_MONOTONIC, CLOCK_REALTIME, EVENT_READ},
    },
    time,
};

#[derive(Debug)]
struct Timeout {
    pub scheme_id: SchemeId,
    pub event_id: usize,
    pub clock: usize,
    pub time: u128,
}

type Registry = VecDeque<Timeout>;

static REGISTRY: Once<OrderedMutex<L1, Registry>> = Once::new();

/// Initialize registry, called if needed
fn init_registry() -> OrderedMutex<L1, Registry> {
    OrderedMutex::new(Registry::new())
}

/// Get the global timeouts list
fn registry(token: LockToken<'_, L0>) -> OrderedMutexGuard<'_, L1, Registry> {
    REGISTRY.call_once(init_registry).lock(token)
}

pub fn register(
    scheme_id: SchemeId,
    event_id: usize,
    clock: usize,
    time: TimeSpec,
    token: &mut CleanLockToken,
) {
    let mut registry = registry(token.token());
    registry.push_back(Timeout {
        scheme_id,
        event_id,
        clock,
        time: (time.tv_sec as u128 * time::NANOS_PER_SEC) + (time.tv_nsec as u128),
    });
}

pub fn trigger(token: &mut CleanLockToken) {
    let mono = time::monotonic();
    let real = time::realtime();

    let mut i = 0;
    loop {
        // Acquire registry lock for this iteration and possibly remove a timeout
        let timeout_opt = {
            let mut registry = registry(token.token());
            if i < registry.len() {
                let trigger = match registry[i].clock {
                    CLOCK_MONOTONIC => {
                        let time = registry[i].time;
                        mono >= time
                    }
                    CLOCK_REALTIME => {
                        let time = registry[i].time;
                        real >= time
                    }
                    clock => {
                        println!("timeout::trigger: unknown clock {}", clock);
                        true
                    }
                };
                if trigger {
                    Some(registry.remove(i).unwrap())
                } else {
                    i += 1;
                    None
                }
            } else {
                None
            }
        };
        match timeout_opt {
            Some(timeout) => {
                // Registry lock is dropped, safe to use token again
                event::trigger(timeout.scheme_id, timeout.event_id, EVENT_READ, token);
            }
            None => break,
        }
    }
}

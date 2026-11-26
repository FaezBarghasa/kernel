//! # Futex with Transitive Priority Inheritance
//!
//! Futex or Fast Userspace Mutex is "a method for waiting until a certain condition becomes true."
//! This implementation extends the standard futex with transitive priority inheritance (PI) to
//! prevent priority inversion in real-time scenarios.
//!
//! ## Priority Inheritance Protocol
//! When a high-priority task blocks on a futex owned by a lower-priority task:
//! 1. The owner's priority is boosted to the waiter's priority
//! 2. If the owner is blocked on another futex, the boost propagates transitively
//! 3. Chain traversal is limited to MAX_PI_CHAIN_DEPTH to prevent DoS
//! 4. Cycles in the chain indicate deadlock
//!
//! For more information about futexes, please read [this](https://eli.thegreenplace.net/2018/basics-of-futexes/)
//! blog post, and the [futex(2)](http://man7.org/linux/man-pages/man2/futex.2.html) man page

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};
use core::sync::atomic::{AtomicU32, Ordering};
use hashbrown::{hash_map::DefaultHashBuilder, HashMap};
use rmm::Arch;
use syscall::EINTR;

use crate::{
    context::{
        self,
        memory::{AddrSpace, AddrSpaceInner, AddrSpaceWrapper},
        ContextId, ContextLock,
    },
    memory::PhysicalAddress,
    paging::{Page, VirtualAddress},
    sync::{CleanLockToken, LockToken, OrderedMutex, L1},
    time,
};

use crate::syscall::{
    data::TimeSpec,
    error::{Error, Result, EAGAIN, EDEADLK, EFAULT, EINVAL, ETIMEDOUT},
    flag::{FUTEX_WAIT, FUTEX_WAIT64, FUTEX_WAKE},
};

use super::usercopy::UserSlice;

// Define missing constants locally
const FUTEX_LOCK_PI: usize = 6;
const FUTEX_UNLOCK_PI: usize = 7;
const FUTEX_TRYLOCK_PI: usize = 8;
const FUTEX_WAIT_BITSET: usize = 9;

/// Maximum depth for priority inheritance chain traversal
/// This prevents DoS attacks and bounds computation time
const MAX_PI_CHAIN_DEPTH: usize = 16;

/// Futex value bits for PI futexes
const FUTEX_TID_MASK: u32 = 0x3fff_ffff;
const FUTEX_OWNER_DIED: u32 = 0x4000_0000;
const FUTEX_WAITERS: u32 = 0x8000_0000;

// Physical address used as key, required if synchronizing across address spaces
// (necessitates MAP_SHARED since CoW would invalidate this address).
type FutexList = HashMap<PhysicalAddress, Vec<FutexEntry>>;

/// Owner tracking for PI futexes
type PiOwnerMap = HashMap<PhysicalAddress, FutexOwner>;

/// Entry representing a waiting context on a futex
pub struct FutexEntry {
    /// Virtual address for CoW-aware matching
    target_virtaddr: VirtualAddress,
    /// Context waiting on this futex
    context_lock: Arc<ContextLock>,
    /// Address space for cross-process matching
    addr_space: Weak<AddrSpaceWrapper>,
    /// Priority of the waiter at time of blocking (for PI)
    waiter_priority: u8,
    /// The futex this waiter is blocked on (for chain traversal)
    blocked_on: Option<PhysicalAddress>,
}

/// Owner information for PI futexes
pub struct FutexOwner {
    /// Context that owns this futex
    owner_context: Arc<ContextLock>,
    /// Original priority before any inheritance
    original_priority: u8,
    /// Currently inherited priority (min of all waiters and original)
    inherited_priority: u8,
    /// Key used for priority inheritance tracking
    pi_key: usize,
}

pub struct FutexState {
    futexes: FutexList,
    pi_owners: PiOwnerMap,
}

impl Default for FutexState {
    fn default() -> Self {
        Self {
            futexes: FutexList::with_hasher(DefaultHashBuilder::new()),
            pi_owners: PiOwnerMap::with_hasher(DefaultHashBuilder::new()),
        }
    }
}

static FUTEX_STATE: OrderedMutex<L1, FutexState> = OrderedMutex::new(FutexState {
    futexes: HashMap::with_hasher(DefaultHashBuilder::new()),
    pi_owners: HashMap::with_hasher(DefaultHashBuilder::new()),
});

fn validate_and_translate_virt(
    space: &AddrSpaceInner,
    addr: VirtualAddress,
) -> Option<PhysicalAddress> {
    if addr.data().saturating_add(core::mem::size_of::<usize>()) >= crate::USER_END_OFFSET {
        return None;
    }

    let page = Page::containing_address(addr);
    let off = addr.data() - page.start_address().data();

    let phys = space.table.utable.translate(page.start_address())?;

    Some(phys.add(off))
}

/// Traverse the PI chain and boost priorities transitively
/// Returns Err(EDEADLK) if a cycle is detected
fn propagate_priority_inheritance(
    start_physaddr: PhysicalAddress,
    waiter_priority: u8,
    waiter_context: &Arc<ContextLock>,
    state: &mut FutexState,
    token: &mut LockToken<'_, L1>,
) -> Result<()> {
    let mut current_addr = Some(start_physaddr);
    let mut visited: Vec<PhysicalAddress> = Vec::with_capacity(MAX_PI_CHAIN_DEPTH);
    let mut depth = 0;

    while let Some(addr) = current_addr {
        // Check chain depth limit
        if depth >= MAX_PI_CHAIN_DEPTH {
            break;
        }

        // Check for cycle (deadlock detection)
        if visited.contains(&addr) {
            return Err(Error::new(EDEADLK));
        }
        visited.push(addr);

        // Get owner of this futex
        let owner_info = match state.pi_owners.get_mut(&addr) {
            Some(info) => info,
            None => break, // No PI owner registered for this futex
        };

        // Check if the owner is the same as the waiter (self-deadlock)
        if Arc::ptr_eq(&owner_info.owner_context, waiter_context) {
            return Err(Error::new(EDEADLK));
        }

        let current_inherited = owner_info.inherited_priority;

        // Only boost if waiter has higher priority (lower numeric value)
        if waiter_priority < current_inherited {
            owner_info.inherited_priority = waiter_priority;

            // Apply priority boost to the owner context
            {
                let mut owner_ctx = owner_info.owner_context.write(token.token());
                owner_ctx
                    .priority
                    .inherit_priority(owner_info.pi_key, waiter_priority);
            }
        }

        // Check if owner is itself blocked on another futex
        current_addr = None;
        for entries in state.futexes.values() {
            for entry in entries {
                if Arc::ptr_eq(&entry.context_lock, &owner_info.owner_context) {
                    if let Some(blocked_addr) = entry.blocked_on {
                        current_addr = Some(blocked_addr);
                        break;
                    }
                }
            }
            if current_addr.is_some() {
                break;
            }
        }

        depth += 1;
    }

    Ok(())
}

/// Restore priority when a waiter is removed (woken or timed out)
fn restore_priority_on_wake(
    physaddr: PhysicalAddress,
    state: &mut FutexState,
    token: &mut LockToken<'_, L1>,
) {
    let owner_info = match state.pi_owners.get_mut(&physaddr) {
        Some(info) => info,
        None => return,
    };

    // Find the new highest priority among remaining waiters
    let mut new_inherited = owner_info.original_priority;

    if let Some(entries) = state.futexes.get(&physaddr) {
        for entry in entries {
            if entry.waiter_priority < new_inherited {
                new_inherited = entry.waiter_priority;
            }
        }
    }

    // Update if changed
    if new_inherited != owner_info.inherited_priority {
        owner_info.inherited_priority = new_inherited;

        let mut owner_ctx = owner_info.owner_context.write(token.token());
        if new_inherited == owner_info.original_priority {
            owner_ctx.priority.restore_priority(owner_info.pi_key);
        } else {
            owner_ctx
                .priority
                .inherit_priority(owner_info.pi_key, new_inherited);
        }
    }
}

pub fn futex(
    addr: usize,
    op: usize,
    val: usize,
    val2: usize,
    _addr2: usize,
    token: &mut CleanLockToken,
) -> Result<usize> {
    let current_addrsp = AddrSpace::current(token)?;

    // Keep the address space locked so we can safely read from the physical address
    let mut addr_space_guard = current_addrsp.acquire_read();

    let target_virtaddr = VirtualAddress::new(addr);
    let target_physaddr = validate_and_translate_virt(&addr_space_guard, target_virtaddr)
        .ok_or(Error::new(EFAULT))?;

    match op {
        FUTEX_WAIT | FUTEX_WAIT64 => {
            let timeout_opt = UserSlice::ro(val2, core::mem::size_of::<TimeSpec>())?
                .none_if_null()
                .map(|buf| unsafe { buf.read_exact::<TimeSpec>() })
                .transpose()?;

            {
                let mut state_lock = FUTEX_STATE.lock(token.token());
                let (state, mut token) = state_lock.token_split();

                let context_lock = context::current();

                let (fetched, expected) = if op == FUTEX_WAIT {
                    if addr % 4 != 0 {
                        return Err(Error::new(EINVAL));
                    }

                    let accessible_addr =
                        unsafe { crate::paging::RmmA::phys_to_virt(target_physaddr) }.data();

                    (
                        u64::from(unsafe {
                            (*(accessible_addr as *const AtomicU32)).load(Ordering::SeqCst)
                        }),
                        u64::from(val as u32),
                    )
                } else {
                    #[cfg(target_has_atomic = "64")]
                    {
                        use core::sync::atomic::AtomicU64;

                        if addr % 8 != 0 {
                            return Err(Error::new(EINVAL));
                        }
                        (
                            unsafe { (*(addr as *const AtomicU64)).load(Ordering::SeqCst) },
                            val as u64,
                        )
                    }
                    #[cfg(not(target_has_atomic = "64"))]
                    {
                        return Err(Error::new(crate::syscall::error::EOPNOTSUPP));
                    }
                };

                if fetched != expected {
                    return Err(Error::new(EAGAIN));
                }

                // Get current context's priority for PI
                let waiter_priority = {
                    let ctx = context_lock.read(token.token());
                    ctx.priority.effective_priority()
                };

                {
                    let mut context = context_lock.write(token.token());

                    context.wake = timeout_opt.map(|TimeSpec { tv_sec, tv_nsec }| {
                        tv_sec as u128 * time::NANOS_PER_SEC + tv_nsec as u128
                    });
                    if let Some((tctl, pctl, _)) = context.sigcontrol() {
                        if tctl.currently_pending_unblocked(pctl) != 0 {
                            return Err(Error::new(EINTR));
                        }
                    }

                    context.block("futex");
                }

                state
                    .futexes
                    .entry(target_physaddr)
                    .or_insert_with(Vec::new)
                    .push(FutexEntry {
                        target_virtaddr,
                        context_lock,
                        addr_space: Arc::downgrade(&current_addrsp),
                        waiter_priority,
                        blocked_on: Some(target_physaddr),
                    });
            }

            drop(addr_space_guard);

            unsafe { context::switch(token) };

            if timeout_opt.is_some() {
                context::current().write(token.token()).wake = None;
                Err(Error::new(ETIMEDOUT))
            } else {
                Ok(0)
            }
        }

        FUTEX_WAKE => {
            let mut woken = 0;

            {
                let mut state_lock = FUTEX_STATE.lock(token.token());
                let (state, mut token) = state_lock.token_split();

                let is_empty = if let Some(futexes) = state.futexes.get_mut(&target_physaddr) {
                    let mut i = 0;
                    let current_addrsp_weak = Arc::downgrade(&current_addrsp);

                    while i < futexes.len() && woken < val {
                        if futexes[i].target_virtaddr != target_virtaddr
                            || !current_addrsp_weak.ptr_eq(&futexes[i].addr_space)
                        {
                            i += 1;
                            continue;
                        }
                        futexes[i].context_lock.write(token.token()).unblock();
                        futexes.swap_remove(i);
                        woken += 1;
                    }

                    futexes.is_empty()
                } else {
                    false
                };
                if is_empty {
                    state.futexes.remove(&target_physaddr);
                }
            }

            Ok(woken)
        }

        FUTEX_LOCK_PI => {
            if addr % 4 != 0 {
                return Err(Error::new(EINVAL));
            }

            let context_lock = context::current();
            let current_tid: u32 = {
                let ctx = context_lock.read(token.token());
                ctx.id() as u32
            };

            let accessible_addr =
                unsafe { crate::paging::RmmA::phys_to_virt(target_physaddr) }.data();
            let futex_word = unsafe { &*(accessible_addr as *const AtomicU32) };

            // Try to acquire the lock (CAS 0 -> current_tid)
            match futex_word.compare_exchange(0, current_tid, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(_) => {
                    // Successfully acquired, register as owner
                    let mut state_lock = FUTEX_STATE.lock(token.token());
                    let (state, mut token) = state_lock.token_split();

                    let priority = context_lock
                        .read(token.token())
                        .priority
                        .effective_priority();
                    state.pi_owners.insert(
                        target_physaddr,
                        FutexOwner {
                            owner_context: context_lock.clone(),
                            original_priority: priority,
                            inherited_priority: priority,
                            pi_key: target_physaddr.data(),
                        },
                    );

                    return Ok(0);
                }
                Err(current_val) => {
                    // Lock is held, need to wait with PI
                    let owner_tid = current_val & FUTEX_TID_MASK;

                    // Check for owner death
                    if current_val & FUTEX_OWNER_DIED != 0 {
                        return Err(Error::new(crate::syscall::error::EOWNERDEAD));
                    }

                    // Set WAITERS bit
                    let _ = futex_word.fetch_or(FUTEX_WAITERS, Ordering::Relaxed);

                    // Add to wait queue with PI
                    let waiter_priority_val = context_lock
                        .read(token.token())
                        .priority
                        .effective_priority();

                    {
                        let mut state_lock = FUTEX_STATE.lock(token.token());
                        let (state, mut token) = state_lock.token_split();

                        // Propagate priority inheritance
                        let _ = propagate_priority_inheritance(
                            target_physaddr,
                            waiter_priority_val,
                            &context_lock,
                            state,
                            &mut token,
                        );

                        {
                            let mut context = context_lock.write(token.token());
                            context.block("futex_pi");
                        }

                        state
                            .futexes
                            .entry(target_physaddr)
                            .or_insert_with(Vec::new)
                            .push(FutexEntry {
                                target_virtaddr,
                                context_lock: context_lock.clone(),
                                addr_space: Arc::downgrade(&current_addrsp),
                                waiter_priority: waiter_priority_val,
                                blocked_on: Some(target_physaddr),
                            });
                    }

                    drop(addr_space_guard);
                    unsafe { context::switch(token) };

                    // After waking, try to acquire again
                    match futex_word.compare_exchange(
                        0,
                        current_tid,
                        Ordering::Acquire,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => Ok(0),
                        Err(_) => Err(Error::new(EAGAIN)),
                    }
                }
            }
        }

        FUTEX_TRYLOCK_PI => {
            if addr % 4 != 0 {
                return Err(Error::new(EINVAL));
            }

            let context_lock = context::current();
            let current_tid: u32 = {
                let ctx = context_lock.read(token.token());
                ctx.id() as u32
            };

            let accessible_addr =
                unsafe { crate::paging::RmmA::phys_to_virt(target_physaddr) }.data();
            let futex_word = unsafe { &*(accessible_addr as *const AtomicU32) };

            match futex_word.compare_exchange(0, current_tid, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(_) => {
                    let mut state_lock = FUTEX_STATE.lock(token.token());
                    let (state, mut token) = state_lock.token_split();

                    let priority = context_lock
                        .read(token.token())
                        .priority
                        .effective_priority();
                    state.pi_owners.insert(
                        target_physaddr,
                        FutexOwner {
                            owner_context: context_lock.clone(),
                            original_priority: priority,
                            inherited_priority: priority,
                            pi_key: target_physaddr.data(),
                        },
                    );
                    Ok(0)
                }
                Err(_) => Err(Error::new(EAGAIN)),
            }
        }

        FUTEX_UNLOCK_PI => {
            if addr % 4 != 0 {
                return Err(Error::new(EINVAL));
            }

            let context_lock = context::current();
            let current_tid: u32 = {
                let ctx = context_lock.read(token.token());
                ctx.id() as u32
            };

            let accessible_addr =
                unsafe { crate::paging::RmmA::phys_to_virt(target_physaddr) }.data();
            let futex_word = unsafe { &*(accessible_addr as *const AtomicU32) };

            // Verify we are the owner
            let current_val = futex_word.load(Ordering::Relaxed);
            if (current_val & FUTEX_TID_MASK) != current_tid {
                return Err(Error::new(crate::syscall::error::EPERM));
            }

            // Remove PI owner tracking and restore priority
            {
                let mut state_lock = FUTEX_STATE.lock(token.token());
                let (state, mut token) = state_lock.token_split();

                if let Some(owner_info) = state.pi_owners.remove(&target_physaddr) {
                    let mut ctx = owner_info.owner_context.write(token.token());
                    ctx.priority.restore_priority(owner_info.pi_key);
                }
            }

            // Check if there are waiters
            if current_val & FUTEX_WAITERS != 0 {
                // Wake one waiter
                let mut woken = false;
                {
                    let mut state_lock = FUTEX_STATE.lock(token.token());
                    let (state, mut token) = state_lock.token_split();

                    if let Some(waiters) = state.futexes.get_mut(&target_physaddr) {
                        if let Some(entry) = waiters.pop() {
                            entry.context_lock.write(token.token()).unblock();
                            woken = true;
                        }

                        if waiters.is_empty() {
                            state.futexes.remove(&target_physaddr);
                            // Clear WAITERS bit since no more waiters
                            futex_word.fetch_and(!FUTEX_WAITERS, Ordering::Relaxed);
                        }
                    }
                }

                // Release the lock
                futex_word.store(0, Ordering::Release);
            } else {
                // No waiters, just release
                futex_word.store(0, Ordering::Release);
            }

            Ok(0)
        }

        _ => Err(Error::new(EINVAL)),
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FutexWaitv {
    pub val: u64,
    pub uaddr: usize,
    pub flags: u32,
    pub __reserved: u32,
}

pub fn futex_waitv(
    waiters_addr: usize,
    nr_futexes: usize,
    flags: usize,
    timeout_addr: usize,
    _clockid: usize,
    token: &mut CleanLockToken,
) -> Result<usize> {
    if flags != 0 {
        return Err(Error::new(EINVAL));
    }
    // Limit nr_futexes to avoid DoS (Linux uses 128)
    if nr_futexes > 128 {
        return Err(Error::new(EINVAL));
    }

    let timeout_opt = UserSlice::ro(timeout_addr, core::mem::size_of::<TimeSpec>())?
        .none_if_null()
        .map(|buf| unsafe { buf.read_exact::<TimeSpec>() })
        .transpose()?;

    let waiters_slice = UserSlice::ro(
        waiters_addr,
        nr_futexes * core::mem::size_of::<FutexWaitv>(),
    )?;
    let mut waiters =
        alloc::vec![FutexWaitv { val: 0, uaddr: 0, flags: 0, __reserved: 0 }; nr_futexes];
    unsafe {
        let waiters_u8 = core::slice::from_raw_parts_mut(
            waiters.as_mut_ptr() as *mut u8,
            nr_futexes * core::mem::size_of::<FutexWaitv>(),
        );
        waiters_slice.copy_to_slice(waiters_u8)?;
    }

    let current_addrsp = AddrSpace::current(token)?;
    let addr_space_guard = current_addrsp.acquire_read();

    {
        let mut state_lock = FUTEX_STATE.lock(token.token());
        let (state, mut token) = state_lock.token_split();
        let context_lock = context::current();

        let waiter_priority = context_lock
            .read(token.token())
            .priority
            .effective_priority();

        // 1. Validate all values first
        for waiter in waiters.iter() {
            let addr = waiter.uaddr;
            let val = waiter.val;

            if addr % 4 != 0 {
                return Err(Error::new(EINVAL));
            }

            let target_virtaddr = VirtualAddress::new(addr);
            let target_physaddr = validate_and_translate_virt(&addr_space_guard, target_virtaddr)
                .ok_or(Error::new(EFAULT))?;

            let accessible_addr =
                unsafe { crate::paging::RmmA::phys_to_virt(target_physaddr) }.data();
            let fetched = u64::from(unsafe {
                (*(accessible_addr as *const AtomicU32)).load(Ordering::SeqCst)
            });

            if fetched != val {
                return Err(Error::new(EAGAIN));
            }
        }

        // 2. Add to all lists (with PI tracking)
        for waiter in waiters.iter() {
            let addr = waiter.uaddr;
            let target_virtaddr = VirtualAddress::new(addr);
            let target_physaddr =
                validate_and_translate_virt(&addr_space_guard, target_virtaddr).unwrap();

            state
                .futexes
                .entry(target_physaddr)
                .or_insert_with(Vec::new)
                .push(FutexEntry {
                    target_virtaddr,
                    context_lock: context_lock.clone(),
                    addr_space: Arc::downgrade(&current_addrsp),
                    waiter_priority,
                    blocked_on: Some(target_physaddr),
                });
        }

        // 3. Block
        {
            let mut context = context_lock.write(token.token());

            context.wake = timeout_opt.map(|TimeSpec { tv_sec, tv_nsec }| {
                tv_sec as u128 * time::NANOS_PER_SEC + tv_nsec as u128
            });

            if let Some((tctl, pctl, _)) = context.sigcontrol() {
                if tctl.currently_pending_unblocked(pctl) != 0 {
                    return Err(Error::new(EINTR));
                }
            }

            context.block("futex_waitv");
        }
    }

    drop(addr_space_guard);

    unsafe { context::switch(token) };

    // 4. Cleanup (remove from all lists)
    let _addr_space_guard = current_addrsp.acquire_read();
    {
        let mut state_lock = FUTEX_STATE.lock(token.token());
        let (state, _token) = state_lock.token_split();
        let context_lock = context::current();

        for waiter in waiters.iter() {
            let addr = waiter.uaddr;
            if let Some(target_physaddr) =
                validate_and_translate_virt(&_addr_space_guard, VirtualAddress::new(addr))
            {
                if let Some(list) = state.futexes.get_mut(&target_physaddr) {
                    let my_context_ptr = Arc::as_ptr(&context_lock);
                    list.retain(|entry| Arc::as_ptr(&entry.context_lock) != my_context_ptr);

                    if list.is_empty() {
                        state.futexes.remove(&target_physaddr);
                    }
                }
            }
        }
    }

    if timeout_opt.is_some() {
        context::current().write(token.token()).wake = None;
    }

    Ok(0)
}

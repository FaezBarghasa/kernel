# Sync Module

The `sync` module contains synchronization primitives that are used throughout the kernel. This module has been significantly enhanced with lock-free and priority-inheritance-capable primitives to minimize IPC latency.

## Core Primitives

- `ordered.rs`: Implements compile-time lock ordering levels to prevent deadlocks. Includes `Mutex` and `RwLock` with **Priority Inheritance** support.
- `wait_condition.rs`: A condition variable to block contexts until a condition is met.
- `wait_queue.rs`: A standard queue of contexts waiting for an event.

## Performance Optimizations

- `lockfree_queue.rs`: Implements a Michael-Scott lock-free MPMC queue. Used in hot paths to eliminate spinlock contention.
- `optimized_wait_queue.rs`: A high-performance drop-in replacement for `WaitQueue`. It leverages the lock-free queue and optimizes for the "no-waiter" fast path.
- `priority.rs`: Implements the `PriorityTracker` and `IpcCriticalGuard`. Provides dynamic priority boosting (RCU-boost style) and priority inheritance logic.
- `mod.rs`: Central export point for all synchronization primitives.

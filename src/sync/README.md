# Sync Module

The `sync` module contains synchronization primitives that are used throughout the kernel.

This module contains the following files:

*   `ordered.rs`: This file contains the `Ordered` struct, which is used to ensure that operations are performed in a specific order.
*   `wait_condition.rs`: This file contains the `WaitCondition` struct, which is a condition variable that can be used to block a context until a condition is met.
*   `wait_queue.rs`: This file contains the `WaitQueue` struct, which is a queue of contexts that are waiting for a specific event to occur.

# Context Module

The `context` module manages execution contexts, including thread state, memory management, and scheduling integration.

## Core Components

- `context.rs`: Defines the `Context` struct, representing an execution thread (including registers, stack, and priority).
- `switch.rs`: Core logic for context switching.
- `memory.rs`: Memory space management and paging for contexts.
- `signal.rs`: Logic for asynchronous signals.
- `timeout.rs`: Management of context-specific timeouts.

## Performance & Optimization

- `optimized_switch.rs`: Implements ultra-fast context switching with:
  - **Same Address Space Fast Path**: Skips TLB flushes when switching between threads in the same process.
  - **Lazy FPU Saving**: Postpones FPU/SIMD state saving until strictly necessary.
  - **Prefetching**: Issues CPU hints to pre-load the next context's state.
  - **TSC Profiling**: Tracks per-CPU switch latency in cycles.
- `list.rs`: Management of the global context list.
- `reap.rs`: Cleanup of terminated contexts.
- `page_count.rs`: Tracking allocated memory pages.
- `file.rs`: File descriptor management per context.

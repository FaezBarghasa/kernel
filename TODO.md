# RedoxOS Kernel TODO List

This document tracks the tasks required to fix the current build state, address missing features, and complete the ongoing IPC and context switch optimizations.

## Immediate Compilation Fixes

### `src/sync/mod.rs`

- [ ] **Resolve duplicate definitions**: `CleanLockToken` and `OptimizedWaitQueue` are defined multiple times. Check for conflicting `pub use` statements or duplicate module declarations.
- [ ] **Remove unused imports**: Clean up unused imports like `ArcRwLockWriteGuard`, `Higher`, `L3`, etc.

### `src/context/memory.rs`

- [ ] **Fix `grant_flags`**: Resolve duplicate definition error.
- [ ] **Fix `TlbShootdownActions`**: The type is missing from `crate::paging`. It needs to be defined or imported correctly.
- [ ] **Fix Type Mismatches**: Address errors where `&PageTable` is expected but `PageTable` is found.

### `src/syscall/mod.rs`

- [ ] **Restore Syscalls**: Uncomment `SYS_MLOCK` and `SYS_MUNLOCK` handlers once the `redox_syscall` crate is updated to include these constants.

### `src/syscall/process.rs`

- [ ] **Fix `ContextRef` usage**: It is being used as a function or tuple struct constructor, but it is likely a type alias.

### `src/panic.rs`

- [ ] **Resolve `panic_impl` conflict**: There is a duplicate `panic_impl` lang item. This is likely due to a dependency (like `parking_lot`) pulling in `std`.
  - *Action*: Update `Cargo.toml` to ensure `parking_lot` uses `default-features = false` or does not enable the `std` feature.

### `src/scheme/`

- [ ] **`src/scheme/user.rs`**: Fix missing `empty()` method for `PageSpan`.
- [ ] **`src/scheme/user.rs`**: Fix missing methods in `AddrSpaceInner` (e.g., `mmap`, `mmap_anywhere`).
- [ ] **`src/scheme/ring.rs`**: Fix missing `tracker` field in `RwLockReadGuard`.
- [ ] **`src/scheme/ring.rs`**: Fix missing `cq_mask` field in `IpcRing`.

## Configuration & Dependencies

### `Cargo.toml`

- [ ] **Add Missing Features**: The compiler is warning about unexpected `cfg` conditions. Add the following to `[features]` or configure `check-cfg`:
  - `dtb`
  - `sys_fdstat`
  - `qemu_debug`
  - `lpss_debug`
  - `system76_ec_debug`
  - `x86_kvm_pv`
  - `pti`
- [ ] **`parking_lot`**: Verify `no_std` compatibility settings.

## Refactoring & Cleanup

- [ ] **Unused Imports**: Remove unused imports across the codebase as indicated by compiler warnings.
- [ ] **Dead Code**: Address `dead_code` warnings.
- [ ] **Documentation**: Add missing documentation for public items, especially in the `syscall` module.

## IPC & Context Switch Optimization (Ongoing)

- [ ] **Lock-Free Queue**: Continue implementation and verification of `src/sync/lockfree_queue.rs`.
- [ ] **Optimized Wait Queue**: Finalize `OptimizedWaitQueue` implementation in `src/sync/optimized_wait_queue.rs` and ensure safety comments are accurate.
- [ ] **Priority Inheritance**: Implement Priority Inheritance and Dynamic Boosting in `src/sync/priority.rs`.
- [ ] **Context Switch**: Optimize the context switch path in `src/context/optimized_switch.rs`.

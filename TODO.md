# RedoxOS Kernel TODO List

This document tracks the tasks required to fix the current build state, address missing features, and complete the ongoing IPC and context switch optimizations.

## Overview

This TODO list is divided into several sections:

- **Immediate Compilation Fixes**: Critical issues that are currently preventing the kernel from building successfully.
- **Configuration & Dependencies**: Tasks related to `Cargo.toml` and dependency management.
- **Refactoring & Cleanup**: General code quality improvements.
- **IPC & Context Switch Optimization**: Ongoing work to improve kernel performance.
- **TODOs and FIXMEs from code**: A collection of smaller tasks and fixes extracted from comments in the source code.
- **Unimplemented code**: A list of functions and features that are yet to be implemented.

## Immediate Compilation Fixes

### `src/sync/mod.rs`

- [x] **Resolve duplicate definitions**: `CleanLockToken` and `OptimizedWaitQueue` are defined multiple times. Check for conflicting `pub use` statements or duplicate module declarations.
- [x] **Remove unused imports**: Clean up unused imports like `ArcRwLockWriteGuard`, `Higher`, `L3`, etc.

### `src/context/memory.rs`

- [x] **Fix `grant_flags`**: Resolve duplicate definition error.
- [x] **Fix `TlbShootdownActions`**: The type is missing from `crate::paging`. It needs to be defined or imported correctly.
- [x] **Fix Type Mismatches**: Address errors where `&PageTable` is expected but `PageTable` is found.

### `src/syscall/mod.rs`

- [x] **Restore Syscalls**: Uncomment `SYS_MLOCK` and `SYS_MUNLOCK` handlers once the `redox_syscall` crate is updated to include these constants.

### `src/syscall/process.rs`

- [x] **Fix `ContextRef` usage**: It is being used as a function or tuple struct constructor, but it is likely a type alias.

### `src/panic.rs`

- [x] **Resolve `panic_impl` conflict**: There is a duplicate `panic_impl` lang item. This is likely due to a dependency (like `parking_lot`) pulling in `std`.
  - *Action*: Update `Cargo.toml` to ensure `parking_lot` uses `default-features = false` or does not enable the `std` feature. (Resolved by disabling `parking_lot`).

### `src/scheme/`

- [ ] **`src/scheme/user.rs`**: Fix missing `empty()` method for `PageSpan`.
- [ ] **`src/scheme/user.rs`**: Fix missing methods in `AddrSpaceInner` (e.g., `mmap`, `mmap_anywhere`).
- [ ] **`src/scheme/ring.rs`**: Fix missing `tracker` field in `RwLockReadGuard`.
- [ ] **`src/scheme/ring.rs`**: Fix missing `cq_mask` field in `IpcRing`.

## Configuration & Dependencies

### `Cargo.toml`

- [x] **Add Missing Features**: The compiler is warning about unexpected `cfg` conditions. Add the following to `[features]` or configure `check-cfg`:
  - `dtb`
  - `sys_fdstat`
  - `qemu_debug`
  - `lpss_debug`
  - `system76_ec_debug`
  - `x86_kvm_pv`
  - `pti`
- [x] **`parking_lot`**: Verify `no_std` compatibility settings.

## Refactoring & Cleanup

- [x] **Unused Imports**: Remove unused imports across the codebase as indicated by compiler warnings.
- [ ] **Dead Code**: Address `dead_code` warnings.
- [x] **Documentation**: Add missing documentation for public items, especially in the `syscall` module.

## IPC & Context Switch Optimization (Ongoing)

- [x] **Lock-Free Queue**: Continue implementation and verification of `src/sync/lockfree_queue.rs` (Replaced with Mutex for stability).
- [x] **Optimized Wait Queue**: Finalize `OptimizedWaitQueue` implementation in `src/sync/optimized_wait_queue.rs` and ensure safety comments are accurate (Fixed race condition).
- [x] **Priority Inheritance**: Implement Priority Inheritance and Dynamic Boosting in `src/sync/priority.rs`.
- [ ] **Context Switch**: Optimize the context switch path in `src/context/optimized_switch.rs`.

## New Implementations

- [x] **RISC-V Stubs**: Implemented `kstop`/`kreset`.
- [x] **AArch64 GIC**: Implemented GICv2 support.
- [x] **KPTI**: Implemented KPTI hooks for x86_64.
- [x] **ACPI**: Added FADT parsing and AML tables iterator.
- [x] **Stress Tests**: Added kernel stress test suite (`stress_test` feature).

## TODOs and FIXMEs from code

### `../rmm/src/allocator/frame/buddy.rs`
- [ ] TODO: sort areas?

### `../rmm/src/arch/aarch64.rs`
- [ ] TODO
- [ ] TODO: what makes an address valid on aarch64?
- [ ] TODO: Separate the two?

### `../rmm/src/arch/emulate.rs`
- [ ] TODO: allow reading past page boundaries
- [ ] TODO: cleanup
- [ ] TODO: allow writing past page boundaries
- [ ] TODO: Don't see why an emulated arch would have any problems with canonicalness...

### `../rmm/src/arch/mod.rs`
- [ ] TODO: Support having all page tables compile on all architectures
- [ ] TODO: this stub only works on x86_64, maybe make the arch implement this?

### `../rmm/src/arch/riscv64/sv39.rs`
- [ ] (address.data() >> Self::PAGE_SHIFT);  Convert to PPN (TODO: ensure alignment)

### `../rmm/src/arch/riscv64/sv48.rs`
- [ ] (address.data() >> Self::PAGE_SHIFT);  Convert to PPN (TODO: ensure alignment)

### `../rmm/src/arch/x86.rs`
- [ ] TODO: USE PAE

### `../rmm/src/arch/x86_64.rs`
- [ ] TODO: 5-level paging

### `../rmm/src/lib.rs`
- [ ] TODO: Use this throughout the code

### `../rmm/src/main.rs`
- [ ] TODO: Allow allocations up to maximum pageable size
- [ ] TODO: This causes fragmentation, since neighbors are not identified
- [ ] TODO: remainders less than PAGE_SIZE will be lost

### `../rmm/src/page/flags.rs`
- [ ] TODO: write xor execute?

### `../rmm/src/page/flush.rs`
- [ ] TODO: Might remove Drop and add #[must_use] again, but ergonomically I prefer being able to pass

### `../rmm/src/page/mapper.rs`
- [ ] TODO: correct flags?
- [ ] TODO: Use a counter? This would reduce the remaining number of available bits, bu
- [ ] TODO: check for overwriting entry
- [ ] TODO: verify virt is aligned
- [ ] TODO: Higher-level PageEntry::new interface?
- [ ] TODO: verify virt and phys are aligned
- [ ] TODO: verify flags have correct bits
- [ ] TODO: This is a bad idea for architectures where the kernel mappings are done in the p

### `src/acpi/hpet.rs`
- [ ] TODO: x86 use assumes only one HPET and only one GenericAddressStructure

### `src/acpi/madt/arch/aarch64.rs`
- [ ] TODO: get GICRs
- [ ] TODO: support more GICCs

### `src/acpi/madt/arch/x86.rs`
- [ ] TODO: do not have writable and executable!
- [ ] TODO: Is this necessary (this fence)?

### `src/acpi/madt/mod.rs`
- [ ] TODO: optional field introduced in ACPI 6.5: pub trbe_interrupt: u16,

### `src/acpi/mod.rs`
- [ ] TODO: support this on any arch
- [ ] TODO: Let userspace setup HPET, and then provide an interface to specify which timer
- [ ] TODO: Enumerate processors in userspace, and then provide an ACPI-independent interfa
- [ ] TODO: Don't touch ACPI tables in kernel?

### `src/acpi/rsdp.rs`
- [ ] TODO: Validate

### `src/acpi/spcr.rs`
- [ ] TODO: support more types!
- [ ] TODO: these fields are optional based on the table revision
- [ ] TODO: enable IRQ on more platforms and interrupt types

### `src/arch/aarch64/device/irqchip/gicv3.rs`
- [ ] TODO: GICC, GICH, GICV?
- [ ] TODO: deinit?

### `src/arch/aarch64/device/irqchip/irq_bcm2835.rs`
- [ ] TODO: support smp self.read(LOCAL_IRQ_PENDING + 4 * cpu)
- [ ] TODO: check bank && irq

### `src/arch/aarch64/device/serial.rs`
- [ ] TODO: what should chip index be?
- [ ] TODO: get actual register size from device tree
- [ ] TODO: find actual serial device, not just any PL011

### `src/arch/aarch64/interrupt/exception.rs`
- [ ] TODO: RMW instructions may "involve" writing to (possibly invalid) memory, but AArch64
- [ ] kind: 0,  TODO

### `src/arch/aarch64/interrupt/irq.rs`
- [ ] TODO
- [ ] FIXME add_irq accepts a u8 as irq number

### `src/arch/aarch64/ipi.rs`
- [ ] FIXME implement

### `src/arch/aarch64/paging/mapper.rs`
- [ ] TODO: Push to TLB "mailbox" or tell it to reload CR3 if there are too many entries.

### `src/arch/aarch64/paging/mod.rs`
- [ ] TODO assert!(address.data() < 0x0000_8000_0000_0000 || address.data() >= 0xffff_8000_0000_

### `src/arch/aarch64/start.rs`
- [ ] TODO: use env {DTB,RSDT}_{BASE,SIZE}?

### `src/arch/aarch64/time.rs`
- [ ] TODO: aarch64 generic timer counter

### `src/arch/riscv64/device/irqchip/clint_sbi.rs`
- [ ] TODO IPI
- [ ] FIXME dirty hack map M-mode interrupts (handled by SBI) to S-mode interrupts we get f

### `src/arch/riscv64/device/irqchip/plic.rs`
- [ ] TODO spread irqs over all the cores when we have them?

### `src/arch/riscv64/device/serial.rs`
- [ ] COM1.lock().enable_irq();  FIXME receive int is enabled by default in 16550. Disable by
- [ ] TODO: get actual register size from device tree

### `src/arch/riscv64/interrupt/exception.rs`
- [ ] FIXME use align(4)
- [ ] TODO
- [ ] FIXME use extern "custom"
- [ ] FIXME retrieve from percpu area
- [ ] FIXME can these conditions be distinguished? Should they be?

### `src/arch/riscv64/interrupt/handler.rs`
- [ ] TODO

### `src/arch/riscv64/ipi.rs`
- [ ] FIXME implement

### `src/arch/riscv64/paging/mapper.rs`
- [ ] TODO: Push to TLB "mailbox" or tell it to reload CR3 if there are too many entries.
- [ ] TODO: cpu id

### `src/arch/riscv64/paging/mod.rs`
- [ ] TODO: detect Svpbmt present/enabled and override device memory with PBMT=IO

### `src/arch/riscv64/start.rs`
- [ ] FIXME bringup AP HARTs

### `src/arch/x86/interrupt/handler.rs`
- [ ] TODO: Unmap PTI (split "add esp, 8" into two "add esp, 4"s maybe?)
- [ ] FIXME: The interrupt stack on which this is called, is always from userspace, but make
- [ ] TODO: Unmap PTI
- [ ] TODO: Map PTI

### `src/arch/x86_64/interrupt/syscall.rs`
- [ ] TODO: Should we unconditionally jump or avoid jumping, to hint to the branch predictor that
- [ ] TODO: Map PTI
- [ ] TODO: Which one is faster?
- [ ] TODO: macro?
- [ ] TODO: Unmap PTI

### `src/arch/x86_shared/cpuid.rs`
- [ ] FIXME check for cpuid availability during early boot and error out if it doesn't exist.

### `src/arch/x86_shared/device/ioapic.rs`
- [ ] FIXME: With ACPI moved to userspace, we should instead allow userspace to check whether t

### `src/arch/x86_shared/device/mod.rs`
- [ ] TODO: fix HPET on i686

### `src/arch/x86_shared/device/serial.rs`
- [ ] FIXME remove serial_debug feature once ACPI SPCR is respected on UEFI boots.
- [ ] FIXME remove explicit LPSS handling once ACPI SPCR is supported
- [ ] TODO: Make this configurable

### `src/arch/x86_shared/device/system76_ec.rs`
- [ ] TODO: timeout

### `src/arch/x86_shared/device/tsc.rs`
- [ ] TODO: Implement KVM paravirtualized TSC reading

### `src/arch/x86_shared/idt.rs`
- [ ] TODO: use_default_irqs! but also the legacy IRQs that are only needed on one CPU
- [ ] TODO: VecMap?

### `src/arch/x86_shared/paging/mod.rs`
- [ ] TODO assert!(address.data() < 0x0000_8000_0000_0000 || address.data() >= 0xffff_8000_0000_

### `src/arch/x86_shared/start.rs`
- [ ] FIXME use extern "custom"

### `src/arch/x86_shared/stop.rs`
- [ ] TODO: Waitpid with timeout? Because, what if the ACPI driver would crash?
- [ ] TODO: Switch directly to whichever process is handling the kstop pipe. We would add an

### `src/arch/x86_shared/time.rs`
- [ ] TODO: improve performance
- [ ] TODO: handle rollover?

### `src/dtb/irqchip.rs`
- [ ] TODO: support multi level interrupt constrollers
- [ ] FIXME use the helper when fixed (see gh#37)
- [ ] FIXME use interrupts() helper when fixed (see gh#12)

### `src/dtb/mod.rs`
- [ ] FIXME assumes all the devices are connected to CPUs via the /soc bus
- [ ] FIXME traverse device tree up

## Unimplemented code

### `../rmm/src/allocator/frame/bump.rs`
- [ ] unimplemented!("BumpAllocator::free not implemented");

### `../rmm/src/arch/aarch64.rs`
- [ ] unimplemented!("AArch64Arch::init unimplemented");

### `../rmm/src/arch/emulate.rs`
- [ ] unimplemented!("EmulateArch::invalidate not implemented");

### `../rmm/src/arch/riscv64/sv39.rs`
- [ ] unimplemented!("RiscV64Sv39Arch::init unimplemented");

### `../rmm/src/arch/riscv64/sv48.rs`
- [ ] unimplemented!("RiscV64Sv48Arch::init unimplemented");

### `../rmm/src/arch/x86.rs`
- [ ] unimplemented!("X86Arch::init unimplemented");

### `../rmm/src/arch/x86_64.rs`
- [ ] unimplemented!("X8664Arch::init unimplemented");

### `src/arch/aarch64/device/irqchip/null.rs`
- [ ] unimplemented!()

### `src/arch/riscv64/device/cpu/mod.rs`
- [ ] unimplemented!()

### `src/arch/riscv64/rmm.rs`
- [ ] unimplemented!()

### `src/arch/riscv64/stop.rs`
- [ ] unimplemented!()

### `src/arch/x86_shared/interrupt/irq.rs`
- [ ] IrqMethod::Apic => Ok(Vec::from(&b"(not implemented for APIC yet)"[..])),

### `src/context/arch/riscv64.rs`
- [ ] unimplemented!()

### `src/scheme/proc.rs`
- [ ] todo!(),

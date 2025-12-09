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

## TODOs and FIXMEs from code

- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/device/serial.rs**: FIXME remove serial_debug feature once ACPI SPCR is respected on UEFI boots.
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/mod.rs**: TODO: Support having all page tables compile on all architectures
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/acpi/spcr.rs**: TODO: support more types!
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/interrupt/exception.rs**: FIXME use align(4)
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/idt.rs**: TODO: use_default_irqs! but also the legacy IRQs that are only needed on one CPU
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/main.rs**: TODO: Allow allocations up to maximum pageable size
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/emulate.rs**: TODO: allow reading past page boundaries
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_64/interrupt/syscall.rs**: TODO: Should we unconditionally jump or avoid jumping, to hint to the branch predictor that
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/acpi/rsdp.rs**: TODO: Validate
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/device/irqchip/irq_bcm2835.rs**: TODO: support smp self.read(LOCAL_IRQ_PENDING + 4 * cpu)
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/start.rs**: FIXME use extern "custom"
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/dtb/mod.rs**: FIXME assumes all the devices are connected to CPUs via the /soc bus
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/page/mapper.rs**: TODO: correct flags?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/paging/mapper.rs**: TODO: Push to TLB "mailbox" or tell it to reload CR3 if there are too many entries.
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/interrupt/exception.rs**: TODO: RMW instructions may "involve" writing to (possibly invalid) memory, but AArch64
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/device/system76_ec.rs**: TODO: timeout
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/page/mapper.rs**: TODO: Use a counter? This would reduce the remaining number of available bits, bu
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/acpi/madt/mod.rs**: TODO: optional field introduced in ACPI 6.5: pub trbe_interrupt: u16,
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/mod.rs**: TODO: this stub only works on x86_64, maybe make the arch implement this?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/device/irqchip/plic.rs**: TODO spread irqs over all the cores when we have them?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86/interrupt/handler.rs**: TODO: Unmap PTI (split "add esp, 8" into two "add esp, 4"s maybe?)
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/aarch64.rs**: TODO
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/device/serial.rs**: COM1.lock().enable_irq();  FIXME receive int is enabled by default in 16550. Disable by
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/device/mod.rs**: TODO: fix HPET on i686
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/time.rs**: TODO: aarch64 generic timer counter
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/acpi/mod.rs**: TODO: support this on any arch
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/device/irqchip/gicv3.rs**: TODO: GICC, GICH, GICV?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/device/irqchip/irq_bcm2835.rs**: TODO: check bank && irq
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/device/irqchip/clint_sbi.rs**: TODO IPI
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/acpi/spcr.rs**: TODO: these fields are optional based on the table revision
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/x86_64.rs**: TODO: 5-level paging
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/interrupt/irq.rs**: TODO
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/paging/mod.rs**: TODO assert!(address.data() < 0x0000_8000_0000_0000 || address.data() >= 0xffff_8000_0000_
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/stop.rs**: TODO: Waitpid with timeout? Because, what if the ACPI driver would crash?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/ipi.rs**: FIXME implement
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/device/serial.rs**: FIXME remove explicit LPSS handling once ACPI SPCR is supported
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/page/mapper.rs**: TODO: check for overwriting entry
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/allocator/frame/buddy.rs**: TODO: sort areas?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/device/serial.rs**: TODO: what should chip index be?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/device/irqchip/clint_sbi.rs**: FIXME dirty hack map M-mode interrupts (handled by SBI) to S-mode interrupts we get f
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_64/interrupt/syscall.rs**: TODO: Map PTI
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86/interrupt/handler.rs**: FIXME: The interrupt stack on which this is called, is always from userspace, but make
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/device/tsc.rs**: TODO: Implement KVM paravirtualized TSC reading
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/paging/mapper.rs**: TODO: Push to TLB "mailbox" or tell it to reload CR3 if there are too many entries.
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/stop.rs**: TODO: Switch directly to whichever process is handling the kstop pipe. We would add an
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/paging/mapper.rs**: TODO: cpu id
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/emulate.rs**: TODO: cleanup
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/acpi/hpet.rs**: TODO: x86 use assumes only one HPET and only one GenericAddressStructure
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/device/serial.rs**: TODO: Make this configurable
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/page/mapper.rs**: TODO: verify virt is aligned
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/page/mapper.rs**: TODO: Higher-level PageEntry::new interface?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86/interrupt/handler.rs**: TODO: Unmap PTI
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/ipi.rs**: FIXME implement
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/dtb/irqchip.rs**: TODO: support multi level interrupt constrollers
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/cpuid.rs**: FIXME check for cpuid availability during early boot and error out if it doesn't exist.
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/start.rs**: FIXME bringup AP HARTs
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/aarch64.rs**: TODO: what makes an address valid on aarch64?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/interrupt/irq.rs**: FIXME add_irq accepts a u8 as irq number
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/acpi/mod.rs**: TODO: Let userspace setup HPET, and then provide an interface to specify which timer 
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/device/ioapic.rs**: FIXME: With ACPI moved to userspace, we should instead allow userspace to check whether t
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/dtb/irqchip.rs**: FIXME use the helper when fixed (see gh#37)
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86/interrupt/handler.rs**: TODO: Map PTI
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/device/serial.rs**: TODO: get actual register size from device tree
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/acpi/spcr.rs**: TODO: enable IRQ on more platforms and interrupt types
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/device/irqchip/gicv3.rs**: TODO: deinit?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/interrupt/exception.rs**: TODO
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/page/mapper.rs**: TODO: verify virt and phys are aligned
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/page/flush.rs**: TODO: Might remove Drop and add #[must_use] again, but ergonomically I prefer being able to pass
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/acpi/mod.rs**: TODO: Enumerate processors in userspace, and then provide an ACPI-independent interfa
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/acpi/mod.rs**: TODO: Don't touch ACPI tables in kernel?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86/interrupt/handler.rs**: TODO: Unmap PTI
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/riscv64/sv48.rs**: (address.data() >> Self::PAGE_SHIFT);  Convert to PPN (TODO: ensure alignment)
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/interrupt/handler.rs**: TODO
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/emulate.rs**: TODO: allow writing past page boundaries
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/acpi/madt/arch/aarch64.rs**: TODO: get GICRs
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_64/interrupt/syscall.rs**: TODO: Which one is faster?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/emulate.rs**: TODO: Don't see why an emulated arch would have any problems with canonicalness...
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/interrupt/exception.rs**: FIXME use extern "custom"
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/interrupt/exception.rs**: FIXME retrieve from percpu area
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/interrupt/exception.rs**: FIXME can these conditions be distinguished? Should they be?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/paging/mod.rs**: TODO: detect Svpbmt present/enabled and override device memory with PBMT=IO
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/interrupt/exception.rs**: kind: 0,  TODO
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/time.rs**: TODO: improve performance
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/x86.rs**: TODO: USE PAE
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/dtb/irqchip.rs**: FIXME use interrupts() helper when fixed (see gh#12)
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/start.rs**: TODO: use env {DTB,RSDT}_{BASE,SIZE}?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_64/interrupt/syscall.rs**: TODO: macro?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/acpi/madt/arch/x86.rs**: TODO: do not have writable and executable!
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/main.rs**: TODO: This causes fragmentation, since neighbors are not identified
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/lib.rs**: TODO: Use this throughout the code
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/device/serial.rs**: TODO: get actual register size from device tree
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/paging/mod.rs**: TODO assert!(address.data() < 0x0000_8000_0000_0000 || address.data() >= 0xffff_8000_0000_
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/page/mapper.rs**: TODO: verify flags have correct bits
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_64/interrupt/syscall.rs**: TODO: Unmap PTI
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/page/flags.rs**: TODO: write xor execute?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/device/serial.rs**: TODO: find actual serial device, not just any PL011
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/acpi/madt/arch/aarch64.rs**: TODO: support more GICCs
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/aarch64.rs**: TODO: Separate the two?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/page/mapper.rs**: TODO: This is a bad idea for architectures where the kernel mappings are done in the p
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/acpi/madt/arch/x86.rs**: TODO: Is this necessary (this fence)?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/riscv64/sv39.rs**: (address.data() >> Self::PAGE_SHIFT);  Convert to PPN (TODO: ensure alignment)
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/time.rs**: TODO: handle rollover?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/main.rs**: TODO: remainders less than PAGE_SIZE will be lost
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/idt.rs**: TODO: VecMap?
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/dtb/mod.rs**: FIXME traverse device tree up

## Unimplemented code

- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/riscv64/sv39.rs**: unimplemented!("RiscV64Sv39Arch::init unimplemented");
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/x86_64.rs**: unimplemented!("X8664Arch::init unimplemented");
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/riscv64/sv48.rs**: unimplemented!("RiscV64Sv48Arch::init unimplemented");
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/device/irqchip/null.rs**: unimplemented!()
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/allocator/frame/bump.rs**: unimplemented!("BumpAllocator::free not implemented");
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/context/arch/riscv64.rs**: unimplemented!()
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/rmm.rs**: unimplemented!()
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/x86.rs**: unimplemented!("X86Arch::init unimplemented");
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/stop.rs**: unimplemented!()
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/device/cpu/mod.rs**: unimplemented!()
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/aarch64.rs**: unimplemented!("AArch64Arch::init unimplemented");
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/emulate.rs**: unimplemented!("EmulateArch::invalidate not implemented");
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/scheme/proc.rs**: todo!(),
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/riscv64/sv39.rs**: unimplemented!("RiscV64Sv39Arch::init unimplemented");
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/x86_64.rs**: unimplemented!("X8664Arch::init unimplemented");
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/aarch64/device/irqchip/null.rs**: unimplemented!()
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/allocator/frame/bump.rs**: unimplemented!("BumpAllocator::free not implemented");
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/context/arch/riscv64.rs**: unimplemented!()
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/stop.rs**: unimplemented!()
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/rmm.rs**: unimplemented!()
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/aarch64.rs**: unimplemented!("AArch64Arch::init unimplemented");
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/x86_shared/interrupt/irq.rs**: IrqMethod::Apic => Ok(Vec::from(&b"(not implemented for APIC yet)"[..])),
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/riscv64/sv48.rs**: unimplemented!("RiscV64Sv48Arch::init unimplemented");
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/x86.rs**: unimplemented!("X86Arch::init unimplemented");
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/src/arch/riscv64/device/cpu/mod.rs**: unimplemented!()
- [ ] **/home/jrad/RustroverProjects/redoxos/kernel/rmm/src/arch/emulate.rs**: unimplemented!("EmulateArch::invalidate not implemented");

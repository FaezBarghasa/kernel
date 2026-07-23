# RedoxOS Kernel TODO List

This document tracks active pending tasks, refactoring items, and unresolved code TODOs/FIXMEs.

## Overview

This list contains remaining pending work:

- **Refactoring & Cleanup**: Open code quality items.
- **TODOs and FIXMEs from code**: Collection of uncompleted tasks from comments in source code.
- **Mainstream Redox OS Perspective**: High-level roadmap goals and current focus areas of the upstream project.

## Refactoring & Cleanup

- [ ] **Dead Code**: Address `dead_code` warnings across modules.

## Mainstream Redox OS Perspective (Remote)

This section captures the high-level roadmap and focus areas of the mainstream Redox OS project and its core developers.

### System & Architecture
- [ ] **Sandboxing by Default**: Move towards capability-based security where applications only have access to necessary resources.
- [ ] **Self-Hosting**: Continue progress toward compiling the entirety of Redox OS from within Redox OS itself.
- [ ] **Linux Driver Emulation**: Implement strategies like running a stripped-down Linux kernel in QEMU to provide legacy hardware support without porting thousands of drivers manually.
- [ ] **Stability & Maturation**: Transition from pre-stable experimental phases to a stable general-purpose microkernel operating system.

### Desktop & Applications
- [ ] **Ecosystem Expansion**: Port more common user-space applications and environments to native Redox.
- [ ] **Orbital Windowing System**: Refine and stabilize the native GUI.
- [ ] **Wayland Support**: Improve compatibility to run Wayland-based applications directly on Redox OS.

### Hardware Compatibility
- [ ] **Real Hardware Booting**: Improve the boot process robustness across varying physical hardware, ensuring failures in specific drivers do not halt the entire system.
- [ ] **Hardware Enablement**: Continued work on essential drivers (e.g., networking, storage, USB, audio) and resolving regressions on bare metal.

### Upstream Alignment & Personal Integration Roadmap
- [ ] **Bring Forks Up to Date**: Add the official `redox-os` upstream remote to local forks (e.g., `orbital`, `redoxfs`), fetch, resolve merge conflicts, and push to GitHub.
- [ ] **Transition to Contributor**: Tackle "good first issues" on the official GitLab to establish a contribution history.
- [ ] **Upstream `rmm` Crate**: Initiate discussions with Redox maintainers on Matrix to propose integrating the isolated `rmm` abstractions into the core project.
- [ ] **Submit Mobile Shell PR**: Once `orbital` is synced, submit a Merge Request for the "mobile shell" feature with clear documentation of its benefits.
- [ ] **Build Public Presence**: Document the learning/contribution journey via blog posts, participate in community channels, and help mentor newcomers.

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

### `src/arch/riscv64/paging/mapper.rs`
- [ ] TODO: Push to TLB "mailbox" or tell it to reload CR3 if there are too many entries.
- [ ] TODO: cpu id

### `src/arch/riscv64/paging/mod.rs`
- [ ] TODO: detect Svpbmt present/enabled and override device memory with PBMT=IO

### `src/arch/riscv64/start.rs`
- [ ] FIXME bringup AP HARTs

### `src/arch/x86/interrupt/handler.rs`
- [ ] FIXME: The interrupt stack on which this is called, is always from userspace, but make

### `src/arch/x86_64/interrupt/syscall.rs`
- [ ] TODO: Should we unconditionally jump or avoid jumping, to hint to the branch predictor that
- [ ] TODO: Which one is faster?
- [ ] TODO: macro?

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

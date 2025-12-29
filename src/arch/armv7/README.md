# ARMv7 Architecture Support

This directory contains the ARMv7-A (32-bit ARM) architecture support for the Redox kernel.

## Overview

ARMv7-A is a 32-bit ARM architecture commonly found in:

- **Single Board Computers**: Raspberry Pi 2, BeagleBone Black, ODROID-C1
- **Embedded Systems**: Various industrial and IoT devices
- **Development Boards**: Many ARM Cortex-A based boards

## Memory Layout

```
0x0000_0000 - 0xBFFF_FFFF  User Space (3 GB)
0xC000_0000 - 0xCFFF_FFFF  Kernel Direct Map (256 MB)
0xD000_0000 - 0xD0FF_FFFF  Kernel Heap (16 MB)
0xD100_0000 - 0xD1FF_FFFF  Per-CPU Variables (16 MB)
0xD200_0000 - 0xFFFF_FFFF  Reserved/Future Use
```

## Module Structure

- **`mod.rs`** - Architecture module entry point
- **`consts.rs`** - Architecture-specific constants
- **`start.rs`** - Boot sequence and CPU initialization
- **`vectors.rs`** - Exception vector table
- **`debug.rs`** - Early debug output via UART
- **`stop.rs`** - Shutdown and halt procedures
- **`misc.rs`** - Miscellaneous utilities
- **`linker.rs`** - Linker script integration
- **`ipi.rs`** - Inter-processor interrupts
- **`time.rs`** - Timer and clock management
- **`rmm.rs`** - Memory manager integration

### Device Drivers (`device/`)

- **`gic.rs`** - Generic Interrupt Controller (GICv2)

### Interrupt Handling (`interrupt/`)

- **`irq.rs`** - IRQ interrupt handling
- **`syscall.rs`** - SVC (Supervisor Call) syscall entry

### Paging (`paging/`)

- **`entry.rs`** - Page table entry definitions
- **`mapper.rs`** - Page table mapping

## Exception Vectors

ARMv7 defines 8 exception vectors:

| Offset | Exception Type      | Handler             |
|--------|---------------------|---------------------|
| 0x00   | Reset               | `reset_handler`     |
| 0x04   | Undefined Instr     | `undefined_handler` |
| 0x08   | SVC (Syscall)       | `svc_handler`       |
| 0x0C   | Prefetch Abort      | `prefetch_abort_handler` |
| 0x10   | Data Abort          | `data_abort_handler` |
| 0x14   | Reserved            | -                   |
| 0x18   | IRQ                 | `irq_handler`       |
| 0x1C   | FIQ                 | `fiq_handler`       |

## Interrupt Controller

The GICv2 (Generic Interrupt Controller version 2) is used for interrupt management:

- **Distributor**: Routes interrupts to CPU interfaces
- **CPU Interface**: Handles interrupt acknowledgment and EOI

## Build Status

✅ Core architecture files complete
✅ Exception vectors implemented
✅ GICv2 driver implemented
⚠️ Paging/MMU support in progress
⚠️ Full device tree parsing pending
⚠️ Multi-core support pending

## Testing

To build for ARMv7:

```bash
cd kernel
cargo build --target armv7-unknown-none-eabihf
```

To test in QEMU:

```bash
make ARCH=armv7 qemu
```

## References

- [ARM Architecture Reference Manual ARMv7-A](https://developer.arm.com/documentation/ddi0406/latest/)
- [ARM Generic Interrupt Controller Architecture Specification](https://developer.arm.com/documentation/ihi0048/latest/)
- [ARM Cortex-A Series Programmer's Guide](https://developer.arm.com/documentation/den0013/latest/)

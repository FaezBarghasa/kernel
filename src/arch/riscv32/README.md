# RISC-V 32-bit Architecture Support

This directory contains the RISC-V 32-bit (RV32) architecture support for the Redox kernel.

## Overview

RISC-V 32 is a 32-bit RISC-V architecture commonly found in:

- **Microcontrollers**: ESP32-C3, ESP32-C6, ESP32-H2
- **SoCs**: SiFive FE310, GD32VF103
- **Development Boards**: HiFive1, Longan Nano

## Module Structure

- **`mod.rs`** - Architecture module entry point
- **`consts.rs`** - Architecture-specific constants
- **`context.rs`** - Context switching
- **`debug.rs`** - Debug output via UART
- **`device/`** - Device drivers (PLIC, CLINT)
- **`interrupt/`** - Interrupt handling
- **`ipi.rs`** - Inter-processor interrupts
- **`misc.rs`** - Miscellaneous utilities
- **`paging/`** - Page table management
- **`rmm.rs`** - Memory manager integration
- **`start.rs`** - Boot sequence
- **`stop.rs`** - Shutdown procedures
- **`time.rs`** - Timer and clock management

## 32-bit Specific Adaptations

### 64-bit Counter Handling

RISC-V 32 has 64-bit `mtime` and `mtimecmp` registers that require special handling:

```rust
// Reading 64-bit mtime on RV32
let counter_hi: u32;
let counter_lo: u32;
asm!(
    "rdtimeh {hi}",
    "rdtime {lo}",
    hi = out(reg) counter_hi,
    lo = out(reg) counter_lo,
);
let counter = ((counter_hi as u64) << 32) | (counter_lo as u64);
```

### Interrupt Controllers

- **PLIC**: Platform-Level Interrupt Controller for external interrupts
- **CLINT**: Core Local Interruptor for timer and software interrupts

## Build Status

✅ **17 Rust files** (up from 6)
✅ **PLIC driver implemented**
✅ **CLINT driver implemented**
✅ **64-bit mtime/mtimecmp handling**
✅ **All core modules ported from riscv64**

## Testing

To build for RISC-V 32:

```bash
cd kernel
cargo build --target riscv32imac-unknown-none-elf
```

To test in QEMU:

```bash
make ARCH=riscv32 qemu
```

## References

- [RISC-V Privileged Specification](https://riscv.org/technical/specifications/)
- [RISC-V PLIC Specification](https://github.com/riscv/riscv-plic-spec)
- [SiFive Interrupt Cookbook](https://sifive.cdn.prismic.io/sifive/0d163928-2128-42be-a75a-464df65e04e0_sifive-interrupt-cookbook.pdf)

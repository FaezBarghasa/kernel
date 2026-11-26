# Arch Module

The `arch` module contains the architecture-specific code for the kernel. This module is responsible for handling the differences between the supported architectures.

This module contains the following submodules:

*   `aarch64`: This submodule contains the architecture-specific code for the ARM64 architecture.
*   `riscv64`: This submodule contains the architecture-specific code for the RISC-V 64-bit architecture.
*   `x86`: This submodule contains the architecture-specific code for the x86 architecture.
*   `x86_64`: This submodule contains the architecture-specific code for the x86-64 architecture.
*   `x86_shared`: This submodule contains code that is shared between the x86 and x86-64 architectures.

# Syscall Module

The `syscall` module is responsible for handling the system calls that are made by user-space applications. System calls are the primary interface between user-space applications and the kernel.

This module contains the following files:

* `debug.rs`: This file contains the implementation of the `log` system call.
* `fs.rs`: This file contains the implementation of the file system related system calls.
* `futex.rs`: This file contains the implementation of the `futex` system call.
* `privilege.rs`: This file contains the implementation of the privilege related system calls.
* `process.rs`: This file contains the implementation of the process related system calls.
* `time.rs`: This file contains the implementation of the time related system calls.
* `usercopy.rs`: This file contains the implementation of the user memory access related system calls.
* `mod.rs`: This file contains the main `syscall` entry point function which dispatches calls to the appropriate handlers.

# Scheme Module

The `scheme` module implements the Redox OS concept of "schemes". Schemes are used to represent kernel resources as file-like objects, which can be accessed by user-space applications using the standard file operations.

This module contains the following files:

*   `acpi.rs`: This file contains the ACPI scheme.
*   `debug.rs`: This file contains the debug scheme.
*   `dtb.rs`: This file contains the DTB scheme.
*   `event.rs`: This file contains the event scheme.
*   `irq.rs`: This file contains the IRQ scheme.
*   `memory.rs`: This file contains the memory scheme.
*   `pipe.rs`: This file contains the pipe scheme.
*   `proc.rs`: This file contains the proc scheme.
*   `ring.rs`: This file contains the ring buffer scheme.
*   `root.rs`: This file contains the root scheme.
*   `serio.rs`: This file contains the serial I/O scheme.
*   `time.rs`: This file contains the time scheme.
*   `user.rs`: This file contains the user scheme.

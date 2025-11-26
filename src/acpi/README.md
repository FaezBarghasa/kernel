# ACPI Module

The `acpi` module is responsible for parsing the ACPI tables provided by the firmware. This module is only compiled if the `acpi` feature is enabled.

This module contains the following files:

*   `madt.rs`: This file contains the code for parsing the Multiple APIC Description Table (MADT).
*   `gtdt.rs`: This file contains the code for parsing the Generic Timer Description Table (GTDT).
*   `hpet.rs`: This file contains the code for parsing the High Precision Event Timer (HPET) table.
*   `rsdp.rs`: This file contains the code for parsing the Root System Description Pointer (RSDP).
*   `rsdt.rs`: This file contains the code for parsing the Root System Description Table (RSDT).
*   `sdt.rs`: This file contains the code for parsing the System Description Table (SDT).
*   `spcr.rs`: This file contains the code for parsing the Serial Port Console Redirection (SPCR) table.
*   `xsdt.rs`: This file contains the code for parsing the Extended System Description Table (XSDT).

use core::fmt::{Result, Write};

pub fn cpu_info<W: Write>(w: &mut W) -> Result {
    write!(w, "RISC-V 64-bit")
}

//! # RISC-V 32-bit Syscall Handler

#[allow(unused_variables)]
pub unsafe extern "C" fn syscall(
    a0: usize, a1: usize, a2: usize, a3: usize, a4: usize, a5: usize, number: usize
) -> usize {
    0
}
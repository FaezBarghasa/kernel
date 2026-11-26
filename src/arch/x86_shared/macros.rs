//! x86_64 Assembly Macros
//! Stub implementations for interrupt handling macros

#[macro_export]
macro_rules! interrupt {
    ($name:ident, || $code:block) => {
        mod $name {
            use super::*;

            #[unsafe(naked)]
            pub unsafe extern "C" fn wrapper() {
                core::arch::naked_asm!(
                    "push rax
                    push rcx
                    push rdx
                    push rsi
                    push rdi
                    push r8
                    push r9
                    push r10
                    push r11
                    push fs
                    mov rax, 0x18
                    mov fs, ax
                    push rbp
                    mov rbp, rsp
                    push rbp
                    call {inner}
                    pop rbp
                    pop rbp
                    pop fs
                    pop r11
                    pop r10
                    pop r9
                    pop r8
                    pop rdi
                    pop rsi
                    pop rdx
                    pop rcx
                    pop rax
                    iretq",
                    inner = sym inner
                );
            }

            pub unsafe extern "C" fn inner() {
                $code
            }
        }
        pub use self::$name::wrapper as $name;
    };
}

#[macro_export]
macro_rules! interrupt_stack {
    ($name:ident, |$stack:ident| $code:block) => {
        mod $name {
            use super::*;

            #[unsafe(naked)]
            pub unsafe extern "C" fn wrapper() {
                core::arch::naked_asm!(
                    push_scratch!(),
                    "mov rax, 0x18
                    mov fs, ax
                    ",
                    push_preserved!(),
                    "mov rdi, rsp
                    call {inner}
                    ",
                    pop_preserved!(),
                    pop_scratch!(),
                    "iretq",
                    inner = sym inner
                );
            }

            pub unsafe extern "C" fn inner($stack: &mut $crate::arch::x86_64::interrupt::handler::InterruptStack) {
                $code
            }
        }
        pub use self::$name::wrapper as $name;
    };
    ($name:ident, @paranoid, |$stack:ident| $code:block) => {
        mod $name {
            use super::*;

            #[unsafe(naked)]
            pub unsafe extern "C" fn wrapper() {
                core::arch::naked_asm!(
                    push_scratch!(),
                    "mov rax, 0x18
                    mov fs, ax
                    ",
                    push_preserved!(),
                    "mov rdi, rsp
                    call {inner}
                    ",
                    pop_preserved!(),
                    pop_scratch!(),
                    "iretq",
                    inner = sym inner
                );
            }

            pub unsafe extern "C" fn inner($stack: &mut $crate::arch::x86_64::interrupt::handler::InterruptStack) {
                $code
            }
        }
        pub use self::$name::wrapper as $name;
    };
}

#[macro_export]
macro_rules! interrupt_error {
    ($name:ident, |$stack:ident, $code:ident| $code_block:block) => {
        mod $name {
            use super::*;

            #[unsafe(naked)]
            pub unsafe extern "C" fn wrapper() {
                core::arch::naked_asm!(
                    push_scratch!(),
                    "mov rax, 0x18
                    mov fs, ax
                    ",
                    push_preserved!(),
                    "mov rdi, rsp
                    mov rsi, [rsp + 128]
                    call {inner}
                    ",
                    pop_preserved!(),
                    pop_scratch!(),
                    "add rsp, 8
                    iretq",
                    inner = sym inner
                );
            }

            pub unsafe extern "C" fn inner($stack: &mut $crate::arch::x86_64::interrupt::handler::InterruptStack, $code: usize) {
                $code_block
            }
        }
        pub use self::$name::wrapper as $name;
    };
}

#[macro_export]
macro_rules! push_scratch {
    () => {
        "push rax
        push rcx
        push rdx
        push rsi
        push rdi
        push r8
        push r9
        push r10
        push r11
        push fs
        "
    };
}

#[macro_export]
macro_rules! pop_scratch {
    () => {
        "pop fs
        pop r11
        pop r10
        pop r9
        pop r8
        pop rdi
        pop rsi
        pop rdx
        pop rcx
        pop rax
        "
    };
}

#[macro_export]
macro_rules! push_preserved {
    () => {
        "push rbx
        push rbp
        push r12
        push r13
        push r14
        push r15
        "
    };
}

#[macro_export]
macro_rules! pop_preserved {
    () => {
        "pop r15
        pop r14
        pop r13
        pop r12
        pop rbp
        pop rbx
        "
    };
}

#[macro_export]
macro_rules! nop {
    () => {
        unsafe { core::arch::asm!("nop", options(nostack, preserves_flags)) }
    };
}

#[macro_export]
macro_rules! swapgs_iff_ring3_fast {
    ($scratch:ident) => {
        "cmp word ptr [rsp + 16], 0x18
        je 1f
        swapgs
        1:"
    };
}

#[macro_export]
macro_rules! swapgs_iff_ring3_fast_errorcode {
    ($scratch:ident) => {
        "cmp word ptr [rsp + 24], 0x18
        je 1f
        swapgs
        1:"
    };
}

#[macro_export]
macro_rules! conditional_swapgs_paranoid {
    () => {};
}

#[macro_export]
macro_rules! conditional_swapgs_back_paranoid {
    () => {};
}

#[macro_export]
macro_rules! expand_bool {
    ($e:expr) => {
        $e
    };
}

#[macro_export]
macro_rules! saturating_sub {
    ($a:expr, $b:expr) => {
        ($a).saturating_sub($b)
    };
}

#[macro_export]
macro_rules! alternative2 {
    (feature: $feat1:expr, $then1:block, feature: $feat2:expr, $then2:block, else: $else:block) => {
        $else
    };
}

#[macro_export]
macro_rules! alternative_auto {
    ($($tokens:tt)*) => {};
}

pub fn kfx_size() -> usize {
    512 // Default FPU/SSE state size
}

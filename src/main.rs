//! # The Redox OS Kernel
//!
//! The Redox OS Kernel is a microkernel that supports multiple architectures and
//! provides Unix-like syscalls for primarily Rust applications.

#![allow(clippy::if_same_then_else)]
#![allow(clippy::many_single_char_names)]
#![allow(clippy::module_inception)]
#![allow(clippy::new_without_default)]
#![allow(clippy::or_fun_call)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::identity_op)]
// Strict safety enforcement
#![deny(clippy::not_unsafe_ptr_arg_deref)]
#![deny(clippy::cast_ptr_alignment)]
#![deny(clippy::indexing_slicing)]
#![deny(clippy::arithmetic_side_effects)]
#![deny(clippy::unwrap_used)]
#![deny(static_mut_refs)]
#![deny(unreachable_patterns)]
#![deny(unused_must_use)]
#![feature(core_intrinsics)]
#![feature(if_let_guard)]
#![feature(int_roundings)]
#![feature(iter_next_chunk)]
#![feature(sync_unsafe_cell)]
#![feature(variant_count)]
#![feature(naked_functions)]
#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

#[macro_use]
extern crate alloc;

#[macro_use]
extern crate bitflags;

use crate::{consts::*, context::switch::SwitchResult, scheme::SchemeNamespace};
use core::sync::atomic::{AtomicU32, Ordering};

#[macro_use]
mod common;
#[macro_use]
mod macros;
#[macro_use]
mod arch;
use crate::arch::*;

#[cfg(feature = "acpi")]
mod acpi;
mod allocator;
mod alternative;
mod context;
mod cpu_set;
mod cpu_stats;
#[cfg(feature = "debugger")]
mod debugger;
mod devices;
#[cfg(feature = "dtb")]
mod dtb;
mod event;
#[cfg(not(test))]
mod externs;
mod gdt;
mod log;
mod memory;
mod misc;
mod panic;
mod percpu;
mod profiling;
mod ptrace;
mod scheme;
mod startup;
#[macro_use]
mod stubs;
use sync::CleanLockToken;
mod sync;
mod syscall;
mod time;
mod topology;

#[cfg_attr(not(test), global_allocator)]
static ALLOCATOR: allocator::Allocator = allocator::Allocator;

#[inline(always)]
fn cpu_id() -> crate::cpu_set::LogicalCpuId {
    crate::percpu::PercpuBlock::current().cpu_id
}

static CPU_COUNT: AtomicU32 = AtomicU32::new(1);

#[inline(always)]
fn cpu_count() -> u32 {
    CPU_COUNT.load(Ordering::Relaxed)
}

fn init_env() -> &'static [u8] {
    crate::BOOTSTRAP.get().expect("BOOTSTRAP was not set").env
}

extern "C" fn userspace_init() {
    let mut token = unsafe { CleanLockToken::new() };
    let bootstrap = crate::BOOTSTRAP.get().expect("BOOTSTRAP was not set");
    unsafe { crate::syscall::process::usermode_bootstrap(bootstrap, &mut token) }
}

struct Bootstrap {
    base: crate::memory::Frame,
    page_count: usize,
    env: &'static [u8],
}

static BOOTSTRAP: spin::Once<Bootstrap> = spin::Once::new();

extern "C" fn kmain_reaper() {
    loop {
        context::reap::reap_grants();
        core::hint::spin_loop();
    }
}

fn kmain(bootstrap: Bootstrap) -> ! {
    let mut token = unsafe { CleanLockToken::new() };
    context::init();
    scheme::init_schemes();

    info!("BSP: {} CPUs", cpu_count());
    debug!("Env: {:?}", ::core::str::from_utf8(bootstrap.env));

    BOOTSTRAP.call_once(|| bootstrap);
    profiling::ready_for_profiling();

    let owner = None;
    match context::spawn(false, owner.clone(), || kmain_reaper(), &mut token) {
        Ok(context_lock) => {
            let mut context = context_lock.write(token.token());
            context.status = context::Status::Runnable;
            context.name.clear();
            context.name.push_str("[kmain_reaper]");
        }
        Err(err) => {
            panic!("failed to spawn kmain_reaper: {:?}", err);
        }
    }
    match context::spawn(true, owner, || userspace_init(), &mut token) {
        Ok(context_lock) => {
            let mut context = context_lock.write(token.token());
            context.status = context::Status::Runnable;
            context.name.clear();
            context.name.push_str("[bootstrap]");
        }
        Err(err) => {
            panic!("failed to spawn userspace_init: {:?}", err);
        }
    }

    run_userspace(&mut token)
}

#[allow(unreachable_code, unused_variables, dead_code)]
fn kmain_ap(cpu_id: crate::cpu_set::LogicalCpuId) -> ! {
    let mut token = unsafe { CleanLockToken::new() };

    #[cfg(feature = "profiling")]
    profiling::maybe_run_profiling_helper_forever(cpu_id);

    if !cfg!(feature = "multi_core") {
        info!("AP {}: Disabled", cpu_id);
        loop {
            unsafe {
                interrupt::disable();
                interrupt::halt();
            }
        }
    }

    context::init();
    info!("AP {}", cpu_id);
    profiling::ready_for_profiling();
    run_userspace(&mut token)
}

fn run_userspace(token: &mut CleanLockToken) -> ! {
    loop {
        unsafe {
            interrupt::disable();
            match context::switch(token) {
                SwitchResult::Switched => {
                    interrupt::enable_and_nop();
                }
                SwitchResult::AllContextsIdle => {
                    interrupt::enable_and_halt();
                }
            }
        }
    }
}

macro_rules! linker_offsets(
    ($($name:ident),*) => {
        $(
        #[inline]
        pub fn $name() -> usize {
            unsafe extern "C" {
                static $name: u8;
            }
            (&raw const $name) as usize
        }
        )*
    }
);

mod kernel_executable_offsets {
    linker_offsets!(
        __text_start,
        __text_end,
        __rodata_start,
        __rodata_end,
        __usercopy_start,
        __usercopy_end
    );

    #[cfg(target_arch = "x86_64")]
    linker_offsets!(__altrelocs_start, __altrelocs_end);
}

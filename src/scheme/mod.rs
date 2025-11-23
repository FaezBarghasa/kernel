//! Kernel Schemes
//!
//! Schemes are the primary abstraction in Redox (like "everything is a file" in Unix).
//! This module manages the registry of built-in kernel schemes.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::sync::atomic::AtomicUsize;
use spin::RwLock;

use crate::syscall::error::{Error, Result, ENODEV};

pub mod acpi;
pub mod debug;
pub mod dtb;
pub mod event;
pub mod irq;
pub mod memory;
pub mod pipe;
pub mod proc;
pub mod ring; // Added: Ring Buffer IPC Scheme
pub mod root;
pub mod serio;
pub mod sys;
pub mod time;
pub mod user;

pub use self::ring::RingScheme; // Export the new scheme

/// Kernel scheme trait
pub trait KernelScheme: Send + Sync {
    fn kopen(&self, path: &str, flags: usize, ctx: CallerCtx) -> Result<OpenResult>;
    
    fn kclose(&self, id: usize) -> Result<usize> {
        Err(Error::new(ENODEV))
    }

    fn kread(&self, id: usize, buf: &mut [u8]) -> Result<usize> {
        Err(Error::new(ENODEV))
    }

    fn kwrite(&self, id: usize, buf: &[u8]) -> Result<usize> {
        Err(Error::new(ENODEV))
    }
    
    fn kfmap(&self, id: usize, addr: usize, len: usize, flags: crate::syscall::flag::MapFlags, arg: usize) -> Result<usize> {
        Err(Error::new(ENODEV))
    }

    fn kfstat(&self, id: usize, buf: &mut crate::syscall::data::Stat) -> Result<usize> {
        Err(Error::new(ENODEV))
    }
    
    fn kfpath(&self, id: usize, buf: &mut [u8]) -> Result<usize> {
        Err(Error::new(ENODEV))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CallerCtx {
    pub uid: u32,
    pub gid: u32,
    pub pid: usize,
}

pub enum OpenResult {
    SchemeLocal(usize),
    External(usize),
}

/// Global scheme registry
pub static SCHEMES: RwLock<BTreeMap<Box<str>, Arc<dyn KernelScheme>>> = RwLock::new(BTreeMap::new());

pub fn init_schemes() {
    let mut schemes = SCHEMES.write();
    
    schemes.insert(Box::from("debug"), Arc::new(debug::DebugScheme::new()));
    schemes.insert(Box::from("event"), Arc::new(event::EventScheme::new()));
    schemes.insert(Box::from("memory"), Arc::new(memory::MemoryScheme::new()));
    schemes.insert(Box::from("pipe"), Arc::new(pipe::PipeScheme::new()));
    schemes.insert(Box::from("ring"), Arc::new(RingScheme::new())); // Initialize RingScheme
    schemes.insert(Box::from("serio"), Arc::new(serio::SerioScheme::new()));
    schemes.insert(Box::from("irq"), Arc::new(irq::IrqScheme::new()));
    schemes.insert(Box::from("time"), Arc::new(time::TimeScheme::new()));
}
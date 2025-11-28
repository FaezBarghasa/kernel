//! # Virtual Memory Management for Contexts

use crate::memory::{self, Enomem, Frame, RaiiFrame};
use crate::paging::{Page, PageFlags};
use crate::sync::CleanLockToken;

#[derive(Debug)]
pub enum PfError {
    Oom,
    Segv,
    RecursionLimitExceeded,
    NonfatalInternalError,
}

impl From<Enomem> for PfError {
    fn from(_: Enomem) -> Self {
        Self::Oom
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum AccessMode {
    Read,
    Write,
    InstrFetch,
}

#[derive(Debug)]
pub struct Grant {
    start: Page,
    end: Page,
    flags: PageFlags,
    phys: Option<RaiiFrame>,
}

impl Grant {
    pub fn new(start: Page, end: Page, flags: PageFlags) -> Self {
        Self {
            start,
            end,
            flags,
            phys: None,
        }
    }

    pub fn phys(&self) -> Option<Frame> {
        self.phys.as_ref().map(|f| f.get())
    }

    pub fn set_phys(&mut self, frame: Frame) {
        let raii = unsafe { RaiiFrame::new_unchecked(frame) };
        self.phys = Some(raii);
    }

    pub fn unmap(mut self) {
        drop(self.phys.take());
    }
}

pub fn try_correcting_page_tables(
    _faulting_page: Page,
    _access: AccessMode,
    _token: &mut CleanLockToken,
) -> Result<(), PfError> {
    Err(PfError::Segv)
}
use alloc::collections::VecDeque;
use core::fmt;
use spin::{Mutex, MutexGuard};

use crate::devices::graphical_debug::{DebugDisplay, DEBUG_DISPLAY};

/// The global logger.
pub static LOG: Mutex<Option<Log>> = Mutex::new(None);

/// Initializes the global logger.
pub fn init() {
    *LOG.lock() = Some(Log::new(1024 * 1024));
}

/// A circular buffer for storing log messages.
pub struct Log {
    /// The circular buffer.
    data: VecDeque<u8>,
    /// The maximum size of the buffer.
    size: usize,
}

impl Log {
    /// Creates a new `Log` with the given size.
    pub fn new(size: usize) -> Log {
        Log {
            data: VecDeque::with_capacity(size),
            size,
        }
    }

    /// Reads the log buffer as a pair of slices.
    pub fn read(&self) -> (&[u8], &[u8]) {
        self.data.as_slices()
    }

    /// Writes to the log buffer.
    pub fn write(&mut self, buf: &[u8]) {
        for &b in buf {
            while self.data.len() + 1 >= self.size {
                self.data.pop_front();
            }
            self.data.push_back(b);
        }
    }
}

/// A log writer.
///
/// This struct is used to write to the global logger, the debug display, and the architecture-specific
/// debug output.
pub struct Writer<'a> {
    /// A lock on the global logger.
    log: MutexGuard<'a, Option<Log>>,
    /// A lock on the debug display.
    display: MutexGuard<'a, Option<DebugDisplay>>,
    /// The architecture-specific debug writer.
    arch: crate::arch::debug::Writer<'a>,
}

impl<'a> Writer<'a> {
    /// Creates a new `Writer`.
    pub fn new() -> Writer<'a> {
        Writer {
            log: LOG.lock(),
            display: DEBUG_DISPLAY.lock(),
            arch: crate::arch::debug::Writer::new(),
        }
    }

    /// Writes to the log.
    pub fn write(&mut self, buf: &[u8], preserve: bool) {
        if preserve {
            if let Some(ref mut log) = *self.log {
                log.write(buf);
            }
        }

        if let Some(display) = &mut *self.display {
            display.write(buf);
        }

        self.arch.write(buf);
    }
}

impl fmt::Write for Writer<'_> {
    fn write_str(&mut self, s: &str) -> Result<(), fmt::Error> {
        self.write(s.as_bytes(), true);
        Ok(())
    }
}

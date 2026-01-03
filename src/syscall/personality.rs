//! Personality-based Syscall Redirection
//!
//! This module provides detection and redirection of foreign ABI syscalls
//! (Windows NT, Linux, Android) to userspace personality servers.
//!
//! # Architecture
//!
//! When a process with a non-native personality executes a syscall:
//! 1. `detect_abi()` examines the context to determine the ABI
//! 2. `redirect_foreign_syscall()` packages arguments and sends to personality server
//! 3. Personality server translates to native Redox calls
//! 4. Response is returned to caller

use crate::{
    context,
    ipc::{MessageHeader, ZeroCopyMessage},
    scheme::SchemeId,
    sync::CleanLockToken,
    syscall::error::{Error, Result, ENOSYS},
};

/// Supported ABI personalities
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PersonalityABI {
    /// Native Redox syscalls
    Redox = 0,
    /// Linux x86_64 syscall ABI
    Linux = 1,
    /// Windows NT syscall ABI (int 0x2e / syscall)
    Windows = 2,
    /// Android/Bionic syscall ABI
    Android = 3,
}

impl PersonalityABI {
    /// Convert from raw value
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Redox),
            1 => Some(Self::Linux),
            2 => Some(Self::Windows),
            3 => Some(Self::Android),
            _ => None,
        }
    }
}

impl Default for PersonalityABI {
    fn default() -> Self {
        Self::Redox
    }
}

/// Syscall arguments package for IPC transfer
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SyscallArgs {
    pub number: usize,
    pub arg0: usize,
    pub arg1: usize,
    pub arg2: usize,
    pub arg3: usize,
    pub arg4: usize,
    pub arg5: usize,
}

impl SyscallArgs {
    pub fn new(number: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> Self {
        Self {
            number,
            arg0: a,
            arg1: b,
            arg2: c,
            arg3: d,
            arg4: e,
            arg5: f,
        }
    }
}

/// Personality server descriptor
#[derive(Clone, Debug)]
pub struct PersonalityServer {
    /// Scheme ID for the personality server
    pub scheme_id: SchemeId,
    /// File handle for communication
    pub handle: usize,
    /// Whether the server is active
    pub active: bool,
}

/// Per-context personality state
#[derive(Clone, Debug)]
pub struct PersonalityState {
    /// The ABI personality for this context
    pub abi: PersonalityABI,
    /// Registered personality servers
    pub servers: [Option<PersonalityServer>; 4],
}

impl Default for PersonalityState {
    fn default() -> Self {
        Self {
            abi: PersonalityABI::Redox,
            servers: [None, None, None, None],
        }
    }
}

/// Linux syscall numbers that need redirection
pub mod linux_syscall {
    pub const SYS_READ: usize = 0;
    pub const SYS_WRITE: usize = 1;
    pub const SYS_OPEN: usize = 2;
    pub const SYS_CLOSE: usize = 3;
    pub const SYS_STAT: usize = 4;
    pub const SYS_FSTAT: usize = 5;
    pub const SYS_POLL: usize = 7;
    pub const SYS_MMAP: usize = 9;
    pub const SYS_MPROTECT: usize = 10;
    pub const SYS_MUNMAP: usize = 11;
    pub const SYS_BRK: usize = 12;
    pub const SYS_IOCTL: usize = 16;
    pub const SYS_CLONE: usize = 56;
    pub const SYS_FORK: usize = 57;
    pub const SYS_EXECVE: usize = 59;
    pub const SYS_EXIT: usize = 60;
    pub const SYS_WAIT4: usize = 61;
    pub const SYS_FUTEX: usize = 202;
    // ... more syscalls as needed
}

/// Windows NT syscall numbers
pub mod windows_syscall {
    pub const NT_CREATE_FILE: usize = 0x55;
    pub const NT_OPEN_FILE: usize = 0x33;
    pub const NT_READ_FILE: usize = 0x06;
    pub const NT_WRITE_FILE: usize = 0x08;
    pub const NT_CLOSE: usize = 0x0F;
    pub const NT_CREATE_PROCESS: usize = 0x4D;
    pub const NT_TERMINATE_PROCESS: usize = 0x2E;
    pub const NT_ALLOCATE_VIRTUAL_MEMORY: usize = 0x18;
    pub const NT_FREE_VIRTUAL_MEMORY: usize = 0x1E;
    // ... more syscalls as needed
}

/// Detect the ABI personality of the current context
///
/// This examines the context's registered personality and returns
/// the appropriate ABI type. Called early in syscall dispatch.
pub fn detect_abi(token: &CleanLockToken) -> PersonalityABI {
    let current = context::current();
    let guard = current.read(token.ticket());

    // Check if context has a registered personality
    // For now, default to Redox unless explicitly set
    // TODO: Check ELF header magic, PE signature, etc.

    // Placeholder: would read from context's personality state
    PersonalityABI::Redox
}

/// Check if a syscall number belongs to a foreign ABI
///
/// Linux syscall numbers > 400 are typically Redox-specific
/// Windows NT syscalls are routed through int 0x2e or different entry
pub fn is_foreign_syscall(abi: PersonalityABI, syscall_number: usize) -> bool {
    match abi {
        PersonalityABI::Redox => false,
        PersonalityABI::Linux => true, // All Linux syscalls need translation
        PersonalityABI::Windows => true,
        PersonalityABI::Android => true,
    }
}

/// Redirect a foreign syscall to the appropriate personality server
///
/// This packages the syscall arguments and sends them via IPC to the
/// personality server, which performs the translation and execution.
pub fn redirect_foreign_syscall(
    abi: PersonalityABI,
    args: SyscallArgs,
    token: &mut CleanLockToken,
) -> Result<usize> {
    match abi {
        PersonalityABI::Redox => {
            // Should not happen - native syscalls don't need redirection
            Err(Error::new(ENOSYS))
        }
        PersonalityABI::Linux => redirect_to_linux_server(args, token),
        PersonalityABI::Windows => redirect_to_windows_server(args, token),
        PersonalityABI::Android => redirect_to_android_server(args, token),
    }
}

/// Redirect to Linux compatibility server
fn redirect_to_linux_server(args: SyscallArgs, _token: &mut CleanLockToken) -> Result<usize> {
    // Package args into IPC message
    let msg = create_syscall_message(PersonalityABI::Linux, args);

    // Send to linux-compat-server scheme
    // For now, return ENOSYS until server is implemented
    // TODO: IPC send to "linux:" scheme

    Err(Error::new(ENOSYS))
}

/// Redirect to Windows compatibility server
fn redirect_to_windows_server(args: SyscallArgs, _token: &mut CleanLockToken) -> Result<usize> {
    let msg = create_syscall_message(PersonalityABI::Windows, args);

    // TODO: IPC send to "windows:" scheme
    Err(Error::new(ENOSYS))
}

/// Redirect to Android compatibility server
fn redirect_to_android_server(args: SyscallArgs, _token: &mut CleanLockToken) -> Result<usize> {
    let msg = create_syscall_message(PersonalityABI::Android, args);

    // TODO: IPC send to "android:" scheme
    Err(Error::new(ENOSYS))
}

/// Create an IPC message for syscall forwarding
fn create_syscall_message(abi: PersonalityABI, args: SyscallArgs) -> ZeroCopyMessage {
    let mut header = MessageHeader::default();
    header.msg_type = abi as u32;
    header.payload_size = core::mem::size_of::<SyscallArgs>() as u32;

    let mut msg = ZeroCopyMessage::default();
    msg.header = header;

    // Copy syscall args to inline payload
    let bytes = unsafe {
        core::slice::from_raw_parts(
            &args as *const SyscallArgs as *const u8,
            core::mem::size_of::<SyscallArgs>(),
        )
    };

    unsafe {
        msg.payload.inline[..bytes.len()].copy_from_slice(bytes);
    }

    msg
}

/// Set the personality for the current context
pub fn set_personality(abi: PersonalityABI, token: &mut CleanLockToken) -> Result<()> {
    // TODO: Store personality in context
    // This would be called by exec() when loading a foreign binary
    Ok(())
}

/// Register a personality server for handling foreign syscalls
pub fn register_personality_server(
    abi: PersonalityABI,
    scheme_id: SchemeId,
    handle: usize,
    _token: &mut CleanLockToken,
) -> Result<()> {
    // TODO: Store server registration globally
    // Personality servers register themselves on startup
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_personality_abi() {
        assert_eq!(PersonalityABI::from_u8(0), Some(PersonalityABI::Redox));
        assert_eq!(PersonalityABI::from_u8(1), Some(PersonalityABI::Linux));
        assert_eq!(PersonalityABI::from_u8(2), Some(PersonalityABI::Windows));
        assert_eq!(PersonalityABI::from_u8(3), Some(PersonalityABI::Android));
        assert_eq!(PersonalityABI::from_u8(4), None);
    }

    #[test]
    fn test_syscall_args() {
        let args = SyscallArgs::new(1, 2, 3, 4, 5, 6, 7);
        assert_eq!(args.number, 1);
        assert_eq!(args.arg0, 2);
        assert_eq!(args.arg5, 7);
    }

    #[test]
    fn test_is_foreign_syscall() {
        assert!(!is_foreign_syscall(PersonalityABI::Redox, 100));
        assert!(is_foreign_syscall(PersonalityABI::Linux, 100));
        assert!(is_foreign_syscall(PersonalityABI::Windows, 100));
    }
}

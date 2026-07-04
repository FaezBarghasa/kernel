#![forbid(unsafe_code)]

use alloc::string::String;
use alloc::vec;
use spin::Once;
use zerocopy::{FromBytes, IntoBytes, Immutable};

use crate::sched::scx_bridge::{ScxBridge, ScxError, ScxStats};
use crate::sched::scx_types::ScxPolicyInfo;
use crate::syscall::error::{Error, EEXIST, EINVAL, ESRCH, EAGAIN};
use crate::syscall::usercopy::{UserSliceRo, UserSliceWo};

/// Syscall number for SCX operations
pub const SYS_SCX_REGISTER: usize = 0x1000;
pub const SYS_SCX_UNREGISTER: usize = 0x1001;
pub const SYS_SCX_GET_STATS: usize = 0x1002;

static SCX_BRIDGE: Once<ScxBridge> = Once::new();

/// Returns the global SCX bridge instance.
pub fn get_scx_bridge() -> &'static ScxBridge {
    SCX_BRIDGE.call_once(|| ScxBridge::new(2048))
}

#[derive(FromBytes, IntoBytes, Immutable, Default)]
#[repr(C)]
struct RawPolicyInfo {
    name_ptr: u64,
    name_len: u64,
    version_ptr: u64,
    version_len: u64,
    pid: u64,
    timeout_ns: u64,
}

impl From<ScxError> for Error {
    fn from(err: ScxError) -> Self {
        match err {
            ScxError::NoPolicyRegistered => Error::new(ESRCH),
            ScxError::PolicyAlreadyRegistered => Error::new(EEXIST),
            ScxError::InvalidPolicyConfig(_) => Error::new(EINVAL),
            ScxError::QueueFull | ScxError::ResponseQueueFull => Error::new(EAGAIN),
        }
    }
}

/// Handles SCX-related syscalls
pub fn handle_scx_syscall(
    syscall_num: usize,
    args: &[usize],
    bridge: &ScxBridge,
) -> Result<usize, ScxError> {
    match syscall_num {
        SYS_SCX_REGISTER => {
            if args.is_empty() {
                return Err(ScxError::InvalidPolicyConfig("Missing arguments".into()));
            }

            let raw_info_slice = UserSliceRo::new(args[0], core::mem::size_of::<RawPolicyInfo>())
                .map_err(|_| ScxError::InvalidPolicyConfig("Invalid pointer to policy info".into()))?;

            let mut raw_info = RawPolicyInfo::default();
            raw_info_slice.copy_to_slice(raw_info.as_mut_bytes())
                .map_err(|_| ScxError::InvalidPolicyConfig("Failed to copy policy info struct".into()))?;

            let name_len = raw_info.name_len as usize;
            let name_slice = UserSliceRo::new(raw_info.name_ptr as usize, name_len)
                .map_err(|_| ScxError::InvalidPolicyConfig("Invalid name pointer".into()))?;
            let mut name_buf = vec![0u8; name_len];
            name_slice.copy_to_slice(&mut name_buf)
                .map_err(|_| ScxError::InvalidPolicyConfig("Failed to copy name string".into()))?;
            let name = String::from_utf8(name_buf)
                .map_err(|_| ScxError::InvalidPolicyConfig("Name is not UTF-8".into()))?;

            let version_len = raw_info.version_len as usize;
            let version_slice = UserSliceRo::new(raw_info.version_ptr as usize, version_len)
                .map_err(|_| ScxError::InvalidPolicyConfig("Invalid version pointer".into()))?;
            let mut version_buf = vec![0u8; version_len];
            version_slice.copy_to_slice(&mut version_buf)
                .map_err(|_| ScxError::InvalidPolicyConfig("Failed to copy version string".into()))?;
            let version = String::from_utf8(version_buf)
                .map_err(|_| ScxError::InvalidPolicyConfig("Version is not UTF-8".into()))?;

            let policy = ScxPolicyInfo {
                name,
                version,
                pid: raw_info.pid,
                timeout_ns: raw_info.timeout_ns,
                is_active: true,
            };

            bridge.register_policy(policy)?;
            Ok(0)
        }

        SYS_SCX_UNREGISTER => {
            bridge.unregister_policy();
            Ok(0)
        }

        SYS_SCX_GET_STATS => {
            if args.is_empty() {
                return Err(ScxError::InvalidPolicyConfig("Missing arguments".into()));
            }

            let stats = bridge.get_stats();
            let stats_slice = UserSliceWo::new(args[0], core::mem::size_of::<ScxStats>())
                .map_err(|_| ScxError::InvalidPolicyConfig("Invalid stats buffer pointer".into()))?;

            stats_slice.copy_from_slice(stats.as_bytes())
                .map_err(|_| ScxError::InvalidPolicyConfig("Failed to copy stats to user-space".into()))?;

            Ok(core::mem::size_of::<ScxStats>())
        }

        _ => Err(ScxError::InvalidPolicyConfig("Unknown syscall".into())),
    }
}

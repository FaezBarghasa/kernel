use crate::syscall::flag::MapFlags;

#[derive(Debug, Default)]
pub struct SyscallDebugInfo {
    // Placeholder fields
}

impl SyscallDebugInfo {
    pub fn format_call(
        &self,
        _a: usize,
        _b: usize,
        _c: usize,
        _d: usize,
        _e: usize,
        _f: usize,
    ) -> alloc::string::String {
        alloc::string::String::from("syscall")
    }
}

pub fn format_call(
    _a: usize,
    _b: usize,
    _c: usize,
    _d: usize,
    _e: usize,
    _f: usize,
) -> alloc::string::String {
    alloc::string::String::from("syscall")
}

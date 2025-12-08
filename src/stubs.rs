//! Stub implementations for missing kernel functions and types

use crate::{context::memory::PageSpan, memory::Frame, paging::PhysicalAddress};

/// Topology types
pub mod topology {
    use core::fmt;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NumaNodeId(pub u8);

    impl fmt::Display for NumaNodeId {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    pub struct CpuTopology;

    impl CpuTopology {
        pub fn get(&self) -> Option<()> {
            Some(())
        }
    }

    pub static CPU_TOPOLOGY: CpuTopology = CpuTopology;
}

/// GDT Module
pub mod gdt {
    use x86::segmentation::SegmentSelector;

    pub const GDT_KERNEL_CODE: u16 = 0x08;
    pub const GDT_USER_CODE32_UNUSED: u16 = 0x18;
    pub const GDT_USER_DATA: u16 = 0x20;
    pub const GDT_USER_CODE: u16 = 0x28;

    pub struct ProcessorControlRegion;

    pub unsafe fn init_bsp() {}
    pub unsafe fn install_pcr(_cpu_id: usize, _region: &ProcessorControlRegion) {}
    pub fn pcr() -> &'static ProcessorControlRegion {
        static PCR: ProcessorControlRegion = ProcessorControlRegion;
        &PCR
    }
    pub unsafe fn set_tss_stack(_index: usize, _stack: usize) {}
    pub unsafe fn set_userspace_io_allowed(_allowed: bool, _pcr: &ProcessorControlRegion) {}
}

/// Syscall types
pub mod syscall_types {
    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    pub struct FloatRegisters {
        pub data: [u8; 512],
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    pub struct IntRegisters {
        pub rax: usize,
        pub rbx: usize,
        pub rcx: usize,
        pub rdx: usize,
        pub rsi: usize,
        pub rdi: usize,
        pub rbp: usize,
        pub rsp: usize,
        pub r8: usize,
        pub r9: usize,
        pub r10: usize,
        pub r11: usize,
        pub r12: usize,
        pub r13: usize,
        pub r14: usize,
        pub r15: usize,
        pub rip: usize,
        pub rflags: usize,
    }

    pub const F_DUPFD_CLOEXEC: i32 = 1030;

    #[macro_export]
    macro_rules! ptrace_event {
        ($event:expr, $data:expr) => {
            crate::syscall::data::PtraceEvent {
                cause: $event,
                a: $data,
                b: 0,
                c: 0,
                d: 0,
                e: 0,
                f: 0,
            }
        };
        ($event:expr) => {
            crate::syscall::data::PtraceEvent {
                cause: $event,
                a: 0,
                b: 0,
                c: 0,
                d: 0,
                e: 0,
                f: 0,
            }
        };
    }
}

/// Context functions
pub mod context_helpers {
    use crate::{
        context::{Context, ContextLock},
        sync::CleanLockToken,
    };
    use alloc::sync::Arc;

    pub fn spawn<F>(
        _user: bool,
        _owner: Option<core::num::NonZeroUsize>,
        _func: F,
        _token: &mut CleanLockToken,
    ) -> crate::syscall::error::Result<Arc<ContextLock>>
    where
        F: FnOnce() + 'static,
    {
        Err(crate::syscall::error::Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn is_current(_context: &Arc<ContextLock>) -> bool {
        false
    }

    pub fn try_current() -> Option<Arc<ContextLock>> {
        None
    }

    pub fn contexts_mut(
    ) -> spin::RwLockWriteGuard<'static, alloc::collections::BTreeMap<usize, Arc<ContextLock>>>
    {
        use alloc::collections::BTreeMap;
        static CONTEXTS: spin::RwLock<BTreeMap<usize, Arc<ContextLock>>> =
            spin::RwLock::new(BTreeMap::new());
        CONTEXTS.write()
    }

    pub fn switch_finish_hook() {}

    pub mod signal {
        use syscall::Exception;

        pub fn excp_handler(exception: Exception) {
            let kind = exception.kind;
            let code = exception.code;
            let address = exception.address;
            log::error!(
                "Exception: kind={}, code={}, address={:#x}",
                kind,
                code,
                address
            );

            // For now, panic on exceptions
            // TODO: Implement proper exception handling (signals, etc.)
            panic!("Unhandled CPU exception: {:?}", exception);
        }
    }
}

/// Syscall helpers
pub mod syscall_helpers {
    use crate::context::memory::PageSpan;
    pub fn validate_region(_ptr: usize, _len: usize) -> crate::syscall::error::Result<PageSpan> {
        // Placeholder returning a dummy PageSpan or error
        Err(crate::syscall::error::Error::new(crate::syscall::error::ENOMEM))
    }

    pub fn exit_this_context(_status: usize) -> ! {
        loop {}
    }

    pub fn copy_path_to_buf(_path: crate::syscall::usercopy::UserSliceRo, _max_len: usize) -> crate::syscall::error::Result<alloc::string::String> {
        Ok(alloc::string::String::new())
    }
}

/// Exception stack
pub mod exception {
    #[repr(C)]
    pub struct ExceptionStack {
        pub rip: u64,
        pub cs: u64,
        pub rflags: u64,
        pub rsp: u64,
        pub ss: u64,
    }

    // Exception handlers
    pub unsafe extern "C" fn divide_by_zero() {}
    pub unsafe extern "C" fn debug() {}
    pub unsafe extern "C" fn non_maskable() {}
    pub unsafe extern "C" fn breakpoint() {}
    pub unsafe extern "C" fn overflow() {}
    pub unsafe extern "C" fn bound_range() {}
    pub unsafe extern "C" fn invalid_opcode() {}
    pub unsafe extern "C" fn device_not_available() {}
    pub unsafe extern "C" fn double_fault() {}
    pub unsafe extern "C" fn invalid_tss() {}
    pub unsafe extern "C" fn segment_not_present() {}
    pub unsafe extern "C" fn stack_segment() {}
    pub unsafe extern "C" fn protection() {}
    pub unsafe extern "C" fn page() {}
    pub unsafe extern "C" fn fpu_fault() {}
    pub unsafe extern "C" fn alignment_check() {}
    pub unsafe extern "C" fn machine_check() {}
    pub unsafe extern "C" fn simd() {}
    pub unsafe extern "C" fn virtualization() {}
    pub unsafe extern "C" fn security() {}
}

/// IRQ handlers
pub mod irq_handlers {
    pub unsafe extern "C" fn pit_stack() {}
    pub unsafe extern "C" fn keyboard() {}
    pub unsafe extern "C" fn cascade() {}
    pub unsafe extern "C" fn com2() {}
    pub unsafe extern "C" fn com1() {}
    pub unsafe extern "C" fn lpt2() {}
    pub unsafe extern "C" fn floppy() {}
    pub unsafe extern "C" fn lpt1() {}
    pub unsafe extern "C" fn rtc() {}
    pub unsafe extern "C" fn pci1() {}
    pub unsafe extern "C" fn pci2() {}
    pub unsafe extern "C" fn pci3() {}
    pub unsafe extern "C" fn mouse() {}
    pub unsafe extern "C" fn fpu() {}
    pub unsafe extern "C" fn ata1() {}
    pub unsafe extern "C" fn ata2() {}
    pub unsafe extern "C" fn lapic_timer() {}
    pub unsafe extern "C" fn lapic_error() {}
}

/// IPI handlers
pub mod ipi_handlers {
    pub unsafe extern "C" fn wakeup() {}
    pub unsafe extern "C" fn switch() {}
    pub unsafe extern "C" fn tlb() {}
    pub unsafe extern "C" fn pit() {}
}

/// Memory helpers
impl Frame {
    pub fn containing_address(addr: PhysicalAddress) -> Self {
        Frame::containing(addr)
    }
}

/// TimeSpec
pub mod time_helpers {
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct TimeSpec {
        pub tv_sec: i64,
        pub tv_nsec: i64,
    }
}

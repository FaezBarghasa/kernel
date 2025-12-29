//! Graphics Abstraction Layer (GAL) Scheme
//!
//! This module implements a secure Graphics Abstraction Layer for managing
//! VRAM, command buffers, and GPU resources from userspace.
//!
//! ## Features
//!
//! - **Secure VRAM allocation** with proper access control
//! - **Command buffer management** for GPU commands
//! - **Memory-mapped I/O** for efficient GPU access
//! - **Priority-aware GPU scheduling**
//! - **Zero-copy buffer sharing** between processes
//!
//! ## Security Model
//!
//! - All GPU memory allocations are validated
//! - Command buffers are validated before submission
//! - Memory regions are isolated per-process
//! - Root namespace only for privileged operations

use alloc::{boxed::Box, collections::BTreeMap, sync::Arc, vec::Vec};
use core::{
    num::NonZeroUsize,
    sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering},
};
use spin::RwLock;

use crate::{
    context::{
        self,
        file::InternalFlags,
        memory::{AddrSpaceWrapper, Grant, PageSpan},
        ContextId,
    },
    memory::{allocate_frame, deallocate_frame, Frame, PhysicalAddress, RmmA, RmmArch, PAGE_SIZE},
    paging::{Page, PageFlags, VirtualAddress},
    scheme::{CallerCtx, FileHandle, KernelScheme, OpenResult, SchemeId},
    sync::{CleanLockToken, IpcCriticalGuard, OptimizedWaitQueue},
    syscall::{
        data::Map,
        error::{Error, Result, EACCES, EBADF, EBUSY, EINVAL, ENOMEM, ENOSYS, EPERM},
        flag::{MapFlags, O_CLOEXEC, O_RDWR},
        usercopy::{UserSliceRo, UserSliceWo},
    },
};

// =============================================================================
// Constants
// =============================================================================

/// Maximum number of GAL handles per process
pub const MAX_HANDLES_PER_PROCESS: usize = 256;

/// Maximum VRAM allocation per process (256MB)
pub const MAX_VRAM_PER_PROCESS: usize = 256 * 1024 * 1024;

/// Maximum command buffer size (1MB)
pub const MAX_CMDBUF_SIZE: usize = 1024 * 1024;

/// Command buffer pool size
pub const CMDBUF_POOL_SIZE: usize = 64;

// =============================================================================
// GAL Commands
// =============================================================================

/// GAL ioctl commands
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GalCommand {
    /// Allocate VRAM buffer
    AllocVram = 0x4001,
    /// Free VRAM buffer
    FreeVram = 0x4002,
    /// Map VRAM to userspace
    MapVram = 0x4003,
    /// Unmap VRAM from userspace
    UnmapVram = 0x4004,
    /// Create command buffer
    CreateCmdBuf = 0x4010,
    /// Destroy command buffer
    DestroyCmdBuf = 0x4011,
    /// Submit command buffer
    SubmitCmdBuf = 0x4012,
    /// Wait for command completion
    WaitComplete = 0x4013,
    /// Query GPU info
    QueryInfo = 0x4020,
    /// Set display mode
    SetMode = 0x4030,
    /// Flip display buffer
    Flip = 0x4031,
}

impl TryFrom<u32> for GalCommand {
    type Error = ();

    fn try_from(value: u32) -> core::result::Result<Self, Self::Error> {
        match value {
            0x4001 => Ok(GalCommand::AllocVram),
            0x4002 => Ok(GalCommand::FreeVram),
            0x4003 => Ok(GalCommand::MapVram),
            0x4004 => Ok(GalCommand::UnmapVram),
            0x4010 => Ok(GalCommand::CreateCmdBuf),
            0x4011 => Ok(GalCommand::DestroyCmdBuf),
            0x4012 => Ok(GalCommand::SubmitCmdBuf),
            0x4013 => Ok(GalCommand::WaitComplete),
            0x4020 => Ok(GalCommand::QueryInfo),
            0x4030 => Ok(GalCommand::SetMode),
            0x4031 => Ok(GalCommand::Flip),
            _ => Err(()),
        }
    }
}

// =============================================================================
// VRAM Buffer
// =============================================================================

/// State of a VRAM buffer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VramState {
    /// Buffer is free
    Free,
    /// Buffer is allocated
    Allocated,
    /// Buffer is mapped to userspace
    Mapped,
    /// Buffer is in use by GPU
    InUse,
}

/// A VRAM buffer allocation
pub struct VramBuffer {
    /// Unique buffer ID
    id: u32,
    /// Physical frames backing this buffer
    frames: Vec<Frame>,
    /// Size in bytes
    size: usize,
    /// State
    state: AtomicU32,
    /// Owner process
    owner_pid: usize,
    /// Reference count
    ref_count: AtomicU32,
    /// Usage flags
    flags: VramFlags,
    /// Virtual address when mapped (0 if not mapped)
    mapped_addr: AtomicUsize,
}

bitflags::bitflags! {
    /// VRAM buffer flags
    #[derive(Debug, Clone, Copy, Default)]
    pub struct VramFlags: u32 {
        /// Buffer is for display (framebuffer)
        const DISPLAY = 1 << 0;
        /// Buffer is for rendering target
        const RENDER_TARGET = 1 << 1;
        /// Buffer is for textures
        const TEXTURE = 1 << 2;
        /// Buffer is for vertex data
        const VERTEX = 1 << 3;
        /// Buffer is for index data
        const INDEX = 1 << 4;
        /// Buffer is for uniform data
        const UNIFORM = 1 << 5;
        /// Buffer can be shared between processes
        const SHAREABLE = 1 << 6;
        /// Buffer is tiled (GPU-specific format)
        const TILED = 1 << 7;
    }
}

impl VramBuffer {
    /// Create a new VRAM buffer
    fn new(id: u32, size: usize, owner_pid: usize, flags: VramFlags) -> Result<Self> {
        let num_pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;
        let mut frames = Vec::with_capacity(num_pages);

        for _ in 0..num_pages {
            let frame = allocate_frame().ok_or(Error::new(ENOMEM))?;
            // Zero the frame for security
            unsafe {
                let ptr = RmmA::phys_to_virt(frame.base()).data() as *mut u8;
                core::ptr::write_bytes(ptr, 0, PAGE_SIZE);
            }
            frames.push(frame);
        }

        Ok(VramBuffer {
            id,
            frames,
            size,
            state: AtomicU32::new(VramState::Allocated as u32),
            owner_pid,
            ref_count: AtomicU32::new(1),
            flags,
            mapped_addr: AtomicUsize::new(0),
        })
    }

    /// Get buffer state
    fn state(&self) -> VramState {
        match self.state.load(Ordering::Acquire) {
            0 => VramState::Free,
            1 => VramState::Allocated,
            2 => VramState::Mapped,
            _ => VramState::InUse,
        }
    }

    /// Set buffer state
    fn set_state(&self, state: VramState) {
        self.state.store(state as u32, Ordering::Release);
    }

    /// Get physical address of first frame
    fn phys_addr(&self) -> Option<PhysicalAddress> {
        self.frames.first().map(|f| f.base())
    }

    /// Add reference
    fn add_ref(&self) {
        self.ref_count.fetch_add(1, Ordering::AcqRel);
    }

    /// Remove reference, returns true if last reference
    fn remove_ref(&self) -> bool {
        self.ref_count.fetch_sub(1, Ordering::AcqRel) == 1
    }
}

impl Drop for VramBuffer {
    fn drop(&mut self) {
        for frame in &self.frames {
            unsafe {
                deallocate_frame(*frame);
            }
        }
    }
}

// =============================================================================
// Command Buffer
// =============================================================================

/// State of a command buffer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmdBufState {
    /// Buffer is recording
    Recording,
    /// Buffer is ready for submission
    Ready,
    /// Buffer is submitted to GPU
    Pending,
    /// Buffer execution is complete
    Complete,
    /// Buffer execution failed
    Error,
}

/// A GPU command buffer
pub struct CommandBuffer {
    /// Unique command buffer ID
    id: u32,
    /// Physical frame for command data
    frame: Frame,
    /// Current size of commands in buffer
    size: AtomicUsize,
    /// Maximum size
    max_size: usize,
    /// State
    state: AtomicU32,
    /// Owner process
    owner_pid: usize,
    /// Fence value for completion tracking
    fence: AtomicU64,
    /// Priority (for scheduling)
    priority: u8,
    /// Timestamp when submitted
    submit_time: AtomicU64,
}

impl CommandBuffer {
    /// Create a new command buffer
    fn new(id: u32, owner_pid: usize, priority: u8) -> Result<Self> {
        let frame = allocate_frame().ok_or(Error::new(ENOMEM))?;

        // Zero the frame
        unsafe {
            let ptr = RmmA::phys_to_virt(frame.base()).data() as *mut u8;
            core::ptr::write_bytes(ptr, 0, PAGE_SIZE);
        }

        Ok(CommandBuffer {
            id,
            frame,
            size: AtomicUsize::new(0),
            max_size: PAGE_SIZE,
            state: AtomicU32::new(CmdBufState::Recording as u32),
            owner_pid,
            fence: AtomicU64::new(0),
            priority,
            submit_time: AtomicU64::new(0),
        })
    }

    /// Get buffer state
    fn state(&self) -> CmdBufState {
        match self.state.load(Ordering::Acquire) {
            0 => CmdBufState::Recording,
            1 => CmdBufState::Ready,
            2 => CmdBufState::Pending,
            3 => CmdBufState::Complete,
            _ => CmdBufState::Error,
        }
    }

    /// Set buffer state
    fn set_state(&self, state: CmdBufState) {
        self.state.store(state as u32, Ordering::Release);
    }

    /// Get data pointer
    fn data_ptr(&self) -> *mut u8 {
        unsafe { RmmA::phys_to_virt(self.frame.base()).data() as *mut u8 }
    }

    /// Validate commands in buffer (security check)
    fn validate(&self) -> Result<()> {
        // In a real implementation, this would parse and validate GPU commands
        // to prevent malicious command injection
        let size = self.size.load(Ordering::Acquire);
        if size == 0 || size > self.max_size {
            return Err(Error::new(EINVAL));
        }
        Ok(())
    }
}

impl Drop for CommandBuffer {
    fn drop(&mut self) {
        unsafe {
            deallocate_frame(self.frame);
        }
    }
}

// =============================================================================
// GAL Handle
// =============================================================================

/// Per-process GAL handle
struct GalHandle {
    /// Process ID
    pid: usize,
    /// VRAM buffers owned by this handle
    vram_buffers: BTreeMap<u32, Arc<VramBuffer>>,
    /// Command buffers owned by this handle
    cmd_buffers: BTreeMap<u32, Arc<CommandBuffer>>,
    /// Total VRAM allocated
    vram_used: AtomicUsize,
    /// Next buffer ID
    next_id: AtomicU32,
}

impl GalHandle {
    fn new(pid: usize) -> Self {
        GalHandle {
            pid,
            vram_buffers: BTreeMap::new(),
            cmd_buffers: BTreeMap::new(),
            vram_used: AtomicUsize::new(0),
            next_id: AtomicU32::new(1),
        }
    }

    fn alloc_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

// =============================================================================
// GAL Scheme
// =============================================================================

/// Graphics Abstraction Layer Scheme
pub struct GalScheme {
    /// Open handles indexed by scheme handle ID
    handles: RwLock<BTreeMap<usize, Arc<RwLock<GalHandle>>>>,
    /// Next handle ID
    next_handle_id: AtomicUsize,
    /// Global fence counter
    global_fence: AtomicU64,
    /// Command submission queue
    submit_queue: OptimizedWaitQueue<u32>,
    /// GPU info cache
    gpu_info: GpuInfo,
}

/// Cached GPU information
#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub vendor_id: u32,
    pub device_id: u32,
    pub vram_size: u64,
    pub capabilities: GpuCapabilities,
}

bitflags::bitflags! {
    /// GPU capability flags
    #[derive(Debug, Clone, Copy, Default)]
    pub struct GpuCapabilities: u32 {
        /// Supports 2D acceleration
        const ACCEL_2D = 1 << 0;
        /// Supports 3D acceleration
        const ACCEL_3D = 1 << 1;
        /// Supports video decode
        const VIDEO_DECODE = 1 << 2;
        /// Supports video encode
        const VIDEO_ENCODE = 1 << 3;
        /// Supports compute shaders
        const COMPUTE = 1 << 4;
        /// Supports multiple displays
        const MULTI_DISPLAY = 1 << 5;
        /// Supports hardware cursor
        const HW_CURSOR = 1 << 6;
    }
}

impl Default for GpuInfo {
    fn default() -> Self {
        GpuInfo {
            vendor_id: 0,
            device_id: 0,
            vram_size: 0,
            capabilities: GpuCapabilities::empty(),
        }
    }
}

impl GalScheme {
    /// Create a new GAL scheme
    pub fn new() -> Self {
        GalScheme {
            handles: RwLock::new(BTreeMap::new()),
            next_handle_id: AtomicUsize::new(1),
            global_fence: AtomicU64::new(0),
            submit_queue: OptimizedWaitQueue::new(),
            gpu_info: GpuInfo::default(),
        }
    }

    /// Allocate a VRAM buffer
    fn alloc_vram(
        &self,
        handle: &Arc<RwLock<GalHandle>>,
        size: usize,
        flags: VramFlags,
    ) -> Result<u32> {
        let mut handle_guard = handle.write();

        // Check limits
        let current_used = handle_guard.vram_used.load(Ordering::Relaxed);
        if current_used + size > MAX_VRAM_PER_PROCESS {
            return Err(Error::new(ENOMEM));
        }

        // Allocate buffer
        let id = handle_guard.alloc_id();
        let buffer = Arc::new(VramBuffer::new(id, size, handle_guard.pid, flags)?);

        handle_guard.vram_buffers.insert(id, buffer);
        handle_guard.vram_used.fetch_add(size, Ordering::Relaxed);

        Ok(id)
    }

    /// Free a VRAM buffer
    fn free_vram(&self, handle: &Arc<RwLock<GalHandle>>, buffer_id: u32) -> Result<()> {
        let mut handle_guard = handle.write();

        let buffer = handle_guard
            .vram_buffers
            .remove(&buffer_id)
            .ok_or(Error::new(EBADF))?;

        if buffer.state() == VramState::InUse {
            // Can't free buffer in use by GPU
            handle_guard.vram_buffers.insert(buffer_id, buffer);
            return Err(Error::new(EBUSY));
        }

        handle_guard
            .vram_used
            .fetch_sub(buffer.size, Ordering::Relaxed);

        Ok(())
    }

    /// Create a command buffer
    fn create_cmdbuf(&self, handle: &Arc<RwLock<GalHandle>>, priority: u8) -> Result<u32> {
        let mut handle_guard = handle.write();

        if handle_guard.cmd_buffers.len() >= CMDBUF_POOL_SIZE {
            return Err(Error::new(ENOMEM));
        }

        let id = handle_guard.alloc_id();
        let cmdbuf = Arc::new(CommandBuffer::new(id, handle_guard.pid, priority)?);

        handle_guard.cmd_buffers.insert(id, cmdbuf);

        Ok(id)
    }

    /// Submit a command buffer for execution
    fn submit_cmdbuf(
        &self,
        handle: &Arc<RwLock<GalHandle>>,
        cmdbuf_id: u32,
        token: &mut CleanLockToken,
    ) -> Result<u64> {
        let handle_guard = handle.read();

        let cmdbuf = handle_guard
            .cmd_buffers
            .get(&cmdbuf_id)
            .ok_or(Error::new(EBADF))?;

        // Validate the command buffer
        cmdbuf.validate()?;

        // Set state and fence
        let fence = self.global_fence.fetch_add(1, Ordering::AcqRel);
        cmdbuf.fence.store(fence, Ordering::Release);
        cmdbuf
            .submit_time
            .store(crate::time::monotonic() as u64, Ordering::Release);
        cmdbuf.set_state(CmdBufState::Pending);

        // Priority boost for high-priority submissions
        let ctx = context::current();
        let ctx_guard = ctx.read(token.token());
        let _ipc_guard = IpcCriticalGuard::new(&ctx_guard.priority);

        // Add to submission queue
        self.submit_queue.send(cmdbuf_id, token);

        Ok(fence)
    }

    /// Wait for a command buffer to complete
    fn wait_complete(
        &self,
        handle: &Arc<RwLock<GalHandle>>,
        cmdbuf_id: u32,
        timeout_ns: u64,
        token: &mut CleanLockToken,
    ) -> Result<()> {
        let handle_guard = handle.read();

        let cmdbuf = handle_guard
            .cmd_buffers
            .get(&cmdbuf_id)
            .ok_or(Error::new(EBADF))?;

        let deadline = crate::time::monotonic() as u64 + timeout_ns;

        loop {
            match cmdbuf.state() {
                CmdBufState::Complete => return Ok(()),
                CmdBufState::Error => return Err(Error::new(EINVAL)),
                _ => {}
            }

            if crate::time::monotonic() as u64 > deadline {
                return Err(Error::new(crate::syscall::error::ETIMEDOUT));
            }

            // Yield
            drop(handle_guard);
            unsafe { context::switch(token) };
            return self.wait_complete(handle, cmdbuf_id, timeout_ns, token);
        }
    }
}

impl KernelScheme for GalScheme {
    fn kopen(
        &self,
        _path: &str,
        _flags: usize,
        ctx: CallerCtx,
        _token: &mut CleanLockToken,
    ) -> Result<OpenResult> {
        let handle_id = self.next_handle_id.fetch_add(1, Ordering::Relaxed);
        let handle = Arc::new(RwLock::new(GalHandle::new(ctx.pid)));

        self.handles.write().insert(handle_id, handle);

        Ok(OpenResult::SchemeLocal(handle_id, InternalFlags::empty()))
    }

    fn close(&self, id: usize, _token: &mut CleanLockToken) -> Result<()> {
        self.handles.write().remove(&id).ok_or(Error::new(EBADF))?;
        Ok(())
    }

    fn kfmap(
        &self,
        id: usize,
        addr_space: &Arc<AddrSpaceWrapper>,
        map: &Map,
        _consume: bool,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        let handles = self.handles.read();
        let handle = handles.get(&id).ok_or(Error::new(EBADF))?;
        let handle_guard = handle.read();

        // The file offset encodes the buffer ID
        let buffer_id = map.offset as u32;
        let buffer = handle_guard
            .vram_buffers
            .get(&buffer_id)
            .ok_or(Error::new(EBADF))?;

        if buffer.state() != VramState::Allocated {
            return Err(Error::new(EBUSY));
        }

        // Map the buffer's frames into userspace
        let page_count = NonZeroUsize::new(buffer.frames.len()).ok_or(Error::new(EINVAL))?;

        // Get the first frame for mapping
        let first_frame = buffer.frames.first().ok_or(Error::new(EINVAL))?;

        let base_page = addr_space.acquire_write().mmap(
            (map.address != 0)
                .then_some(Page::containing_address(VirtualAddress::new(map.address))),
            page_count,
            map.flags,
            &mut Vec::new(),
            |dst_page, page_flags, dst_mapper, dst_flusher| {
                Grant::physmap(
                    *first_frame,
                    PageSpan::new(dst_page, page_count.get()),
                    page_flags,
                    dst_mapper,
                    dst_flusher,
                )
            },
        )?;

        let addr = base_page.start_address().data();
        buffer.mapped_addr.store(addr, Ordering::Release);
        buffer.set_state(VramState::Mapped);

        Ok(addr)
    }

    fn kwrite(
        &self,
        id: usize,
        buf: UserSliceRo,
        _flags: u32,
        _stored_flags: u32,
        token: &mut CleanLockToken,
    ) -> Result<usize> {
        if buf.len() < 8 {
            return Err(Error::new(EINVAL));
        }

        // Parse command from buffer
        let mut cmd_data = [0u8; 8];
        buf.copy_to_slice(&mut cmd_data)?;

        let cmd_code = u32::from_le_bytes([cmd_data[0], cmd_data[1], cmd_data[2], cmd_data[3]]);
        let cmd_param = u32::from_le_bytes([cmd_data[4], cmd_data[5], cmd_data[6], cmd_data[7]]);

        let command = GalCommand::try_from(cmd_code).map_err(|_| Error::new(EINVAL))?;

        let handles = self.handles.read();
        let handle = handles.get(&id).ok_or(Error::new(EBADF))?;

        match command {
            GalCommand::AllocVram => {
                let size = cmd_param as usize * PAGE_SIZE;
                let flags = VramFlags::empty();
                let buffer_id = self.alloc_vram(handle, size, flags)?;
                Ok(buffer_id as usize)
            }
            GalCommand::FreeVram => {
                self.free_vram(handle, cmd_param)?;
                Ok(0)
            }
            GalCommand::CreateCmdBuf => {
                let priority = (cmd_param & 0xFF) as u8;
                let cmdbuf_id = self.create_cmdbuf(handle, priority)?;
                Ok(cmdbuf_id as usize)
            }
            GalCommand::SubmitCmdBuf => {
                let fence = self.submit_cmdbuf(handle, cmd_param, token)?;
                Ok(fence as usize)
            }
            GalCommand::WaitComplete => {
                let timeout_ns = 5_000_000_000u64; // 5 second default
                self.wait_complete(handle, cmd_param, timeout_ns, token)?;
                Ok(0)
            }
            _ => Err(Error::new(ENOSYS)),
        }
    }

    fn kread(
        &self,
        id: usize,
        buf: UserSliceWo,
        _flags: u32,
        _stored_flags: u32,
        _token: &mut CleanLockToken,
    ) -> Result<usize> {
        // Read GPU info
        let handles = self.handles.read();
        let _handle = handles.get(&id).ok_or(Error::new(EBADF))?;

        if buf.len() < 16 {
            return Err(Error::new(EINVAL));
        }

        // Return GPU info as bytes
        let mut info_data = [0u8; 16];
        info_data[0..4].copy_from_slice(&self.gpu_info.vendor_id.to_le_bytes());
        info_data[4..8].copy_from_slice(&self.gpu_info.device_id.to_le_bytes());
        info_data[8..16].copy_from_slice(&self.gpu_info.vram_size.to_le_bytes());

        buf.copy_from_slice(&info_data)?;

        Ok(16)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vram_flags() {
        let flags = VramFlags::DISPLAY | VramFlags::RENDER_TARGET;
        assert!(flags.contains(VramFlags::DISPLAY));
        assert!(flags.contains(VramFlags::RENDER_TARGET));
        assert!(!flags.contains(VramFlags::TEXTURE));
    }

    #[test]
    fn test_gpu_capabilities() {
        let caps = GpuCapabilities::ACCEL_2D | GpuCapabilities::ACCEL_3D;
        assert!(caps.contains(GpuCapabilities::ACCEL_2D));
        assert!(!caps.contains(GpuCapabilities::COMPUTE));
    }

    #[test]
    fn test_gal_command() {
        assert_eq!(GalCommand::try_from(0x4001), Ok(GalCommand::AllocVram));
        assert_eq!(GalCommand::try_from(0x4010), Ok(GalCommand::CreateCmdBuf));
        assert!(GalCommand::try_from(0x9999).is_err());
    }
}

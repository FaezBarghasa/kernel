//! ARMv7 interrupt handling

pub mod irq;
pub mod syscall;

pub use irq::handle as irq_handle;
pub use syscall::handle as syscall_handle;

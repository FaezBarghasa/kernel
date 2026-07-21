use crate::ipi::{ipi, IpiKind, IpiTarget};

use super::RmmA;

pub use rmm::{Flusher, PageFlush, PageFlushAll};

pub struct InactiveFlusher {
    _inner: (),
}
impl Flusher<RmmA> for InactiveFlusher {
    fn consume(&mut self, flush: PageFlush<RmmA>) {
        unsafe {
            flush.ignore();
        }
    }
}
impl Drop for InactiveFlusher {
    fn drop(&mut self) {
        ipi(IpiKind::Tlb, IpiTarget::Other);
    }
}

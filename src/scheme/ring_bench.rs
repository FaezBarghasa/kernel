
use crate::scheme::{RingScheme, KernelScheme, CallerCtx};
use crate::sync::CleanLockToken;
use crate::syscall::flag::{O_RDWR, O_CREAT};
use crate::scheme::OpenResult;
use crate::syscall::usercopy::UserSliceRo;

pub fn benchmark_ring() {
    let ring_scheme = RingScheme::new();
    let mut token = unsafe { CleanLockToken::new() };
    let ctx = CallerCtx { uid: 0, gid: 0, pid: 1 };

    // Benchmark Open
    let start = crate::time::monotonic();
    let open_res = ring_scheme.kopen("ring:", O_RDWR | O_CREAT, ctx, &mut token);
    let end = crate::time::monotonic();
    
    if let Ok(OpenResult::SchemeLocal(id, _flags)) = open_res {
        crate::println!("Ring Open: {} ns", end - start);
        
        // Benchmark Write (Doorbell)
        let buf = [0u8; 8];
        if let Ok(user_buf) = UserSliceRo::new(buf.as_ptr() as usize, buf.len()) {
             let start_write = crate::time::monotonic();
             let _ = ring_scheme.kwrite(id, user_buf, 0, 0, &mut token);
             let end_write = crate::time::monotonic();
             crate::println!("Ring Write: {} ns", end_write - start_write);
        }

        // Close
        let _ = ring_scheme.close(id, &mut token);
    } else {
        crate::println!("Ring Open Failed");
    }
}

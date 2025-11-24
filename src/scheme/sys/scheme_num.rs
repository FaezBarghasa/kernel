use alloc::vec::Vec;

use crate::{context, scheme, sync::CleanLockToken, syscall::error::Result};

pub fn resource(token: &mut CleanLockToken) -> Result<Vec<u8>> {
    let scheme_ns = context::current().read(token.token()).ens;

    let mut data = Vec::new();

    let schemes = scheme::schemes(token.token());
    for (name, &scheme_id) in schemes.iter_name(scheme_ns) {
        let id_bytes = format!("{:>4}: ", scheme_id.get());
        data.extend_from_slice(id_bytes.as_bytes());
        data.extend_from_slice((*name).as_bytes());
        data.push(b'\n');
    }

    Ok(data)
}

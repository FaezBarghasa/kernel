#![forbid(unsafe_code)]

use alloc::vec::Vec;

/// ORC Entry rules record for x86_64.
#[derive(Debug, Clone, Copy)]
pub struct OrcEntry {
    pub sp_offset: i16,
    pub fp_offset: i16,
    pub sp_reg: u8,
    pub fp_reg: u8,
    pub type_: u8,
}

#[derive(Debug, Clone, Copy)]
pub enum KernelError {
    InvalidStackFrame,
}

/// Lookup the matching ORC entry for the given instruction pointer (rip).
pub fn lookup_orc(ip: usize) -> Option<OrcEntry> {
    let ip_slice = crate::arch::misc::get_orc_unwind_ip_slice();
    let unwind_slice = crate::arch::misc::get_orc_unwind_slice();
    let entry_size = 7; // Size of OrcEntry (sp_offset: 2, fp_offset: 2, sp_reg: 1, fp_reg: 1, type: 1)
    let num_entries = unwind_slice.len() / entry_size;

    if num_entries == 0 || ip_slice.len() != num_entries {
        return None;
    }

    // Binary search over instruction pointers
    let mut low = 0;
    let mut high = num_entries - 1;
    let mut found_idx = None;

    let ip_section_base = ip_slice.as_ptr() as usize;

    while low <= high {
        let mid = (low + high) / 2;
        let entry_ip_addr = ip_section_base + mid * 4;
        let entry_pc = (entry_ip_addr as isize + ip_slice[mid] as isize) as usize;

        if ip >= entry_pc {
            found_idx = Some(mid);
            low = mid + 1;
        } else {
            if mid == 0 {
                break;
            }
            high = mid - 1;
        }
    }

    if let Some(idx) = found_idx {
        let offset = idx * entry_size;
        if offset + entry_size <= unwind_slice.len() {
            let entry_bytes = &unwind_slice[offset..offset + entry_size];
            let entry = OrcEntry {
                sp_offset: i16::from_ne_bytes([entry_bytes[0], entry_bytes[1]]),
                fp_offset: i16::from_ne_bytes([entry_bytes[2], entry_bytes[3]]),
                sp_reg: entry_bytes[4],
                fp_reg: entry_bytes[5],
                type_: entry_bytes[6],
            };
            return Some(entry);
        }
    }

    None
}

/// Unwinds the stack starting from target ip, sp, and fp, verifying boundaries.
pub fn unwind_stack(
    mut ip: usize,
    mut sp: usize,
    mut fp: usize,
    stack_start: usize,
    stack_end: usize,
) -> Result<Vec<usize>, KernelError> {
    let mut trace = Vec::new();
    trace.push(ip);

    while trace.len() < 64 {
        if ip == 0 {
            break;
        }

        // Boundary check
        if sp < stack_start || sp >= stack_end {
            return Err(KernelError::InvalidStackFrame);
        }

        let entry = match lookup_orc(ip) {
            Some(e) => e,
            None => break,
        };

        // Compute next SP and FP based on ORC rules
        // sp_reg: 1 = SP, 2 = BP, etc.
        let next_sp = match entry.sp_reg {
            1 => (sp as isize + entry.sp_offset as isize) as usize,
            2 => (fp as isize + entry.sp_offset as isize) as usize,
            _ => return Err(KernelError::InvalidStackFrame),
        };

        let next_fp = match entry.fp_reg {
            1 => (sp as isize + entry.fp_offset as isize) as usize,
            2 => (fp as isize + entry.fp_offset as isize) as usize,
            _ => fp,
        };

        // Return address is at next_sp - 8
        if next_sp < stack_start + 8 || next_sp > stack_end {
            return Err(KernelError::InvalidStackFrame);
        }

        let next_ip = match crate::arch::misc::read_stack_ptr(next_sp - 8) {
            Some(val) => val,
            None => return Err(KernelError::InvalidStackFrame),
        };

        ip = next_ip;
        sp = next_sp;
        fp = next_fp;

        trace.push(ip);
    }

    Ok(trace)
}

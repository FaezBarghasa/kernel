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

#[cfg(test)]
mod tests {
    use super::*;

    fn init_mock_data(relative_pcs: &[isize], entries: &[OrcEntry]) -> usize {
        let ip_vec = vec![0i32; relative_pcs.len()];
        let leaked_ip: &'static mut [i32] = Box::leak(ip_vec.into_boxed_slice());
        
        let base_addr = leaked_ip.as_ptr() as usize;
        for i in 0..relative_pcs.len() {
            leaked_ip[i] = relative_pcs[i] as i32;
        }
        
        let mut unwind_bytes = Vec::new();
        for entry in entries {
            unwind_bytes.extend_from_slice(&entry.sp_offset.to_ne_bytes());
            unwind_bytes.extend_from_slice(&entry.fp_offset.to_ne_bytes());
            unwind_bytes.push(entry.sp_reg);
            unwind_bytes.push(entry.fp_reg);
            unwind_bytes.push(entry.type_);
        }
        let leaked_unwind: &'static [u8] = Box::leak(unwind_bytes.into_boxed_slice());
        
        crate::arch::misc::set_mock_orc_data(leaked_unwind, leaked_ip);
        base_addr
    }

    #[test]
    fn test_lookup_orc_empty() {
        crate::arch::misc::set_mock_orc_data(&[], &[]);
        assert!(lookup_orc(0x1000).is_none());
    }

    #[test]
    fn test_lookup_orc_basic() {
        let relative_pcs = vec![10, 20, 30];
        let entries = vec![
            OrcEntry { sp_offset: 8, fp_offset: 0, sp_reg: 1, fp_reg: 0, type_: 1 },
            OrcEntry { sp_offset: 16, fp_offset: 8, sp_reg: 1, fp_reg: 2, type_: 2 },
            OrcEntry { sp_offset: 24, fp_offset: 16, sp_reg: 2, fp_reg: 2, type_: 3 },
        ];
        let base_addr = init_mock_data(&relative_pcs, &entries);

        // entry_pc[0] = base_addr + 10
        let e1 = lookup_orc(base_addr + 10).unwrap();
        assert_eq!(e1.sp_offset, 8);
        assert_eq!(e1.type_, 1);

        // In between PCs
        let e2 = lookup_orc(base_addr + 15).unwrap();
        assert_eq!(e2.sp_offset, 8);

        // entry_pc[1] = base_addr + 4 + 20 = base_addr + 24
        let e3 = lookup_orc(base_addr + 24).unwrap();
        assert_eq!(e3.sp_offset, 16);
        assert_eq!(e3.type_, 2);

        let e4 = lookup_orc(base_addr + 30).unwrap();
        assert_eq!(e4.sp_offset, 16);

        // entry_pc[2] = base_addr + 8 + 30 = base_addr + 38
        let e5 = lookup_orc(base_addr + 38).unwrap();
        assert_eq!(e5.sp_offset, 24);

        // Below the first PC
        assert!(lookup_orc(base_addr + 5).is_none());
    }

    #[test]
    fn test_unwind_stack_success() {
        let relative_pcs = vec![10, 20];
        let entries = vec![
            OrcEntry { sp_offset: 16, fp_offset: 0, sp_reg: 1, fp_reg: 0, type_: 1 },
            OrcEntry { sp_offset: 16, fp_offset: 0, sp_reg: 1, fp_reg: 0, type_: 1 },
        ];
        let base_addr = init_mock_data(&relative_pcs, &entries);

        let pc0 = base_addr + 10;
        let pc1 = base_addr + 4 + 20;

        // Prepare a mock stack: 16 words.
        let mut mock_stack = [0usize; 16];
        
        let sp = mock_stack.as_ptr() as usize;
        let stack_start = sp;
        let stack_end = sp + 16 * 8;

        // sp + 8 is mock_stack[1], which stores return address of func1 (pc1)
        mock_stack[1] = pc1;
        // next_sp is sp + 16 (mock_stack[2]).
        // next frame's return address is at next_sp + 8 (mock_stack[3]), which stores 0 (terminate)
        mock_stack[3] = 0;

        let trace = unwind_stack(pc0, sp, 0, stack_start, stack_end).unwrap();
        assert_eq!(trace, vec![pc0, pc1, 0]);
    }

    #[test]
    fn test_unwind_stack_invalid_frame() {
        let relative_pcs = vec![10];
        let entries = vec![
            OrcEntry { sp_offset: 16, fp_offset: 0, sp_reg: 1, fp_reg: 0, type_: 1 },
        ];
        let base_addr = init_mock_data(&relative_pcs, &entries);
        let pc0 = base_addr + 10;

        let mut mock_stack = [0usize; 16];
        let sp = mock_stack.as_ptr() as usize;
        
        // Pass stack boundaries that make SP out of bounds immediately
        let res = unwind_stack(pc0, sp, 0, sp + 8, sp + 64);
        assert!(res.is_err());
    }
}

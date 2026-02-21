//! Unit tests for the vmm: scheme.
//!
//! Tests cover:
//!   - VcpuState memory layout (offsets match svm.rs/vmx.rs asm)
//!   - VmInstance memory region management (set_memory overlap detection)
//!   - VmInstance vCPU lifecycle (create, get/set regs)
//!   - VmExitReason From<u32> conversion

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheme::vmm::{MemoryRegion, VcpuState, VmExitReason, VmInstance};
    use core::mem::offset_of;

    // ── VcpuState layout ─────────────────────────────────────────────────────

    #[test]
    fn vcpu_state_rax_at_0x00() {
        assert_eq!(offset_of!(VcpuState, rax), 0x00);
    }

    #[test]
    fn vcpu_state_rbx_at_0x08() {
        assert_eq!(offset_of!(VcpuState, rbx), 0x08);
    }

    #[test]
    fn vcpu_state_rcx_at_0x10() {
        assert_eq!(offset_of!(VcpuState, rcx), 0x10);
    }

    #[test]
    fn vcpu_state_rdx_at_0x18() {
        assert_eq!(offset_of!(VcpuState, rdx), 0x18);
    }

    #[test]
    fn vcpu_state_rsi_at_0x20() {
        assert_eq!(offset_of!(VcpuState, rsi), 0x20);
    }

    #[test]
    fn vcpu_state_rdi_at_0x28() {
        assert_eq!(offset_of!(VcpuState, rdi), 0x28);
    }

    #[test]
    fn vcpu_state_rsp_at_0x30() {
        assert_eq!(offset_of!(VcpuState, rsp), 0x30);
    }

    #[test]
    fn vcpu_state_rbp_at_0x38() {
        assert_eq!(offset_of!(VcpuState, rbp), 0x38);
    }

    #[test]
    fn vcpu_state_r8_at_0x40() {
        assert_eq!(offset_of!(VcpuState, r8), 0x40);
    }

    #[test]
    fn vcpu_state_r9_at_0x48() {
        assert_eq!(offset_of!(VcpuState, r9), 0x48);
    }

    #[test]
    fn vcpu_state_r10_at_0x50() {
        assert_eq!(offset_of!(VcpuState, r10), 0x50);
    }

    #[test]
    fn vcpu_state_r11_at_0x58() {
        assert_eq!(offset_of!(VcpuState, r11), 0x58);
    }

    #[test]
    fn vcpu_state_r12_at_0x60() {
        assert_eq!(offset_of!(VcpuState, r12), 0x60);
    }

    #[test]
    fn vcpu_state_r13_at_0x68() {
        assert_eq!(offset_of!(VcpuState, r13), 0x68);
    }

    #[test]
    fn vcpu_state_r14_at_0x70() {
        assert_eq!(offset_of!(VcpuState, r14), 0x70);
    }

    #[test]
    fn vcpu_state_r15_at_0x78() {
        assert_eq!(offset_of!(VcpuState, r15), 0x78);
    }

    #[test]
    fn vcpu_state_rip_at_0x80() {
        assert_eq!(offset_of!(VcpuState, rip), 0x80);
    }

    #[test]
    fn vcpu_state_rflags_at_0x88() {
        assert_eq!(offset_of!(VcpuState, rflags), 0x88);
    }

    // ── VcpuState::new ────────────────────────────────────────────────────────

    #[test]
    fn vcpu_state_new_rflags_has_reserved_bit() {
        let vcpu = VcpuState::new();
        assert_eq!(vcpu.rflags & 0x2, 0x2, "RFLAGS reserved bit must be set");
    }

    #[test]
    fn vcpu_state_new_all_gpr_zero() {
        let vcpu = VcpuState::new();
        assert_eq!(vcpu.rax, 0);
        assert_eq!(vcpu.rbx, 0);
        assert_eq!(vcpu.rcx, 0);
        assert_eq!(vcpu.rdx, 0);
        assert_eq!(vcpu.rsi, 0);
        assert_eq!(vcpu.rdi, 0);
        assert_eq!(vcpu.rsp, 0);
        assert_eq!(vcpu.rbp, 0);
        assert_eq!(vcpu.r8, 0);
        assert_eq!(vcpu.r9, 0);
        assert_eq!(vcpu.r10, 0);
        assert_eq!(vcpu.r11, 0);
        assert_eq!(vcpu.r12, 0);
        assert_eq!(vcpu.r13, 0);
        assert_eq!(vcpu.r14, 0);
        assert_eq!(vcpu.r15, 0);
        assert_eq!(vcpu.rip, 0);
    }

    // ── VmInstance memory regions ─────────────────────────────────────────────

    fn make_region(gpa: u64, size: u64) -> MemoryRegion {
        MemoryRegion {
            guest_phys_addr: gpa,
            memory_size: size,
            userspace_addr: 0x1000_0000,
            flags: 0,
            _pad: 0,
        }
    }

    #[test]
    fn set_memory_accepts_non_overlapping_regions() {
        let mut vm = VmInstance::new(0);
        assert!(vm.set_memory(make_region(0x0000, 0x1000)).is_ok());
        assert!(vm.set_memory(make_region(0x1000, 0x1000)).is_ok());
        assert!(vm.set_memory(make_region(0x2000, 0x1000)).is_ok());
        assert_eq!(vm.memory_regions.len(), 3);
    }

    #[test]
    fn set_memory_rejects_overlapping_regions() {
        let mut vm = VmInstance::new(0);
        assert!(vm.set_memory(make_region(0x0000, 0x2000)).is_ok());
        // Overlaps: [0x1000, 0x3000) overlaps [0x0000, 0x2000).
        assert!(vm.set_memory(make_region(0x1000, 0x2000)).is_err());
    }

    #[test]
    fn set_memory_rejects_zero_size() {
        let mut vm = VmInstance::new(0);
        assert!(vm.set_memory(make_region(0x0000, 0)).is_err());
    }

    #[test]
    fn set_memory_rejects_overflow() {
        let mut vm = VmInstance::new(0);
        // u64::MAX + 1 overflows.
        assert!(vm.set_memory(make_region(u64::MAX, 1)).is_err());
    }

    // ── VmInstance vCPU lifecycle ─────────────────────────────────────────────

    #[test]
    fn create_vcpu_inserts_state() {
        let mut vm = VmInstance::new(0);
        assert!(vm.create_vcpu(0).is_ok());
        assert!(vm.vcpus.contains_key(&0));
    }

    #[test]
    fn create_vcpu_duplicate_returns_err() {
        let mut vm = VmInstance::new(0);
        assert!(vm.create_vcpu(0).is_ok());
        assert!(vm.create_vcpu(0).is_err());
    }

    #[test]
    fn get_set_vcpu_regs_roundtrip() {
        let mut vm = VmInstance::new(0);
        vm.create_vcpu(0).unwrap();

        let mut regs = VcpuState::new();
        regs.rax = 0xDEAD_BEEF;
        regs.rip = 0x1000;
        vm.set_vcpu_regs(0, regs.clone()).unwrap();

        let got = vm.get_vcpu_regs(0).unwrap();
        assert_eq!(got.rax, 0xDEAD_BEEF);
        assert_eq!(got.rip, 0x1000);
    }

    #[test]
    fn get_vcpu_regs_unknown_id_returns_err() {
        let vm = VmInstance::new(0);
        assert!(vm.get_vcpu_regs(99).is_err());
    }

    // ── VmExitReason ─────────────────────────────────────────────────────────

    #[test]
    fn vm_exit_reason_from_known_codes() {
        assert_eq!(VmExitReason::from(1), VmExitReason::ExternalInterrupt);
        assert_eq!(VmExitReason::from(30), VmExitReason::IoInstruction);
        assert_eq!(VmExitReason::from(48), VmExitReason::MmioAccess);
        assert_eq!(VmExitReason::from(12), VmExitReason::Halt);
        assert_eq!(VmExitReason::from(26), VmExitReason::Shutdown);
    }

    #[test]
    fn vm_exit_reason_from_unknown_code() {
        assert_eq!(VmExitReason::from(0xFF), VmExitReason::Unknown);
    }
}

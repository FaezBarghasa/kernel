use raw_cpuid::{CpuId, CpuIdResult, ExtendedFeatures, FeatureInfo};

/// Returns a `CpuId` instance that can be used to query CPU features.
pub fn cpuid() -> CpuId {
    // FIXME check for cpuid availability during early boot and error out if it doesn't exist.
    CpuId::with_cpuid_fn(|a, c| {
        #[cfg(target_arch = "x86")]
        let result = unsafe { core::arch::x86::__cpuid_count(a, c) };
        #[cfg(target_arch = "x86_64")]
        let result = unsafe { core::arch::x86_64::__cpuid_count(a, c) };
        CpuIdResult {
            eax: result.eax,
            ebx: result.ebx,
            ecx: result.ecx,
            edx: result.edx,
        }
    })
}

/// Returns the CPU's feature information.
#[cfg_attr(not(target_arch = "x86_64"), expect(dead_code))]
pub fn feature_info() -> FeatureInfo {
    cpuid()
        .get_feature_info()
        .expect("x86_64 requires CPUID leaf=0x01 to be present")
}

/// Returns true if the CPU has the specified extended feature.
#[cfg_attr(not(target_arch = "x86_64"), expect(dead_code))]
pub fn has_ext_feat(feat: impl FnOnce(ExtendedFeatures) -> bool) -> bool {
    cpuid().get_extended_feature_info().is_some_and(feat)
}

/// Query a specific CPUID leaf and subleaf.
pub fn cpuid_count(a: u32, c: u32) -> CpuIdResult {
    #[cfg(target_arch = "x86")]
    let result = unsafe { core::arch::x86::__cpuid_count(a, c) };
    #[cfg(target_arch = "x86_64")]
    let result = unsafe { core::arch::x86_64::__cpuid_count(a, c) };
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let result = CpuIdResult { eax: 0, ebx: 0, ecx: 0, edx: 0 };
    CpuIdResult {
        eax: result.eax,
        ebx: result.ebx,
        ecx: result.ecx,
        edx: result.edx,
    }
}

/// Query AMD Extended Cache Topology Leaf 0x8000_001d
pub fn get_amd_cache_properties(ecx: u32) -> Option<CpuIdResult> {
    let res = cpuid_count(0x8000_001d, ecx);
    if (res.eax & 0x1F) == 0 {
        None
    } else {
        Some(res)
    }
}

/// Query AMD Extended Feature Leaf 0x8000_0021
pub fn get_amd_feature_leaf_21() -> CpuIdResult {
    cpuid_count(0x8000_0021, 0)
}

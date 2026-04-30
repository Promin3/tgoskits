// Copyright 2025 The Axvisor Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use raw_cpuid::{CpuId, CpuIdReader};

/// SVM feature bits reported by CPUID leaf `0x8000_000A`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SvmFeatures {
    pub nested_paging: bool,
    pub lbr_virtualization: bool,
    pub svm_lock: bool,
    pub nrip_save: bool,
    pub tsc_rate_msr: bool,
    pub vmcb_clean_bits: bool,
    pub flush_by_asid: bool,
    pub decode_assists: bool,
    pub pause_filter: bool,
    pub pause_filter_threshold: bool,
    pub avic: bool,
    pub vmsave_vmload_virtualization: bool,
    pub virtual_gif: bool,
    pub guest_mode_execute_trap: bool,
    pub supervisor_shadow_stack: bool,
    pub spec_ctrl_virtualization: bool,
    pub host_mce_override: bool,
    pub tlb_control: bool,
}

/// Consolidated SVM capability snapshot for the current CPU.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SvmCapabilities {
    pub supported: bool,
    pub revision: u8,
    pub asid_count: u32,
    pub features: SvmFeatures,
}

/// Returns whether the current CPU reports AMD SVM support.
pub fn has_svm() -> bool {
    CpuId::new()
        .get_extended_processor_and_feature_identifiers()
        .is_some_and(|f| f.has_svm())
}

/// Returns the SVM revision as `(minor, major)`.
pub fn svm_revision() -> (u8, u8) {
    let revision = svm_capabilities().revision;
    (revision & 0x0f, revision >> 4)
}

/// Returns decoded SVM feature bits for the current CPU.
pub fn svm_features() -> SvmFeatures {
    svm_capabilities().features
}

/// Returns whether SVM nested paging is supported by the current CPU.
pub fn np_supported() -> bool {
    svm_features().nested_paging
}

/// Returns whether SVM NRIP save is supported by the current CPU.
pub fn nrip_supported() -> bool {
    svm_features().nrip_save
}

/// Returns the number of ASIDs reported by the current CPU.
pub fn asid_count() -> u32 {
    svm_capabilities().asid_count
}

/// Returns a consolidated SVM capability snapshot for the current CPU.
pub fn svm_capabilities() -> SvmCapabilities {
    svm_capabilities_from_cpuid(&CpuId::new())
}

fn svm_capabilities_from_cpuid<R: CpuIdReader>(cpuid: &CpuId<R>) -> SvmCapabilities {
    let supported = cpuid
        .get_extended_processor_and_feature_identifiers()
        .is_some_and(|f| f.has_svm());
    let Some(info) = cpuid.get_svm_info() else {
        return SvmCapabilities {
            supported,
            ..SvmCapabilities::default()
        };
    };

    SvmCapabilities {
        supported,
        revision: info.revision(),
        asid_count: info.supported_asids(),
        features: SvmFeatures {
            nested_paging: info.has_nested_paging(),
            lbr_virtualization: info.has_lbr_virtualization(),
            svm_lock: info.has_svm_lock(),
            nrip_save: info.has_nrip(),
            tsc_rate_msr: info.has_tsc_rate_msr(),
            vmcb_clean_bits: info.has_vmcb_clean_bits(),
            flush_by_asid: info.has_flush_by_asid(),
            decode_assists: info.has_decode_assists(),
            pause_filter: info.has_pause_filter(),
            pause_filter_threshold: info.has_pause_filter_threshold(),
            avic: info.has_avic(),
            vmsave_vmload_virtualization: info.has_vmsave_virtualization(),
            virtual_gif: info.has_gif(),
            guest_mode_execute_trap: info.has_gmet(),
            supervisor_shadow_stack: info.has_sss_check(),
            spec_ctrl_virtualization: info.has_spec_ctrl(),
            host_mce_override: info.has_host_mce_override(),
            tlb_control: info.has_tlb_ctrl(),
        },
    }
}

#[cfg(test)]
mod tests {
    use raw_cpuid::{CpuId, CpuIdResult};

    use super::*;

    fn amd_svm_cpuid(eax: u32, _ecx: u32) -> CpuIdResult {
        match eax {
            0x0000_0000 => CpuIdResult {
                eax: 0x0000_0016,
                ebx: 0x6874_7541,
                ecx: 0x444d_4163,
                edx: 0x6974_6e65,
            },
            0x8000_0000 => CpuIdResult {
                eax: 0x8000_000a,
                ebx: 0,
                ecx: 0,
                edx: 0,
            },
            0x8000_0001 => CpuIdResult {
                eax: 0,
                ebx: 0,
                ecx: 1 << 2,
                edx: 0,
            },
            0x8000_000a => CpuIdResult {
                eax: 0x0000_0001,
                ebx: 0x0000_8000,
                ecx: 0,
                edx: 0x0013_bcff,
            },
            _ => CpuIdResult {
                eax: 0,
                ebx: 0,
                ecx: 0,
                edx: 0,
            },
        }
    }

    fn intel_no_svm_cpuid(eax: u32, _ecx: u32) -> CpuIdResult {
        match eax {
            0x0000_0000 => CpuIdResult {
                eax: 0x0000_0016,
                ebx: 0x756e_6547,
                ecx: 0x6c65_746e,
                edx: 0x4965_6e69,
            },
            0x8000_0000 => CpuIdResult {
                eax: 0x8000_0008,
                ebx: 0,
                ecx: 0,
                edx: 0,
            },
            0x8000_0001 => CpuIdResult {
                eax: 0,
                ebx: 0,
                ecx: 0,
                edx: 0,
            },
            _ => CpuIdResult {
                eax: 0,
                ebx: 0,
                ecx: 0,
                edx: 0,
            },
        }
    }

    #[test]
    fn decodes_amd_svm_capabilities() {
        let cpuid = CpuId::with_cpuid_fn(amd_svm_cpuid);
        let caps = svm_capabilities_from_cpuid(&cpuid);

        assert!(caps.supported);
        assert_eq!(caps.revision, 1);
        assert_eq!(caps.asid_count, 0x8000);
        assert!(caps.features.nested_paging);
        assert!(caps.features.nrip_save);
        assert!(caps.features.virtual_gif);
        assert!(caps.features.spec_ctrl_virtualization);
    }

    #[test]
    fn reports_default_capabilities_without_svm() {
        let cpuid = CpuId::with_cpuid_fn(intel_no_svm_cpuid);
        let caps = svm_capabilities_from_cpuid(&cpuid);

        assert!(!caps.supported);
        assert_eq!(caps, SvmCapabilities::default());
    }
}

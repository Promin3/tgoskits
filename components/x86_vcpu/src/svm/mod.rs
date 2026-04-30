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

mod cpuid;
mod percpu;
mod vcpu;
mod vmcb;

pub use cpuid::{
    SvmCapabilities, SvmFeatures, asid_count, has_svm, np_supported, nrip_supported,
    svm_capabilities, svm_features, svm_revision,
};
pub use percpu::SvmPerCpuState;
pub use vcpu::{ContiguousFrames, Iopm, Msrpm, SvmVcpu, VmcbFrame};
pub use vmcb::{
    EventInj, EventType, InterceptCr, InterceptDr, InterceptException, InterceptInst1,
    InterceptInst2, NestedPageControl, SvmExitCode, SvmExitInfo, VirtualInterruptControl, Vmcb,
    VmcbControlArea, VmcbDescriptorTable, VmcbSaveArea, VmcbSegment,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvmExitReason {
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SvmInterruptInfo {
    pub vector: u8,
    pub error_code: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SvmIoExitInfo {
    pub port: u16,
    pub access_size: u8,
    pub is_in: bool,
}

pub type SvmArchVCpu = SvmVcpu;
pub type SvmArchPerCpuState = SvmPerCpuState;

pub fn has_hardware_support() -> bool {
    has_svm()
}

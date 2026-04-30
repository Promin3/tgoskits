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

pub use cpuid::has_svm;
pub use percpu::SvmPerCpuState;
pub use vcpu::SvmVcpu;
pub use vmcb::{SvmExitCode as SvmExitReason, SvmExitInfo};

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

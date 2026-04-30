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

use ax_errno::{AxResult, ax_err};
use axaddrspace::{GuestPhysAddr, HostPhysAddr};
use axvcpu::{AxArchPerCpu, AxArchVCpu, AxVCpuExitReason};
use axvisor_api::vmm::{VCpuId, VMId};

use crate::regs::GeneralRegisters;

mod cpuid;

pub use cpuid::{
    SvmCapabilities, SvmFeatures, asid_count, has_svm, np_supported, nrip_supported,
    svm_capabilities, svm_features, svm_revision,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvmExitReason {
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SvmExitInfo {
    pub exit_code: u64,
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

#[derive(Debug)]
pub struct SvmPerCpuState {
    _cpu_id: usize,
}

#[derive(Debug, Default)]
pub struct SvmVcpu {
    guest_regs: GeneralRegisters,
}

impl AxArchPerCpu for SvmPerCpuState {
    fn new(cpu_id: usize) -> AxResult<Self> {
        Ok(Self { _cpu_id: cpu_id })
    }

    fn is_enabled(&self) -> bool {
        false
    }

    fn hardware_enable(&mut self) -> AxResult {
        ax_err!(
            Unsupported,
            "AMD SVM support is not implemented yet for this target"
        )
    }

    fn hardware_disable(&mut self) -> AxResult {
        ax_err!(
            Unsupported,
            "AMD SVM support is not implemented yet for this target"
        )
    }
}

impl AxArchVCpu for SvmVcpu {
    type CreateConfig = ();
    type SetupConfig = ();

    fn new(_vm_id: VMId, _vcpu_id: VCpuId, _config: Self::CreateConfig) -> AxResult<Self> {
        ax_err!(Unsupported, "AMD SVM VCpu creation is not implemented yet")
    }

    fn set_entry(&mut self, _entry: GuestPhysAddr) -> AxResult {
        ax_err!(Unsupported, "AMD SVM entry setup is not implemented yet")
    }

    fn set_ept_root(&mut self, _ept_root: HostPhysAddr) -> AxResult {
        ax_err!(
            Unsupported,
            "AMD SVM nested page table setup is not implemented yet"
        )
    }

    fn setup(&mut self, _config: Self::SetupConfig) -> AxResult {
        ax_err!(Unsupported, "AMD SVM VCpu setup is not implemented yet")
    }

    fn run(&mut self) -> AxResult<AxVCpuExitReason> {
        ax_err!(
            Unsupported,
            "AMD SVM guest execution is not implemented yet"
        )
    }

    fn bind(&mut self) -> AxResult {
        ax_err!(Unsupported, "AMD SVM VCpu binding is not implemented yet")
    }

    fn unbind(&mut self) -> AxResult {
        ax_err!(Unsupported, "AMD SVM VCpu unbinding is not implemented yet")
    }

    fn set_gpr(&mut self, reg: usize, val: usize) {
        self.guest_regs.set_reg_of_index(reg as u8, val as u64);
    }

    fn inject_interrupt(&mut self, _vector: usize) -> AxResult {
        ax_err!(
            Unsupported,
            "AMD SVM interrupt injection is not implemented yet"
        )
    }

    fn set_return_value(&mut self, val: usize) {
        self.guest_regs.rax = val as u64;
    }
}

pub type SvmArchVCpu = SvmVcpu;
pub type SvmArchPerCpuState = SvmPerCpuState;

pub fn has_hardware_support() -> bool {
    has_svm()
}

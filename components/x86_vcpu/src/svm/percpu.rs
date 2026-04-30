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
use ax_memory_addr::{MemoryAddr, PAGE_SIZE_4K as PAGE_SIZE};
use axaddrspace::HostPhysAddr;
use axvcpu::AxArchPerCpu;
use axvisor_api::memory::PhysFrame;
use bitflags::bitflags;
use x86_64::registers::model_specific::EferFlags;

use crate::{msr::Msr, svm::has_hardware_support};

bitflags! {
    /// AMD VM_CR flags used by SVM global enable checks.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct VmCrFlags: u64 {
        /// Lock SVM enable state until reset.
        const SVM_LOCK = 1 << 3;
        /// SVM disabled by firmware.
        const SVM_DISABLE = 1 << 4;
    }
}

/// Represents the per-CPU state for AMD Secure Virtual Machine (SVM).
///
/// SVM does not need a VMXON-like region. Enabling the CPU only requires the
/// host-save area MSR and the EFER.SVME bit to be configured for this CPU.
#[derive(Debug)]
pub struct SvmPerCpuState {
    /// CPU id associated with this state.
    cpu_id: usize,
    /// Host save area consumed by the processor for SVM state transitions.
    vm_hsave_frame: Option<PhysFrame>,
}

impl SvmPerCpuState {
    /// Returns the physical address currently programmed into VM_HSAVE_PA.
    pub fn vm_hsave_pa(&self) -> Option<HostPhysAddr> {
        self.vm_hsave_frame.as_ref().map(PhysFrame::start_paddr)
    }

    fn vm_cr() -> VmCrFlags {
        VmCrFlags::from_bits_truncate(Msr::AMD_VM_CR.read())
    }

    fn enable_svme() {
        let old_value = Msr::IA32_EFER.read();
        let new_value = old_value | EferFlags::SECURE_VIRTUAL_MACHINE_ENABLE.bits();
        unsafe { Msr::IA32_EFER.write(new_value) };
    }

    fn disable_svme() {
        let old_value = Msr::IA32_EFER.read();
        let new_value = old_value & !EferFlags::SECURE_VIRTUAL_MACHINE_ENABLE.bits();
        unsafe { Msr::IA32_EFER.write(new_value) };
    }

    fn efer_has_svme() -> bool {
        Msr::IA32_EFER.read() & EferFlags::SECURE_VIRTUAL_MACHINE_ENABLE.bits() != 0
    }

    fn lock_vm_cr_if_needed() {
        let old_value = Msr::AMD_VM_CR.read();
        if old_value & VmCrFlags::SVM_LOCK.bits() == 0 {
            unsafe { Msr::AMD_VM_CR.write(old_value | VmCrFlags::SVM_LOCK.bits()) };
        }
    }
}

impl AxArchPerCpu for SvmPerCpuState {
    fn new(cpu_id: usize) -> AxResult<Self> {
        Ok(Self {
            cpu_id,
            vm_hsave_frame: None,
        })
    }

    fn is_enabled(&self) -> bool {
        Self::efer_has_svme()
    }

    fn hardware_enable(&mut self) -> AxResult {
        if !has_hardware_support() {
            return ax_err!(Unsupported, "CPU does not support feature SVM");
        }
        if self.is_enabled() {
            return ax_err!(ResourceBusy, "SVM is already turned on");
        }

        let vm_cr = Self::vm_cr();
        if vm_cr.contains(VmCrFlags::SVM_DISABLE) {
            return ax_err!(Unsupported, "SVM disabled by BIOS");
        }

        let frame = PhysFrame::alloc_zero()?;
        let vm_hsave_pa = frame.start_paddr();
        if !vm_hsave_pa.is_aligned(PAGE_SIZE) {
            return ax_err!(BadState, "SVM host save area is not page aligned");
        }

        unsafe { Msr::AMD_VM_HSAVE_PA.write(vm_hsave_pa.as_usize() as u64) };
        Self::lock_vm_cr_if_needed();
        Self::enable_svme();

        self.vm_hsave_frame = Some(frame);
        log::info!(
            "[AxVM] succeeded to turn on SVM on CPU {}, host save area {:#x}.",
            self.cpu_id,
            vm_hsave_pa.as_usize()
        );
        Ok(())
    }

    fn hardware_disable(&mut self) -> AxResult {
        if !self.is_enabled() {
            return ax_err!(BadState, "SVM is not enabled");
        }

        Self::disable_svme();
        unsafe { Msr::AMD_VM_HSAVE_PA.write(0) };
        self.vm_hsave_frame = None;
        log::info!("[AxVM] succeeded to turn off SVM on CPU {}.", self.cpu_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::mock::MockMmHal;

    #[test]
    fn test_vm_cr_flag_bits() {
        assert_eq!(VmCrFlags::SVM_LOCK.bits(), 1 << 3);
        assert_eq!(VmCrFlags::SVM_DISABLE.bits(), 1 << 4);
    }

    #[test]
    fn test_efer_svme_bit() {
        assert_eq!(EferFlags::SECURE_VIRTUAL_MACHINE_ENABLE.bits(), 1_u64 << 12);
    }

    #[test]
    fn test_svm_per_cpu_state_new() {
        MockMmHal::reset();
        let state = SvmPerCpuState::new(2).unwrap();

        assert_eq!(state.cpu_id, 2);
        assert_eq!(state.vm_hsave_pa(), None);
    }
}

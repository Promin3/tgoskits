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

#![no_std]

//! This crate provides a minimal VM monitor (VMM) for running guest VMs.
//!
//! This crate contains:
//! - [`AxVM`]: The main structure representing a VM.

#[cfg(all(target_arch = "x86_64", feature = "vmx", feature = "svm"))]
compile_error!("features `vmx` and `svm` are mutually exclusive on x86_64");

#[cfg(all(target_arch = "x86_64", not(any(feature = "vmx", feature = "svm"))))]
compile_error!("x86_64 requires either feature `vmx` or feature `svm`");

extern crate alloc;
#[macro_use]
extern crate log;

mod hal;
mod vcpu;
mod vm;

pub mod config;

pub use vm::{AxVCpuRef, AxVM, AxVMRef, VMMemoryRegion, VMStatus};

/// The architecture-independent per-CPU type.
pub type AxVMPerCpu = axvcpu::AxPerCpu<vcpu::AxVMArchPerCpuImpl>;

/// Whether the hardware has virtualization support.
pub fn has_hardware_support() -> bool {
    vcpu::has_hardware_support()
}

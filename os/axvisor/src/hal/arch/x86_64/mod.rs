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

pub mod cache;

pub fn hardware_check() {
    if axvm::has_hardware_support() {
        return;
    }

    #[cfg(feature = "vmx")]
    panic!("CPU does not support Intel VMX");

    #[cfg(feature = "svm")]
    panic!("CPU does not support AMD SVM");

    #[cfg(not(any(feature = "vmx", feature = "svm")))]
    panic!("x86_64 virtualization backend is not selected");
}

pub fn inject_interrupt(_vector: u8) {}

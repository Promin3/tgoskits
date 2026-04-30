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

use alloc::collections::VecDeque;

use ax_errno::{AxResult, ax_err, ax_err_type};
use ax_memory_addr::{MemoryAddr, PAGE_SIZE_4K as PAGE_SIZE};
use axaddrspace::{GuestPhysAddr, HostPhysAddr};
use axvcpu::{AxArchVCpu, AxVCpuExitReason};
use axvisor_api::{
    memory::{self, PhysFrame},
    vmm::{VCpuId, VMId},
};
use x86_64::registers::control::Cr0Flags;
use x86_vlapic::EmulatedLocalApic;

#[cfg(not(test))]
use crate::msr::Msr;
use crate::{
    regs::GeneralRegisters,
    svm::{
        InterceptException, InterceptInst1, InterceptInst2, NestedPageControl, Vmcb,
        VmcbDescriptorTable, VmcbSegment,
    },
};

const QEMU_EXIT_PORT: u16 = 0x604;
const IA32_UMWAIT_CONTROL: u32 = 0xe1;
const DEFAULT_ASID: u32 = 1;

/// A VMCB-backed SVM vCPU. The first two fields are reserved for the VMRUN
/// assembly path implemented in the next stage.
#[repr(C)]
pub struct SvmVcpu {
    guest_regs: GeneralRegisters,
    host_stack_top: u64,

    launched: bool,
    entry: Option<GuestPhysAddr>,
    ept_root: Option<HostPhysAddr>,

    vmcb: VmcbFrame,
    iopm: Iopm,
    msrpm: Msrpm,

    pending_events: VecDeque<(u8, Option<u32>)>,
    vlapic: EmulatedLocalApic,
}

impl SvmVcpu {
    pub fn new(vm_id: VMId, vcpu_id: VCpuId) -> AxResult<Self> {
        let mut vcpu = Self {
            guest_regs: GeneralRegisters::default(),
            host_stack_top: 0,
            launched: false,
            entry: None,
            ept_root: None,
            vmcb: VmcbFrame::new()?,
            iopm: Iopm::passthrough_all()?,
            msrpm: Msrpm::passthrough_all()?,
            pending_events: VecDeque::with_capacity(8),
            vlapic: EmulatedLocalApic::new(vm_id, vcpu_id),
        };
        vcpu.setup_iopm()?;
        vcpu.setup_msrpm()?;
        log::info!(
            "[HV] created SvmVcpu(vmcb: {:#x})",
            vcpu.vmcb.phys_addr().as_usize()
        );
        Ok(vcpu)
    }

    pub fn vmcb(&self) -> &Vmcb {
        self.vmcb.vmcb()
    }

    pub fn vmcb_mut(&mut self) -> &mut Vmcb {
        self.vmcb.vmcb_mut()
    }

    pub fn setup_vmcb(&mut self, entry: GuestPhysAddr, ept_root: HostPhysAddr) -> AxResult {
        self.vmcb.clear();
        self.setup_vmcb_guest(entry);
        self.setup_vmcb_control(ept_root);
        Ok(())
    }

    fn setup_vmcb_guest(&mut self, entry: GuestPhysAddr) {
        let save = &mut self.vmcb.vmcb_mut().save;

        let data_segment = VmcbSegment::new(0, 0x93, 0xffff, 0);
        save.es = data_segment;
        save.ss = data_segment;
        save.ds = data_segment;
        save.fs = data_segment;
        save.gs = data_segment;
        save.cs = VmcbSegment::new(0, 0x9b, 0xffff, 0);
        save.tr = VmcbSegment::new(0, 0x8b, 0xffff, 0);
        save.ldtr = VmcbSegment::new(0, 0x82, 0xffff, 0);

        save.gdtr = VmcbDescriptorTable::new(0xffff, 0);
        save.idtr = VmcbDescriptorTable::new(0xffff, 0);

        save.cr0 =
            (Cr0Flags::NOT_WRITE_THROUGH | Cr0Flags::CACHE_DISABLE | Cr0Flags::EXTENSION_TYPE)
                .bits();
        save.cr2 = 0;
        save.cr3 = 0;
        save.cr4 = 0;
        save.dr6 = 0;
        save.dr7 = 0x400;
        save.efer = 0;
        save.rflags = 0x2;
        save.rip = entry.as_usize() as u64;
        save.rsp = 0;
        save.rax = 0;
        save.gpat = host_pat();
    }

    fn setup_vmcb_control(&mut self, ept_root: HostPhysAddr) {
        let control = &mut self.vmcb.vmcb_mut().control;

        control.intercept_exception = InterceptException::UD.bits();
        control.intercept_instruction1 = (InterceptInst1::INTR
            | InterceptInst1::NMI
            | InterceptInst1::CPUID
            | InterceptInst1::HLT
            | InterceptInst1::IOIO_PROT
            | InterceptInst1::MSR_PROT
            | InterceptInst1::SHUTDOWN)
            .bits();
        control.intercept_instruction2 = (InterceptInst2::VMRUN
            | InterceptInst2::VMMCALL
            | InterceptInst2::VMLOAD
            | InterceptInst2::VMSAVE
            | InterceptInst2::STGI
            | InterceptInst2::CLGI
            | InterceptInst2::SKINIT
            | InterceptInst2::RDTSCP
            | InterceptInst2::WBINVD
            | InterceptInst2::XSETBV
            | InterceptInst2::EFER_WRITE)
            .bits();
        control.iopm_base_pa = self.iopm.phys_addr().as_usize() as u64;
        control.msrpm_base_pa = self.msrpm.phys_addr().as_usize() as u64;
        control.tsc_offset = 0;
        control.guest_asid = DEFAULT_ASID;
        control.tlb_control = 0;
        control.nested_page_control = NestedPageControl::ENABLE.bits();
        control.n_cr3 = ept_root.as_usize() as u64;
    }

    fn setup_iopm(&mut self) -> AxResult {
        self.iopm.set_intercept(QEMU_EXIT_PORT, true);
        Ok(())
    }

    fn setup_msrpm(&mut self) -> AxResult {
        self.msrpm.set_write_intercept(IA32_UMWAIT_CONTROL, true)?;
        self.msrpm.set_read_intercept(IA32_UMWAIT_CONTROL, true)?;

        for msr in 0x800..=0x83f {
            self.msrpm.set_read_intercept(msr, true)?;
            self.msrpm.set_write_intercept(msr, true)?;
        }
        Ok(())
    }
}

impl AxArchVCpu for SvmVcpu {
    type CreateConfig = ();
    type SetupConfig = ();

    fn new(vm_id: VMId, vcpu_id: VCpuId, _config: Self::CreateConfig) -> AxResult<Self> {
        Self::new(vm_id, vcpu_id)
    }

    fn set_entry(&mut self, entry: GuestPhysAddr) -> AxResult {
        self.entry = Some(entry);
        Ok(())
    }

    fn set_ept_root(&mut self, ept_root: HostPhysAddr) -> AxResult {
        self.ept_root = Some(ept_root);
        Ok(())
    }

    fn setup(&mut self, _config: Self::SetupConfig) -> AxResult {
        self.setup_vmcb(
            self.entry
                .ok_or_else(|| ax_err_type!(BadState, "SVM vCPU entry is not set"))?,
            self.ept_root
                .ok_or_else(|| ax_err_type!(BadState, "SVM vCPU NPT root is not set"))?,
        )
    }

    fn run(&mut self) -> AxResult<AxVCpuExitReason> {
        ax_err!(
            Unsupported,
            "AMD SVM guest execution is not implemented yet"
        )
    }

    fn bind(&mut self) -> AxResult {
        Ok(())
    }

    fn unbind(&mut self) -> AxResult {
        self.launched = false;
        Ok(())
    }

    fn set_gpr(&mut self, reg: usize, val: usize) {
        self.guest_regs.set_reg_of_index(reg as u8, val as u64);
    }

    fn inject_interrupt(&mut self, vector: usize) -> AxResult {
        self.pending_events.push_back((vector as u8, None));
        Ok(())
    }

    fn set_return_value(&mut self, val: usize) {
        self.guest_regs.rax = val as u64;
    }
}

#[derive(Debug)]
pub struct VmcbFrame {
    frame: PhysFrame,
}

impl VmcbFrame {
    pub fn new() -> AxResult<Self> {
        let frame = PhysFrame::alloc_zero()?;
        if !frame.start_paddr().is_aligned(PAGE_SIZE) {
            return ax_err!(BadState, "SVM VMCB frame is not page aligned");
        }
        Ok(Self { frame })
    }

    pub fn phys_addr(&self) -> HostPhysAddr {
        self.frame.start_paddr()
    }

    pub fn vmcb(&self) -> &Vmcb {
        unsafe { &*(self.frame.as_mut_ptr() as *const Vmcb) }
    }

    pub fn vmcb_mut(&mut self) -> &mut Vmcb {
        unsafe { &mut *(self.frame.as_mut_ptr() as *mut Vmcb) }
    }

    pub fn clear(&mut self) {
        *self.vmcb_mut() = Vmcb::default();
    }
}

#[derive(Debug)]
pub struct ContiguousFrames {
    paddr: HostPhysAddr,
    pages: usize,
}

impl ContiguousFrames {
    pub fn alloc_zero(pages: usize) -> AxResult<Self> {
        let paddr = memory::alloc_contiguous_frames(pages, PAGE_SIZE)
            .ok_or_else(|| ax_err_type!(NoMemory, "allocate contiguous physical frames failed"))?;
        if !paddr.is_aligned(PAGE_SIZE) {
            memory::dealloc_contiguous_frames(paddr, pages);
            return ax_err!(BadState, "contiguous frames are not page aligned");
        }

        let vaddr = memory::phys_to_virt(paddr);
        unsafe { core::ptr::write_bytes(vaddr.as_mut_ptr(), 0, pages * PAGE_SIZE) };
        Ok(Self { paddr, pages })
    }

    pub fn phys_addr(&self) -> HostPhysAddr {
        self.paddr
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(
                memory::phys_to_virt(self.paddr).as_mut_ptr(),
                self.pages * PAGE_SIZE,
            )
        }
    }
}

impl Drop for ContiguousFrames {
    fn drop(&mut self) {
        memory::dealloc_contiguous_frames(self.paddr, self.pages);
    }
}

#[derive(Debug)]
pub struct Iopm {
    frames: ContiguousFrames,
}

impl Iopm {
    const PAGES: usize = 3;

    pub fn passthrough_all() -> AxResult<Self> {
        Ok(Self {
            frames: ContiguousFrames::alloc_zero(Self::PAGES)?,
        })
    }

    pub fn phys_addr(&self) -> HostPhysAddr {
        self.frames.phys_addr()
    }

    pub fn set_intercept(&mut self, port: u16, intercept: bool) {
        set_bitmap_bit(self.frames.as_mut_slice(), port as usize, intercept);
    }
}

#[derive(Debug)]
pub struct Msrpm {
    frames: ContiguousFrames,
}

impl Msrpm {
    const PAGES: usize = 2;
    const LOW_MSR_LIMIT: u32 = 0x1fff;
    const HIGH_MSR_BASE: u32 = 0xc000_0000;
    const HIGH_MSR_LIMIT: u32 = 0xc001_1fff;
    const BITMAP_BYTES_PER_RANGE: usize = 0x800;

    pub fn passthrough_all() -> AxResult<Self> {
        Ok(Self {
            frames: ContiguousFrames::alloc_zero(Self::PAGES)?,
        })
    }

    pub fn phys_addr(&self) -> HostPhysAddr {
        self.frames.phys_addr()
    }

    pub fn set_read_intercept(&mut self, msr: u32, intercept: bool) -> AxResult {
        self.set_intercept(msr, false, intercept)
    }

    pub fn set_write_intercept(&mut self, msr: u32, intercept: bool) -> AxResult {
        self.set_intercept(msr, true, intercept)
    }

    fn set_intercept(&mut self, msr: u32, is_write: bool, intercept: bool) -> AxResult {
        let Some((range_offset, bit_index)) = Self::bitmap_position(msr, is_write) else {
            return ax_err!(InvalidInput, "MSR is outside the SVM MSRPM ranges");
        };
        set_bitmap_bit(
            &mut self.frames.as_mut_slice()
                [range_offset..range_offset + Self::BITMAP_BYTES_PER_RANGE],
            bit_index,
            intercept,
        );
        Ok(())
    }

    fn bitmap_position(msr: u32, is_write: bool) -> Option<(usize, usize)> {
        if msr <= Self::LOW_MSR_LIMIT {
            Some((if is_write { 0x800 } else { 0 }, msr as usize))
        } else if (Self::HIGH_MSR_BASE..=Self::HIGH_MSR_LIMIT).contains(&msr) {
            let msr_index = (msr - Self::HIGH_MSR_BASE) as usize;
            Some((if is_write { 0x1800 } else { 0x1000 }, msr_index))
        } else {
            None
        }
    }
}

fn set_bitmap_bit(bitmap: &mut [u8], bit: usize, set: bool) {
    let byte = bit / 8;
    let bit_in_byte = bit % 8;
    if set {
        bitmap[byte] |= 1 << bit_in_byte;
    } else {
        bitmap[byte] &= !(1 << bit_in_byte);
    }
}

#[cfg(not(test))]
fn host_pat() -> u64 {
    Msr::IA32_PAT.read()
}

#[cfg(test)]
fn host_pat() -> u64 {
    0x0007_0406_0007_0406
}

#[cfg(test)]
mod tests {
    use axvisor_api::memory::MemoryIf;
    use spin::{Mutex, MutexGuard};

    use super::*;
    use crate::test_utils::mock::MockMmHal;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock_test() -> MutexGuard<'static, ()> {
        TEST_LOCK.lock()
    }

    fn bit_is_set(bitmap: &[u8], bit: usize) -> bool {
        bitmap[bit / 8] & (1 << (bit % 8)) != 0
    }

    #[test]
    fn vmcb_frame_allocates_page_aligned_vmcb() {
        let _guard = lock_test();
        MockMmHal::reset();
        let mut frame = VmcbFrame::new().unwrap();
        frame.vmcb_mut().save.rip = 0x1234;

        assert!(frame.phys_addr().is_aligned(PAGE_SIZE));
        assert_eq!(frame.vmcb().save.rip, 0x1234);
    }

    #[test]
    fn iopm_sets_port_intercepts() {
        let _guard = lock_test();
        MockMmHal::reset();
        let mut iopm = Iopm::passthrough_all().unwrap();

        iopm.set_intercept(QEMU_EXIT_PORT, true);
        assert!(bit_is_set(
            iopm.frames.as_mut_slice(),
            QEMU_EXIT_PORT as usize
        ));

        iopm.set_intercept(QEMU_EXIT_PORT, false);
        assert!(!bit_is_set(
            iopm.frames.as_mut_slice(),
            QEMU_EXIT_PORT as usize
        ));
    }

    #[test]
    fn msrpm_sets_low_and_high_msr_intercepts() {
        let _guard = lock_test();
        MockMmHal::reset();
        let mut msrpm = Msrpm::passthrough_all().unwrap();

        msrpm.set_read_intercept(IA32_UMWAIT_CONTROL, true).unwrap();
        msrpm.set_write_intercept(0xc000_0080, true).unwrap();

        let bitmap = msrpm.frames.as_mut_slice();
        assert!(bit_is_set(bitmap, IA32_UMWAIT_CONTROL as usize));
        assert!(bit_is_set(bitmap, 0x1800 * 8 + 0x80));
    }

    #[test]
    fn svm_vcpu_setup_initializes_vmcb() {
        let _guard = lock_test();
        MockMmHal::reset();
        let entry = GuestPhysAddr::from(0x8020_0000);
        let npt_root = HostPhysAddr::from(0x1234_5000);
        let mut vcpu = SvmVcpu::new(1, 2).unwrap();

        vcpu.set_entry(entry).unwrap();
        vcpu.set_ept_root(npt_root).unwrap();
        vcpu.setup(()).unwrap();

        let vmcb = vcpu.vmcb();
        assert_eq!(vmcb.save.cs, VmcbSegment::new(0, 0x9b, 0xffff, 0));
        assert_eq!(vmcb.save.ds, VmcbSegment::new(0, 0x93, 0xffff, 0));
        assert_eq!(vmcb.save.rip, entry.as_usize() as u64);
        assert_eq!(vmcb.save.rflags, 0x2);
        assert_eq!(vmcb.control.guest_asid, DEFAULT_ASID);
        assert_eq!(vmcb.control.n_cr3, npt_root.as_usize() as u64);
        assert_eq!(
            vmcb.control.nested_page_control,
            NestedPageControl::ENABLE.bits()
        );
        assert_eq!(
            vmcb.control.iopm_base_pa,
            vcpu.iopm.phys_addr().as_usize() as u64
        );
        assert_eq!(
            vmcb.control.msrpm_base_pa,
            vcpu.msrpm.phys_addr().as_usize() as u64
        );
        assert_ne!(vcpu.vlapic.virtual_apic_page_addr().as_usize(), 0);

        let iopm = vcpu.iopm.frames.as_mut_slice();
        assert!(bit_is_set(iopm, QEMU_EXIT_PORT as usize));

        let msrpm = vcpu.msrpm.frames.as_mut_slice();
        assert!(bit_is_set(msrpm, IA32_UMWAIT_CONTROL as usize));
    }

    #[test]
    fn mock_allocator_supports_contiguous_frames() {
        let _guard = lock_test();
        MockMmHal::reset();
        let paddr = MockMmHal::alloc_contiguous_frames(3, PAGE_SIZE).unwrap();
        assert!(paddr.is_aligned(PAGE_SIZE));
        MockMmHal::dealloc_contiguous_frames(paddr, 3);
    }
}

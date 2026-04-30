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
use core::{arch::naked_asm, mem::size_of};

use ax_errno::{AxResult, ax_err, ax_err_type};
use ax_memory_addr::{MemoryAddr, PAGE_SIZE_4K as PAGE_SIZE};
use axaddrspace::{
    GuestPhysAddr, HostPhysAddr, MappingFlags, NestedPageFaultInfo,
    device::{AccessWidth, Port, SysRegAddr, SysRegAddrRange},
};
use axdevice_base::BaseDeviceOps;
use axvcpu::{AxArchVCpu, AxVCpuExitReason};
use axvisor_api::{
    memory::{self, PhysFrame},
    vmm::{VCpuId, VMId},
};
use bit_field::BitField;
use x86_64::registers::control::Cr0Flags;
use x86_vlapic::EmulatedLocalApic;

use super::vmcb::{
    EventInj, InterceptException, InterceptInst1, InterceptInst2, NestedPageControl, SvmExitCode,
    SvmExitInfo, Vmcb, VmcbDescriptorTable, VmcbSegment,
};
#[cfg(not(test))]
use crate::msr::Msr;
use crate::regs::GeneralRegisters;

const QEMU_EXIT_PORT: u16 = 0x604;
const QEMU_EXIT_MAGIC: u64 = 0x2000;
const IA32_UMWAIT_CONTROL: u32 = 0xe1;
const DEFAULT_ASID: u32 = 1;
const X2APIC_MSR_BASE: u32 = 0x800;
const X2APIC_MSR_END: u32 = 0x83f;

/// A VMCB-backed SVM vCPU. The first fields are part of the VMRUN assembly ABI.
#[repr(C)]
pub struct SvmVcpu {
    guest_regs: GeneralRegisters,
    host_stack_top: u64,
    vmcb_pa: u64,

    launched: bool,
    entry: Option<GuestPhysAddr>,
    ept_root: Option<HostPhysAddr>,

    vmcb: VmcbFrame,
    iopm: Iopm,
    msrpm: Msrpm,

    pending_events: VecDeque<EventInj>,
    vlapic: EmulatedLocalApic,
}

impl SvmVcpu {
    pub fn new(vm_id: VMId, vcpu_id: VCpuId) -> AxResult<Self> {
        let mut vcpu = Self {
            guest_regs: GeneralRegisters::default(),
            host_stack_top: 0,
            vmcb_pa: 0,
            launched: false,
            entry: None,
            ept_root: None,
            vmcb: VmcbFrame::new()?,
            iopm: Iopm::passthrough_all()?,
            msrpm: Msrpm::passthrough_all()?,
            pending_events: VecDeque::with_capacity(8),
            vlapic: EmulatedLocalApic::new(vm_id, vcpu_id),
        };
        vcpu.vmcb_pa = vcpu.vmcb.phys_addr().as_usize() as u64;
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
        self.vmcb_pa = self.vmcb.phys_addr().as_usize() as u64;
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

        for msr in X2APIC_MSR_BASE..=X2APIC_MSR_END {
            self.msrpm.set_read_intercept(msr, true)?;
            self.msrpm.set_write_intercept(msr, true)?;
        }
        Ok(())
    }

    /// Enter the guest through AMD SVM `VMRUN`.
    ///
    /// This function never returns directly from its own instruction stream. On `#VMEXIT`,
    /// execution resumes after `vmrun` and the same naked frame restores the host stack before
    /// returning to Rust.
    #[unsafe(naked)]
    unsafe extern "C" fn vmrun(&mut self) {
        naked_asm!(
            save_regs_to_stack!(),
            "mov    [rdi + {host_stack_top}], rsp",
            "mov    rsp, rdi",
            restore_regs_from_stack!(),
            "mov    rax, [rsp + {vmcb_pa}]",
            "vmrun  rax",
            save_regs_to_stack!(),
            "mov    rsp, [rsp + {host_stack_top}]",
            restore_regs_from_stack!(),
            "ret",
            host_stack_top = const size_of::<GeneralRegisters>(),
            vmcb_pa = const size_of::<u64>(),
        );
    }

    /// Execute one raw `VMRUN` round without VMEXIT handling.
    unsafe fn raw_vmrun(&mut self) {
        self.vmcb.vmcb_mut().save.rax = self.guest_regs.rax;
        unsafe { self.vmrun() };
        self.guest_regs.rax = self.vmcb.vmcb().save.rax;
        self.launched = true;
    }

    fn inner_run(&mut self) -> AxResult<Option<SvmExitInfo>> {
        self.inject_pending_events();

        unsafe { self.raw_vmrun() };

        let exit_info = self.exit_info();
        match self.vmexit_handler(&exit_info) {
            Some(result) => {
                result?;
                Ok(None)
            }
            None => Ok(Some(exit_info)),
        }
    }

    pub fn exit_info(&self) -> SvmExitInfo {
        SvmExitInfo::from_vmcb(self.vmcb())
    }

    fn vmexit_handler(&mut self, exit_info: &SvmExitInfo) -> Option<AxResult> {
        match exit_info.exit_code {
            SvmExitCode::CPUID => Some(self.handle_cpuid()),
            SvmExitCode::MSR
                if (X2APIC_MSR_BASE..=X2APIC_MSR_END).contains(&(self.guest_regs.rcx as u32)) =>
            {
                Some(self.handle_apic_msr_access(Self::msr_exit_is_write(exit_info)))
            }
            SvmExitCode::NMI => Some(Ok(())),
            _ => None,
        }
    }

    fn inject_pending_events(&mut self) {
        let control = &mut self.vmcb.vmcb_mut().control;
        if control.eventinj & EventInj::VALID.bits() != 0 {
            return;
        }
        if let Some(event) = self.pending_events.pop_front() {
            control.eventinj = event.bits();
        }
    }

    fn advance_rip(&mut self, fallback_len: u64) -> AxResult {
        self.set_next_rip(self.vmcb.vmcb().control.next_rip, fallback_len)
    }

    fn advance_rip_with(&mut self, next_rip: u64, fallback_len: u64) -> AxResult {
        self.set_next_rip(next_rip, fallback_len)
    }

    fn set_next_rip(&mut self, next_rip: u64, fallback_len: u64) -> AxResult {
        let save = &mut self.vmcb.vmcb_mut().save;
        if next_rip != 0 {
            save.rip = next_rip;
        } else {
            save.rip = save
                .rip
                .checked_add(fallback_len)
                .ok_or_else(|| ax_err_type!(BadState, "SVM guest RIP overflow"))?;
        }
        Ok(())
    }

    fn handle_cpuid(&mut self) -> AxResult {
        use raw_cpuid::{CpuIdResult, cpuid};

        const VMEXIT_INSTR_LEN_CPUID: u64 = 2;
        const LEAF_FEATURE_INFO: u32 = 0x1;
        const LEAF_EXTENDED_FEATURE_INFO: u32 = 0x8000_0001;
        const LEAF_STRUCTURED_EXTENDED_FEATURE_FLAGS_ENUMERATION: u32 = 0x7;
        const LEAF_PROCESSOR_EXTENDED_STATE_ENUMERATION: u32 = 0xd;
        const EAX_FREQUENCY_INFO: u32 = 0x16;
        const LEAF_HYPERVISOR_INFO: u32 = 0x4000_0000;
        const LEAF_HYPERVISOR_FEATURE: u32 = 0x4000_0001;
        const VENDOR_STR: &[u8; 12] = b"RVMRVMRVMRVM";
        let vendor_regs = unsafe { &*(VENDOR_STR.as_ptr() as *const [u32; 3]) };

        let regs = self.guest_regs;
        let function = regs.rax as u32;
        let res = match function {
            LEAF_FEATURE_INFO => {
                const FEATURE_HYPERVISOR: u32 = 1 << 31;
                const FEATURE_MCE: u32 = 1 << 7;
                let mut res = cpuid!(regs.rax, regs.rcx);
                res.ecx |= FEATURE_HYPERVISOR;
                res.eax &= !FEATURE_MCE;
                res
            }
            LEAF_EXTENDED_FEATURE_INFO => {
                const FEATURE_SVM: u32 = 1 << 2;
                let mut res = cpuid!(regs.rax, regs.rcx);
                res.ecx &= !FEATURE_SVM;
                res
            }
            LEAF_STRUCTURED_EXTENDED_FEATURE_FLAGS_ENUMERATION => {
                let mut res = cpuid!(regs.rax, regs.rcx);
                if regs.rcx == 0 {
                    res.ecx.set_bit(5, false);
                    res.ecx.set_bit(16, false);
                }
                res
            }
            LEAF_PROCESSOR_EXTENDED_STATE_ENUMERATION => cpuid!(regs.rax, regs.rcx),
            LEAF_HYPERVISOR_INFO => CpuIdResult {
                eax: LEAF_HYPERVISOR_FEATURE,
                ebx: vendor_regs[0],
                ecx: vendor_regs[1],
                edx: vendor_regs[2],
            },
            LEAF_HYPERVISOR_FEATURE => CpuIdResult {
                eax: 0,
                ebx: 0,
                ecx: 0,
                edx: 0,
            },
            EAX_FREQUENCY_INFO => {
                const TIMER_FREQUENCY_MHZ: u32 = 3_000;
                let mut res = cpuid!(regs.rax, regs.rcx);
                if res.eax == 0 {
                    log::warn!(
                        "handle_cpuid: Failed to get TSC frequency by CPUID, default to \
                         {TIMER_FREQUENCY_MHZ} MHz"
                    );
                    res.eax = TIMER_FREQUENCY_MHZ;
                }
                res
            }
            _ => cpuid!(regs.rax, regs.rcx),
        };

        self.guest_regs.rax = res.eax as u64;
        self.guest_regs.rbx = res.ebx as u64;
        self.guest_regs.rcx = res.ecx as u64;
        self.guest_regs.rdx = res.edx as u64;
        self.vmcb.vmcb_mut().save.rax = self.guest_regs.rax;
        self.advance_rip(VMEXIT_INSTR_LEN_CPUID)
    }

    fn handle_apic_msr_access(&mut self, write: bool) -> AxResult {
        const VMEXIT_INSTR_LEN_RDMSR_WRMSR: u64 = 2;

        self.advance_rip(VMEXIT_INSTR_LEN_RDMSR_WRMSR)?;

        let msr = self.guest_regs.rcx as usize;
        if write {
            let value = self.read_edx_eax() as usize;
            <EmulatedLocalApic as BaseDeviceOps<SysRegAddrRange>>::handle_write(
                &self.vlapic,
                SysRegAddr::new(msr),
                AccessWidth::Qword,
                value,
            )
        } else {
            let value = <EmulatedLocalApic as BaseDeviceOps<SysRegAddrRange>>::handle_read(
                &self.vlapic,
                SysRegAddr::new(msr),
                AccessWidth::Qword,
            )? as u64;
            self.write_edx_eax(value);
            Ok(())
        }
    }

    fn read_edx_eax(&self) -> u64 {
        ((self.guest_regs.rdx & 0xffff_ffff) << 32) | (self.guest_regs.rax & 0xffff_ffff)
    }

    fn write_edx_eax(&mut self, val: u64) {
        self.guest_regs.rax = val & 0xffff_ffff;
        self.guest_regs.rdx = val >> 32;
        self.vmcb.vmcb_mut().save.rax = self.guest_regs.rax;
    }

    fn msr_exit_is_write(exit_info: &SvmExitInfo) -> bool {
        exit_info.exitinfo1 & 1 != 0
    }

    fn io_exit_info(exit_info: &SvmExitInfo) -> SvmIoExitInfoDecoded {
        let access_size = match exit_info.exitinfo1.get_bits(4..7) {
            0b001 => 1,
            0b010 => 2,
            0b100 => 4,
            _ => 0,
        };
        SvmIoExitInfoDecoded {
            access_size,
            is_in: exit_info.exitinfo1.get_bit(0),
            is_string: exit_info.exitinfo1.get_bit(2),
            is_repeat: exit_info.exitinfo1.get_bit(3),
            port: exit_info.exitinfo1.get_bits(16..32) as u16,
            next_rip: exit_info.exitinfo2,
        }
    }

    fn nested_page_fault_info(exit_info: &SvmExitInfo) -> NestedPageFaultInfo {
        let mut access_flags = MappingFlags::empty();
        if exit_info.exitinfo1.get_bit(1) {
            access_flags |= MappingFlags::WRITE;
        }
        if exit_info.exitinfo1.get_bit(4) {
            access_flags |= MappingFlags::EXECUTE;
        }
        if access_flags.is_empty() {
            access_flags |= MappingFlags::READ;
        }
        NestedPageFaultInfo {
            access_flags,
            fault_guest_paddr: GuestPhysAddr::from(exit_info.exitinfo2 as usize),
        }
    }

    fn interrupt_vector(exit_info: &SvmExitInfo) -> u64 {
        if exit_info.exitintinfo.get_bit(31) {
            exit_info.exitintinfo.get_bits(0..8)
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SvmIoExitInfoDecoded {
    port: u16,
    access_size: u8,
    is_in: bool,
    is_string: bool,
    is_repeat: bool,
    next_rip: u64,
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
        let Some(exit_info) = self.inner_run()? else {
            return Ok(AxVCpuExitReason::Nothing);
        };

        Ok(match exit_info.exit_code {
            SvmExitCode::INVALID => AxVCpuExitReason::FailEntry {
                hardware_entry_failure_reason: exit_info.exitcode_raw,
            },
            SvmExitCode::VMMCALL => {
                self.advance_rip(3)?;
                AxVCpuExitReason::Hypercall {
                    nr: self.guest_regs.rax,
                    args: [
                        self.guest_regs.rdi,
                        self.guest_regs.rsi,
                        self.guest_regs.rdx,
                        self.guest_regs.rcx,
                        self.guest_regs.r8,
                        self.guest_regs.r9,
                    ],
                }
            }
            SvmExitCode::IOIO => {
                let io_info = Self::io_exit_info(&exit_info);
                self.advance_rip_with(io_info.next_rip, 0)?;

                if io_info.is_repeat || io_info.is_string {
                    log::warn!("SVM unsupported IO-Exit: {io_info:#x?} of {exit_info:#x?}");
                    AxVCpuExitReason::Halt
                } else {
                    let width = match AccessWidth::try_from(io_info.access_size as usize) {
                        Ok(width) => width,
                        Err(_) => {
                            log::warn!("SVM invalid IO-Exit: {io_info:#x?} of {exit_info:#x?}");
                            return Ok(AxVCpuExitReason::Halt);
                        }
                    };

                    if io_info.is_in {
                        AxVCpuExitReason::IoRead {
                            port: Port(io_info.port),
                            width,
                        }
                    } else if io_info.port == QEMU_EXIT_PORT
                        && width == AccessWidth::Word
                        && self.guest_regs.rax == QEMU_EXIT_MAGIC
                    {
                        AxVCpuExitReason::SystemDown
                    } else {
                        AxVCpuExitReason::IoWrite {
                            port: Port(io_info.port),
                            width,
                            data: self.guest_regs.rax.get_bits(width.bits_range()),
                        }
                    }
                }
            }
            SvmExitCode::INTR => AxVCpuExitReason::ExternalInterrupt {
                vector: Self::interrupt_vector(&exit_info),
            },
            SvmExitCode::MSR if Self::msr_exit_is_write(&exit_info) => {
                AxVCpuExitReason::SysRegWrite {
                    addr: SysRegAddr::new(self.guest_regs.rcx as usize),
                    value: self.read_edx_eax(),
                }
            }
            SvmExitCode::MSR => AxVCpuExitReason::SysRegRead {
                addr: SysRegAddr::new(self.guest_regs.rcx as usize),
                reg: 0,
            },
            SvmExitCode::HLT => {
                self.advance_rip(1)?;
                AxVCpuExitReason::Halt
            }
            SvmExitCode::SHUTDOWN => AxVCpuExitReason::SystemDown,
            SvmExitCode::NPF => {
                let fault = Self::nested_page_fault_info(&exit_info);
                AxVCpuExitReason::NestedPageFault {
                    addr: fault.fault_guest_paddr,
                    access_flags: fault.access_flags,
                }
            }
            _ => {
                log::warn!("SVM unsupported VM-Exit: {exit_info:#x?}");
                AxVCpuExitReason::Halt
            }
        })
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
        self.pending_events
            .push_back(EventInj::external_interrupt(vector as u8));
        Ok(())
    }

    fn set_return_value(&mut self, val: usize) {
        self.guest_regs.rax = val as u64;
        self.vmcb.vmcb_mut().save.rax = val as u64;
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
    use core::mem::{offset_of, size_of};

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
    fn svm_vcpu_vmrun_assembly_offsets_match_layout() {
        assert_eq!(offset_of!(SvmVcpu, guest_regs), 0);
        assert_eq!(
            offset_of!(SvmVcpu, host_stack_top),
            size_of::<GeneralRegisters>()
        );
        assert_eq!(
            offset_of!(SvmVcpu, vmcb_pa),
            size_of::<GeneralRegisters>() + size_of::<u64>()
        );
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
    fn svm_io_exit_info_decodes_exitinfo1() {
        let exit_info = SvmExitInfo {
            exitcode_raw: SvmExitCode::IOIO.into(),
            exit_code: SvmExitCode::IOIO,
            exitinfo1: 1 | (1 << 2) | (1 << 3) | (0b100 << 4) | (0x3f8 << 16),
            exitinfo2: 0x401000,
            exitintinfo: 0,
            guest_rip: 0,
            next_rip: 0,
        };

        let io = SvmVcpu::io_exit_info(&exit_info);
        assert_eq!(io.access_size, 4);
        assert!(io.is_string);
        assert!(io.is_repeat);
        assert!(io.is_in);
        assert_eq!(io.port, 0x3f8);
        assert_eq!(io.next_rip, 0x401000);
    }

    #[test]
    fn svm_npf_info_uses_exitinfo1_as_error_code_and_exitinfo2_as_gpa() {
        let exit_info = SvmExitInfo {
            exitcode_raw: SvmExitCode::NPF.into(),
            exit_code: SvmExitCode::NPF,
            exitinfo1: (1 << 1) | (1 << 4),
            exitinfo2: 0xdead_beef,
            exitintinfo: 0,
            guest_rip: 0,
            next_rip: 0,
        };

        let fault = SvmVcpu::nested_page_fault_info(&exit_info);
        assert_eq!(fault.fault_guest_paddr, GuestPhysAddr::from(0xdead_beef));
        assert!(fault.access_flags.contains(MappingFlags::WRITE));
        assert!(fault.access_flags.contains(MappingFlags::EXECUTE));
        assert!(!fault.access_flags.contains(MappingFlags::READ));
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
        assert_eq!(vcpu.vmcb_pa, vcpu.vmcb.phys_addr().as_usize() as u64);
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

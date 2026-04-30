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

use bit_field::BitField;
use bitflags::bitflags;

bitflags! {
    /// VMCB control-area CR intercept bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct InterceptCr: u32 {
        const READ_CR0  = 1 << 0;
        const READ_CR3  = 1 << 3;
        const READ_CR4  = 1 << 4;
        const READ_CR8  = 1 << 8;
        const WRITE_CR0 = 1 << 16;
        const WRITE_CR3 = 1 << 19;
        const WRITE_CR4 = 1 << 20;
        const WRITE_CR8 = 1 << 24;
    }
}

bitflags! {
    /// VMCB control-area DR intercept bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct InterceptDr: u32 {
        const READ_DR0   = 1 << 0;
        const READ_DR1   = 1 << 1;
        const READ_DR2   = 1 << 2;
        const READ_DR3   = 1 << 3;
        const READ_DR6   = 1 << 6;
        const READ_DR7   = 1 << 7;
        const WRITE_DR0  = 1 << 16;
        const WRITE_DR1  = 1 << 17;
        const WRITE_DR2  = 1 << 18;
        const WRITE_DR3  = 1 << 19;
        const WRITE_DR6  = 1 << 22;
        const WRITE_DR7  = 1 << 23;
    }
}

bitflags! {
    /// VMCB control-area exception intercept bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct InterceptException: u32 {
        const DE = 1 << 0;
        const DB = 1 << 1;
        const NMI = 1 << 2;
        const BP = 1 << 3;
        const OF = 1 << 4;
        const BR = 1 << 5;
        const UD = 1 << 6;
        const NM = 1 << 7;
        const DF = 1 << 8;
        const TS = 1 << 10;
        const NP = 1 << 11;
        const SS = 1 << 12;
        const GP = 1 << 13;
        const PF = 1 << 14;
        const MF = 1 << 16;
        const AC = 1 << 17;
        const MC = 1 << 18;
        const XM = 1 << 19;
        const VE = 1 << 20;
        const CP = 1 << 21;
    }
}

bitflags! {
    /// VMCB control-area instruction intercept bits in the first 32-bit word.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct InterceptInst1: u32 {
        const INTR      = 1 << 0;
        const NMI       = 1 << 1;
        const SMI       = 1 << 2;
        const INIT      = 1 << 3;
        const VINTR     = 1 << 4;
        const CR0_SEL_WRITE = 1 << 5;
        const IDTR_READ = 1 << 6;
        const GDTR_READ = 1 << 7;
        const LDTR_READ = 1 << 8;
        const TR_READ   = 1 << 9;
        const IDTR_WRITE = 1 << 10;
        const GDTR_WRITE = 1 << 11;
        const LDTR_WRITE = 1 << 12;
        const TR_WRITE  = 1 << 13;
        const RDTSC     = 1 << 14;
        const RDPMC     = 1 << 15;
        const PUSHF     = 1 << 16;
        const POPF      = 1 << 17;
        const CPUID     = 1 << 18;
        const RSM       = 1 << 19;
        const IRET      = 1 << 20;
        const SWINT     = 1 << 21;
        const INVD      = 1 << 22;
        const PAUSE     = 1 << 23;
        const HLT       = 1 << 24;
        const INVLPG    = 1 << 25;
        const INVLPGA   = 1 << 26;
        const IOIO_PROT = 1 << 27;
        const MSR_PROT  = 1 << 28;
        const TASK_SWITCHES = 1 << 29;
        const FERR_FREEZE = 1 << 30;
        const SHUTDOWN  = 1 << 31;
    }
}

bitflags! {
    /// VMCB control-area instruction intercept bits in the second 32-bit word.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct InterceptInst2: u32 {
        const VMRUN     = 1 << 0;
        const VMMCALL   = 1 << 1;
        const VMLOAD    = 1 << 2;
        const VMSAVE    = 1 << 3;
        const STGI      = 1 << 4;
        const CLGI      = 1 << 5;
        const SKINIT    = 1 << 6;
        const RDTSCP    = 1 << 7;
        const ICEBP     = 1 << 8;
        const WBINVD    = 1 << 9;
        const MONITOR   = 1 << 10;
        const MWAIT     = 1 << 11;
        const MWAIT_CONDITIONAL = 1 << 12;
        const XSETBV    = 1 << 13;
        const RDPRU     = 1 << 14;
        const EFER_WRITE = 1 << 15;
        const CR_WRITE_TRAP = 1 << 16;
        const INVLPGB   = 1 << 17;
        const ILLEGAL_INVLPGB = 1 << 18;
        const PCOMMIT   = 1 << 19;
        const TLBSYNC   = 1 << 20;
        const BUS_LOCK  = 1 << 21;
        const IDLE_HLT  = 1 << 22;
    }
}

bitflags! {
    /// VMCB nested-paging control bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NestedPageControl: u64 {
        const ENABLE = 1 << 0;
    }
}

bitflags! {
    /// VMCB EVENTINJ field flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct EventInj: u64 {
        const ERROR_CODE_VALID = 1 << 11;
        const VALID = 1 << 31;
    }
}

/// VMCB event injection type encoded in EVENTINJ bits 10:8.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    ExternalInterrupt = 0,
    Nmi               = 2,
    Exception         = 3,
    SoftwareInterrupt = 4,
}

impl EventInj {
    pub fn new(vector: u8, event_type: EventType, error_code: Option<u32>) -> Self {
        let mut bits = vector as u64;
        bits.set_bits(8..11, event_type as u64);
        if let Some(error_code) = error_code {
            bits.set_bit(11, true);
            bits.set_bits(32..64, error_code as u64);
        }
        bits.set_bit(31, true);
        Self::from_bits_retain(bits)
    }

    pub fn vector(self) -> u8 {
        self.bits().get_bits(0..8) as u8
    }

    pub fn error_code(self) -> Option<u32> {
        self.contains(Self::ERROR_CODE_VALID)
            .then(|| self.bits().get_bits(32..64) as u32)
    }
}

/// VMCB virtual interrupt control field at offset 0x060.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VirtualInterruptControl {
    pub raw: u64,
}

/// VMCB segment register format.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VmcbSegment {
    pub selector: u16,
    pub attrib: u16,
    pub limit: u32,
    pub base: u64,
}

/// VMCB descriptor-table register format.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VmcbDescriptorTable {
    pub limit: u16,
    pub reserved: [u8; 6],
    pub base: u64,
}

/// VMCB control area. AMD APM Vol. 2, Appendix B.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmcbControlArea {
    pub intercept_cr: u32,           // 0x000
    pub intercept_dr: u32,           // 0x004
    pub intercept_exception: u32,    // 0x008
    pub intercept_instruction1: u32, // 0x00c
    pub intercept_instruction2: u32, // 0x010
    pub intercept_instruction3: u32, // 0x014
    reserved_018: [u8; 0x03c - 0x018],
    pub pause_filter_threshold: u16, // 0x03c
    pub pause_filter_count: u16,     // 0x03e
    pub iopm_base_pa: u64,           // 0x040
    pub msrpm_base_pa: u64,          // 0x048
    pub tsc_offset: u64,             // 0x050
    pub guest_asid: u32,             // 0x058
    pub tlb_control: u8,             // 0x05c
    reserved_05d: [u8; 3],
    pub virtual_interrupt: VirtualInterruptControl, // 0x060
    pub interrupt_shadow: u64,                      // 0x068
    pub exitcode: u64,                              // 0x070
    pub exitinfo1: u64,                             // 0x078
    pub exitinfo2: u64,                             // 0x080
    pub exitintinfo: u64,                           // 0x088
    pub nested_page_control: u64,                   // 0x090
    pub avic_apic_bar: u64,                         // 0x098
    pub ghcb: u64,                                  // 0x0a0
    pub eventinj: u64,                              // 0x0a8
    pub n_cr3: u64,                                 // 0x0b0
    pub lbr_virtualization_enable: u64,             // 0x0b8
    pub vmcb_clean_bits: u32,                       // 0x0c0
    reserved_0c4: [u8; 4],
    pub next_rip: u64,               // 0x0c8
    pub instruction_bytes: [u8; 16], // 0x0d0
    pub avic_apic_backing_page: u64, // 0x0e0
    reserved_0e8: [u8; 8],
    pub avic_logical_table: u64,  // 0x0f0
    pub avic_physical_table: u64, // 0x0f8
    reserved_100: [u8; 0x3e0 - 0x100],
    pub reserved_for_encrypted_state: u64, // 0x3e0
    reserved_3e8: [u8; 0x400 - 0x3e8],
}

impl Default for VmcbControlArea {
    fn default() -> Self {
        Self {
            intercept_cr: 0,
            intercept_dr: 0,
            intercept_exception: 0,
            intercept_instruction1: 0,
            intercept_instruction2: 0,
            intercept_instruction3: 0,
            reserved_018: [0; 0x03c - 0x018],
            pause_filter_threshold: 0,
            pause_filter_count: 0,
            iopm_base_pa: 0,
            msrpm_base_pa: 0,
            tsc_offset: 0,
            guest_asid: 0,
            tlb_control: 0,
            reserved_05d: [0; 3],
            virtual_interrupt: VirtualInterruptControl::default(),
            interrupt_shadow: 0,
            exitcode: 0,
            exitinfo1: 0,
            exitinfo2: 0,
            exitintinfo: 0,
            nested_page_control: 0,
            avic_apic_bar: 0,
            ghcb: 0,
            eventinj: 0,
            n_cr3: 0,
            lbr_virtualization_enable: 0,
            vmcb_clean_bits: 0,
            reserved_0c4: [0; 4],
            next_rip: 0,
            instruction_bytes: [0; 16],
            avic_apic_backing_page: 0,
            reserved_0e8: [0; 8],
            avic_logical_table: 0,
            avic_physical_table: 0,
            reserved_100: [0; 0x3e0 - 0x100],
            reserved_for_encrypted_state: 0,
            reserved_3e8: [0; 0x400 - 0x3e8],
        }
    }
}

/// VMCB state-save area. AMD APM Vol. 2, Appendix B.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmcbSaveArea {
    pub es: VmcbSegment,           // 0x400
    pub cs: VmcbSegment,           // 0x410
    pub ss: VmcbSegment,           // 0x420
    pub ds: VmcbSegment,           // 0x430
    pub fs: VmcbSegment,           // 0x440
    pub gs: VmcbSegment,           // 0x450
    pub gdtr: VmcbDescriptorTable, // 0x460
    pub ldtr: VmcbSegment,         // 0x470
    pub idtr: VmcbDescriptorTable, // 0x480
    pub tr: VmcbSegment,           // 0x490
    reserved_4a0: [u8; 0x4cb - 0x4a0],
    pub cpl: u8, // 0x4cb
    reserved_4cc: [u8; 4],
    pub efer: u64, // 0x4d0
    reserved_4d8: [u8; 0x548 - 0x4d8],
    pub cr4: u64,    // 0x548
    pub cr3: u64,    // 0x550
    pub cr0: u64,    // 0x558
    pub dr7: u64,    // 0x560
    pub dr6: u64,    // 0x568
    pub rflags: u64, // 0x570
    pub rip: u64,    // 0x578
    reserved_580: [u8; 0x5d8 - 0x580],
    pub rsp: u64, // 0x5d8
    reserved_5e0: [u8; 0x5f8 - 0x5e0],
    pub rax: u64,            // 0x5f8
    pub star: u64,           // 0x600
    pub lstar: u64,          // 0x608
    pub cstar: u64,          // 0x610
    pub sfmask: u64,         // 0x618
    pub kernel_gs_base: u64, // 0x620
    pub sysenter_cs: u64,    // 0x628
    pub sysenter_esp: u64,   // 0x630
    pub sysenter_eip: u64,   // 0x638
    pub cr2: u64,            // 0x640
    reserved_648: [u8; 0x668 - 0x648],
    pub gpat: u64,           // 0x668
    pub dbgctl: u64,         // 0x670
    pub br_from: u64,        // 0x678
    pub br_to: u64,          // 0x680
    pub last_excp_from: u64, // 0x688
    pub last_excp_to: u64,   // 0x690
    reserved_698: [u8; 0xc00 - 0x698],
}

impl Default for VmcbSaveArea {
    fn default() -> Self {
        Self {
            es: VmcbSegment::default(),
            cs: VmcbSegment::default(),
            ss: VmcbSegment::default(),
            ds: VmcbSegment::default(),
            fs: VmcbSegment::default(),
            gs: VmcbSegment::default(),
            gdtr: VmcbDescriptorTable::default(),
            ldtr: VmcbSegment::default(),
            idtr: VmcbDescriptorTable::default(),
            tr: VmcbSegment::default(),
            reserved_4a0: [0; 0x4cb - 0x4a0],
            cpl: 0,
            reserved_4cc: [0; 4],
            efer: 0,
            reserved_4d8: [0; 0x548 - 0x4d8],
            cr4: 0,
            cr3: 0,
            cr0: 0,
            dr7: 0,
            dr6: 0,
            rflags: 0,
            rip: 0,
            reserved_580: [0; 0x5d8 - 0x580],
            rsp: 0,
            reserved_5e0: [0; 0x5f8 - 0x5e0],
            rax: 0,
            star: 0,
            lstar: 0,
            cstar: 0,
            sfmask: 0,
            kernel_gs_base: 0,
            sysenter_cs: 0,
            sysenter_esp: 0,
            sysenter_eip: 0,
            cr2: 0,
            reserved_648: [0; 0x668 - 0x648],
            gpat: 0,
            dbgctl: 0,
            br_from: 0,
            br_to: 0,
            last_excp_from: 0,
            last_excp_to: 0,
            reserved_698: [0; 0xc00 - 0x698],
        }
    }
}

/// AMD SVM Virtual Machine Control Block.
#[repr(C, align(4096))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vmcb {
    pub control: VmcbControlArea,
    pub save: VmcbSaveArea,
    reserved: [u8; 0x1000 - 0xc00],
}

impl Default for Vmcb {
    fn default() -> Self {
        Self {
            control: VmcbControlArea::default(),
            save: VmcbSaveArea::default(),
            reserved: [0; 0x1000 - 0xc00],
        }
    }
}

numeric_enum_macro::numeric_enum! {
#[repr(u64)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[allow(non_camel_case_types)]
/// SVM VMEXIT exit codes.
pub enum SvmExitCode {
    CR_READ = 0x000,
    CR_WRITE = 0x010,
    DR_READ = 0x020,
    DR_WRITE = 0x030,
    EXCP_BASE = 0x040,
    INTR = 0x060,
    NMI = 0x061,
    SMI = 0x062,
    INIT = 0x063,
    VINTR = 0x064,
    CR0_SEL_WRITE = 0x065,
    IDTR_READ = 0x066,
    GDTR_READ = 0x067,
    LDTR_READ = 0x068,
    TR_READ = 0x069,
    IDTR_WRITE = 0x06a,
    GDTR_WRITE = 0x06b,
    LDTR_WRITE = 0x06c,
    TR_WRITE = 0x06d,
    RDTSC = 0x06e,
    RDPMC = 0x06f,
    PUSHF = 0x070,
    POPF = 0x071,
    CPUID = 0x072,
    RSM = 0x073,
    IRET = 0x074,
    SWINT = 0x075,
    INVD = 0x076,
    PAUSE = 0x077,
    HLT = 0x078,
    INVLPG = 0x079,
    INVLPGA = 0x07a,
    IOIO = 0x07b,
    MSR = 0x07c,
    TASK_SWITCH = 0x07d,
    FERR_FREEZE = 0x07e,
    SHUTDOWN = 0x07f,
    VMRUN = 0x080,
    VMMCALL = 0x081,
    VMLOAD = 0x082,
    VMSAVE = 0x083,
    STGI = 0x084,
    CLGI = 0x085,
    SKINIT = 0x086,
    RDTSCP = 0x087,
    ICEBP = 0x088,
    WBINVD = 0x089,
    MONITOR = 0x08a,
    MWAIT = 0x08b,
    MWAIT_CONDITIONAL = 0x08c,
    XSETBV = 0x08d,
    RDPRU = 0x08e,
    EFER_WRITE_TRAP = 0x08f,
    CR_WRITE_TRAP = 0x090,
    INVLPGB = 0x091,
    ILLEGAL_INVLPGB = 0x092,
    PCOMMIT = 0x093,
    TLBSYNC = 0x094,
    BUS_LOCK = 0x095,
    IDLE_HLT = 0x096,
    NPF = 0x400,
    AVIC_INCOMPLETE_IPI = 0x401,
    AVIC_NOACCEL = 0x402,
    VMGEXIT = 0x403,
    INVALID = 0xffff_ffff_ffff_ffff,
}
}

/// SVM VMEXIT information captured from VMCB control fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SvmExitInfo {
    pub exit_code: SvmExitCode,
    pub exitinfo1: u64,
    pub exitinfo2: u64,
    pub exitintinfo: u64,
    pub guest_rip: u64,
    pub next_rip: u64,
}

impl SvmExitInfo {
    pub fn from_vmcb(vmcb: &Vmcb) -> Self {
        let exit_code =
            SvmExitCode::try_from(vmcb.control.exitcode).unwrap_or(SvmExitCode::INVALID);
        Self {
            exit_code,
            exitinfo1: vmcb.control.exitinfo1,
            exitinfo2: vmcb.control.exitinfo2,
            exitintinfo: vmcb.control.exitintinfo,
            guest_rip: vmcb.save.rip,
            next_rip: vmcb.control.next_rip,
        }
    }
}

#[cfg(test)]
mod tests {
    use core::mem::{align_of, offset_of, size_of};

    use super::*;

    #[test]
    fn vmcb_layout_has_expected_size_and_alignment() {
        assert_eq!(size_of::<VmcbSegment>(), 16);
        assert_eq!(size_of::<VmcbDescriptorTable>(), 16);
        assert_eq!(size_of::<VmcbControlArea>(), 0x400);
        assert_eq!(size_of::<VmcbSaveArea>(), 0x800);
        assert_eq!(size_of::<Vmcb>(), 0x1000);
        assert_eq!(align_of::<Vmcb>(), 0x1000);
    }

    #[test]
    fn vmcb_control_offsets_match_apm() {
        assert_eq!(offset_of!(Vmcb, control), 0x000);
        assert_eq!(offset_of!(Vmcb, save), 0x400);
        assert_eq!(offset_of!(VmcbControlArea, intercept_cr), 0x000);
        assert_eq!(offset_of!(VmcbControlArea, intercept_dr), 0x004);
        assert_eq!(offset_of!(VmcbControlArea, intercept_exception), 0x008);
        assert_eq!(offset_of!(VmcbControlArea, intercept_instruction1), 0x00c);
        assert_eq!(offset_of!(VmcbControlArea, intercept_instruction2), 0x010);
        assert_eq!(offset_of!(VmcbControlArea, iopm_base_pa), 0x040);
        assert_eq!(offset_of!(VmcbControlArea, msrpm_base_pa), 0x048);
        assert_eq!(offset_of!(VmcbControlArea, tsc_offset), 0x050);
        assert_eq!(offset_of!(VmcbControlArea, guest_asid), 0x058);
        assert_eq!(offset_of!(VmcbControlArea, virtual_interrupt), 0x060);
        assert_eq!(offset_of!(VmcbControlArea, exitcode), 0x070);
        assert_eq!(offset_of!(VmcbControlArea, exitinfo1), 0x078);
        assert_eq!(offset_of!(VmcbControlArea, exitinfo2), 0x080);
        assert_eq!(offset_of!(VmcbControlArea, exitintinfo), 0x088);
        assert_eq!(offset_of!(VmcbControlArea, nested_page_control), 0x090);
        assert_eq!(offset_of!(VmcbControlArea, eventinj), 0x0a8);
        assert_eq!(offset_of!(VmcbControlArea, n_cr3), 0x0b0);
        assert_eq!(offset_of!(VmcbControlArea, vmcb_clean_bits), 0x0c0);
        assert_eq!(offset_of!(VmcbControlArea, next_rip), 0x0c8);
        assert_eq!(offset_of!(VmcbControlArea, instruction_bytes), 0x0d0);
    }

    #[test]
    fn vmcb_save_offsets_match_apm() {
        assert_eq!(offset_of!(VmcbSaveArea, es), 0x000);
        assert_eq!(offset_of!(VmcbSaveArea, cs), 0x010);
        assert_eq!(offset_of!(VmcbSaveArea, ss), 0x020);
        assert_eq!(offset_of!(VmcbSaveArea, ds), 0x030);
        assert_eq!(offset_of!(VmcbSaveArea, fs), 0x040);
        assert_eq!(offset_of!(VmcbSaveArea, gs), 0x050);
        assert_eq!(offset_of!(VmcbSaveArea, gdtr), 0x060);
        assert_eq!(offset_of!(VmcbSaveArea, ldtr), 0x070);
        assert_eq!(offset_of!(VmcbSaveArea, idtr), 0x080);
        assert_eq!(offset_of!(VmcbSaveArea, tr), 0x090);
        assert_eq!(offset_of!(VmcbSaveArea, cpl), 0x0cb);
        assert_eq!(offset_of!(VmcbSaveArea, efer), 0x0d0);
        assert_eq!(offset_of!(VmcbSaveArea, cr4), 0x148);
        assert_eq!(offset_of!(VmcbSaveArea, cr3), 0x150);
        assert_eq!(offset_of!(VmcbSaveArea, cr0), 0x158);
        assert_eq!(offset_of!(VmcbSaveArea, dr7), 0x160);
        assert_eq!(offset_of!(VmcbSaveArea, dr6), 0x168);
        assert_eq!(offset_of!(VmcbSaveArea, rflags), 0x170);
        assert_eq!(offset_of!(VmcbSaveArea, rip), 0x178);
        assert_eq!(offset_of!(VmcbSaveArea, rsp), 0x1d8);
        assert_eq!(offset_of!(VmcbSaveArea, rax), 0x1f8);
        assert_eq!(offset_of!(VmcbSaveArea, star), 0x200);
        assert_eq!(offset_of!(VmcbSaveArea, cr2), 0x240);
        assert_eq!(offset_of!(VmcbSaveArea, gpat), 0x268);
    }

    #[test]
    fn eventinj_encodes_vector_type_and_error_code() {
        let event = EventInj::new(14, EventType::Exception, Some(0x1234));

        assert!(event.contains(EventInj::VALID));
        assert_eq!(event.vector(), 14);
        assert_eq!(event.bits().get_bits(8..11), EventType::Exception as u64);
        assert_eq!(event.error_code(), Some(0x1234));
    }

    #[test]
    fn exit_info_reads_vmcb_fields() {
        let mut vmcb = Vmcb::default();
        vmcb.control.exitcode = SvmExitCode::NPF.into();
        vmcb.control.exitinfo1 = 1;
        vmcb.control.exitinfo2 = 0xdead_beef;
        vmcb.control.exitintinfo = 2;
        vmcb.control.next_rip = 0x3000;
        vmcb.save.rip = 0x2000;

        let info = SvmExitInfo::from_vmcb(&vmcb);
        assert_eq!(info.exit_code, SvmExitCode::NPF);
        assert_eq!(info.exitinfo1, 1);
        assert_eq!(info.exitinfo2, 0xdead_beef);
        assert_eq!(info.exitintinfo, 2);
        assert_eq!(info.guest_rip, 0x2000);
        assert_eq!(info.next_rip, 0x3000);
    }
}

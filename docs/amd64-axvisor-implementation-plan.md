# AMD64 (SVM) Axvisor 实现方案

> **目标**：让 axvisor 在 AMD x86_64 平台上具备与现有 Intel VMX 实现同等程度（对齐级）的虚拟化能力，能够启动 ArceOS 客户机（TCG 方式，不含 KVM）。

---

## 一、现有 Intel VMX 实现架构总览

在动手之前，先理解当前 Intel VMX 实现的完整结构。

### 1.1 代码分层

```
os/axvisor/                          ← VMM 应用层（调度、配置、镜像加载）
    src/vmm/
        mod.rs                       ← VMM 初始化/启动
        vcpus.rs                     ← VCpu 任务调度、WaitQueue
        images/mod.rs                ← 客户机镜像加载（kernel/ramdisk/dtb）
        config/                      ← VM 配置解析
        vm_list.rs                   ← VM 列表管理
    src/hal/
        arch/x86_64/mod.rs           ← x86_64 架构 HAL（当前为空壳）
        impl_vmm.rs                  ← VmmIf API 实现
        impl_memory.rs               ← 内存分配实现

components/axvm/                     ← 架构无关的 VM 抽象
    src/vcpu.rs                      ← 按 arch 绑定 AxArchVCpuImpl
    src/vm.rs                        ← AxVM 核心：地址空间、内存区域、设备

components/axvcpu/                   ← 架构无关的 VCpu 抽象
    src/vcpu.rs                      ← AxVCpu<A: AxArchVCpu> 状态机
    src/arch_vcpu.rs                 ← AxArchVCpu trait 定义
    src/percpu.rs                    ← AxArchPerCpu trait 定义
    src/exit.rs                      ← AxVCpuExitReason 枚举

components/x86_vcpu/                 ← Intel VMX 具体实现
    src/lib.rs                       ← feature="vmx" 条件编译入口
    src/vmx/
        mod.rs                       ← 模块入口（has_hardware_support）
        percpu.rs                    ← VmxPerCpuState（VMXON/VMXOFF）
        vcpu.rs                      ← VmxVcpu（VMLAUNCH/VMRESUME/VM-exit）
        vmcs.rs                      ← VMCS 字段定义、exit info 结构
        structs.rs                   ← VmxRegion/IOBitmap/MsrBitmap 等
        definitions.rs               ← VmxExitReason/VmxInstructionError
        instructions.rs              ← INVEPT 等指令
    src/regs/mod.rs                  ← GeneralRegisters + 汇编宏
    src/msr.rs                       ← MSR 常量与读写
    src/ept.rs                       ← GuestPageWalkInfo
```

### 1.2 关键抽象接口

**`AxArchVCpu` trait**（`axvcpu/src/arch_vcpu.rs`）：
- `new(vm_id, vcpu_id, CreateConfig)` → Self
- `set_entry(entry: GuestPhysAddr)` / `set_ept_root(root: HostPhysAddr)`
- `setup(SetupConfig)` → 完成 VMCS/VMCB 初始化
- `run()` → 进入客户机，返回 `AxVCpuExitReason`
- `bind()` / `unbind()` → CPU 绑定/解绑
- `set_gpr(reg, val)` / `inject_interrupt(vector)` / `set_return_value(val)`

**`AxArchPerCpu` trait**（`axvcpu/src/percpu.rs`）：
- `new(cpu_id)` / `is_enabled()` / `hardware_enable()` / `hardware_disable()`

**`AxVCpuExitReason`**（`axvcpu/src/exit.rs`）：
- `Nothing` / `Hypercall` / `IoRead` / `IoWrite` / `SysRegRead` / `SysRegWrite`
- `ExternalInterrupt` / `FailEntry` / `Halt` / `SystemDown` / `NestedPageFault` / `EmulatedDevice`

### 1.3 Intel VMX 实现要点速查

| 要点 | VMX 方式 |
|------|---------|
| 硬件检测 | CPUID.1.ECX[5] (has_vmx) |
| 开启虚拟化 | CR4.VMXE=1, IA32_FEATURE_CONTROL lock, VMXON |
| 关闭虚拟化 | VMXOFF, CR4.VMXE=0 |
| 控制结构 | VMCS（4KB，含 host/guest/control 字段），通过 VMREAD/VMWRITE 访问 |
| 进入客户机 | VMLAUNCH（首次）/ VMRESUME（后续），通过汇编 naked 函数 |
| 退出客户机 | VM-exit，在 vmx_exit 中保存/恢复寄存器 |
| 二级地址翻译 | EPT（Extended Page Table），4 级页表，通过 EPTP 指向 |
| IO 拦截 | I/O Bitmap A/B（各 4KB） |
| MSR 拦截 | MSR Bitmap（4KB，分读/写，低/高 MSR 区域） |
| 中断注入 | VM-entry interruption-information 字段 |
| 中断窗口 | Primary Controls.INTERRUPT_WINDOW_EXITING |
| CPUID 处理 | VmxVcpu::handle_cpuid() 过滤 VMX bit、隐藏 LA57 等 |
| XSAVE/XRSTOR | XState 管理 host/guest XCR0、XSS 切换 |
| CR 拦截 | CR0/CR4 通过 shadow+mask 实现，CR3 直通 |

---

## 二、AMD SVM 与 Intel VMX 核心差异对照

| 维度 | Intel VMX | AMD SVM |
|------|-----------|---------|
| **硬件检测** | CPUID.1.ECX[5] `has_vmx()` | CPUID.8000_0001.ECX[2] `has_svm()` |
| **开启虚拟化** | CR4.VMXE=1 → VMXON | EFER.SVME=1 → 无需单独指令 |
| **控制结构** | VMCS（4KB），通过 VMREAD/VMWRITE 特殊指令读写 | VMCB（4KB），通过普通内存读写直接访问 |
| **进入客户机** | VMLAUNCH / VMRESUME | VMRUN（统一指令） |
| **退出客户机** | VM-exit → host RIP 由 VMCS 指定 | `#VMEXIT` → 从 VMCB 恢复 host 状态 |
| **状态保存** | VMCS 中 host-state area + guest-state area | VMCB 中 `vmcb_save_area`（guest 状态）+ 物理寄存器（host 状态自动保存/恢复） |
| **二级地址翻译** | EPT，VMCS 中 EPTP 字段 | NPT，VMCB 中 `ncr3` 字段，页表格式相同 |
| **TLB 标记** | VPID | ASID（Address Space ID） |
| **IO 拦截** | I/O Bitmap A/B（各 4KB） | IOPM（I/O Permission Map，共 4KB，覆盖 0~0xFFFF 需 12KB） |
| **MSR 拦截** | MSR Bitmap（4KB） | MSRPM（MSR Permission Map，共 8KB） |
| **中断注入** | VM-entry interruption-info 字段 | VMCB 中 `eventinj` 字段 |
| **中断窗口** | INTERRUPT_WINDOW_EXITING control | VMCB 中 `v_irq` / `v_intr` 虚拟中断机制 |
| **NPT 页故障** | VM-exit reason=48（EPT violation） | `#VMEXIT` exitcode=0x400（NPT FAULT） |
| **CPUID 拦截** | Primary Controls 开启即可 | VMCB 中 intercept 位图控制 |
| **XSAVE** | 通过 VMCS host/guest XCR0 管理 | 由 VMRUN 自动保存/恢复（host） |
| **预制定时器** | VMX-preemption timer | 无直接等价，需用外部方式替代 |
| **TLB 刷新** | INVEPT 指令 | 不需要单独指令（NPT 由硬件管理） |

---

## 三、分阶段实现方案

下面将整体工作拆分为 9 个阶段，每个阶段产出独立、可验证、体积小。

---

### 阶段一：创建 `svm` feature 骨架与编译框架

**目标**：让 `x86_vcpu` crate 在 `feature="svm"` 时能编译通过，提供一个空壳实现。

**涉及文件**：
- `components/x86_vcpu/Cargo.toml` — 添加 `svm` feature
- `components/x86_vcpu/src/lib.rs` — 添加 `cfg(feature = "svm")` 分支

**具体工作**：

1. 在 `Cargo.toml` 中新增 feature：
   ```toml
   [features]
   vmx = ["x86"]
   svm = []
   ```

2. 在 `lib.rs` 中将现有的 `cfg_if!` 扩展为支持 svm 分支：
   ```rust
   cfg_if::cfg_if! {
       if #[cfg(feature = "vmx")] {
           mod vmx;
           use vmx as vender;
           pub use vmx::{VmxExitInfo, ...};
           pub use vender::VmxArchVCpu;
           pub use vender::VmxArchPerCpuState;
       } else if #[cfg(feature = "svm")] {
           mod svm;                          // 新建模块
           use svm as vender;
           pub use svm::{SvmExitInfo, ...};  // 占位类型
           pub use vender::SvmArchVCpu;
           pub use vender::SvmArchPerCpuState;
       }
   }
   ```

3. 创建 `components/x86_vcpu/src/svm/mod.rs`，包含占位实现：
   - `pub struct SvmVcpu;` — 占位结构体，implements `AxArchVCpu`
   - `pub struct SvmArchPerCpuState;` — 占位，implements `AxArchPerCpu`
   - `pub fn has_hardware_support() -> bool { false }` — 暂返回 false
   - 占位类型 `SvmExitInfo`、`SvmInterruptInfo`、`SvmIoExitInfo`

4. 所有 `AxArchVCpu` trait 方法返回 `ax_err!(Unsupported, ...)`，让编译通过即可。

5. 同步修改 `components/axvm/src/vcpu.rs`：
   ```rust
   if #[cfg(target_arch = "x86_64")] {
       cfg_if::cfg_if! {
           if #[cfg(feature = "vmx")] {
               pub use x86_vcpu::VmxArchVCpu as AxArchVCpuImpl;
               pub use x86_vcpu::VmxArchPerCpuState as AxVMArchPerCpuImpl;
           } else if #[cfg(feature = "svm")] {
               pub use x86_vcpu::SvmArchVCpu as AxArchVCpuImpl;
               pub use x86_vcpu::SvmArchPerCpuState as AxVMArchPerCpuImpl;
           }
       }
       // 公共部分不变：
       pub use x86_vcpu::has_hardware_support;
       pub type AxVCpuCreateConfig = ();
       pub fn max_guest_page_table_levels() -> usize { 4 }
   }
   ```

**验证方式**：`cargo xtask clippy --package x86_vcpu` 在开启 `--features svm` 后通过。

---

### 阶段二：实现 SVM 硬件检测与 CPU 能力查询

**目标**：正确检测 AMD SVM 硬件能力，实现 `has_hardware_support()` 和 MSR 能力读取。

**涉及文件**：
- `components/x86_vcpu/src/svm/mod.rs`
- `components/x86_vcpu/src/svm/cpuid.rs`（新增）
- `components/x86_vcpu/src/msr.rs`（扩展 AMD MSR 定义）

**具体工作**：

1. **实现 `has_hardware_support()`**：
   ```rust
   pub fn has_hardware_support() -> bool {
       CpuId::new()
           .get_extended_processor_and_feature_identifiers()
           .map(|f| f.has_svm())
           .unwrap_or(false)
   }
   ```
   - CPUID Fn8000_0001.ECX[2] = SVM 支持标志

2. **扩展 MSR 定义**（`msr.rs`），新增 AMD SVM 相关 MSR：
   ```rust
   VM_CR        = 0xc001_0114,  // SVM 全局控制
   VM_HSAVE_PA  = 0xc001_0117,  // Host save 物理地址（用于 SMM）
   EFER         = 0xc000_0080,  // 已存在，SVM 需要 SVME bit
   ```
   以及 MSR 范围 `0xc001_0000~0xc001_1fff` 的其他 SVM MSR。

3. **读取 SVM 能力位**：
   - CPUID Fn8000_000A（SVM revision and feature identification）
   - 解析 NPT 支持、NRIPS（Next RIP save）、SSS（Selective Save State）、虚拟 GIF 等能力
   - 创建 `SvmCapabilities` 结构体记录这些信息

4. **创建 `cpuid.rs`** 模块，封装 AMD CPUID 查询函数：
   ```rust
   pub fn svm_revision() -> (u8, u8)  // (minor, major)
   pub fn svm_features() -> SvmFeatures
   pub fn np_supported() -> bool
   pub fn nrip_supported() -> bool
   pub fn asid_count() -> u32
   ```

**验证方式**：在 AMD 机器上运行简单的检测代码，确认能正确识别 SVM 可用性并打印能力信息。

---

### 阶段三：实现 `SvmPerCpuState` — SVM 启用与禁用

**目标**：实现 per-CPU 的 SVM 开启/关闭流程（对标 `VmxPerCpuState`）。

**涉及文件**：
- `components/x86_vcpu/src/svm/percpu.rs`（新增）

**具体工作**：

1. **结构体设计**（对标 VMX 的 VMXON region）：
   ```rust
   pub struct SvmPerCpuState {
       /// Host save area physical address (VMCB 不单独需要 VMXON region，
       /// SVM 仅需要设置 EFER.SVME)
       vm_hsave_pa: HostPhysAddr,
   }
   ```

2. **`hardware_enable()` 实现**：
   - Step 1: 检查 CPUID Fn8000_0001.ECX[2]（has_svm）
   - Step 2: 设置 EFER.SVME bit（`Msr::IA32_EFER` 的 bit 12）
   - Step 3: 设置 `VM_HSAVE_PA` MSR 指向一个 4KB 物理页（SMM 使用，即使不用 SMM 也建议设置）
   - Step 4: 检查 `VM_CR.SVMDIS` 为 0（BIOS 可能禁用 SVM）
   - Step 5: 配置 `VM_CR` 中必要的全局控制位（如 `LOCK` = 1 锁定配置）

3. **`hardware_disable()` 实现**：
   - Step 1: 清除 EFER.SVME bit
   - Step 2: 释放 VM_HSAVE_PA 页面

4. **`is_enabled()` 实现**：
   - 检查 EFER.SVME 是否为 1

**关键对比 VMX**：
| 步骤 | VMX | SVM |
|------|-----|-----|
| 开启 | CR4.VMXE=1 → VMXON(VMXON Region PA) | EFER.SVME=1, VM_HSAVE_PA 设置 |
| 关闭 | VMXOFF → CR4.VMXE=0 | EFER.SVME=0 |
| 检测 | CR4.VMXE 标志 | EFER.SVME 标志 |

**验证方式**：`hardware_enable()` 后读 EFER 确认 SVME=1，`hardware_disable()` 后确认 SVME=0。单步测试即可。

---

### 阶段四：VMCB 数据结构定义

**目标**：完整定义 VMCB（Virtual Machine Control Block）的所有字段（对标 VMX VMCS 定义）。

**涉及文件**：
- `components/x86_vcpu/src/svm/vmcb.rs`（新增）

**具体工作**：

VMCB 在 AMD 手册（APM Vol.2, Appendix B）中有完整定义。核心结构如下：

```rust
/// VMCB 总大小：4KB（第 0 页为控制区，第 1 页可选中断控制）
#[repr(C, align(4096))]
pub struct Vmcb {
    /// VMCB Control Area (offset 0x000 ~ 0x400)
    pub control: VmcbControl,
    /// VMCB State Save Area (offset 0x400 ~ 0xC00)
    pub save: VmcbSaveArea,
}
```

**VMCB Control Area 关键字段**（对标 VMCS Control Fields）：

| 偏移 | 字段 | 说明 | VMX 对应 |
|------|------|------|----------|
| 0x000 | `intercept_cr` | CR 拦截位图 | CR mask/shadow |
| 0x004 | `intercept_dr` | DR 拦截位图 | — |
| 0x008 | `intercept_exception` | 异常拦截位图 | Exception Bitmap |
| 0x00C | `intercept_instruction1` | 指令拦截位图 1 | Primary Controls |
| 0x010 | `intercept_instruction2` | 指令拦截位图 2 | Secondary Controls |
| 0x048 | `iopm_base_pa` | IOPM 物理地址 | IO_BITMAP_A/B_ADDR |
| 0x050 | `msrpm_base_pa` | MSRPM 物理地址 | MSR_BITMAPS_ADDR |
| 0x058 | `tsch_offset` | TSC 偏移 | TSC_OFFSET |
| 0x068 | `asid` | Address Space ID | VPID |
| 0x090 | `ncr3` | Nested Page Table CR3 | EPTP |
| 0x0A0 | `eventinj` | 事件注入 | VM-entry interruption-info |
| 0x0A8 | `v_intr` | 虚拟中断控制 | — |
| 0x0B0 | `avic_*` | AVIC 相关（暂忽略） | — |

**VMCB State Save Area 关键字段**（对标 VMCS Guest-State）：

| 偏移 | 字段 | 说明 |
|------|------|------|
| 0x400 | `es.sel/attrib/limit/base` | ES 段寄存器 |
| 0x410 | `cs.sel/attrib/limit/base` | CS 段寄存器 |
| ... | ... | SS/DS/FS/GS/LDTR/TR |
| 0x480 | `gdtr` / `idtr` | GDT/IDT 限定+基址 |
| 0x4C0 | `cr0/cr2/cr3/cr4` | 控制寄存器 |
| 0x500 | `dr6/dr7` | 调试寄存器 |
| 0x520 | `efer` | Extended Feature Enable |
| 0x580 | `rsp/rip/rflags` | 通用状态 |
| 0x5C0 | `rax` | RAX 寄存器 |
| 0x5C8 | `star/lstar/cstar/sfmask` | syscall MSR |
| 0x668 | `gpat` | Guest PAT |
| 0x678 | `next_rip` | NRIPS 保存的 next RIP（若支持） |
| 0x67C | `exitcode` | #VMEXIT 退出原因码 |
| 0x680 | `exitinfo1/exitinfo2` | 退出附加信息 |

**工作内容**：

1. 用 `repr(C)` 精确按偏移定义所有结构体字段
2. 定义截取位图的 bitflag 类型：
   - `InterceptCr` / `InterceptDr` / `InterceptException`
   - `InterceptInst1` / `InterceptInst2`
   - `EventInj`（事件注入格式）
3. 定义 `SvmExitCode` 枚举（对标 `VmxExitReason`）：
   ```rust
   pub enum SvmExitCode {
       CR_READ          = 0x00,
       CR_WRITE         = 0x01,
       DR_READ          = 0x02,
       DR_WRITE         = 0x03,
       EXCP_BASE        = 0x40,  // 0x40+x 为第 x 号异常
       INTR             = 0x60,  // 外部中断
       NMI              = 0x61,
       SMI              = 0x62,
       INIT             = 0x63,
       VINTR            = 0x64,
       HLT              = 0x78,
       INVLPG           = 0x79,
       IOIO             = 0x7B,
       MSR              = 0x7C,
       SHUTDOWN         = 0x7F,
       VMRUN            = 0x80,
       VMMCALL          = 0x81,
       VMLOAD           = 0x82,
       VMSAVE           = 0x83,
       STGI             = 0x84,
       CLGI             = 0x85,
       SKINIT           = 0x86,
       RDTSCP           = 0x87,
       WBINVD           = 0x8A,
       MONITOR          = 0x8D,
       MWAIT_UNCOND     = 0x8E,
       NPF              = 0x400,
       // ... 更多
   }
   ```

4. 定义 `SvmExitInfo` 结构体（对标 `VmxExitInfo`）

**验证方式**：确认结构体 `size_of::<Vmcb>() == 4096`，关键字段偏移与 AMD APM 手册匹配。

---

### 阶段五：实现 `SvmVcpu` 基础结构与 VMCB 初始化

**目标**：实现 `SvmVcpu` 结构体，完成 VMCB 的内存分配和基本初始化设置（对标 `VmxVcpu::setup_vmcs` 的 guest/host/control 设置）。

**涉及文件**：
- `components/x86_vcpu/src/svm/vcpu.rs`（新增）

**具体工作**：

1. **`SvmVcpu` 结构体**（对标 `VmxVcpu`）：
   ```rust
   #[repr(C)]
   pub struct SvmVcpu {
       // 必须是最前面两个字段（用于汇编 save/restore）
       guest_regs: GeneralRegisters,
       host_stack_top: u64,

       // VMCB 与 bitmap
       vmcb: VmcbPhysAddr,       // 指向 4KB 对齐物理页
       iopm: Iopm,               // I/O Permission Map（12KB）
       msrpm: Msrpm,             // MSR Permission Map（8KB）

       // 状态
       launched: bool,           // 是否已经 VMRUN（SVM 中无首次/后续区分，但保留）
       entry: Option<GuestPhysAddr>,
       ept_root: Option<HostPhysAddr>,

       // 中断
       pending_events: VecDeque<(u8, Option<u32>)>,

       // vlapic
       vlapic: EmulatedLocalApic,

       // XSAVE 状态管理（与 VMX 相同）
       xstate: XState,

       vm_id: VMId,
       vcpu_id: VCpuId,
   }
   ```

2. **`VmcbPhysAddr`** — VMCB 物理页管理：
   - 分配一个 4KB 对齐的 `PhysFrame`
   - 提供 `as_mut_ptr()` 返回 `*mut Vmcb` 用于直接读写
   - 提供 `pa()` 返回 `HostPhysAddr`

3. **VMCB 初始化 `setup_vmcb(entry, ept_root)`**：
   - **Guest 状态初始化**（Save Area）：
     - `cs.sel=0, cs.attrib=0x9b, cs.limit=0xffff, cs.base=0`
     - 设置所有段寄存器（ES/CS/SS/DS/FS/GS）的默认值
     - `cr0 = ET|NW|CD|EXT_TYPE`，`cr4 = 0`，`cr3 = 0`
     - `rflags = 0x2`
     - `rip = entry`，`rsp = 0`
     - `gpat = host PAT`
     - `efer = 0`
     - GDTR/IDTR base=0, limit=0xffff

   - **Control Area 初始化**：
     - 设置 Intercept 位图（对标 `setup_vmcs_control`）：
       - `intercept_instruction1` 开启：INTR, NMI, HLT, IOIO, MSR, CPUID, VMMCALL, RDTSCP 等
       - `intercept_instruction2` 开启：VMRUN, VMLOAD/VMSAVE, WBINVD 等
       - `intercept_exception` 至少拦截 #UD(6)
       - 设置 `ncr3 = ept_root`
       - 设置 `iopm_base_pa` / `msrpm_base_pa`
       - 设置 `asid = 1`（初始值）
       - 设置 `tsch_offset = 0`

   - **中断控制**（对标 `setup_vmcs_control` 中的 Pinbased Controls）：
     - 开启外部中断拦截（`v_irq` 机制）
     - 开启 NMI 拦截

4. **IOPM 初始化**（对标 `IOBitmap`）：
   - IOPM 覆盖端口 0x0000~0xFFFF，每个端口 1 bit，共 12KB
   - 默认全部直通（全 0），拦截 QEMU exit 端口 0x604

5. **MSRPM 初始化**（对标 `MsrBitmap`）：
   - MSRPM 覆盖 MSR 范围 0x0000_0000~0x0000_1FFF 和 0xC000_0000~0xC001_1FFF
   - 每个 MSR 有读/写各 1 bit，共 2×2×2KB = 8KB
   - 拦截 x2APIC MSR（0x800~0x8FF）
   - 拦截 `IA32_UMWAIT_CONTROL`（0xe1）

**验证方式**：`SvmVcpu::new()` 成功分配 VMCB 页，`setup_vmcb()` 后读取 VMCB 字段确认值与预期一致。

---

### 阶段六：实现 VMRUN 进入/退出汇编与寄存器保存/恢复

**目标**：实现 SVM 的客户机进入/退出汇编代码（对标 VMX 的 `vmx_launch/vmx_resume/vmx_exit`）。

**涉及文件**：
- `components/x86_vcpu/src/svm/vcpu.rs`（汇编部分）

**具体工作**：

1. **VMRUN 进入汇编**（对标 `vmx_entry_with!`）：

```
vmrun_entry:
    save host regs to stack       ; push rax, rcx, rdx, ... r15
    mov [rdi + host_stack_off], rsp  ; 保存 host RSP
    mov rsp, rdi                     ; RSP 指向 guest_regs
    restore guest regs from stack    ; pop rax, rcx, ... r15
    mov rax, [rdi + vmcb_pa_off]    ; RAX = VMCB 物理地址
    vmrun                            ; VMRUN (RAX 指向 VMCB)
    ; ⬆ 从 #VMEXIT 返回后在这里继续
```

2. **#VMEXIT 返回处理**（对标 `vmx_exit`）：

```
vmrun_exit:
    save guest regs to stack        ; push rax, rcx, ... r15
    mov rsp, [rsp + host_stack_off] ; RSP = host_stack_top
    restore host regs from stack    ; pop rax, rcx, ... r15
    ret
```

3. **使用 `naked_asm!` 宏**，与现有 VMX 保持一致的风格。

4. **关键差异**：
   - VMX 需要区分首次 VMLAUNCH 和后续 VMRESUME；SVM 统一使用 VMRUN
   - VMRUN 前 RAX 必须指向 VMCB 的物理地址
   - `#VMEXIT` 后不区分 entry failure，exitcode=0xFFFFFFFF 表示 VMRUN 失败
   - SVM 不自动保存 host 的 RAX, RCX, RDX, R8-R15，需要手动 save/restore（RSP 和 RFLAGS 由 VMRUN 自动处理）

**验证方式**：编写单元测试，用 mock 方式验证栈切换和寄存器保存/恢复逻辑。

---

### 阶段七：实现 VM-Exit 处理与 `AxArchVCpu` trait

**目标**：实现 `SvmVcpu` 的 `run()` 方法和 exit handler（对标 `VmxVcpu::inner_run` 和 `builtin_vmexit_handler`），完成 `AxArchVCpu` trait 实现。

**涉及文件**：
- `components/x86_vcpu/src/svm/vcpu.rs`

**具体工作**：

1. **内建 VM-Exit 处理器** `builtin_vmexit_handler(exit_code: SvmExitCode)`：
   - `VMMCALL` → 自身处理（类似 VMX VMCALL）
   - `CPUID` → 转调 `handle_cpuid()`（与 VMX 共用逻辑，提取到公共模块）
   - `MSR` → 判断是否为 x2APIC MSR，如果是则本地处理 vlapic
   - `IOIO` → 判断是否为 I/O 拦截（对标 VMX IO_INSTRUCTION）
   - `INTR` → 标记为外部中断，返回给上层
   - `NMI` → 暂忽略
   - `HLT` → 挂起
   - `NPF` → 返回嵌套页故障信息给上层
   - 其他 → 返回未处理

2. **Exit 信息解析**：
   - 从 VMCB Save Area 读取 `exitcode`、`exitinfo1`、`exitinfo2`、`next_rip`
   - 构建对应的 exit info 结构

3. **`run()` 方法**（`AxArchVCpu::run` 实现）：
   ```rust
   fn run(&mut self) -> AxResult<AxVCpuExitReason> {
       // 1. 注入 pending events
       self.inject_pending_events()?;
       // 2. 切换 XSAVE 状态
       self.load_guest_xstate();
       // 3. 执行 VMRUN
       unsafe { self.vmrun(); }
       // 4. 恢复 host XSAVE
       self.load_host_xstate();
       // 5. 读取 exitcode
       let exit_code = self.read_exitcode();
       // 6. 分发处理
       match self.builtin_vmexit_handler(exit_code) {
           Some(result) => // 已内部处理，返回 Nothing 或 Halt
           None => match exit_code {
               VMMCALL => AxVCpuExitReason::Hypercall { ... },
               IOIO => AxVCpuExitReason::IoRead/Write { ... },
               INTR => AxVCpuExitReason::ExternalInterrupt { ... },
               MSR => AxVCpuExitReason::SysRegRead/Write { ... },
               NPF => AxVCpuExitReason::NestedPageFault { ... },
               _ => AxVCpuExitReason::Halt,
           }
       }
   }
   ```

4. **其他 `AxArchVCpu` 方法**：
   - `new()` / `set_entry()` / `set_ept_root()` / `setup()` → 已在前阶段实现
   - `bind()` / `unbind()` → 暂为空（SVM VMRUN 不绑定特定 CPU）
   - `set_gpr(reg, val)` → 修改 `guest_regs` 或 VMCB save area
   - `inject_interrupt(vector)` → 写入 VMCB `eventinj` 字段
   - `set_return_value(val)` → 设置 RAX

5. **IO 信息解析**（对标 `VmxIoExitInfo`）：
   - `exitinfo1` bit [2:0] = 访问宽度，bit 3 = STR，bit 4 = REP，bit 5 = IN/OUT 方向
   - `exitinfo1` bit [31:16] = port 号
   - 构建 IO exit info 结构

**验证方式**：在 AMD 机器上执行 `SvmVcpu::run()`，期待第一个 VM-Exit 是某种拦截事件或 `#UD` 异常。

---

### 阶段八：集成到 axvm/axvisor 框架

**目标**：将 SVM Vcpu 接入 axvm 和 axvisor 的主流程，使 VMM 能够创建 AMD SVM 客户机。

**涉及文件**：
- `components/axvm/src/vcpu.rs` — feature 条件选择（已在阶段一准备）
- `components/axvm/Cargo.toml` — svm feature 传导
- `os/axvisor/Cargo.toml` — 添加 svm feature
- `os/axvisor/src/hal/arch/x86_64/mod.rs` — 实现 `hardware_check()`
- `os/axvisor/configs/board/` — 可能需要新增 amd 板级配置

**具体工作**：

1. **`axvm/Cargo.toml`** feature 传导：
   ```toml
   [features]
   svm = ["x86_vcpu/svm"]
   vmx = ["x86_vcpu/vmx"]
   ```

2. **`axvisor/Cargo.toml`** 添加编译选项：
   ```toml
   [features]
   svm = ["axvm/svm"]
   vmx = ["axvm/vmx"]
   ```

3. **`hal/arch/x86_64/mod.rs`** — 区分 vmx/svm：
   ```rust
   #[cfg(feature = "vmx")]
   pub fn hardware_check() {
       if !x86_vcpu::has_hardware_support() {
           panic!("CPU does not support VMX");
       }
   }

   #[cfg(feature = "svm")]
   pub fn hardware_check() {
       if !x86_vcpu::has_hardware_support() {
           panic!("CPU does not support SVM");
       }
   }
   ```

4. **NPT 页表设置**：确认 ncr3 格式正确
   - NPT 使用与 EPT 相同的 4 级页表格式
   - `ncr3` 指向 PML4 表基址
   - 不需要 INVEPT 等效操作（NPT 硬件 TLB 由 ASID 管理）

5. **ASID 分配**：初始使用 ASID=1，后续考虑 ASID 分配策略

6. **确保所有 `AxArchVCpu` trait 方法正确实现**，特别是：
   - `inject_interrupt()` — 正确写入 `eventinj`
   - `set_gpr()` — 修改 `guest_regs` 和 VMCB save area

**验证方式**：
- `cargo xtask clippy --package axvisor --features svm` 通过
- 在 AMD 机器上启动 axvisor，观察是否能进入客户机初始化流程

---

### 阶段九：客户机启动调试与对齐 Intel 能力

**目标**：让 ArceOS 客户机在 AMD SVM 上成功启动（对标 Intel 的已有能力），不做额外的功能扩展。

**涉及文件**：
- `components/x86_vcpu/src/svm/vcpu.rs` — VM-Exit 处理完善
- `os/axvisor/configs/` — AMD 测试配置文件

**具体工作**：

1. **补齐内建 VM-Exit 处理**，确保 ArceOS 启动过程中遇到的 exit 都能处理：
   - `CPUID` — 过滤功能位（隐藏 SVM bit、隐藏 LA57 等），对标 VMX `handle_cpuid()`
   - `CR_READ/CR_WRITE` — 正确处理 guest CR 读写
   - `MSR` — 区分 x2APIC MSR（本地处理）和其他 MSR（传给上层）
   - `RDTSCP` — 直通/模拟
   - `NPF` — 正确返回嵌套页故障信息给 axvm 处理
   - `HLT` — idle 处理
   - `INVLPG` — 无需处理（NPT 硬件管理）
   - `SHUTDOWN` — 视为 halt

2. **修复启动过程中发现的问题**：
   - 段寄存器初始化值是否符合 AMD 实际预期
   - CR0/CR4 的拦截与透传是否正确
   - NPT 页故障参数传递是否正确

3. **创建测试配置**：
   - `os/axvisor/configs/board/qemu-amd64.toml`（QEMU 模拟 AMD CPU 配置）
   - 测试用 VM 配置（复用现有 arceos-x86_64 配置）

4. **QEMU 测试命令示例**：
   ```bash
   qemu-system-x86_64 \
       -cpu EPYC-v4 \           # AMD EPYC CPU 模型（启用 SVM）
       -enable-kvm \            # 或 TCG（自动检测）
       -machine q35 \
       -smp 4 \
       -m 4G \
       -kernel <axvisor binary> \
       ...
   ```

5. **对齐清单**（Intel 已有 / AMD 需做到）：
   - [x] 客户机启动到 ArceOS shell
   - [x] 基本 I/O 操作（串口输出）
   - [x] CPUID 虚拟化（隐藏 hypervisor feature）
   - [x] x2APIC MSR 模拟
   - [x] EPT/NPT 二级地址翻译
   - [x] 外部中断注入
   - [x] 系统关机（QEMU exit port）
   - [ ] VMX preemption timer → SVM 暂无等价，用其他方案替代或忽略
   - [ ] 多 vCPU（暂不要求，Intel 也未完善）

6. **调试技巧**：
   - 在 VMCB save area 的 `exitinfo1/exitinfo2` 中读取详细错误原因
   - NPF 故障信息在 `exitinfo1`（guest physical addr）和 `exitinfo2`（error code）
   - 用 `qemu -d cpu_reset,int,mmu -D qemu.log` 对比参考日志

**验证方式**：QEMU 中使用 AMD CPU 模型启动 axvisor + ArceOS 客户机，客户机能到达 shell 交互状态。

---

## 四、文件变更总览

| 文件 | 操作 | 所在阶段 |
|------|------|----------|
| `components/x86_vcpu/Cargo.toml` | 修改（添加 svm feature） | S1 |
| `components/x86_vcpu/src/lib.rs` | 修改（svm 分支） | S1 |
| `components/x86_vcpu/src/svm/mod.rs` | **新增**（模块入口） | S1, S2 |
| `components/x86_vcpu/src/svm/cpuid.rs` | **新增**（CPU 能力检测） | S2 |
| `components/x86_vcpu/src/msr.rs` | 修改（AMD MSR 定义） | S2 |
| `components/x86_vcpu/src/svm/percpu.rs` | **新增**（per-CPU SVM 管理） | S3 |
| `components/x86_vcpu/src/svm/vmcb.rs` | **新增**（VMCB 数据结构） | S4 |
| `components/x86_vcpu/src/svm/vcpu.rs` | **新增**（SvmVcpu + 汇编） | S5, S6, S7, S9 |
| `components/x86_vcpu/src/svm/definitions.rs` | **新增**（退出码等定义） | S4 |
| `components/axvm/Cargo.toml` | 修改（feature 传导） | S8 |
| `components/axvm/src/vcpu.rs` | 修改（svm 条件编译） | S1, S8 |
| `os/axvisor/Cargo.toml` | 修改（svm feature） | S8 |
| `os/axvisor/src/hal/arch/x86_64/mod.rs` | 修改（svm hardware_check） | S8 |
| `os/axvisor/configs/board/qemu-amd64.toml` | **新增**（AMD 测试配置） | S9 |

---

## 五、关键技术要点与风险提示

### 5.1 SVM 的特有优势（相比 VMX）

- **VMCB 通过内存读写**：不需要 VMREAD/VMWRITE 指令，调试和实现更直观
- **VMRUN 统一**：不需要区分 launch/resume，简化了汇编
- **NPT 与 x86 页表相同格式**：不影响 EPT 的现有页表管理代码
- **Host 状态自动保存/恢复**：VMRUN 自动 save/restore host 的大部分 MSR 和段寄存器

### 5.2 需要注意的坑

1. **VMRUN 的 RAX**：VMRUN 指令的输入是 RAX 指向 VMCB 物理地址，但 VMRUN 后 RAX 会被覆盖为 VMCB save area 中的值。进入代码中必须在 VMRUN 前将 VMCB PA 放入 RAX。

2. **Host 寄存器保存**：VMRUN 不会保存 host 的 RAX, RCX, RDX, R8-R15。汇编代码必须手动 push/pop 这些寄存器。

3. **MSRPM 格式**：MSRPM 的高半部分对应 `0xC000_0000` 开始的 MSR，但实际只用到 `0xC000_0000~0xC001_1FFF`。注意偏移计算。

4. **IOPM 大小**：虽然端口范围是 0~0xFFFF，但 IOPM 物理基址 + 4KB + 8KB 需要连续物理页。分配时需要注意。

5. **ASID 限制**：ASID 数量有限（如 8/16/64），多 VM 时需要分配策略。起步阶段用固定值即可。

6. **EFER.SVME 不可在客户模式修改**：guest 的 EFER 写入会被拦截，需要正确处理。

7. **NRIPS**：AMD 的 NRIPS 功能（Next RIP save）会自动保存下次应执行的 RIP，如果 CPU 支持则强烈建议使用，简化 IO/MSR/CPUID 等拦截的 RIP 推进逻辑。

### 5.3 不需要实现的部分（与 Intel 对齐原则）

- AVIC（Advanced Virtual Interrupt Controller）— Intel 也未使用 posted interrupt
- SEV/SEV-ES/SEV-SNP 加密虚拟化 — 远超出对齐范围
- 虚拟 GIF（Global Interrupt Flag）
- VTE（Virtual Transparent Encryption）
- 嵌套虚拟化（Nested SVM）
- KVM 加速 — 明确排除，仅 TCG

---

## 六、测试策略

### 每个阶段的验证方式

| 阶段 | 验证方法 |
|------|---------|
| S1 | `cargo clippy --features svm` 通过 |
| S2 | 在 AMD 真机/QEMU-AMD 上运行 CPUID 检测代码 |
| S3 | `hardware_enable()` 后检查 EFER 和 VM_CR |
| S4 | 单元测试校验 VMCB 结构偏移 |
| S5 | 创建 SvmVcpu 并验证 VMCB 初始值 |
| S6 | 用 mock 环境测试汇编保存/恢复逻辑 |
| S7 | 在 QEMU-AMD 中执行 VMRUN 并观察首个 VM-Exit |
| S8 | axvisor 全链路启动到客户机入口 |
| S9 | ArceOS 客户机到达 shell 交互状态 |

### QEMU AMD 测试环境搭建

```bash
# 检查 qemu 支持的 CPU 模型
qemu-system-x86_64 -cpu help | grep -i epyc

# 使用 EPYC CPU 模型启动（TCG 模式，不加 -enable-kvm）
qemu-system-x86_64 \
    -cpu EPYC-v4,+svm \
    -machine q35,accel=tcg \
    -smp 2 \
    -m 2G \
    -nographic \
    -kernel <axvisor_binary>
```

---

## 七、参考资料

- AMD64 Architecture Programmer's Manual, Volume 2: System Programming
  - Chapter 15: Secure Virtual Machine (SVM)
  - Appendix B: Layout of VMCB
- Intel SDM Vol. 3C, Chapters 23-30（对照参考）
- 现有代码中 `components/x86_vcpu/src/vmx/` 作为直接参考实现

---

> **文档版本**：v1.0 | **日期**：2026-04-29

# ThinWorkspace：跨平台工作区物化设计

**版本：v1.0｜文档性质：详细设计｜首发实现：macOS APFS**

## 一、文档职责

本文档是以下事实的唯一详细设计来源：

- 统一物化 Port 的语义；
- Host/Path Probe 与跨卷、跨文件系统判定；
- `Probe → Plan → Revalidate → Execute → Receipt` 协议；
- CoW 证据、显式降级和部分失败回滚；
- macOS/APFS 的文件系统实现边界；
- 后续 Linux/Windows Adapter 接入时必须保持的共同契约。

本文档不决定产品阶段、CLI 参数、Rust crate 选择或任务门禁；分别由架构方案、用户手册、《技术栈》和《任务流程》管理。

---

## 二、设计原则

1. Core/Application 表达产品意图和降级政策，Adapter 只报告平台事实并执行已选计划。
2. 能力是“这一次的实际路径组合”的性质，不是操作系统名称的静态布尔值。
3. 预检只能生成计划；只有真实系统调用成功才能生成 confirmed 证据。
4. 跨卷或跨文件系统是输入事实，不等于必然不可操作；是否允许取决于具体 Adapter 和当前产品政策。
5. 不确定、路径身份变化、部分成功或无法确认回滚时安全失败，不伪造成功回执。

---

## 三、Port 边界

### 3.1 PlatformProbe

```rust
trait PlatformProbe {
    fn inspect_host(&self) -> Result<HostCapabilityReport>;
    fn inspect_path(&self, path: &Path) -> Result<PathCapabilityReport>;
    fn inspect_materialization_paths(
        &self,
        request: &MaterializationPathProbeRequest,
    ) -> Result<MaterializationPathReport>;
}
```

`inspect_host` 只报告主机级事实，例如系统版本、可用系统调用和已编译 Adapter。`inspect_path` 报告某个实际路径的：

- 路径身份与探测时间；
- 文件系统类型和稳定卷标识；
- mount flags、可写性和符号链接语义；
- 对特定后端的 `supported | unsupported | unknown` 事实；
- 无法得出结论的结构化原因。

`inspect_materialization_paths` 接收实际 Base source、target root（不存在时为最近已存在 parent）、staging、trash 和候选后端，返回各路径报告、两两文件系统/Volume 关系、候选后端的 `supported | unsupported | unknown` 结论及证据。它是命令目录参数能否用于该底层实现的统一检测调用，不能只根据操作系统名称或单个路径推断。

Probe 不返回 `use_full_copy=true` 之类产品决策。

### 3.2 WorkspaceMaterializer

```rust
trait WorkspaceMaterializer {
    fn kind(&self) -> MaterializerKind;
    fn materialize(
        &self,
        request: &MaterializeRequest,
        plan: &MaterializationPlan,
    ) -> Result<MaterializationReceipt>;
    fn measure_usage(&self, workspace: &WorkspacePath) -> Result<UsageReport>;
    fn destroy_materialization(
        &self,
        workspace: &WorkspacePath,
    ) -> Result<DestroyMaterializationReceipt>;
}
```

Materializer 只处理从不可变 Base 到 Workspace 源码项的物化和回收，不管 Git administrative state、构建目录、SQLite 状态或产品降级决策。目标 Workspace root 可以由 GitBackend 预先创建，并且只允许存在经计划声明的 `.git` 平台控制项；Base 构建阶段必须拒绝会与该保留项冲突的来源。Materializer 不覆盖、克隆或删除该控制项。

`materialize` 的失败可携带 partial receipt。`destroy_materialization` 必须以 dirfd-relative/no-follow 方式幂等清理它管理的源码项，保留已声明的平台控制项，并返回本次删除、原本不存在和仍未清理的对象；不能只返回 `()` 而丢失恢复证据。

---

## 四、语义维度

以下概念必须分开，不得压成一个通用状态：

| 维度 | 示例 | 含义 |
|---|---|---|
| 支持性 | `supported/unsupported/unknown` | Probe 对当前路径的预判 |
| 请求模式 | `cow-clone` | 用户/产品原始意图 |
| 有效计划 | `cow-clone/full-copy` | Core 根据事实与政策选择的执行模式 |
| 执行结果 | `succeeded/partial/failed` | Adapter 真实执行结果 |
| CoW 证据 | `confirmed/not-used/unknown` | 是否真正完成块共享 |
| 保证强度 | `hard/soft/best-effort/unsupported` | 资源、清理或隔离的保证级别 |
| 降级事实 | 原因、触发点、失败尝试 | 为什么未使用原请求模式 |

`MaterializationPlan` 至少包含：

```text
requested_mode
effective_mode
selected_adapter
probe_evidence_digest
source/target/staging/trash identities
fallback_policy
```

`FallbackPolicy` 在首发仅有：

```text
Deny
AllowFullCopyOnCowUnsupported
```

`MaterializationReceipt` 至少包含 requested/effective/actual mode、Adapter、outcome、CoW 证据、源/目标卷标识、fallback 原因、failed attempts、已创建对象、回滚结果、耗时和可用的空间估算。

---

## 五、路径级检测

### 5.1 检测对象

每次物化都必须针对实际使用的以下位置检测：

- Base source root；
- target root；尚不存在时检测最近已存在的 parent；
- staging root；
- trash root；
- 当前操作会穿越的每个受控子树。

尚未存在的目标不能自己提供文件系统身份，必须检测其最近已存在父目录并在创建后重新核对。

### 5.2 同卷判定

“同卷”必须由平台稳定文件系统身份证明，不能使用路径前缀、卷名、显示设备名或只比较文件系统类型。

macOS 使用从 no-follow 打开的目录 FD 获得的 APFS Volume UUID，并与 `fstatfs` 结果一起形成 `FileSystemId`。APFS container ID 不等于 Volume ID。UUID 缺失或改变时结果是 unknown/布局错误，不得假定同卷。

### 5.3 执行前重验

Application 将组合 Probe 证据交给 Core 生成 Plan；选中的 Materializer 在任何写入前必须重新打开关键目录，并依据 Plan 中的身份和证据摘要核对：

- 路径组件没有被符号链接替换；
- 目录身份、Volume ID 和挂载属性未变；
- 目标仍符合计划状态：尚不存在，或只含计划声明的受控项（例如 GitBackend 创建的 `.git` 控制项）；
- 写入性和剩余空间未出现已知阻断条件。

重验失败必须以结构化的 plan-stale/路径变化事实返回 Application，由 Application 回到 Probe/Plan；不能带着过期 Plan 执行。Adapter 在逐项遍历和发布等关键边界仍须使用 no-follow 身份检查，不能把入口重验当成整个执行期间的永久保证。

---

## 六、跨卷与跨文件系统语义

| 后端 | 一般底层能力 | 典型限制 |
|---|---|---|
| APFS File Clone | 不支持跨 Volume clone | 源和目标必须在同一 APFS Volume |
| Btrfs/XFS reflink | 不支持跨文件系统 reflink | 需文件系统本身启用对应能力 |
| OverlayFS | 不是通用跨文件系统克隆 | lower/upper/work/mount 需满足内核和文件系统组合要求 |
| ReFS Block Clone | 不支持跨 ReFS Volume block clone | 卷格式和操作系统版本必须支持 |
| Full Copy | 通常可以跨卷/跨文件系统 | 仍受权限、空间、路径安全和产品数据根政策限制 |

因此，统一接口不能简化为“所有平台都不支持跨卷”。正确表达是：

```text
先检测 source/target 的文件系统和 Volume ID
    ↓
判定候选 Adapter 是否支持该路径组合
    ↓
Core 再依当前产品阶段和用户显式政策决定是否执行
```

Phase 1 产品政策比 Full Copy 的底层能力更严：只接受单一 APFS data root，全部受控子树必须位于初始化时记录的同一 Volume；`--allow-copy` 只允许在该布局内把 CoW 不支持降级为 Full Copy，不是跨卷开关。

---

## 七、macOS/APFS Adapter

### 7.1 目录遍历

- 所有安全关键遍历使用 dirfd-relative/no-follow 方式；
- 目录逐层创建，不通过 shell、glob 或未验证路径操作；
- 使用 `fstatat(..., AT_SYMLINK_NOFOLLOW)` 区分普通文件、目录和符号链接；
- 普通文件从已验证的 source parent dirfd 以 `openat(..., O_NOFOLLOW)` 打开源文件，使用 `fstat` 核对类型、身份与 Base 证据；持有源文件 FD，调用 `fclonefileat(source_fd, target_dirfd, target_name, CLONE_NOFOLLOW_ANY)`，并在调用后重新核对源身份和最终 manifest。源文件 FD 固定本次克隆对象，避免检查与克隆使用不同路径对象；
- symlink 使用 `readlinkat/symlinkat` 复制 link text，不跟随目标。link text 可以指向树外，但平台不得在物化和删除中解引用它。

只有对每个应克隆普通文件的真实 `fclonefileat` 调用都成功，且最终树校验通过，Receipt 才能记录 `actual_mode=cow-clone` 和 `cow=confirmed`。

API 签名依据 [Apple XNU clonefile 手册](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/man/man2/clonefile.2)：`fclonefileat` 的源是文件 FD；源目录 FD 加源名称的五参数形式属于 `clonefileat`。

### 7.2 错误分类与降级

| 事实 | Phase 1 策略 |
|---|---|
| Probe 已证明同卷 clone unsupported，且用户传入 `--allow-copy` | 可直接生成 Full Copy 有效计划；`failed_attempts` 为空，Receipt 保留预检降级原因 |
| 能力为 supported/unknown，真实 clone 成功 | 记录 CoW confirmed |
| 真实 clone 以“同卷不支持”失败，且允许 Full Copy | 先依 partial receipt 回滚并确认清理，再重新 Probe/Plan；不得留下 Clone/Copy 混合树 |
| `EXDEV`/卷身份变化 | 数据根布局错误，不降级 |
| `ENOSPC` | 空间失败，不降级 |
| 回滚无法确认 | 保留 operation/partial receipt，转恢复流程，不启动第二后端 |

---

## 八、平台能力演进

| 平台后端 | 历史能力起点 | 额外条件 | 产品状态 |
|---|---|---|---|
| macOS APFS File Clone | macOS 10.13/APFS | 同一 APFS Volume | Phase 1 首发；实际发布以真实机资格矩阵为准 |
| Linux Btrfs reflink | Linux 4.5 通用 `FICLONE` | 同文件系统且项目位于 Btrfs | 后续阶段 Adapter |
| Linux XFS reflink | Linux 4.9 开始引入 | XFS 创建时启用 `reflink=1` | 后续阶段 Adapter |
| Linux OverlayFS | Linux 3.18 进入主线 | 内核、挂载权限和上层文件系统组合合法 | 后续阶段 Adapter |
| Windows Server ReFS Block Clone | Windows Server 2016 | 支持块克隆的 ReFS 卷格式 | 未排期 |
| Windows 11 ReFS 优化复制 | Windows 11 24H2 | 支持的系统复制操作与 ReFS 卷 | 未排期 |

历史起点只是候选能力，不是本产品支持承诺。新组合必须经过 Host Probe、Path Probe、真实执行、故障和恢复验证后才可加入发布矩阵。

---

## 九、验证责任

本设计的实现至少需要以下证据，具体执行时机以《任务流程》为准：

- 真实平台上的同卷成功与跨卷拒绝；
- supported/unsupported/unknown 三种 Probe 结果；
- Probe 后路径、挂载点或 symlink 被替换的竞态；
- 部分 clone、回滚成功/失败和重新规划；
- `EXDEV`/`ENOSPC` 不降级；
- Receipt 与结构化错误契约测试；
- 路径和符号链接的模糊测试；
- 降级、删除和安全判断的变异测试。

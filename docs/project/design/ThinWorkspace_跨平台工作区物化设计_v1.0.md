# ThinWorkspace：跨平台工作区物化设计

**版本：v1.0｜文档性质：详细设计｜当前已实现候选：macOS APFS；Linux/Btrfs 扩展准入见 ADR-0007**

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

`inspect_materialization_paths` 接收实际原始 source 目录、用户指定的最终 target（不存在时为最近已存在 parent）、同卷临时 staging/trash 和候选后端，返回各路径报告、两两文件系统/Volume 关系、候选后端的 `supported | unsupported | unknown` 结论及证据。它是命令目录参数能否用于该底层实现的统一检测调用，不能只根据操作系统名称、系统调用是否存在、单个路径、文件系统类型相同或物理磁盘相同推断。Linux reflink 的具体条件见 §6.1。

组合 Probe 在任一路径探测失败时，结构化 Port 错误必须标明失败的 `source | target root | staging | trash` 角色，不携带原始路径字节。Application 据此区分来源不可访问与目标不可访问；即使前一次单路径探测成功、两次探测间权限变化，也不能将来源失败误报为目标不可用。

Probe 不返回 `use_full_copy=true` 之类产品决策。

### 3.2 WorkspaceMaterializer

```rust
trait WorkspaceMaterializer {
    fn kind(&self) -> MaterializerKind;
    fn materialize(
        &self,
        request: &MaterializeRequest,
        plan: &MaterializationPlan,
    ) -> Result<MaterializationReceipt, MaterializationFailure>;
}
```

Materializer 从原始目录物化计划指定的文件项；它不调用 Git、不按 `.gitignore` 或 Git 跟踪状态过滤、不识别开发语言，也不制定构建目录或缓存策略。目标必须不存在或是本操作已验证的空目录；没有预建 `.git` 控制项。来源 `.git` 与其他名称同样处理，不保留 Git 特判。

`MaterializationFailure` 必须同时保留结构化失败分类和本次尝试的 partial receipt；即使写入前失败，partial receipt 也要明确记录没有创建对象及回滚无需执行，不能把部分外部状态藏进普通字符串错误。清理最终 target 需要创建时存于 `~/.thinws` 的持久归属证明、lifecycle lock 和 WorkspaceId，故由现有 `BootstrapStore` Adapter 在 Application 授权后执行，不在物化 Port 增加以裸路径为删除权的接口；清理边界以[《Phase 1 单机 CLI 详细设计》](ThinWorkspace_Phase1单机CLI详细设计_v1.0.md)为准。

### 3.3 Phase 1 实现边界

Core 拥有支持性、请求/有效/实际模式、fallback、执行结果、CoW、回滚、路径身份摘要、Plan 和 Receipt 等跨平台值；Ports 只声明上述两个边界及其结构化失败；macOS Adapter 拥有目录 FD、errno 和系统调用细节。生产 Adapter 不依赖 `experiments/p0/` crate，P0 实验只能作为待重新审核的算法和测试输入。

P1-06 只在 `WorkspaceMaterializer` 落地 `kind/materialize`，交付 APFS 路径 Probe、共同数据模型和 `ApfsCloneMaterializer`。Probe 必须能观察不存在目标的最近存在父目录，供 dry-run 使用；真正执行时，用户指定的 target 必须已经由调用者在已验证 parent 下建立为空目录、持久记录历史归属并绑定到 Plan，Materializer 不自行认领任意目标根。首个物化切片只执行 `cow-clone`，不实现 Full Copy 或 fallback；后续在同一 Port 上增加 Full Copy 和获准降级，不修改 APFS 已冻结的成功、失败或回滚语义。Application 负责 lifecycle lock、Creating/Ready/Error 持久化和公开 `workspace create`，不得塞进 Adapter。

Port 按有真实调用者的任务增量开放；已有 `BootstrapStore` 按 WorkspaceId、登记 target 和持锁归属证据清理，不增加 Port 数量。当前空间统计复用已验证的 target 和历史归属证明，不把裸 `WorkspacePath` 交给物化 Port 重新扫描，也不新建平行 Port；具体空间语义由 Phase 1 单机 CLI 详细设计维护。

执行输入由 source、target、staging、trash 四个规范绝对路径和冻结 Plan 组成。Plan 必须绑定这四类路径的身份/Volume 证据摘要、请求与有效模式、选中 Adapter 和 fallback policy；Adapter 在任何写入前重新 Probe 并逐项核对，不能接受调用者只填一个 Volume ID。成功 Receipt 必须来自执行后的源/目标清单核对，并记录普通文件数、实际成功 clone 数、已创建对象、源/目标卷、CoW、回滚、耗时与可用空间估算；空树或仅目录/链接树的成功 Receipt 仍为 `cow=not-used`。失败 Receipt 保留按创建顺序登记的对象、失败点、执行后源/目标观察和回滚结果；不能只返回“复制失败”。

进程内回滚不采用“按名称检查后直接删除”的竞态序列。Adapter 只把仍与登记身份一致的目标项以 no-replace 原子重命名摘取到 Plan 已绑定的 trash root，摘取后再次核对身份，目录还必须确认不含未知子项；partial receipt 分别记录已确认和未确认的 trash 相对隔离名。身份或目录内容不符时尽力无覆盖还原，无法确认原路径或还原结果时保留数据并将回滚记为 incomplete。隔离项保留待后续单独证明归属并显式授权处置；相对名称或 partial receipt 本身都不授权 GC 自动删除，P1-06 不把安全隔离误报为物理删除。

### 3.4 输入内容与保证范围

源是本次观察到的磁盘目录，不是 Git 提交，也不具有天然不可变性。支持目录、普通文件和符号链接；特殊文件、子挂载或无法表示的名称明确失败，不静默跳过。硬链接的每个普通文件目录项分别创建独立副本，不保留硬链接拓扑，不使用硬链接共享可写源。

首版验收的共同保真范围为相对名称、类型、普通文件字节/长度/权限、目录权限、符号链接文本，以及普通文件和目录的 mtime（按源/目标实际精度比较）。目录权限和 mtime 在子项完成后设置；无法满足这些保证则失败。源的 atime 可能因读取更新，不把整个源元数据完全不变作为承诺。

ctime、出生时间、所有者、ACL、xattr、file flags 和稀疏分配布局不属于首版完整保真保证；各后端额外保留的属性不能泛化为所有后端共有。Receipt 标明保真范围和不保证项，用户手册必须提示，不用保留 mtime 推导缓存命中。已有 P0-02 未验证的 mtime/完整目录范围必须在新实验补证，不能直接继承为已通过。

调用者在创建窗口暂停源写入；执行前后清单、身份与内容核对只用于发现变化，不构成活跃多文件事务的原子快照。发现变化停止；用户下次新建须重新 Probe/Plan，不自动循环重试追赶源。symlink 和 Git 指针均不重定位、不展开外部内容，不承诺副本完全自足。

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

- 原始 source root；
- target root；尚不存在时检测最近已存在的 parent；
- staging root；
- trash root；
- 当前操作会穿越的每个受控子树。

尚未存在的目标不能自己提供文件系统身份，必须检测其最近已存在父目录并在创建后重新核对。

### 5.2 同卷判定

“同卷”必须由平台稳定文件系统身份证明，不能使用路径前缀、卷名、显示设备名或只比较文件系统类型。

macOS 使用从 no-follow 打开的目录 FD 获得的 APFS Volume UUID，并与 `fstatfs` 结果一起形成 `FileSystemId`。APFS container ID 不等于 Volume ID。UUID 缺失或改变时结果是 unknown/布局错误，不得假定同卷。

Linux 后续 Adapter 必须结合实际挂载关系与文件系统身份判断路径组合；`source` 和 `target` 同为 `btrfs` 或同为 `xfs`，甚至位于同一物理磁盘，均不足以证明可 reflink。文件系统身份或挂载边界无法确认时返回 `unknown`，不得凭名称推定 `supported`。

### 5.3 执行前重验

Application 将组合 Probe 证据交给 Core 生成 Plan；选中的 Materializer 在任何写入前必须重新打开关键目录，并依据 Plan 中的身份和证据摘要核对：

- 路径组件没有被符号链接替换；
- 目录身份、Volume ID 和挂载属性未变；
- 源、target 与 `~/.thinws` 控制目录不相等且互不包含；目标仍不存在或是本操作已验证、持久记录归属的空目录；
- 写入性和剩余空间未出现已知阻断条件。

重验失败必须以结构化的 plan-stale/路径变化事实返回 Application；本次创建按状态机以非 Ready 失败结束，用户重新发起创建时才重新 Probe/Plan，不带着过期 Plan 自动重试。仅 §7.2 中已获准、克隆不支持且回滚确认后的 Full Copy 降级，允许在同一次请求内重新 Probe/Plan。Adapter 在逐项遍历和发布等关键边界仍须使用 no-follow 身份检查，不能把入口重验当成整个执行期间的永久保证。

---

## 六、跨卷与跨文件系统语义

| 后端 | 一般底层能力 | 典型限制 |
|---|---|---|
| APFS File Clone | 不支持跨 Volume clone | 源和目标必须在同一 APFS Volume |
| Btrfs reflink | 不支持跨文件系统 reflink | 源与克隆落点须位于同一个 Btrfs 文件系统；文件属性和挂载关系还须满足 §6.1 |
| XFS reflink | 不支持跨文件系统 reflink | 源与克隆落点须位于同一个启用 `reflink=1` 的 XFS 文件系统；还须满足 §6.1 |
| OverlayFS | 不是通用跨文件系统克隆 | lower/upper/work/mount 需满足内核和文件系统组合要求 |
| ReFS Block Clone | 不支持跨 ReFS Volume block clone | 卷格式和操作系统版本必须支持 |
| Full Copy | 通常可以跨卷/跨文件系统 | 仍受权限、空间、路径安全和产品布局政策限制 |

因此，统一接口不能简化为“所有平台都不支持跨卷”。正确表达是：

```text
先检测 source/target 的文件系统和 Volume ID
    ↓
判定候选 Adapter 是否支持该路径组合
    ↓
Core 再依当前产品阶段和用户显式政策决定是否执行
```

Phase 1 产品政策比 Full Copy 的底层能力更严：本次 source、最终 target 与同卷临时 staging/trash 必须位于同一 APFS Volume；固定 `~/.thinws` 控制目录可以位于另一卷，不决定 clone 能力。`--allow-copy` 只允许该路径组合内把 CoW 不支持降级为 Full Copy，不是跨卷开关。source 与 target 不得互相包含。

### 6.1 Linux reflink 候选后端的适用条件

本节定义 Linux Adapter 的候选能力判定；[ADR-0007](../architecture/adr/ADR-0007_Phase1_Linux_Btrfs_CLI适配.md)已将 Btrfs CLI 适配纳入 Phase 1 扩展，但当前支持矩阵仍以用户手册为准，底层实验不等于产品放行。Linux 提供 `FICLONE` 接口不代表所有文件系统或任意两条路径都支持块共享；普通复制成功、`cp --reflink=auto` 成功或 `copy_file_range` 成功，也不能当作 reflink 证据。`FICLONE` 要求源与目标文件位于同一文件系统；跨文件系统返回 `EXDEV`。[Linux `FICLONE` 手册](https://man7.org/linux/man-pages/man2/FICLONE.2const.html)

两种后端共同要求源文件可读、克隆落点可写、相关挂载未只读、文件为可克隆的普通文件，且有足够空间写入新目录项和 CoW 元数据；目录/符号链接按 §3.4 处理，不因文件克隆能力而获得跨文件系统复制能力。路径级判断须覆盖实际 source、私有 staging、最终 target 和 trash，而非只检查两个根目录的名称。

| 判定层 | Btrfs | XFS |
|---|---|---|
| 主机/卷能力 | 内核与已挂载的 Btrfs 文件系统须支持对普通文件执行 reflink；不以 Linux 版本或 `FICLONE` 常量存在单独判定。[Btrfs Reflink 文档](https://btrfs.readthedocs.io/en/latest/Reflink.html) | 内核须支持 XFS reflink，现有 XFS 卷须在格式化时启用 `reflink=1`，并使用 `crc=1`；不能通过挂载参数把已有 `reflink=0` 卷变为支持。以卷特性（例如 `xfs_info` 的 `reflink=1`）核验；DAX 模式与 reflink 不兼容。[XFS 格式化参数说明](https://manpages.debian.org/bullseye/xfsprogs/mkfs.xfs.8.en.html) |
| 本次路径组合 | `source` 与实际克隆落点（私有 staging）须在同一个 Btrfs 文件系统；staging、最终 target 和 trash 的发布/摘取重命名也须处于允许的同文件系统布局。分别挂载的子卷不能仅凭相同文件系统 UUID 断言可用；Debian 5.10 等低于 5.18 的内核在跨两个挂载点 reflink 时会报跨设备错误 | `source` 与实际克隆落点须在同一个已启用 reflink 的 XFS 文件系统；staging、最终 target 和 trash 还须满足同文件系统的发布/摘取要求。两个不同的 XFS 卷即使类型相同也不可共享数据块 |
| 文件级条件 | 源和目标普通文件的 NOCOW 与校验状态须兼容；例如源文件带 `+C` 而目标继承普通 CoW 属性时，不能仅凭目录路径合格就承诺该文件可 reflink | reflink 用于普通文件的数据区；目录和符号链接按 §3.4 各自物化，不以 reflink 处理。权限、只读挂载、不可变属性等仍可能使实际调用失败 |

组合 Probe 的 `supported` 只表示本次路径组合及已知卷特性**预检合格**，不证明目录树内每个普通文件都可克隆，更不等于 `cow=confirmed`。若挂载/卷特性或逐文件属性尚无法核实，须保留 `unknown` 或在执行时按具体失败记录；已知为 ext4、Parallels 共享文件系统 `prl_fs`、XFS `reflink=0`，或源与目标位于不同文件系统时，对 Btrfs/XFS reflink 候选后端为 `unsupported`。只有各应克隆普通文件的真实 reflink 调用和最终树核验全部成功，才可按 §7.1 同一 CoW 证据规则记录 `confirmed`，不能静默改用 Full Copy。

例如，在 Debian VM 中，`/media/psf/data/code` 若由 Parallels 以 `prl_fs` 挂载，即使把 target 放在 VM 内的 Btrfs/XFS 卷，仍是跨文件系统，不能从该共享目录直接 reflink。要验证 Linux 薄克隆，原始 source 与 target/staging 必须同处一个可用的 VM 内 Btrfs/XFS 文件系统；从共享目录先做一次普通复制只能建立新的 VM 内 source，不会让后续宿主机对共享目录的编辑自动同步到它。

---

## 七、macOS/APFS Adapter

### 7.1 目录遍历

- 所有安全关键遍历使用 dirfd-relative/no-follow 方式；
- 目录逐层创建，不通过 shell、glob 或未验证路径操作；
- 使用 `fstatat(..., AT_SYMLINK_NOFOLLOW)` 区分普通文件、目录和符号链接；
- 普通文件从已验证的 source parent dirfd 以 `openat(..., O_NOFOLLOW)` 打开源文件，使用 `fstat` 核对类型、身份与源清单证据；持有源文件 FD，调用 `fclonefileat(source_fd, staging_dirfd, staged_name, CLONE_NOFOLLOW_ANY)`，再按下述身份固定与发布规则写入 target，并在调用后重新核对源身份和最终 manifest。源文件 FD 固定本次克隆对象，避免检查与克隆使用不同路径对象；
- symlink 使用 `readlinkat/symlinkat` 在 staging 复制 link text 后发布，不跟随目标。link text 可以指向树外，但平台不得在物化和删除中解引用它。

对于 `mkdirat`、`symlinkat`、`fclonefileat` 这类成功时不返回新对象 FD 的调用，不得在公开 target 名称上创建后再从该名称首次认领身份。Phase 1 先在已验证 target parent 下本次操作的私有 staging 中以独占名称创建并固定类型/身份，再用同卷、不覆盖目标的 rename 发布；发布前登记已知身份，发布后核对 target 名称与该身份。目标名称在发布后被替换时必须失败，不写入、接管或回滚删除替换对象。创建失败或发布失败须清理可证明归属的 staging 项；清理无法确认时报告失败，不触发 Full Copy 降级。普通 Full Copy 文件可直接以独占新建并持有的 FD 固定身份。

失败 Receipt 的 `created` 与 `rollback` 只描述 target root；独占 rename 返回失败时未发布的对象不得冒充 target 已创建项。若 staging 项的清理无法确认，另以 `unconfirmed_staging` 记录相对 staging root 的名称、类型和已知或未知身份，结果为 partial，即使 target 未修改且 target 回滚为 `not-needed` 也如此。该字段表示清理未获确认，不凭名称存在与否推断可自动回收；存在此证据时禁止启动 Full Copy 降级。成功 `fclonefileat` 调用在系统调用返回成功时计数，后续身份核验、发布或清理失败不得把该事实从失败 Receipt 中抹去；计数不等于最终 CoW 成功声明。

上述 staging 身份固定依赖已验证 target parent 下的私有 staging 目录在操作期间没有外部写者；它不把 `0700` 或不可预测名称宣称为对同 UID 恶意进程的隔离。工作区不是 Sandbox，同 UID 主动篡改本次 staging 不在 Phase 1 保证范围；公开 target 名称的并发替换仍须按身份失败。进程中断可能留下未发布 staging 项，不自动续做或以未知身份清理。

只有至少一个普通文件实际执行克隆、每个应克隆普通文件的真实 `fclonefileat` 调用都成功且最终树校验通过，Receipt 才能记录 `cow=confirmed`。空树或仅含目录/链接的树可以创建成功，但 CoW 记为 `not-used`，不能以空集合证明块共享。

API 签名依据 [Apple XNU clonefile 手册](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/man/man2/clonefile.2)：`fclonefileat` 的源是文件 FD；源目录 FD 加源名称的五参数形式属于 `clonefileat`。

### 7.2 错误分类与降级

| 事实 | Phase 1 策略 |
|---|---|
| Probe 已证明同卷 clone unsupported，且用户传入 `--allow-copy` | 可直接生成 Full Copy 有效计划；`failed_attempts` 为空，Receipt 保留预检降级原因 |
| 能力为 supported/unknown，真实 clone 成功 | 记录 CoW confirmed |
| 真实 clone 以“同卷不支持”失败，且允许 Full Copy | 先依 partial receipt 回滚并确认清理，再重新 Probe/Plan；不得留下 Clone/Copy 混合树 |
| `EXDEV`/卷身份变化 | source/target/临时目录布局错误，不降级 |
| `ENOSPC` | 空间失败，不降级 |
| 回滚无法确认 | 保留非 Ready 状态与 partial receipt，报告失败，不启动第二后端；后续仅允许用户显式清理受控目录 |

---

## 八、平台能力演进

| 平台后端 | 历史能力起点 | 额外条件 | 产品状态 |
|---|---|---|---|
| macOS APFS File Clone | macOS 10.13/APFS | 同一 APFS Volume | Phase 1 首发；实际发布以真实机资格矩阵为准 |
| Linux Btrfs reflink | Linux 4.5 通用 `FICLONE`；Btrfs 更早已有专用接口 | 源与克隆落点在同一个 Btrfs 文件系统；挂载及 NOCOW/校验约束见 §6.1 | 后续阶段 Adapter |
| Linux XFS reflink | Linux 4.9 开始引入 | 源与克隆落点在同一个创建时启用 `reflink=1`、`crc=1` 的 XFS 文件系统；DAX 约束见 §6.1 | 后续阶段 Adapter |
| Linux OverlayFS | Linux 3.18 进入主线 | 内核、挂载权限和上层文件系统组合合法 | 后续阶段 Adapter |
| Windows Server ReFS Block Clone | Windows Server 2016 | 支持块克隆的 ReFS 卷格式 | 未排期 |
| Windows 11 ReFS 优化复制 | Windows 11 24H2 | 支持的系统复制操作与 ReFS 卷 | 未排期 |

历史起点只是候选能力，不是本产品支持承诺。新组合必须经过 Host Probe、Path Probe、真实执行及失败边界验证后才可加入发布矩阵。

---

## 九、验证责任

本设计的实现至少需要以下证据，具体执行时机以《任务流程》为准：

- 真实平台上的同卷成功与跨卷拒绝；
- 后续 Linux Adapter 接入时，须覆盖 Btrfs/XFS 合格路径、ext4/`prl_fs`/XFS `reflink=0` 拒绝、同类型不同文件系统拒绝、Btrfs 文件属性不兼容和低于 5.18 内核跨挂载点场景；不得用 Full Copy 成功代替 reflink 成功；
- Linux Btrfs 真实文件系统测试由 `THINWS_LINUX_BTRFS_TEST_ROOT` 显式指定已存在、可写、无路径符号链接的绝对目录；测试先确认实际文件系统为 Btrfs，只在该目录内创建并清理随机命名的私有子目录，不删除配置的根目录，也不默认使用系统临时目录或源码共享目录。未配置或验证失败须记为未执行/失败，不得冒充真实 Btrfs 已通过。先用 `python3 -B tools/linux_btrfs_preflight.py` 在该环境下验证；此变量仅属于测试工具，不是产品 CLI 配置。后续 Linux Adapter 的真实物化测试沿用该测试根约定；
- supported/unsupported/unknown 三种 Probe 结果；
- Probe 后路径、挂载点或 symlink 被替换的竞态；
- 部分 clone、回滚成功/失败和重新规划；
- `EXDEV`/`ENOSPC` 不降级；
- Receipt 与结构化错误契约测试；
- 路径和符号链接的模糊测试；
- 降级、删除和安全判断的变异测试。
- 非 Git 目录、原样 `.git`、未跟踪/ignored 路径均被物化；不存在 Git 预建控制项或内容过滤；
- 本节保真范围、源活跃修改拒绝、源/target/控制目录包含关系、源消失后的副本使用、单侧写入隔离；
- 获准清理包含副本内部 Git 数据但不触达外部指针/符号链接目标。

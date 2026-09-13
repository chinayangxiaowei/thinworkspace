# ThinWorkspace

**单机 CLI 首发、跨平台适配与渐进式架构演进方案**

**版本：v2.1｜定位：简化首发范围，先交付可验证的单机产品**

---

## 一、本次调整结论

本版本对 v2.0 做三项关键调整：

1. **第一期只做单机 CLI。**不引入常驻控制面、远程 Worker、Web 控制台、任务编排和多节点概念。
2. **跨平台能力通过分层适配实现。**macOS、Linux 和未来 Windows 使用相同核心流程，但不假设它们具有相同的文件系统、观察、资源限制和 Sandbox 能力。
3. **按需求逐步引入新概念。**第一期用户只需要理解 Repository 和 Workspace；WorkspaceCheckpoint、SourceSnapshot、Job、Node、Placement 等对象在真正需要时再加入。

新的演进顺序是：

```text
Phase 0 关键技术验证
    ↓
Phase 1 单机 CLI 工作区
    ↓
Phase 2 单机协同增强
    ↓
Phase 3 远程构建与测试
    ↓
Phase 4 多节点开发
    ↓
Phase 5 规模化与企业能力
```

第一期的目标不是一次性建成“Agent 开发云”，而是可靠地回答一个问题：

> 能否在一台机器上，通过简单命令快速创建多个相互隔离、节省空间、Git 状态正确且可安全回收的 Agent 工作区？

---

## 二、产品定位与首发边界

### 2.1 长期产品定位

长期目标仍然是：

> 为多个 Coding Agent 提供低成本独立工作区、修改冲突预警，以及可从单机扩展到多机的构建测试能力。

但是三个价值不要求同时在第一期完整交付：

| 能力 | 首次交付阶段 |
|---|---|
| 低成本独立工作区 | Phase 1 |
| 写前冲突预警 | Phase 2 |
| 远程构建与测试 | Phase 3 |
| 多节点开发工作区 | Phase 4 |

### 2.2 Phase 1 用户只看到两个对象

| 对象 | 含义 |
|---|---|
| Repository | 已接入的 Git 仓库及其本机缓存 |
| Workspace | 从指定基线创建的独立开发目录 |

Phase 1 不要求用户理解：

```text
TaskId
SourceSnapshotId
CheckpointId
JobId / AttemptId
NodeId
PlacementGeneration
Control Plane
```

实现内部使用稳定 ID 区分 Repository、Workspace 和可复用 Base，物理路径不是对象身份。这些内部对象不扩张为首发产品概念。名称/ID 语法、默认名推导、重复命令和稳定错误码属于公开契约，只由 Phase 1 用户操作手册管理。内部 source identity、Base 身份和删除审计以 Phase 1 详细设计为准。

### 2.3 Phase 1 明确不做什么

Phase 1 不做：

- 常驻服务进程和 HTTP API；
- Web 控制台；
- 实时文件事件汇总；
- 修改意图和写前预警；
- 远程执行和任务队列；
- 工作区迁移；
- Agent 会话管理和任务编排；
- 用户命令执行包装、语言识别、构建目录重定向和缓存共享策略；
- 多租户、权限体系和高可用；
- 自研安全 Sandbox。

Phase 1 向用户、编辑器和现有 Agent Runtime 提供可直接使用的普通绝对路径。平台只负责准备目录、展示状态和安全回收，不要求用户命令经过平台。实现语言不限制工作区的开发语言；本次职责收缩的依据与代价见 [ADR-0001](adr/ADR-0001_Phase1普通目录与无执行包装.md)。

---

## 三、Phase 1 用户流程

Phase 1 只交付一条单机闭环：

```text
检查本机能力
  → 接入 Git 仓库
  → 选择并固定基线
  → 创建多个 Workspace
  → 用户或现有 Agent 分别开发/测试
  → 查看状态并按团队 Git 流程交付
  → 受保护地删除 Workspace 和回收空间
```

CLI 命令、参数、stdout/stderr、JSON、退出码和所有用户可见例子只由 [Phase 1 用户操作手册](../reference/ThinWorkspace_Phase1用户操作手册_v1.0.md)管理。本阶段没有后台队列、常驻服务或用户命令执行包装；普通工作区不构成安全 Sandbox。

---

## 四、架构原则：统一核心，不强行统一平台能力

### 4.1 Phase 1 分层

首发架构只包含当前需要的组件：

```text
┌──────────────────────────────────────────────┐
│ CLI：参数、人类输出、JSON、退出码       │
├──────────────────────────────────────────────┤
│ Application：单机用例编排、锁和恢复协调    │
├─────────────────────────────────────────────┤
│ Core/Ports：身份、状态、策略和窄接口       │
├──────────────────────────────────────────────┤
│ Adapters：macOS/APFS、Git CLI、SQLite          │
└──────────────────────────────────────────────┘
```

依赖只能向内。Core/Application 不直接识别 macOS、APFS、SQLite、Git CLI 或具体系统调用；Adapter 只报告能力和执行事实，产品策略由 Core/Application 决定。

Phase 1 不建立 ChangeObserver、WorkspaceCheckpointCodec、SourceSnapshotCodec、ExecutionBackend 或 Linux Adapter 占位代码。当后续阶段真正需要时，再以向后兼容的 Port 版本演进。

### 4.2 详细设计分工

为了避免架构方案同时承担产品路线图和实现手册，细节按两个独立设计域管理：

- [Phase 1 单机 CLI 详细设计](../design/ThinWorkspace_Phase1单机CLI详细设计_v1.0.md)：单机组件、data root、Repository/Base/Workspace、Git 拓扑、状态机、恢复、外部占用检查和 GC。
- [跨平台工作区物化设计](../design/ThinWorkspace_跨平台工作区物化设计_v1.0.md)：PlatformProbe、WorkspaceMaterializer、跨卷判定、CoW 证据、降级和平台 Adapter 契约。

本文档只保留跨阶段架构决策和产品边界，不复制 trait 方法、SQLite 约束、系统调用序列、Git 命令参数或测试矩阵。

---

## 五、平台适配与选择策略

### 5.1 适配方向

文件克隆、reflink、OverlayFS 和 Full Copy 不是同一能力的不同名称。它们对同卷、挂载、可移植性、回收和证据的语义不同，因此必须通过统一 Port 暴露事实，不能把所有平台压成一个 `supported` 布尔值。

跨卷支持由“本次 source/target 组合＋具体 Adapter＋当前产品政策”共同决定。CoW/reflink 类后端通常要求同卷或同文件系统；Full Copy 底层可能跨卷，但产品可以施加更严格的数据根边界。

### 5.2 Phase 1 决策

Phase 1 只发布 macOS 单机 Adapter：

- APFS File Clone 是主物化后端；
- data root 及全部受控子树必须位于初始化时记录的同一 APFS Volume；
- 默认要求真实 CoW，只有用户显式允许时才能在同一受控布局内使用 Full Copy；
- `--allow-copy` 不是跨卷开关，卷身份变化和空间失败不触发降级；
- 只有真实执行成功才能宣称 CoW confirmed。

当前发布资格组合由 Phase 1 用户操作手册公开；macOS 10.13/APFS 之类底层历史能力起点不等于产品支持。新增 CPU、系统或 Git 组合前必须补真实平台门禁并更新该公开契约。

Linux Btrfs/XFS/OverlayFS 和 Windows ReFS 的历史起点、跨卷限制与接入条件由《跨平台工作区物化设计》统一管理，不在本路线图重复表格。

---

## 六、Phase 1 单机 CLI 架构

### 6.1 部署边界

`thinws` 是按命令启动的本机进程。SQLite 是嵌入式实现细节，Git 语义通过系统 Git CLI 完成。多个终端可以并发调用 CLI，但写生命周期由本机有界锁协调；用户直接运行的工具不经过该锁，也不受平台调度。

### 6.2 内部对象

用户仍只需理解 Repository 和 Workspace。实现内部使用 Repository、Base、Workspace、Operation 和删除 tombstone；这些内部对象不扩张为用户执行记录、后台 Job、Agent 会话或远程调度模型。

### 6.3 生命周期决策

Phase 1 的核心不变量是：

1. 配置、data root 所有权标记和卷身份互相校验；数据卷丢失或变更时安全失败。
2. Workspace 的可用性由持久化状态和可验证 Receipt 决定，不由目录是否存在决定。
3. 创建、删除、Repository 导入/更新、Base 发布和 GC 均可从中断中显式、幂等恢复。普通查询不隐式修复。
4. Workspace 删除默认保留托管分支；dirty 内容、已确认的外部进程占用和删除分支时的未保护提交各自使用独立保护。占用探测是尽力能力，不承诺发现所有使用者。
5. 用户直接在工作区运行工具，平台不托管或终止这些进程，也不自动配置其构建输出和缓存。
6. GC 不删除任何活跃 Workspace、未完成 operation、托管 Repository 或 Git refs/objects。

内部 ID、data root 布局、Git 拓扑、Base 身份、状态迁移、operation/Receipt、删除和 GC 细节全部由《Phase 1 单机 CLI 详细设计》管理。命令和可见行为由 Phase 1 用户操作手册管理。

---

## 七、Phase 0 到 Phase 1 的转换

Phase 0 先用可重复实验和 ADR 消除高风险假设，Phase 1 只实现已冻结的一种 Git 拓扑、一套物化契约和一条可恢复生命周期。Phase 0 实验代码不直接升格为生产代码。

Phase 0/Phase 1 的任务编号、依赖和小阶段只由[《Phase 1 实施计划》](../../development/planning/ThinWorkspace_Phase1实施计划_v1.0.md)管理。Phase 0/Phase 1 的产品进入/退出控制点只由本文第十三节的阶段边界表管理；具体收口与人工放行操作由《任务流程》管理。

---

## 八、Phase 2：单机协同增强

Phase 1 稳定后，再解决多个 Agent 在同一机器上互相感知的问题。

### 8.1 新增能力

- 一个本地协调进程 `thinwsd`，按需启动并仅服务本机；
- CLI 与 Agent Adapter 通过版本化 Unix Domain Socket 本地 RPC 访问协调进程，默认不监听 TCP；
- Agent Adapter；
- `prepare_edit/apply_edit` 两步协议；
- 基线到当前内容的变更索引；
- 文件事件观察加周期性重新扫描；
- 修改意图和 TTL；
- 文本区域重叠、删除/修改、同名新文件提示；
- 可恢复 WorkspaceCheckpoint；
- 本地固定输入验证。

此时才向用户引入“WorkspaceCheckpoint”和“冲突预警”。Phase 1 的纯 CLI 工作区命令继续可用；只有启用受管协同时才要求本地协调进程运行。该进程不是远程控制面，也不承担多节点职责。

`thinwsd` 运行期间，它是本机元数据和协调状态的唯一写入者；CLI 的变更命令必须通过本地 RPC 执行，不能与 daemon 同时直接写 SQLite。离线 CLI 只有在取得互斥的 writer ownership 后才能回到直接写模式。lease、socket epoch 和崩溃接管协议在 Phase 2 启动时通过详细设计/ADR 冻结，不在 Phase 1 预实现。

### 8.2 两步编辑协议

`prepare_edit` 返回不可复用的 EditToken：

```text
workspace_id
workspace_generation
规范目标路径
文件前镜像摘要
patch 摘要
协调索引 watermark
过期时间
```

`apply_edit` 必须校验 token、文件前镜像和相关协调版本。prepare 之后若出现新的重叠修改，应要求重新 prepare，而不是使用已经过期的“无冲突”结果继续写入。

Shell、IDE、formatter 和代码生成器仍可能绕开受管协议。产品必须区分：

```text
受管编辑：写前预警
非受管写入：写后发现
```

### 8.3 检查点

Phase 2 的 WorkspaceCheckpoint 用于本机恢复，需要包含：

- 源码内容 manifest；
- 已跟踪和明确允许的未跟踪文件；
- 删除、模式和符号链接；
- HEAD、index、分支关联；
- 恢复本地提交所需的 Git objects/refs。

检查点格式不能依赖 APFS clone 或 OverlayFS upperdir。

Phase 2/Phase 3 使用不同 Port 和显式类型区分两类 manifest：

```text
WorkspaceCheckpointManifest
    源码内容 + Git 恢复状态，Phase 2 使用

SourceSnapshotManifest
    仅包含远程执行所需的固定输入，Phase 3 使用
```

`WorkspaceCheckpointCodec` 只接受 `WorkspaceCheckpointManifest`；Phase 3 的 `SourceSnapshotCodec` 只接受 `SourceSnapshotManifest`。两者可以复用内部内容寻址和文件元数据编码，但不能因为共享底层模块而把本地分支、index 或恢复信息上传给只需要源码输入的 Worker。

---

## 九、Phase 3：远程构建与测试

只有重型构建已经成为单机瓶颈时，才引入远程执行。

### 9.1 新增概念

此阶段新增：

```text
SourceSnapshotId
JobId
AttemptId
WorkerId
```

Workspace 仍在开发机；远程 Worker 只运行固定 SourceSnapshot。

### 9.2 架构

```text
开发机 CLI / 本地服务
    │
    ├── 本机 Workspace
    └── 发布 SourceSnapshot
              │
              ▼
        Remote Execution API
              │
       ┌──────┴──────┐
       ▼             ▼
   Linux Worker A  Linux Worker B
```

所有结果必须绑定：

```text
SourceSnapshot digest
执行配方 digest
工具链/镜像 digest
平台能力
依赖配置
JobId / AttemptId
```

Phase 3 不迁移开发 Workspace，不做源码双向同步，也不共享可变构建目录。

### 9.3 平台适配复用

Linux Worker 可根据本机能力选择：

```text
Btrfs/XFS Reflink Materializer
    ↓
OverlayFsMaterializer
    ↓
FullCopyMaterializer
```

远程执行在本阶段才新增版本化的 `ExecutionBackend` Port 和 Job/Attempt 领域对象，其职责是能力报告、提交、取消与结果查询；方法签名和传输协议在 Phase 3 启动时通过独立详细设计冻结。已有 Repository、Workspace、Git 与 `WorkspaceMaterializer` 核心语义保持不变。

---

## 十、Phase 4：多节点开发工作区

只有出现“开发 Workspace 必须放到其他机器”的真实需求时，才进入本阶段。

### 10.1 新增对象

```text
TaskId
NodeId
Placement
PlacementGeneration
```

### 10.2 架构变化

```text
CLI / Agent Adapter
        │
        ▼
Control Plane
├── Workspace Registry
├── Placement Scheduler
├── Coordination
└── Validation Queue
        │
    ┌───┴───────────┐
    ▼               ▼
Node Runtime A   Node Runtime B
├── Adapters     ├── Adapters
├── Workspace    ├── Workspace
└── Local Cache  └── Local Cache
```

Node Runtime 继续使用相同的 Platform Adapters。控制面只消费统一能力报告，不直接假设节点运行何种文件系统。

### 10.3 写入所有权语义

一个逻辑 Workspace 同时只能有一个正式写入 generation。

网络分区时无法保证旧节点的本地磁盘停止变化，因此系统只承诺：

> 旧 generation 不能向控制面发布为当前 Workspace 的正式状态；旧节点重新上线后的修改被隔离为 fork，不能静默覆盖新节点。

迁移采用停止、持久化 WorkspaceCheckpoint、更新 generation、新节点恢复的流程，不做进程热迁移。

---

## 十一、Phase 5：按需求增加规模化能力

| 方向 | 启动条件 |
|---|---|
| 控制面高可用 | 单控制面停机已经影响业务 |
| 自动扩缩 | 排队持续存在且增加节点有明确收益 |
| 高级冲突分析 | 文本级预警稳定后仍有明确误报瓶颈 |
| 多租户与审计 | 开始跨团队或对外服务 |
| Windows/ReFS | 存在真实 Windows 开发节点需求 |
| 高级 Agent 编排 | 用户已需要平台管理 Agent 生命周期 |

本阶段仍不承诺自动理解所有语义冲突，也不以 LLM 判断替代 Git 合并和测试验证。

---

## 十二、兼容演进规则

为了避免阶段升级时推倒重来，从 Phase 1 开始遵守以下规则：

1. `RepositoryId`、`WorkspaceId` 和 `BaseId` 不使用物理路径作为身份。
2. 元数据具有 schema version，升级必须可迁移。
3. CLI 的 `--json` 输出按版本管理。
4. MaterializationReceipt 永久记录实际后端和降级情况。
5. 平台事件只是提示，重新扫描才是变更事实来源。
6. 本机物化格式不作为网络传输格式。
7. 结果绑定不可变输入，不绑定可变 Workspace。
8. Adapter 分别报告 support state、执行证据和 assurance；不得把 unsupported、planned、confirmed、degraded、soft、hard 或 best-effort 混成同一枚举或伪造能力。
9. 新阶段增加接口实现，尽量不把分布式概念倒灌进 Phase 1 核心。

---

## 十三、阶段、能力与边界控制表

该表是范围控制基线。某项能力只有满足“进入条件”后才能进入对应阶段；如果实现需要突破“明确边界”，必须先更新方案或新增 ADR，不能以内部重构名义提前引入。

| 阶段 | 产品形态与用户概念 | 本阶段必须交付的能力 | 明确边界：本阶段不得承诺或引入 | 进入条件 | 退出控制点 |
|---|---|---|---|---|---|
| Phase 0 技术验证 | 非产品原型；仅验证 Repository、Base、Workspace | 路径级能力检测；APFS clone；Full Copy；平台托管 bare repository＋linked worktree 组合；故障注入 | 不追求完整 CLI；不做 daemon、观察器、WorkspaceCheckpoint、SourceSnapshot、远程执行和 UI | 已确定 macOS/APFS 为首个目标环境；测试仓库和数据根目录就绪 | 同卷 APFS CoW 已由真实调用证实；预检/运行时不支持的显式降级均可解释；APFS 跨卷失败已结构化证明；Full Copy Adapter 可独立跨卷；Phase 1 Application 对跨卷布局保持拒绝；Git 状态干净；创建/删除中断可恢复；不完整 Workspace 不会成为 Ready |
| Phase 1 单机 CLI | 一个二进制；用户只理解 Repository、Workspace | repo add/fetch/list 与 workspace CLI；Host/Path Probe；同卷 APFS/Full Copy WorkspaceMaterializer；普通路径交付与 Git 状态；SQLite 状态；删除保护；doctor、空间统计、gc dry-run 与受保护 GC | 无用户命令包装、语言工具链配置、构建重定向或缓存共享策略；无跨卷产品布局、repo remove 或 Git object GC；无常驻进程、HTTP/远程 API；无实时观察、写前预警、WorkspaceCheckpoint、SourceSnapshot、Job、Node、调度、Web 和 Sandbox 承诺 | Phase 0 全部退出控制点通过；平台托管 bare repository＋linked worktree ADR 和 MaterializationPlan/Receipt 定稿 | 10 个 Workspace 隔离；普通工具可直接操作路径；显式 repair 可恢复；降级不静默；dirty/未保护分支提交/已确认占用阻止删除，扫描不完整如实报告；所有声明支持的 `--json` 和退出码稳定 |
| Phase 2 单机协同 | CLI＋本地 `thinwsd`；新增 WorkspaceCheckpoint、冲突预警 | Unix Socket RPC；Agent Adapter；ChangeObserver；重新扫描校准；prepare/apply；EditToken；文本冲突索引；WorkspaceCheckpoint；本机固定输入验证 | 不做 Remote Worker、网络控制面、Job 调度、Workspace 迁移和多节点所有权；不把非受管写入标成写前预警 | Phase 1 稳定运行；确有多个 Agent 同机协同需求；选定至少一种可控编辑接入 | 受管修改在写入前看到有效报告；竞态会要求重新 prepare；事件丢失可校准；非受管写入明确标为写后发现；WorkspaceCheckpoint 可恢复 Git 和源码状态 |
| Phase 3 远程验证 | 本机开发＋远程 Worker；新增 SourceSnapshot、Job、Attempt、Worker | 可移植 SourceSnapshot；CAS/传输；Worker 注册与租约；远程执行；日志、结果和产物；环境/配方摘要；重试 attempt | 不做远程可写 Workspace、开发任务迁移、源码双向同步、跨机共享可变构建目录和多节点编辑 | Phase 2 的内容清单/寻址内部能力已稳定；SourceSnapshot schema 与泄漏边界 ADR 已批准；重型验证成为已测量瓶颈；外部 Sandbox/Worker 信任模型明确 | 结果严格绑定 SourceSnapshot 和环境摘要；不同 SourceSnapshot/配方摘要的结果互不污染；重试不覆盖旧 attempt；节点丢失、环境失败、测试失败可区分 |
| Phase 4 多节点开发 | 控制面＋Node Runtime；新增 Task、Node、Placement、Generation | Workspace Registry；节点能力报告；放置；全局协调；写入 generation；停机恢复迁移；中央元数据 | 不做同一 Workspace 多节点同时写；不做进程热迁移；不承诺分区期间实时预警；旧节点本地写入不能被描述为物理上已停止 | Phase 3 稳定；存在开发 Workspace 必须远程放置的真实需求；RPO/RTO 已定义 | 旧 generation 无法正式发布；失联修改被隔离为 fork；事件重复/乱序可恢复；新节点内容与持久化 WorkspaceCheckpoint 一致 |
| Phase 5 规模化 | 按需求组合，不作为统一大版本 | 高可用、扩缩、多租户、审计、Windows、高级冲突或 Agent 编排中的必要子集 | 不因“平台化”一次性引入全部能力；LLM 不作为唯一合并或正确性判据 | 指标证明对应瓶颈或业务需求存在；单独立项和安全评审 | 每个子能力使用独立 SLO、容量、恢复和安全验收，不共享模糊的“Phase 5 完成”标准 |

跨阶段控制规则：

1. 阶段只允许增加能力，不改变已经发布对象的身份和已有 CLI 语义；破坏性变化必须版本化迁移。
2. Phase 1 代码中不得出现 Job、Node、Placement、远程租约等未来领域概念；只允许存在不依赖这些概念的窄 Port。
3. support、计划、执行证据、保证强度和 fallback 是不同维度，任何界面和 JSON 不得混用。
4. “实验支持”不能计入退出控制点；必须有故障注入、可重复测试和明确恢复路径。
5. 新平台先实现 PlatformProbe 和能力矩阵，再实现对应 WorkspaceMaterializer Adapter，最后才加入产品支持列表。
6. 超出阶段边界的用户请求可以记录为需求，但不能通过隐藏配置变成未验收功能。

---

## 十四、实现落点

crate 划分、依赖方向、平台 API 和工具链的完整选择由[《技术栈》](../reference/技术栈.md)管理；Phase 1 内部组件职责由[《Phase 1 单机 CLI 详细设计》](../design/ThinWorkspace_Phase1单机CLI详细设计_v1.0.md)管理。本架构方案不再维护第二份 crate 树或源文件清单。

---

## 十五、最终实施建议

第一期只把以下事情做可靠：

```text
一台 macOS 机器
一个 CLI
一个本机数据根目录
APFS Clone 主后端
同一受控数据根内的 Full Copy 显式降级
平台托管 bare repository＋Git linked worktree 元数据
正确的 Git 状态
普通路径直接交给用户、编辑器和 Agent
删除保护和故障恢复
```

第一期明确不解决：

```text
实时协同
写前预警
远程 Worker
分布式控制面
Agent 编排
Web UI
多租户
```

架构上提前保留的是**窄接口和能力报告**，不是提前创建大量服务和业务对象。

这样既能在当前 macOS/APFS 环境快速得到真实可用的产品，也能在后续加入 Linux Reflink、OverlayFS、远程 Worker 和多节点控制面时复用核心流程，而不把任何单一平台的实现细节变成整个系统的协议。

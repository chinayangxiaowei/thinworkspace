# ThinWorkspace Phase 1 单机 CLI 详细设计

**版本：v1.0｜文档性质：详细设计｜适用阶段：Phase 1**

## 一、文档职责

本文档是 Phase 1 内部设计的唯一详细来源，管理：

- 单机 CLI 组件与 Port 边界；
- bootstrap config、data root 身份与受控目录布局；
- Repository、Base、Workspace、Operation 和 Execution 的内部语义；
- Git 托管拓扑、Base 身份、分支与删除保护；
- Workspace 状态机、中断恢复、GC 和进程登记。

跨平台物化契约由《跨平台工作区物化设计》管理；用户可见命令、输出和错误码由 Phase 1 用户操作手册管理；依赖与系统 API 选择由《技术栈》管理。

---

## 二、部署与分层

Phase 1 只有一个按需启动的 `thinws` 进程，没有 daemon、HTTP/gRPC、消息队列或远程控制面。

```text
CLI
  ↓
Application Use Cases
  ↓
Core + Ports
  ↑
Git / macOS / SQLite Adapters
```

依赖只能指向 Core/Ports。Core/Application 不识别 APFS、SQLite、Git CLI 或具体系统调用。

Phase 1 只有七个 Port：

```text
BootstrapStore
PlatformProbe
WorkspaceMaterializer
GitBackend
ProcessSupervisor
MetadataStore
LifecycleLock
```

ChangeObserver、WorkspaceCheckpointCodec、SourceSnapshotCodec、ExecutionBackend 均不在 Phase 1 代码中出现。

---

## 三、实例与 data root

### 3.1 Bootstrap 身份

macOS 生产环境使用：

```text
~/Library/Application Support/ThinWorkspace/
├── config.toml
└── init.lock
```

`config.toml` 是定位 data root 的唯一 bootstrap 指针，至少保存 schema version、InstanceId、规范绝对 data root 和 Volume ID。生产 CLI 不提供环境变量或隐藏参数切换配置位置；测试只能通过显式依赖注入替换。

data root 中的 `.thinws-root.toml` 保存同一 InstanceId、规范路径、Volume ID 和 `initializing|ready` 状态。两侧必须互相校验；任何一侧缺失或冲突时，恢复不得猜测覆盖另一侧。

### 3.2 初始化协议

```text
获取 bootstrap lock
  → 规范化路径并验证最近已存在父目录
  → 创建/验证 0700 data root
  → 写 initializing 根标记
  → 创建受控子树和 SQLite
  → 原子发布 ready 根标记
  → 最后原子发布 bootstrap config
```

全新或空目录可以初始化。非空目录只有在存在本平台上一次中断初始化的可验证标记，且 owner、Volume ID 和目录布局全部匹配时才能续跑。Phase 1 不提供 data root reset/migrate。

### 3.3 受控布局

```text
thinws-data/
├── .thinws-root.toml
├── metadata/state.db
├── repositories/<repository-id>/repo.git/
├── bases/<base-id>/tree/
├── workspaces/<workspace-id>/.state/incomplete
├── workspaces/<workspace-id>/root/
├── staging/
├── builds/<workspace-id>/
└── trash/
```

全部路径由已解析的类型 ID 推导，不接受任意 Workspace 绝对路径。全部受控子树必须在初始化时记录的同一 APFS Volume，不接受子挂载或符号链接重定向。`.state` 位于 Git root `root/` 之外。

---

## 四、内部身份与持久化

InstanceId、RepositoryId、WorkspaceId、ExecutionId 和 OperationId 都是不以物理路径为身份的强类型 ID。Repository/Workspace/Execution/Operation ID 的公开序列化格式由 Phase 1 用户操作手册管理；InstanceId 不属于常规 CLI 契约。BaseId 是由 BaseKey 规范字节派生的内部内容/策略身份；具体哈希与编码由技术栈/实现 ADR 冻结。

SQLite 至少维护：

- schema version；
- Repository、Base 和活跃 Workspace；
- Workspace 状态、Plan/Receipt 和不可变删除意图；
- repo add/fetch、Base build、Workspace create/remove 和 GC 的 operation；
- 活跃 Execution 登记；
- 删除完成的 Workspace tombstone。

数据库级不变量：

- Repository name 唯一，`(source_kind, source_key)` 唯一；
- 活跃 Workspace name、受控 path 和 managed branch 分别唯一；
- 类型 ID 、状态值和活跃关系用 CHECK/PK/FK 约束；
- Deleting 记录必须带开始时的 flags 和 branch OID；
- 删除活跃 Workspace 与写 tombstone 在同一 SQLite 事务内完成；
- Plan、Receipt、operation payload 全部带版本，未知关键版本拒绝读取或复用。

SQLite 不在事务中等待 Git、文件物化或子进程。Git、文件系统和 SQLite 不组成分布式事务；所有外部写入依靠“先 operation/过渡状态，后 Receipt，再完成状态”恢复。

---

## 五、锁和并发

LifecycleLock 有两个 scope：

- bootstrap lock：仅保护首次 `init`，位于用户配置目录；
- data-root lifecycle lock：串行 repo add/fetch、Workspace create/remove、repair 和 GC。

两者都有界等待。长时间 `workspace exec` 不持有 lifecycle lock，只在启动和结束登记时短暂持有。SQLite busy timeout 不替代 OS 锁。具体时限是 CLI 公开契约，以用户手册为准。

---

## 六、Repository 与 Git 拓扑

### 6.1 托管拓扑

Phase 1 使用“平台托管 bare repository＋linked worktree”：

- 本机路径或远程 locator 只是导入来源；
- 平台不使用 alternates，托管 objects 不依赖来源目录继续存活；
- 来源工作树的未提交/未跟踪内容不导入；
- 来源 heads/tags 与平台可写分支使用不同 ref namespace；
- 平台更新来源所用的内部 remote/ref namespace 与用户直接 push 使用的 `origin` 分离，用户修改 `origin` 不改变 `repo fetch` 的来源；
- Workspace linked-worktree administrative state 全部属于平台 bare repository，不修改来源仓库的 worktree 登记。

Repository source identity 是内部幂等键：本机来源使用 Git common directory 的绝对 realpath，使同一 repository 的不同 linked worktree 命中同一来源，独立 clone 保持不同；远程来源使用通过公开语法校验的原始 UTF-8 locator，不猜测不同写法等价。默认名称、公开 locator 语法和冲突错误由用户手册管理。

`repo add/fetch` 的来源语法、可见更新范围和错误行为以用户手册为准。Git common directory 的解析、具体 refspec、命令环境和配置以《技术栈》为准。

### 6.2 Base 解析与身份

`ResolvedBase` 同时保存 commit OID 和 tree OID。Workspace 分支从 commit OID 创建；Base 内容按 tree 和 checkout 语义复用。

```text
BaseKeyV1 {
  repository_id,
  tree_oid_algorithm,
  tree_oid,
  checkout_policy_digest,
  filesystem_semantics_digest
}
```

checkout policy digest 覆盖冻结的 Git checkout 行为、相关 attributes 和实现兼容版本；filesystem semantics digest 覆盖大小写、Unicode 归一化和符号链接支持。BaseKey 必须有版本化、跨平台确定的规范编码和 golden vectors，不使用调试输出、分隔符拼接、本机结构体布局或序列化库默认格式生成身份。哈希与字节编码技术由《技术栈》选择，并在实现 ADR 中冻结；不在架构、规范和手册中复制。

Base 在同卷 staging 中生成，完成路径/模式/内容 manifest 校验和 Receipt 后原子发布。复用前必须验证 Receipt、归属和 manifest；损坏 Base 隔离后重建。

### 6.3 Workspace 分支

每个 Workspace 使用由完整 WorkspaceId 推导的唯一托管分支：

```text
refs/heads/thinws/<workspace-id>
```

显示名称不进入 ref。Phase 1 不支持 detached Workspace，也不允许两个 Workspace 绑定同一可写分支。

---

## 七、Workspace 状态机

可见活跃状态只有：

```text
Creating
Ready
Deleting
Error
```

允许的迁移只有：

```text
∅ → Creating
Creating → Ready | Error
Ready → Deleting | Error
Deleting → ∅ | Error
Error → Ready       仅 doctor --repair 完整重验成功
Error → Deleting    仅显式 remove 且删除保护可证明
```

目录存在不等于 Ready。只有 Git 验证、Base/Git/Materialization Receipts 和 SQLite 状态全部一致时，才能对外返回 Workspace 路径或执行命令。

### 7.1 创建

```text
写 operation 与 Creating
  → 解析并固定 commit/tree
  → 生成或复用 Base
  → 建立 .state/incomplete
  → 注册 no-checkout linked worktree 和唯一分支
  → 依《跨平台工作区物化设计》在保留 .git 控制项的前提下填充源码
  → 初始化独立 index 并验证 clean
  → 写完整 Receipts
  → 移除 incomplete
  → 写 Ready 并完成 operation
```

任何一步中断都保留过渡状态和已知事实。普通查询不续跑，只有显式 repair 可以协调。

### 7.2 删除

```text
运行 Volume/归属/dirty/进程 removal guards
  → 如请求删分支，验证提交保护
  → 写 Deleting 和不可变删除意图
  → 每个破坏性步骤前重验适用保护
  → 由 Materializer 以 no-follow 方式幂等清理源码项，并清理构建目录
  → 在工作树只剩已验证平台控制项后，由 GitBackend 精确解除 linked-worktree 登记并移除 root
  → 如请求删分支，使用带预期 OID 的 ref transaction
  → 同一 SQLite 事务删活跃记录并写 tombstone
  → 完成 operation
```

默认保留分支。`--force` 只放开 dirty 内容保护；`--delete-branch` 是独立意图，不能绕过未保护提交。重试 remove 必须与持久化 flags 完全一致；repair 只按已持久化意图续跑。

tombstone 至少保存 WorkspaceId、RepositoryId、名称、删除 OperationId、时间和结果。它不参与活跃名称解析，不是 Base/目录正引用，也不能单独授权删除归属不明的残留。

---

## 八、查询与恢复

- `workspace list/status` 可以展示全部活跃状态；
- `workspace path/diff/exec` 只允许 Ready；
- 普通查询只读检查一致性，不写 SQLite、不修改 Git、不清理目录；
- `doctor --repair` 获取 lifecycle lock 后，只续跑能从 operation、Receipt、归属和当前系统事实证明安全的步骤；
- 无法证明时保留 Error/未完成 operation 并报告，不按目录外观猜测。

repo add/fetch、Base build 和 GC 也使用相同的 operation/Receipt 协议，但不为此新增用户可见中间状态。

---

## 九、Execution 与外部进程

`workspace exec` 是受信任宿主的前台执行，不是 Sandbox。ProcessSupervisor 使用独立托管进程组，并以结构化 TerminationReport 报告正常退出、signal、超时、中断和清理保证。macOS 只能对托管进程组提供 best effort，不宣称等价 cgroup。

启动协议：

```text
短锁写 Starting execution
  → spawn 独立进程组
  → 写 PID/PGID/进程启动身份和 Running
  → 释放锁并等待前台进程
  → 结束时短锁写终止结果并移除活跃登记
```

进程身份不能只用 PID，必须结合启动身份防止 PID 复用。原 CLI 消失但进程组仍活着时，repair 只报告，不代用户终止。

外部进程占用探测返回：

```text
confirmed-in-use
no-evidence
scan-incomplete
```

只有 confirmed-in-use 是确证据并阻止删除；no-evidence 不代表绝对无占用；scan-incomplete 必须警告并记录。`--force` 不改变该判定。

---

## 十、GC 语义

GC 的正引用根是全部状态的活跃 Workspace、未完成 operation 和显式保留记录。tombstone 只是负向审计证据。

GC 只处理无正引用 Base、到期 trash、明确可回收缓存和失败残留；不删活跃 Workspace、未完成 operation、Git refs/reflog/objects 或托管 Repository。计划在同一 SQLite 只读快照中生成，移动对象前在 lifecycle lock 内重验。

---

## 十一、Phase 1 硬边界

- 仅支持已通过真实机门禁的 macOS/APFS/Git 组合；
- 仅支持 Git SHA-1 object format；
- 拒绝 submodule、Git LFS、sparse checkout、自定义 filter、`working-tree-encoding`、`ident` 和目标卷无法表示的路径冲突；
- 不提供 `repo remove`、Git object GC、自动 maintenance、无范围 worktree prune、detached Workspace 或删除未保护提交的绕过开关；
- 不引入后续阶段对象或空抽象。

具体 Git 命令、checkout policy、BaseKey 编码、SQLite pragma 和 macOS API 细节属于实现 ADR/《技术栈》，不再向架构方案、开发规范、任务流程或用户手册复制。

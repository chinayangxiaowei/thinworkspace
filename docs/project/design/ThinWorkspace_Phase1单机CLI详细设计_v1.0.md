# ThinWorkspace Phase 1 单机 CLI 详细设计

**版本：v1.0（首发前修订）｜文档性质：详细设计｜适用阶段：Phase 1**

## 一、文档职责

本文是单机组件、实例身份、Workspace 生命周期、只读 Git 检查和恢复的唯一详细设计来源。

非职责：物化与元数据保真由《跨平台工作区物化设计》管理；公开命令、输出和错误码由用户手册管理；系统 API 由《技术栈》管理；日志字段由《开发规范》管理；commit 交付由《任务流程》管理。

本次简化依据 [ADR-0002](../architecture/adr/ADR-0002_Phase1原始目录镜像与流程交付.md)。不再维护 Repository、Base、托管分支或 Git 导入生命周期。

## 二、部署与分层

Phase 1 是同步、按需运行的本机 CLI，没有 daemon、网络服务或 LLM 调用。

```text
CLI → Application → Core / Ports ← macOS / Git CLI / SQLite Adapters
```

Core/Application 不直接调用 OS、Git CLI 或 SQLite。Phase 1 只有以下七个 Port：

| Port | 唯一职责 |
|---|---|
| BootstrapStore | 读取、校验和发布实例配置与 data root 标记 |
| PlatformProbe | 报告实际路径与候选后端能力 |
| WorkspaceMaterializer | 目录物化、空间测量及获准的范围内清理 |
| GitInspector | 发现工作区内仓库并只读报告已跟踪变更；不写 refs/index/config、不提交、不联网 |
| ProcessProbe | 尽力报告当前用户可见的外部进程占用，不托管或终止进程 |
| MetadataStore | Workspace、operation、Receipt 和删除结果的持久化 |
| LifecycleLock | bootstrap 与 data root 生命周期的有界互斥 |

不保留旧 GitBackend 的 attach/detach/fetch/commit 等方法，不新增 AuditService 或交付编排 Port。普通持久日志使用既有日志设施。

## 三、实例与 data root

### 3.1 实例身份

macOS 固定 bootstrap 位置：

```text
~/Library/Application Support/ThinWorkspace/
├── config.toml
└── init.lock
```

配置保存 schema version、InstanceId、规范绝对 data root 和 Volume ID。生产 CLI 不提供隐藏环境变量切换位置；测试通过依赖注入替换。

data root 的 `.thinws-root.toml` 保存匹配的实例、路径、卷和 `initializing|ready` 状态。两侧缺失或冲突时不能猜测覆盖。

### 3.2 初始化

取得 bootstrap lock → 验证最近存在父目录 → 创建/验证私有 data root → 写 initializing 标记 → 建立受控目录和 SQLite → 原子发布 ready 标记 → 最后发布 bootstrap config。

只接管新目录、空目录或能证明属于同一次中断初始化的目录。没有 reset/migrate。细化的幂等及错误行为由手册管理。

### 3.3 布局与源目录

```text
thinws-data/
├── .thinws-root.toml
├── metadata/state.db
├── logs/operations.jsonl
├── workspaces/<workspace-id>/.state/incomplete
├── workspaces/<workspace-id>/root/
├── staging/
└── trash/
```

受控目标由 WorkspaceId 推导，全部受控子树位于登记的同一 APFS Volume。源目录是用户提供的只读输入，不需要位于 data root 内，但首版要求与 data root 同卷；拒绝源与 data root 相等或互相包含，防止把平台目录递归复制进自身。根和路径组件不接受未验证链接，遍历边界以物化设计为准。

`.state` 与日志均在副本 `root/` 外；`.git` 不是平台保留项，它仅是被复制的目录内容。用户不能手工移动或改写平台管理目录。

## 四、身份与持久化

InstanceId、WorkspaceId、OperationId 是强类型 ID。源路径不是新领域对象，不创建 SourceId/RepositoryId/BaseId。Workspace 记录源的规范位置、创建时路径/卷证据、创建策略、受控目标、状态和物化 Receipt；这些是本次复制的来源证据，不是 Git 基线或持续同步关系。

SQLite 维护版本、活跃 Workspace、operation、Plan/Receipt、不可变删除意图及最小删除 tombstone：

- 活跃名称、受控目标唯一；ID、状态、关系由数据库约束；
- operation 与 payload 版本化；未知关键版本拒绝自动恢复；
- 删除活跃记录与写 tombstone 在同一事务完成；
- 不在数据库事务中等待文件物化、Git 检查或子进程。

源目录后续消失不影响已创建副本的普通使用或清理；恢复未完成创建时仍须重新验证源。没有 Git、文件系统和 SQLite 的跨系统事务。

## 五、并发与源一致性

bootstrap lock 仅保护初始化；data-root lifecycle lock 串行创建、删除、repair 和 GC，均有界等待。它不能阻止用户工具读写源或副本，SQLite busy timeout 不能替代 OS 锁。

创建期间由调用者暂停源目录写入；物化器检测到变化则停止并保留失败证据。首次没有原子目录快照、后台冻结、锁扫描重写或缓存一致性修复。创建成功后副本不跟随源更新。具体文件范围、时间属性和证据在物化设计中定义。

## 六、只读 Git 检查

### 6.1 发现范围与外部引用

创建不调用 Git，不以 Git 特性拒绝目录。查询或清理时，GitInspector 在已验证副本范围内按需发现根仓库和嵌套仓库，包括 submodule；不跟随目录符号链接，不进入 `.git` 管理目录枚举“子项目”，也不因父仓库忽略某目录而漏掉其中真实存在的子仓库。

只对具有本地 Git 标记的目录进行检查，不能通过 Git 向上搜索把平台父目录的仓库认成副本仓库。解析 Git 元数据引用时核验实际 gitdir、commondir、worktree 是否仍在此副本内；外部、损坏、不可读或无法安全解释的引用返回检查不完整，不自动修复、初始化或改写原仓库。

普通自包含 `.git/`、副本内可解析的 submodule 元数据可检查；原样复制的外部 gitfile/commondir/绝对 worktree 引用不自动隔离。外部符号链接也只保留文本。创建交付的是文件副本，不承诺用户任意 Git 命令或外部访问均与来源隔离。

### 6.2 已跟踪内容与检查报告

逐仓库检查 HEAD 与 index、index 与工作树的差异。包括暂存新增、修改、删除、重命名、模式变化、冲突和 gitlink 引用变化；“跟踪过”指当前 HEAD/index 管理的内容，不扫描全部历史中已移除的路径。

未跟踪、ignored 文件不计数、不提示、不作为 dirty。子仓库内的 tracked 修改独立报告；仅有子仓库未跟踪文件不能使父仓库被误报 dirty。缺失且未初始化的 submodule 不自动下载；已检测到的 gitlink 变化仍需报告。

报告分开表达仓库发现完整性与各仓库 `clean|dirty|unknown` 事实。没有仓库且发现完整为 `not-applicable`，不是 Git clean。缺少 Git、扫描失败、超时、配置/转换无法安全查询等返回 unknown，不冒充无修改。具体只读命令与程序执行限制由技术栈管理。

### 6.3 分支与交付非职责

平台不新建分支、不 commit/push/PR、不计算提交是否已发布或由其他引用保护。镜像复制当时已有的文件和 Git 元数据；直接使用 Git 的行为由用户环境决定。副本内的 Git 数据也会随清理删除，不承诺保留分支、stash 或仅存在于副本的提交。

任务如何在非原分支交付并向主管 Agent 提供有效 commit，只有《任务流程》负责；不变成删除前不可绕过的验收模型。

## 七、Workspace 状态机

活跃状态仅 `Creating / Ready / Deleting / Error`：

```text
∅ → Creating → Ready | Error
Ready → Deleting | Error
Deleting → ∅ | Error
Error → Ready       仅显式 repair 完整重验成功
Error → Deleting    仅显式 remove 且范围/意图可证明
```

Ready 由物化 Receipt、归属和持久化状态一致决定，不要求 Git clean，也不要求任何提交或分支。

### 7.1 创建

写 operation/Creating → 建立 incomplete → Probe/Plan → 对空目标物化原始目录 → 验证 Receipt 和来源观察结果 → 移除 incomplete → 写 Ready/完成 operation。

同名重复请求的公开规则由手册管理；不会隐式刷新已有副本。失败保留已知创建对象和 partial receipt；无法确认回滚时不继续第二后端。repair 不把原样目录存在当作成功。

### 7.2 清理

用户停止相关工具后，按以下顺序执行：

1. 验证实例、WorkspaceId、卷和目标归属；运行只读 Git 与进程检查。
2. Application 按手册决定普通拒绝或接受显式强制意图；拒绝发生在破坏性写入之前。
3. 清理前写持久日志，并持久化 operation、原始检查摘要、force 标记和 Deleting 意图。
4. 每个破坏性步骤前重验范围、卷和适用的占用保护；Materializer 以 no-follow 清理整个副本，包括其中 `.git` 和后来生成的内容。
5. 写 Destroy Receipt；在同一事务删除活跃记录并保留最小 tombstone；记录完成结果。

强制操作不要求说明理由、commit 证明、主管批准或在线服务；只保留日志。日志字段与敏感信息边界见《开发规范》§13。清理前无法持久化日志时停止且报告 I/O 错误；已开始后失败/中断保留 operation 和剩余范围，不能写成成功。

普通清理只保护已跟踪变更，不保护未跟踪文件，也不检查未推送提交。强制可绕过 dirty 或 Git 检查不完整，不能绕过路径/卷归属或已确认进程占用。非 Git 目录也可正常清理。

### 7.3 删除重试与日志

尚未写 Deleting 的普通拒绝不固定后续 flags；用户可重新执行显式强制清理。一旦持久化删除意图，重试 flags 必须一致，repair 只续跑原意图，不自行加 force。

删除已经部分执行时不能把“删除本身造成的 tracked 删除”当作新的准入失败；恢复依据原检查结果和删除意图续跑，并重验对象归属及当前占用。发现新写入、替换对象或无法解释的范围变化则停止，不把它自动纳入旧意图。

日志位于 data root 的 `logs/`，不随 Workspace 清理或 GC 删除。它是普通本机日志，不宣称防篡改或构成成果证明。中断可能只有开始事件；operation 和显式 repair 解释未完成结果，不引入第二套日志恢复状态机。

## 八、查询与恢复

list/status 展示全部活跃状态；path 仅 Ready。查询不写 SQLite/Git，不自动清理或恢复。Git unknown 不会把一个物化完整的 Ready 副本变为不可用，也不阻止获取路径。

doctor 默认只读；显式 repair 在 lifecycle lock 内，依据 operation、Receipt 和当前身份继续或停止。创建输入变化时不自动重新镜像最新源覆盖旧操作；无法证明原创建结果完整则保持 Error，由调用者清理失败副本后新建。

## 九、外部进程占用

ProcessProbe 通过 macOS Adapter 报告当前用户可见的 cwd/open-vnode 占用。进程身份不能只靠 PID；结果包含探测时间和完整性：

- `confirmed-in-use`：阻止删除，包括强制清理；
- `no-evidence`：没有取得占用证据，不等于绝对无人使用；
- `scan-incomplete`：警告并记录，不能伪装成无占用。

平台不停止用户工具，不阻止其他程序在检查后进入目录；用户负责清理窗口内不再写入。内部 Git 子进程的有界退出和输出处理属于 GitInspector 实现，不扩展为执行包装。

## 十、GC 与空间统计

GC 仅处理已完成操作留下、归属可证明且明确标为可回收的 staging/trash 残留。不删除活跃 Workspace（包括 Error）、未完成 operation、日志、用户源目录或外部路径；不运行 Git GC/prune，不管理 Git refs。活跃 Error 必须先按 remove/repair 的意图处理，不能让 GC 绕过强提示。

计划在一致的只读元数据视图中形成，执行在 lifecycle lock 内重验。空间统计区分逻辑大小、可取得的物理估算及未知值；不承诺逐副本精确可回收共享块数量。

## 十一、实现验收重点

- 无 Git 也能创建普通目录副本；Git dirty 不影响 Ready。
- 主/子仓库 tracked 与 untracked 组合、暂存新增/删除、gitlink 与外部元数据分别有真实用例。
- Git 检查不能写源或副本元数据，不执行未批准的外部程序；unknown 可强制清理且记日志。
- 普通拒绝不删文件；显式强制只删正确目标，日志在目标删除后仍可读取。
- 创建、日志写入、部分删除、元数据提交前后中断可解释；重试不依赖已删除 Git 数据。
- 用户工具可以直接操作路径；没有 Git 托管拓扑、自动交付或新审计服务。

发布资格、错误码和可见状态矩阵只由用户手册维护；本节不是已经完成的验证记录。

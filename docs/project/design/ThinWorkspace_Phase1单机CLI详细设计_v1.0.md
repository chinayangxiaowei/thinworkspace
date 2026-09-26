# ThinWorkspace Phase 1 单机 CLI 详细设计

**版本：v1.0（首发前修订）｜文档性质：详细设计｜适用阶段：Phase 1**

## 一、文档职责

本文是单机组件、实例身份、Workspace 生命周期、只读 Git 检查和失败边界的唯一详细设计来源。Phase 1 不实现中断后的自动续做、重放或修复。

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
| BootstrapStore | 准备/校验实例与受控目录，读取并发布标记，按历史归属证明清理 ID 容器 |
| PlatformProbe | 报告实际路径与候选后端能力 |
| WorkspaceMaterializer | 目录物化；空间测量在 P1-13 增量加入 |
| GitInspector | 发现工作区内仓库并只读报告已跟踪变更；不写 refs/index/config、不提交、不联网 |
| ProcessProbe | 尽力报告当前用户可见的外部进程占用，不托管或终止进程 |
| MetadataStore | 初始化/只读打开 SQLite，并持久化 Workspace 状态、最终物化 Receipt 和删除结果 |
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

初始化使用既有七个 Port，不新增平行的目录或数据库初始化 Port。`BootstrapStore` 先创建/验证固定 bootstrap 目录，Application 再取得 bootstrap lock；CLI 只在 composition root 注入具体 Adapter，不直接编排文件系统或 SQLite。

持锁后的首次初始化顺序固定为：读取 config → 验证最近存在父目录并逐层 no-follow 创建/验证私有 data root → 从最终目录打开 FD 取得实际 APFS Volume UUID 和目录身份，形成不可伪造的 prepared proof → 以该 proof 写 `initializing` 标记并将同一打开目录移入 `InitializingProof` → 以该 proof 建立受控子目录和数据库布局证明 → 通过 `MetadataStore` 的 factory 边界事务化初始化 SQLite → 重验布局证明 → 原子发布 `ready` 标记 → 最后发布 bootstrap config。Application 生成 InstanceId 并提供非负 Unix 毫秒时间；Adapter 不替代 Application 决定幂等、冲突或错误码。

data root 已存在时必须是当前用户拥有、模式精确为 `0700` 的真实空目录；不自动改权限，不接管链接或非空目录。路径缺失后缀以 `mkdirat`/等价 dirfd-relative 方式逐层创建为 `0700` 并同步父目录；中途失败允许留下尚无 marker 的空目录，但不冒报初始化成功。root marker 建立后，`metadata`、`logs`、`workspaces`、`staging`、`trash` 只能在同一持有目录下 create-new 为 `0700`；`metadata/state.db` 由布局步骤通过 dirfd-relative、no-follow、create-new 方式预建为 `0600` 并绑定其目录项身份，SQLite 初始化只能以 no-create 方式打开该证明内的文件。日志文件和 lifecycle lock 在各自首次使用时建立，不作为 init 成功的空占位物。

已有 config 时，Application 先将本次 `--data-root` 的规范路径字节与登记值比较；不同立即返回 `E_DATA_ROOT_CHANGE_UNSUPPORTED`，不得访问或创建新路径。相同时，重复 init 只在 Ready marker、当前 data-root Volume ID、受控子目录和 SQLite installation 全部一致时返回 `already-initialized`；它不重写文件、不补建缺失布局。

doctor 使用相同校验的产品状态只读路径和 SQLite read-only 打开，不取得 lifecycle lock、不创建受控目录或配置、不修改主数据库、schema、installation、Workspace 行或 journal mode，也不修复任何对象。SQLite 为读取 WAL 数据库可能创建、更新或删除同目录的 `state.db-wal`/`state.db-shm` 引擎协调文件；这不是产品状态写入，不能宣称文件系统零写入。所需辅助文件无法访问时返回 `E_METADATA`，不得使用 `immutable=1` 绕过锁和变化检测。MetadataStore 的只读快照返回 installation 和按 WorkspaceId 稳定排序的活动 Workspace；doctor 只统计其中非 Ready 项并报告数量，数量大于零本身不是根布局损坏，也不改变 `status=ready`。root marker 仍为 `initializing` 则属于未完成实例初始化并返回 `E_DATA_ROOT_LAYOUT`。P1-03 只报告 Git 检查尚未启用；P1-16 落地 GitInspector 后，由 P1-10 接入公开 status 查询并更新 doctor 的能力事实，doctor 本身不启动 Git。

只接管新目录或空目录；已完整初始化且身份一致时幂等返回。初始化中断留下的非空目录不自动接管，需用户确认后在平台之外显式清理，再重新初始化。没有 reset/migrate。细化的错误行为由手册管理。

### 3.3 布局与源目录

```text
thinws-data/
├── .thinws-root.toml
├── metadata/
│   ├── state.db
│   ├── lifecycle.lock
│   └── ownership-<workspace-id>.toml
├── logs/operations.jsonl
├── workspaces/<workspace-id>/.state/incomplete
├── workspaces/<workspace-id>/root/
├── staging/
└── trash/
```

受控目标由 WorkspaceId 推导，全部受控子树位于登记的同一 APFS Volume。`metadata/ownership-<workspace-id>.toml` 的持久目录身份契约由 ADR-0004 §4.8 维护，不复制到 `root/`，也不随 Workspace 删除。源目录是用户提供的只读输入，不需要位于 data root 内，但首版要求与 data root 同卷；拒绝源与 data root 相等或互相包含，防止把平台目录递归复制进自身。根和路径组件不接受未验证链接，遍历边界以物化设计为准。

`.state` 与日志均在副本 `root/` 外；`.git` 不是平台保留项，它仅是被复制的目录内容。用户不能手工移动或改写平台管理目录。

## 四、身份与持久化

InstanceId、WorkspaceId 是强类型 ID。日志关联用的 OperationId 不表示可重放的持久操作。源路径不是新领域对象，不创建 SourceId/RepositoryId/BaseId。Workspace 记录源的规范位置、创建时路径/卷证据、创建策略、受控目标、状态和最终物化 Receipt；这些是本次复制的来源证据，不是 Git 基线或持续同步关系。

SQLite 维护版本、活跃 Workspace、最终物化 Receipt 及最小删除 tombstone：

- 活跃名称、受控目标唯一；ID、状态、关系由数据库约束；
- 未完成状态不能因目录存在或进程重启而自动转为 Ready；
- 删除活跃记录与写 tombstone 在同一事务完成；
- 不在数据库事务中等待文件物化、Git 检查或子进程。

bootstrap/root marker 的精确字段、SQLite schema/trigger/migration 和双 scope 锁文件协议只由 [ADR-0004](../architecture/adr/ADR-0004_Phase1持久化Schema与双Scope锁.md) 维护；本文只定义它们在生命周期中的职责与顺序。

源目录后续消失不影响已创建副本的普通使用或清理。没有文件系统和 SQLite 的跨系统事务；不提供中断操作重放。

## 五、并发与源一致性

bootstrap lock 仅保护初始化；data-root lifecycle lock 串行创建、删除和 GC，均有界等待。它不能阻止用户工具读写源或副本，SQLite busy timeout 不能替代 OS 锁。

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
Creating/Deleting/Error → Deleting    仅显式 --force 清理且受控范围可证明
```

Ready 由物化 Receipt、归属和持久化状态一致决定，不要求 Git clean，也不要求任何提交或分支。

### 7.1 创建

写 Creating → 建立 incomplete → Probe/Plan → 对空目标物化原始目录 → 验证最终 Receipt 和来源观察结果 → 移除 incomplete → 写 Ready。

物化前的目标 Probe 报告必须与 `prepare_workspace` 持有的原始 `root/` 目录身份一致；运行时 CoW 失败后重新 Probe、选择 Full Copy 时也必须重新核对。同名空目录或同卷路径不能替代该身份。Materializer 的 Plan 重验继续防止核对之后的路径替换。

同名重复请求的公开规则由手册管理；不会隐式刷新已有副本。失败保留可解释的非 Ready 状态；无法确认回滚时不继续第二后端。重启后不续做创建，目录存在也不等于成功。

### 7.2 清理

用户停止相关工具后，按以下顺序执行：

1. 验证实例、WorkspaceId、卷及创建时持久目录归属；运行只读 Git 与进程检查。
2. Application 按手册决定普通拒绝或接受显式强制意图；拒绝发生在破坏性写入之前。
3. 清理前写持久日志，并将 Workspace 标记为 Deleting；日志关联 ID 仅用于定位这次尝试，不用于重放。
4. 每个破坏性步骤前重验范围、历史目录身份、卷和适用的占用保护；Application 只向持有 data-root 锁与目录归属证据的 `BootstrapStore` 授权一个 WorkspaceId。该 Adapter 从已验证目录 FD 以 no-follow 清理副本 `root/` 的全部内容（包括 `.git` 和后来生成的内容），再清理同一容器下的平台标记与空容器。不得把可由调用者构造的裸路径当作删除授权。位于 `metadata/` 的归属文件保留，不作为清理目标。
5. 确认整个 `workspaces/<workspace-id>/` 已不存在；在同一事务删除活跃记录并保留最小 tombstone；记录完成结果。data root 内的日志不随工作区删除。

强制操作不要求说明理由、commit 证明、主管批准或在线服务；只保留日志。日志字段与敏感信息边界见《开发规范》§13。清理前无法持久化日志时停止且报告 I/O 错误；已开始后失败/中断保留非 Ready 状态和剩余范围，不能写成成功。

普通清理只保护已跟踪变更，不保护未跟踪文件，也不检查未推送提交。强制可绕过 dirty 或 Git 检查不完整，不能绕过路径/卷归属或已确认进程占用。非 Git 目录也可正常清理。

### 7.3 未完成清理与日志

尚未开始删除的普通拒绝不固定后续 flags；用户可重新执行显式强制清理。删除中断后不自动续跑，也不重放旧 Git 检查结果。若 WorkspaceId 容器目录仍在，用户可重新发起一次 `remove --force`；每次都重新验证实例、卷、目录归属和当前占用，绝不扩大到登记范围外。

部分删除后 Git 检查可能已不可用，因此普通清理拒绝；只有新的显式 `--force` 可授权清理整个仍归属本实例的剩余目录。若 root 已被删去，须以仍在的历史归属文件和容器身份确认剩余平台标记；若整个 ID 容器经持锁、no-follow 验证已经不存在，即使归属文件缺失，也只完成 Deleting 行的数据库收口，不再执行路径删除。若 root 被替换、归属文件缺失或损坏但容器仍在、容器身份不符，则停止而不删除新对象。创建过程尚未持久化归属文件的极短失败窗口只允许用户在平台外核对并清除残留目录，再显式 force 释放名称；不能为了可恢复性放松删除证明。

日志位于 data root 的 `logs/`，不随 Workspace 清理或 GC 删除。它是普通本机日志，不宣称防篡改或构成成果证明。中断可能只有开始事件；缺少完成事件不能解释为成功。

## 八、查询与未完成状态

list/status 展示全部活跃状态；path 仅 Ready。查询不取得 lifecycle lock，不写 SQLite/Git 产品状态，也不自动清理或恢复；Ready 路径在只读快照、受控目录归属核验及再次只读快照一致后返回。P1-12 引入持久归属文件后，Ready 核验也要对照创建时目录身份；status 对 Ready 完成 Git 检查后再次核验状态与目录归属，避免把检查期间进入非 Ready 的副本仍作为可用路径输出。检测到的并发状态或路径身份变化必须拒绝可用路径，但检查结束后仍不能保证路径持续存在。SQLite read-only 打开可能更新 WAL 协调文件，因此不承诺文件系统字节零变化。Git unknown 不会把一个物化完整的 Ready 副本变为不可用，也不阻止获取路径。

doctor 只读报告不一致及未完成状态，没有 `--repair`。创建中断后不自动重新镜像最新源；调用者可显式强制清理登记的残留 Workspace，再以释放后的名称重新创建。初始化中断的 data root 不属于已登记 Workspace，不能交给 `workspace remove` 删除。

## 九、外部进程占用

ProcessProbe 通过 macOS Adapter 报告当前用户可见进程对已验证 `workspaces/<workspace-id>/` 容器（含普通 `root/`）的 cwd/open-vnode 占用。进程身份不能只靠 PID；结果包含探测时间和完整性：

- `confirmed-in-use`：阻止删除，包括强制清理；
- `no-evidence`：没有取得占用证据，不等于绝对无人使用；
- `scan-incomplete`：警告并记录，不能伪装成无占用。

平台不停止用户工具，不阻止其他程序在检查后进入目录；用户负责清理窗口内不再写入。内部 Git 子进程的有界退出和输出处理属于 GitInspector 实现，不扩展为执行包装。

## 十、GC 与空间统计

GC 仅处理已完成操作留下、归属可证明且明确标为可回收的 staging/trash 残留。不删除任何活跃 Workspace（包括 Creating、Deleting、Error）、日志、用户源目录或外部路径；不运行 Git GC/prune，不管理 Git refs。未完成 Workspace 只能由显式 `workspace remove --force` 清理，不能让 GC 绕过清理边界。

计划在一致的只读元数据视图中形成，执行在 lifecycle lock 内重验。空间统计区分逻辑大小、可取得的物理估算及未知值；不承诺逐副本精确可回收共享块数量。

## 十一、实现验收重点

- 无 Git 也能创建普通目录副本；Git dirty 不影响 Ready。
- 主/子仓库 tracked 与 untracked 组合、暂存新增/删除、gitlink 与外部元数据分别有真实用例。
- Git 检查不能写源或副本元数据，不执行未批准的外部程序；unknown 可强制清理且记日志。
- 普通拒绝不删文件；显式强制只删正确目标，日志在目标删除后仍可读取。
- 创建或删除中断后不报告成功、不自动重放；显式强制清理只作用于登记且归属可证的整个工作区目录。
- 用户工具可以直接操作路径；没有 Git 托管拓扑、自动交付或新审计服务。

发布资格、错误码和可见状态矩阵只由用户手册维护；本节不是已经完成的验证记录。

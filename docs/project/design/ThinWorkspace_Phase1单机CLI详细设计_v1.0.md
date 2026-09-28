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
| BootstrapStore | 准备/校验用户控制目录，读取并发布实例配置；按持久归属证明准备、核验和清理用户指定 target；只读扫描已验证副本的当前空间 |
| PlatformProbe | 报告实际路径与候选后端能力 |
| WorkspaceMaterializer | 目录物化 |
| GitInspector | 发现工作区内仓库并只读报告已跟踪变更；不写 refs/index/config、不提交、不联网 |
| ProcessProbe | 尽力报告当前用户可见的外部进程占用，不托管或终止进程 |
| MetadataStore | 初始化/只读打开 SQLite，并持久化 Workspace 状态、最终物化 Receipt 和删除结果 |
| LifecycleLock | `~/.thinws` 中所有 Workspace 写生命周期的有界互斥 |

不保留旧 GitBackend 的 attach/detach/fetch/commit 等方法，不新增 AuditService 或交付编排 Port。普通持久日志使用既有日志设施。

## 三、实例与用户控制目录

### 3.1 实例身份

macOS 固定控制位置：

```text
~/.thinws/
├── config.toml
├── metadata/state.db
├── lifecycle.lock
└── logs/operations.jsonl
```

配置保存新布局的 schema version 和 InstanceId，不登记单一数据卷或默认 Workspace 输出位置。控制目录固定在当前用户主目录，生产 CLI 不提供隐藏环境变量切换位置；测试通过依赖注入替换。控制目录必须是当前用户拥有、不可由其他用户写入的真实目录，配置和 SQLite 安全打开；此目录所在卷不参与某个 Workspace 的 APFS 同卷判断。

SQLite installation 与配置中的 InstanceId 必须一致。旧 `~/Library/Application Support/ThinWorkspace/` 和旧 data root 不被新版本读取、迁移、接管或删除；旧版无法直接用于新命令。

### 3.2 初始化

初始化使用既有七个职责，不新增平行的目录或数据库初始化 Port。`BootstrapStore` 先创建/验证固定控制目录，Application 再取得其中的 lifecycle lock；CLI 只在 composition root 注入具体 Adapter，不直接编排文件系统或 SQLite。

持锁后的首次初始化顺序为：读取 config → 逐层 no-follow 创建/验证固定私有控制目录 → 建立布局与数据库文件身份的不可伪造证明 → 通过 `MetadataStore` 的 factory 边界事务化初始化新 SQLite schema → 重验控制目录与数据库证明 → 最后原子发布 config。Application 生成 InstanceId 并提供非负 Unix 毫秒时间；Adapter 不替代 Application 决定幂等、冲突或错误码。配置未发布、SQLite 不一致或初始化中断都不得被 doctor 当成 Ready，也不跨进程自动接管。

控制目录不存在时逐层安全创建为 `0700`；已存在时必须是属主和权限均合格的真实目录，不接管符号链接或未知非空目录。`metadata/state.db` 通过 dirfd-relative、no-follow、create-new 预建为 `0600` 并绑定身份，SQLite 初始化只能以 no-create 方式打开该证明内的文件。日志和锁在各自首次使用时建立，不作为 init 成功的空占位物。新布局不在源卷或目标卷创建永久的平台管理根。

已有 config 时，重复 init 只有在固定控制目录、config 与 SQLite installation 全部一致时返回 `already-initialized`；它不重写文件、不补建缺失布局。`init` 不接受 `--data-root`，也不因旧布局存在而自动导入。

doctor 使用相同校验的产品状态只读路径和 SQLite read-only 打开，不取得 lifecycle lock、不创建受控目录或配置、不修改主数据库、schema、installation、Workspace 行或 journal mode，也不修复任何对象。SQLite 为读取 WAL 数据库可能创建、更新或删除同目录的 `state.db-wal`/`state.db-shm` 引擎协调文件；这不是产品状态写入，不能宣称文件系统零写入。所需辅助文件无法访问时返回 `E_METADATA`，不得使用 `immutable=1` 绕过锁和变化检测。MetadataStore 的只读快照返回 installation 和按 WorkspaceId 稳定排序的活动 Workspace；doctor 报告控制目录状态与非 Ready 项数量，某个外置目标卷离线不得被误判为全局实例损坏。Git 可用性仅表示检查能力，不代表对所有目标都已检查。

只接管新控制目录或已具有同一新布局身份的现有目录；未知非空目录及初始化中断残留不自动接管。没有 reset/migrate。细化的错误行为由手册管理。

### 3.3 布局与源目录

最终 target 是用户提供的绝对路径，而不是从控制目录或 WorkspaceId 推导。控制目录保存 WorkspaceId、名称、source、target、目标卷和创建时持久归属证据；target 卷成功状态只保留普通克隆目录，不写 `.state`、日志或标记文件。源目录是只读输入，必须与 target parent 同一 APFS Volume；源、target 和控制目录的包含关系须在写入前拒绝。根和路径组件不接受未验证链接，遍历边界以物化设计为准。若需要同卷暂存或隔离，位置由 target parent 和类型 ID 推导，成功后清除；中断残留不自动回收。

`.git` 不是平台保留项，它仅是被复制的目录内容。用户可直接修改 target 内文件，但手工移动整个 target 后，平台不得仅凭同名路径继续认领或删除。

## 四、身份与持久化

InstanceId、WorkspaceId 是强类型 ID。日志关联用的 OperationId 不表示可重放的持久操作。源路径不是新领域对象，不创建 SourceId/RepositoryId/BaseId。Workspace 记录源的规范位置、用户指定的完整 target、创建时源/目标卷与目录身份、创建策略、状态和最终物化 Receipt；这些是本次复制和删除归属的证据，不是 Git 基线或持续同步关系。

SQLite 维护版本、活跃 Workspace、最终物化 Receipt 及最小删除 tombstone：

- 活跃名称、用户指定的完整 target 唯一；ID、状态、关系及 target 归属由数据库约束；
- 未完成状态不能因目录存在或进程重启而自动转为 Ready；
- 只有确认登记 target 实际存在且归属可证、完成物理清理后，才能在同一事务删除活跃记录并写 tombstone；target 缺失不能收口；
- 不在数据库事务中等待文件物化、Git 检查或子进程。

新控制目录、SQLite schema/trigger、归属证据和单 scope 锁协议由 [ADR-0006](../architecture/adr/ADR-0006_Phase1用户控制目录与显式目标路径.md) 维护；ADR-0004 只描述旧候选，不迁移或兼容。本文只定义它们在生命周期中的职责与顺序。

源目录后续消失不影响已创建副本的普通使用或清理。没有文件系统和 SQLite 的跨系统事务；不提供中断操作重放。

## 五、并发与源一致性

`~/.thinws/lifecycle.lock` 串行初始化、创建和删除，均有界等待。它不能阻止用户工具读写源或副本，SQLite busy timeout 不能替代 OS 锁。Phase 1 不实现 GC，见 [ADR-0005](../architecture/adr/ADR-0005_Phase1暂不实现GC.md)。

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

验证用户指定 target 尚不存在及 source/target parent 同卷 → 写 Creating 及完整 target 登记 → 以不可伪造的父目录证明创建目标并持久记录其历史归属 → Probe/Plan → 对已证明归属的空 target 物化原始目录 → 验证最终 Receipt 和来源观察结果 → 写 Ready。不得在 target 写入平台 `.state` 文件。

物化前的目标 Probe 报告必须与 `prepare_workspace` 持有并已持久登记的 target 目录身份一致；运行时 CoW 失败后重新 Probe、选择 Full Copy 时也必须重新核对。同名空目录或同卷路径不能替代该身份。Materializer 的 Plan 重验继续防止核对之后的路径替换。

同名重复请求的公开规则由手册管理；不会隐式刷新已有副本。失败保留可解释的非 Ready 状态；无法确认回滚时不继续第二后端。重启后不续做创建，目录存在也不等于成功。

### 7.2 清理

用户停止相关工具后，按以下顺序执行：

1. 验证实例、WorkspaceId、登记 target 的存在、卷及创建时持久目录归属；运行只读 Git 与进程检查。持锁的 BootstrapStore 只读指出当前唯一且已证明归属的 target 位置（原位置或上次清理留下的隔离位置）供 ProcessProbe 检查；两处冲突、两处均缺失、任一现存目录无证明都拒绝，不由 Application 自行拼路径或择一。
2. Application 按手册决定普通拒绝或接受显式强制意图；拒绝发生在破坏性写入之前。
3. 清理前写持久日志，并将 Workspace 标记为 Deleting；日志关联 ID 仅用于定位这次尝试，不用于重放。
4. 每个破坏性步骤前重验范围、历史目录身份、卷和适用的占用保护；Application 只向持有 lifecycle lock 与持久目录归属证据的 `BootstrapStore` 授权一个 WorkspaceId。该 Adapter 先以不覆盖的同卷 rename 将已证明归属的 target 隔离到同一 target parent 下本 WorkspaceId 的私有隔离位置，再核对移动后的历史身份；仅在已确认的隔离目录内，从目录 FD 以 no-follow 清理全部副本内容（包括 `.git` 和后来生成的内容）。不得在公开 target 路径上按 stat 后的名称递归 unlink，也不得把可由调用者构造的裸路径当作删除授权。
5. 本次清理开始时必须有已证明归属的 target 原目录或其已登记隔离位置；确认物理清理完成后，才在同一事务删除活跃记录并保留最小 tombstone，记录完成结果。控制目录中的日志不随工作区删除。

强制操作不要求说明理由、commit 证明、主管批准或在线服务；只保留日志。日志字段与敏感信息边界见《开发规范》§13。清理前无法持久化日志时停止且报告 I/O 错误；已开始后失败/中断保留非 Ready 状态和剩余范围，不能写成成功。

普通清理只保护已跟踪变更，不保护未跟踪文件，也不检查未推送提交。强制可绕过 dirty 或 Git 检查不完整，不能绕过路径/卷归属或已确认进程占用。非 Git 目录也可正常清理。

### 7.3 未完成清理与日志

尚未开始删除的普通拒绝不固定后续 flags；用户可重新执行显式强制清理。删除中断后不自动续跑，也不重放旧 Git 检查结果。若已证明归属的 target 仍位于原位置或同卷私有隔离位置，用户可重新发起一次 `remove --force`；每次都重新验证实例、卷、目录归属和当前占用，绝不扩大到登记范围外。两个位置同时存在或隔离项缺少历史证明时拒绝，不合并内容，不择一删除。

部分删除后 Git 检查可能已不可用，因此普通清理拒绝；只有新的显式 `--force` 可授权清理仍存在且归属本实例的剩余目录。**原 target 与已登记隔离目录均不存在时，不能仅凭 SQLite 中的 Deleting 或 Error 状态释放名称或写 tombstone**；外置卷离线、target 被手工删除及物理删除后数据库收口前中断均保留登记。target 被替换、归属证据缺失或损坏时停止而不删除新对象。创建过程尚未持久化归属证据的极短失败窗口不允许产品凭路径外观补造证据或强删。

日志位于 `~/.thinws/logs/`，不随 Workspace 清理删除，也不写进克隆项目目录。持锁的 BootstrapStore 同步追加并 fsync 清理 JSONL 事件；这属于受控文件持久化，不新增日志 Port 或审计服务，不依赖 stderr 日志级别。它是普通本机日志，不宣称防篡改或构成成果证明。中断可能只有开始事件；缺少完成事件不能解释为成功。

## 八、查询与未完成状态

list/status 展示全部活跃状态；path 仅 Ready。查询不取得 lifecycle lock，不写 SQLite/Git 产品状态，也不自动清理或恢复；Ready 路径在只读快照、登记 target 的目录归属核验及再次只读快照一致后返回。Ready 核验要对照 `~/.thinws` 中的创建时目录身份；status 对 Ready 完成 Git 检查后再次核验状态与目录归属，避免把检查期间进入非 Ready 的副本仍作为可用路径输出。检测到的并发状态或路径身份变化必须拒绝可用路径，但检查结束后仍不能保证路径持续存在。SQLite read-only 打开可能更新 WAL 协调文件，因此不承诺文件系统字节零变化。Git unknown 不会把一个物化完整的 Ready 副本变为不可用，也不阻止获取路径。

doctor 只读报告控制目录不一致及未完成状态，没有 `--repair`。创建中断后不自动重新镜像最新源；调用者只能显式强制清理**仍存在且归属可证**的残留 Workspace。target 已缺失的登记不因此释放名称或自动恢复。

## 九、外部进程占用

ProcessProbe 通过 macOS Adapter 报告当前用户可见进程对已验证 target 目录的 cwd/open-vnode 占用。进程身份不能只靠 PID；结果包含探测时间和完整性：

- `confirmed-in-use`：阻止删除，包括强制清理；
- `no-evidence`：没有取得占用证据，不等于绝对无人使用；
- `scan-incomplete`：警告并记录，不能伪装成无占用。

平台不停止用户工具，不阻止其他程序在检查后进入目录；用户负责清理窗口内不再写入。内部 Git 子进程的有界退出和输出处理属于 GitInspector 实现，不扩展为执行包装。

## 十、空间统计与回收边界

Phase 1 没有 GC 命令、GC 计划或自动回收。用户只能显式 `workspace remove` 清理已登记、仍存在且归属可证的 target 或对应删除隔离位置；Creating、Deleting、Error 状态的剩余目录须按 §七重新显式授权 `--force`，不从查询或目录名称推断清理资格。target 缺失时保留登记；无法持久证明归属的暂存/隔离残留不由产品扫描删除。控制目录日志、源目录及其他外部路径不属于 Workspace 清理对象。此范围决定见 [ADR-0005](../architecture/adr/ADR-0005_Phase1暂不实现GC.md)。

空间统计由已有 BootstrapStore 对当前已验证的 Ready target 进行只读、no-follow、同卷的有界扫描，避免新增单方法 Port 或经未绑定路径重新进入；每个普通文件和符号链接目录项的 `st_size` 计入逻辑字节，目录不计入逻辑字节，`st_blocks` 对同一卷内去重后的 inode 计入已分配字节估算。副本内部条目扫描失败、越界、特殊类型、卷变化或条目身份变化时两个数值均为 unknown，不以部分结果冒充完整；target 的归属核验失败仍按 §八拒绝可用路径，不能降为 unknown。非 Ready 不扫描。已分配字节是当前文件系统报告的估算，APFS CoW 共享块可能重复计入，不等于删副本可释放的独占空间。

## 十一、实现验收重点

- 无 Git 也能创建普通目录副本；Git dirty 不影响 Ready。
- 主/子仓库 tracked 与 untracked 组合、暂存新增/删除、gitlink 与外部元数据分别有真实用例。
- Git 检查不能写源或副本元数据，不执行未批准的外部程序；unknown 可强制清理且记日志。
- 普通拒绝不删文件；显式强制只删正确目标，日志在目标删除后仍可读取。
- 创建或删除中断后不报告成功、不自动重放；显式强制清理只作用于登记且归属可证的整个工作区目录。
- 用户工具可以直接操作路径；没有 Git 托管拓扑、自动交付或新审计服务。

发布资格、错误码和可见状态矩阵只由用户手册维护；本节不是已经完成的验证记录。

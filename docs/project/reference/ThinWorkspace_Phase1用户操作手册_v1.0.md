# ThinWorkspace Phase 1 用户操作手册

**版本：v1.0（首发前修订）｜对应阶段：Phase 1 单机 CLI MVP｜性质：目标态公开契约，尚非已发布产品**

## 一、职责与非职责

本手册维护命令、参数、输出、JSON、错误码和用户使用边界。内部状态协调、物化步骤和系统 API 分别由详细设计、跨平台物化设计、技术栈管理；任务交付规则由任务流程管理，不在此重复实现。

Phase 1 只做一件事：把本机现有目录镜像为低额外磁盘占用的普通工作区，供用户或 Agent 直接使用，结束后按明确规则清理。不接管 Git 分支、提交、PR 或用户命令。

## 二、五分钟体验

以下是首版目标操作顺序。当前仓库已有相应开发中命令，但在阶段验收与人工放行前不视为正式发布产品：

```bash
thinws init --data-root /Volumes/data/thinws-data
thinws doctor
thinws workspace create --source /Volumes/data/code/my-app --name auth-refresh --dry-run
thinws workspace create --source /Volumes/data/code/my-app --name auth-refresh
cd "$(thinws workspace path auth-refresh)"
# 直接编辑、运行项目自己的构建或测试命令
thinws workspace status auth-refresh
# 需要交付的内容由你或 Agent 使用自己的 Git 流程保存
cd /Volumes/data/code
thinws workspace remove auth-refresh
```

目录可以不含 Git。源目录不需要先注册；没有 repo add/fetch/list、--repo、--base 或 workspace exec。Phase 1 也不包装 diff：需要 patch 时直接使用 Git。

## 三、初始化与能力检测

```bash
thinws init --data-root /Volumes/data/thinws-data
thinws doctor
```

首发验收目标为 arm64 macOS 15.7.2、APFS 和 Apple Git 2.39.5；这是已有实验环境与后续资格目标，不代表新流程或所有旧 macOS 已验收。Git 仅用于按需检查，缺少 Git 不影响目录物化。

init 只接管新目录或空目录；已完整初始化且身份一致时重复指定原路径幂等。中断初始化留下的非空目录不自动接管，需用户核对后在平台之外显式处理。指定另一 data root 返回 E_DATA_ROOT_CHANGE_UNSUPPORTED，非归属的非空目录返回 E_DATA_ROOT_NOT_EMPTY。无 reset/migrate。

配置固定在 `~/Library/Application Support/ThinWorkspace/config.toml`，不通过环境变量切换。数据卷身份必须与登记匹配；卷不在线时不在其他位置新建替代目录。

doctor 对 ThinWorkspace 产品状态只读，报告主机、数据根、未完成工作区与 Git 检查是否可用。它不修改配置、root marker、主数据库、schema 或 Workspace 记录；SQLite 读取 WAL 数据库时可能管理同目录的 `state.db-wal`/`state.db-shm` 协调文件，因此该承诺不是文件系统字节零变化。只读预检不是 CoW 成功证据；没有 `doctor --repair`，也不自动续做中断操作。

init 的人类输出固定为以下字段；重复初始化只改变 `Result`：

```text
ThinWorkspace initialized
Result:      initialized
Data root:   /Volumes/data/thinws-data
Volume ID:   550e8400-e29b-41d4-a716-446655440000
```

`Result` 只能是 `initialized|already-initialized`。成功 JSON 为：

```json
{
  "schema_version": 1,
  "ok": true,
  "data": {
    "command": "init",
    "result": "initialized",
    "instance_id": "01890a5d-ac96-774b-bd5b-55c7b8d09f33",
    "data_root": "/Volumes/data/thinws-data",
    "data_root_hex": "2f566f6c756d65732f646174612f7468696e77732d64617461",
    "volume_id": "550e8400-e29b-41d4-a716-446655440000"
  }
}
```

`data_root` 是供人阅读的显示值，`data_root_hex` 才是无损路径字节。doctor 仅在 config、Ready marker、当前卷、受控布局和 SQLite installation 全部一致时成功；Git 状态查询接入后的成功输出为：

```text
ThinWorkspace doctor
Status:              ready
Host:                macos/aarch64
Data root:           /Volumes/data/thinws-data
Incomplete workspaces: 0
Git check:           available
```

doctor JSON 的 `data` 固定包含 `command="doctor"`、`status="ready"`、`host.platform`、`host.architecture`、`instance_id`、`data_root`、`data_root_hex`、`volume_id`、`incomplete_workspaces`，以及 `git_check={"available":true,"reason":null}`。`available` 表示公开状态命令已接入 Git 检查，不保证当前系统 Git 或某个仓库可安全查询；实际查询失败由 status 的 `git.state=unknown` 和原因表达。`incomplete_workspaces` 统计已登记但非 Ready 的活动 Workspace；大于零是诊断事实，不使本次只读 doctor 失败。未初始化返回 E_NOT_INITIALIZED；既有 config 指向另一 data root 时 init 返回 E_DATA_ROOT_CHANGE_UNSUPPORTED；非 APFS/无 Volume UUID 返回 E_CAPABILITY_UNAVAILABLE；非空未归属目录返回 E_DATA_ROOT_NOT_EMPTY；已登记 data root 缺失返回 E_DATA_ROOT_UNAVAILABLE；实例身份、权限、受控布局或 root marker 仍处于 initializing 返回 E_DATA_ROOT_LAYOUT；SQLite/schema 失败返回 E_METADATA；锁等待仍为 E_LOCK_TIMEOUT。

## 四、原始目录镜像

### 4.1 创建与计划

```bash
thinws workspace create --source /Volumes/data/code/my-app --name auth-refresh
```

source 和 data-root 参数使用本机绝对目录路径；来源可以位于 data root 外，但首版要求与其同一 APFS Volume，且两者不相等、不互相包含。不支持 URL、任意目标目录、子挂载或根路径的符号链接重定向。

目标输出示例（时间仅为示意，不是性能承诺）：

```text
Workspace ready

Result:          created
Name:            auth-refresh
Workspace ID:    ws_019...
Source:          /Volumes/data/code/my-app
Path:            /Volumes/data/thinws-data/workspaces/ws_019.../root
Actual mode:     cow-clone
CoW:             confirmed
Fallback:        none
Git setup:       not performed
```

加 `--dry-run` 只展示当前 source/target 卷关系、目标路径模式、后端计划和降级原因，不分配 WorkspaceId、不预留名称、不保留可执行 plan token。正式创建重新检测。
预览中的目标位置是已存在的 `workspaces/` 父目录，正式路径仍由新 WorkspaceId 派生；预览不生成实际复制结果或 CoW 证明。重复创建已 Ready 的同名同参数工作区时，人类输出的 Result 为 `already-ready`，JSON 的 result 同名。

名称必填，长度 1–63，匹配 `^[a-z0-9](?:[a-z0-9._-]{0,61}[a-z0-9])?$` 且不含连续 `..`；不自动归一化。活跃名称全局唯一，实际目录只由 WorkspaceId 推导。`workspace path/status` 当前按名称查询；`workspace remove` 可用名称或完整 ID。Workspace ID 为 `ws_` 加标准小写 UUIDv7。若 `workspace remove` 的位置参数同时是某个活跃名称和另一个 Workspace 的完整 ID（含已删除 ID），命令拒绝而不猜测目标；可写 `name:<名称>` 或 `id:<完整 ID>` 明确指定。两种前缀只用于清理目标，不属于 Workspace 名称。清理输出的 Operation ID 为 `op_` 加标准小写 UUIDv7，仅关联本次日志，不代表可恢复操作。示例缩写不是真实可执行 ID。

相同名称、规范源路径和创建策略，若已有 Ready 记录则返回原副本，不重新复制，也不比较源是否已变。想复制当前源的新内容必须使用新名称；同名参数不同返回 E_NAME_CONFLICT，原记录未 Ready 返回 E_WORKSPACE_INCOMPLETE。该幂等返回不会因来源随后消失而失效。

### 4.2 复制哪些内容

创建按磁盘实际内容复制目录、普通文件和符号链接：

- 包括未提交文件、未跟踪文件、ignored 文件和已有构建产物，不执行 Git checkout；
- `.git` 作为普通目录内容复制，不创建托管仓库或自动切换分支；
- sparse 目录只复制磁盘上实际存在的内容，不补全未检出的文件；LFS 指针若尚未下载，复制后仍是指针；
- 不按语言配置缓存，不删除应用锁标记，不自动“解锁”；
- 设备文件、socket、FIFO、子挂载等超出首版物化范围时明确失败，不静默跳过。

创建期间请暂停源目录写入。逐文件复制不是活跃数据库或编译过程的原子快照；检测到源变化时失败并保留可解释状态。

共同保证为名称、类型、普通文件内容/权限、目录权限、链接文本，以及普通文件和目录 mtime。完整 ACL、xattr、所有者、ctime、出生时间、file flags、稀疏分配和硬链接拓扑不作首版保真承诺；各后端实际保留的额外属性不能替代这项边界。来源读取可能改变 atime。缓存能否复用仍由工具决定，不能由保留 mtime 推导命中。

原样镜像不等于外部引用隔离：绝对符号链接、Git worktree 的外部 `.git` 指针等可能仍访问原位置。平台不重定位它们，不保证任意 Git 命令与来源完全隔离；需要此能力时先由调用者准备自包含来源。普通文件副本写入独立不等于 Sandbox。

### 4.3 CoW 与显式完整复制

默认要求 CoW，不能静默改为完整复制。仅在同卷 clone 不支持时，可显式允许：

```bash
thinws workspace create --source /Volumes/data/code/my-app --name auth-refresh --allow-copy
```

输出必须显示 actual mode、cow 和 fallback 原因。Full Copy 记为 `cow=not-used`；只有实际克隆与校验成功才是 confirmed。空树或只有目录/链接时记为 not-used，不把“没有文件需要克隆”当成已证实块共享。

`--allow-copy` 不是跨卷开关；跨卷、卷身份变化、空间不足或普通 I/O 错误不触发复制降级。中途失败先按已知范围回滚，无法确认时保留 Error，不发布混合或不完整目录。
已预留 Workspace 后的创建失败在错误 `context.workspace_id` 中标明受控对象；若物化已产生失败回执，还给出 `materialization_attempt_count`、`rollback_incomplete` 和 `unconfirmed_staging` 摘要。摘要不表示已自动清理，也不把 partial receipt 写成成功的 final receipt。

## 五、直接使用与查询

```bash
thinws workspace path auth-refresh
thinws workspace list
thinws workspace status auth-refresh
```

path 成功时 stdout 只有原始绝对路径字节和末尾换行，常见路径可用于 `cd "$(thinws workspace path auth-refresh)"`。文件名本身若含换行，不能按物理行数解析此输出；脚本可改用 `workspace status --json` 的无损 `path_hex` 处理。工具直接运行，不需要执行包装；终端、Agent 和工具自己管理环境、超时、输出和 Ctrl-C。

list 展示所有活跃 Workspace，按名称稳定排序，列出名称、ID、状态、源路径和已有最终回执的实际物化模式；尚无成功回执时显示 `not-ready`，不为了列表自动扫描所有仓库。status 返回元数据及当前副本内主仓库/子仓库的按需 Git 检查结果，以及当前 Ready 副本的空间估算；不是创建时回执的字节数。人类输出使用 `Logical bytes` 和 `Allocated bytes (estimate)` 两行。逻辑字节计入普通文件和符号链接的当前大小；已分配字节估算来自文件系统报告，APFS 共享块可能重复计入，不代表删除后可释放的空间。非 Ready 或扫描不完整时空间数值为 unknown，不以部分统计结果冒充完整。用户同时写入时统计不是原子快照，需要稳定数值应暂停写入后复测。

Git 检查只关心当前 HEAD/index 已跟踪内容，包括暂存新增、修改、删除、重命名、模式、冲突及子模块引用变化。未跟踪文件和 ignored 文件不展示、不计数、不阻塞。子仓库只有未跟踪文件时不能使父仓库误报有已跟踪修改。已从版本控制移除的历史路径不按“曾经出现过”追溯。

没有仓库显示 `not-applicable`；检查不完整显示 `unknown` 和原因，不冒称 clean。Git 不可用、外部 Git 元数据或不能安全查询时，Ready 副本仍可取得普通路径。

| 命令 | Creating | Ready | Deleting | Error |
|---|---|---|---|---|
| list/status | 只读诊断 | 只读查询 | 只读诊断 | 只读诊断 |
| path | 拒绝 | 允许 | 拒绝 | 拒绝 |
| remove | 仅显式 --force 清理 | 检查后执行 | 仅显式 --force 清理 | 仅显式 --force 清理 |
| doctor | 只读 | 只读 | 只读 | 只读 |

查询不取得 lifecycle lock，不写平台产品状态或 Git 元数据；Ready 的 `path/status` 通过只读快照、当前受控目录归属核验和再次读取状态检查并发变化，status 还会在 Git 检查后复核，避免输出检查期间失效的 Ready 路径。返回的路径是查询时的事实，不保证之后仍存在。SQLite 只读查询可能更新 WAL 协调文件，因此不承诺文件系统字节零变化。Git unknown 不自行将 Ready 改成 Error。非 Ready 的 status 不启动 Git，返回 `git.state=unknown`、`issues=["workspace-not-ready"]`；`path` 返回 E_WORKSPACE_NOT_READY。Ready 路径当前归属核验失败时 `path/status` 均不输出可用路径。物化未证明完整时不能仅因目录存在返回 Ready。

## 六、清理工作区

### 6.1 普通清理

停止相关工具、离开工作区后执行：

```bash
thinws workspace remove auth-refresh
```

对主仓库及发现的子仓库：

- 已跟踪变更：返回 E_WORKSPACE_DIRTY、非零退出，强提示并保留目录；
- 检查不完整：返回 E_GIT_CHECK_INCOMPLETE、非零退出，列出原因并保留目录；
- 已跟踪内容无变化，或完整发现结果为无 Git 仓库：可以清理。

未跟踪及 ignored 文件不提示、不阻塞，会与整个副本一起删除。未推送 commit、分支、stash、PR 或交付证明不在该检查范围内；仅在副本 `.git` 内的内容也会删除。清理命令不是备份或成果验收。

强提示示例：

```text
Workspace removal refused
Code: E_WORKSPACE_DIRTY
Workspace: auth-refresh
Repository: .
Tracked changes: 3
Repository: libs/sdk
Tracked changes: 1

Preserve and commit the required work using your workflow.
To discard the workspace explicitly:
  thinws workspace remove auth-refresh --force
No files were removed.
```

拒绝输出只列仓库相对位置、已跟踪变更摘要和检查不完整的原因短名；不打印未跟踪清单、源码、完整 argv 或凭据。

### 6.2 显式强制清理

```bash
thinws workspace remove auth-refresh --force
```

force 明确授权丢弃副本内容，绕过 tracked dirty 和 Git 检查不完整；不要求填写理由、提供 commit、联网或取得主管在线批准。无额外交互确认，脚本中的显式 flag 就是清理意图。

force 不绕过实例/目标归属、卷身份、路径安全和已确认进程占用，不跟随符号链接或 Git 指针删除工作区外的内容。容器仍在但缺少或损坏平台归属证明、原 `root/` 被其他目录替换时也必须拒绝删除；不能仅凭当前同名目录、属主或权限推定它是原副本。清理期间容器可能位于实例私有隔离位置；两处均经安全核验不存在时，显式 force 才可以只完成未完成记录的清理并释放名称，不删除任何路径。正常成功时删除整个已登记的容器及其隔离残留；data root 内的日志保留。

```text
Workspace removed
Workspace ID: ws_019...
Forced: yes
Operation ID: op_019...
Log: /Volumes/data/thinws-data/logs/operations.jsonl
Delivery verification: not performed
```

普通拒绝后可改用 force。删除已开始但未完成时，不自动续做；只能由用户再次显式执行 `remove --force`，在重新验证原位置与隔离位置的受控目录归属及当前占用后清理剩余内容，包括 `.git`。两处冲突时拒绝，不擅自择一。

### 6.3 日志与结果边界

检查异常、普通拒绝和显式 force 写入 data root 的 `logs/operations.jsonl`。预检流程正常返回检查结果后，Git 或进程占用保护策略拒绝清理（包括 force 仍不能绕过的已确认占用）时记录 `refused`；归属、路径等预检本身失败，以及非 Ready 工作区未使用 force 时记录 `failed`。这些结果都不宣称删除已开始；允许执行时记录 `started` 和结果。删除 Workspace 后日志仍保留；发生中断时可能只有开始记录，不能视为成功。日志不可写时清理尚未开始则返回 E_FILESYSTEM 并保留目录。

没有独立审计服务、提交证明数据库或防篡改承诺。日志字段在开发规范中维护，不在本手册重复。

删除成功仅表示范围内目录已清理，不表示代码已交付。平台不自动创建或保留分支，没有 `--delete-branch`。用户或 LLM 必须自行保存需要保留的内容；不能把副本内部 commit 当作副本之外的备份。

### 6.4 外部占用

尽力检查返回 `confirmed-in-use / no-evidence / scan-incomplete`。已确认占用返回 E_WORKSPACE_BUSY，force 也不能绕过；后两者分别表示无证据和不完整，不能证明绝对无人使用。不完整必须警告并记日志，但不单独阻止清理。即使本次因 Git 或非 Ready 状态拒绝清理，若进程扫描不完整，JSON 错误仍含 `context.process_use="scan-incomplete"`，人类错误仍显示 `Warning: process scan incomplete`。

ThinWorkspace 不终止用户进程；调用者负责停止写入。扫描与清理之间仍存在竞态，不承诺安全 Sandbox。

## 七、由用户或 LLM 交付

创建不自动分配 Git 分支；副本最初保留来源的 Git 状态。使用自包含仓库的示例：

```bash
cd "$(thinws workspace path auth-refresh)"
git switch -c task/auth-refresh
git status
# 按任务范围显式选择文件，不把“平台未提示 untracked”理解成任务无需添加新文件
git add src/auth.rs
git commit -m "Implement token refresh"
git push -u origin task/auth-refresh
```

该例要求 Git 元数据独立、文件存在、分支未占用且 remote/权限已配置；不是平台自动执行的步骤。子仓库分别使用自己的 Git 流程；需要 PR 时由用户或 LLM 创建。非 Git 目录不强制初始化仓库。

工作区清理与任务验收是两件事。ThinWorkspace 自身研发的主管 Agent 如何核验 commit，见[任务流程](../../development/process/任务流程.md)；平台不内置此流程。

## 八、空间与清理范围

```bash
thinws workspace status auth-refresh
thinws workspace remove auth-refresh
```

`workspace status` 的 `space` 是当前副本内容的逻辑字节与已分配字节估算，不能用来推断删除后实际释放的空间。`workspace remove` 仅按第六节清理已登记、归属可证的工作区容器及对应清理隔离位置；它不扫描 data root 的其他 staging/trash、日志、源目录或外部缓存。

首版没有 `thinws gc`、自动回收或 Base 缓存；`thinws gc` 返回 E_USAGE。失败或中断可能留下无法由产品证明归属的全局 staging/trash 项，`workspace remove --force` 也不因此取得删除它们的权限。用户须在产品外自行核对并处置这些残留，平台不会猜测删除。APFS CoW 共享块使已分配字节估算不等于可释放的独占空间。

## 九、中断与失败边界

```bash
thinws doctor
```

- 创建中断：普通查询仅报告非 Ready 状态；不会续做或把残留目录当成可用工作区。确认无需保留后，可显式 `workspace remove <name> --force` 清理登记目录，再重新创建。
- 删除中断：不自动续做，也不依赖可能已损坏的 Git 元数据作普通清理。若原位置或实例私有隔离位置仍有受控目录，用户再次显式 `workspace remove <name> --force`，每次重新核对归属、卷及占用；确认两处均不存在后才报告成功。
- 数据卷缺失：E_DATA_ROOT_UNAVAILABLE，不选择替代路径。
- 归属/配置冲突：报告对应身份或布局错误；force 不覆盖身份冲突，也不删除未登记目录。
- 已删除 Workspace 再按原完整 ID remove：依据本实例最小删除记录返回 already-removed，不删除任何同名新对象；按已不存在名称查询仍是 E_WORKSPACE_NOT_FOUND。

## 十、命令与 JSON 契约

### 10.1 命令矩阵

| 命令 | 成功 --json | 结果内容 |
|---|---|---|
| init | 支持 | data root、卷与初始化结果 |
| doctor | 支持 | 只读检查结果 |
| workspace create / create --dry-run | 支持 | source、目标或目标模式、物化证据；dry-run ID 为 null |
| workspace list | 支持 | 所有活跃记录的稳定排序数组 |
| workspace status | 支持 | 当前状态、Git 检查完整性、逐仓库摘要和当前空间估算 |
| workspace remove（含 --force） | 支持 | ID、operation、forced、removed/already-removed 和日志位置 |
| workspace path | 不支持 | stdout 专用原始绝对路径字节加末尾换行；--json 返回 E_USAGE |

不支持的子命令或旧参数统一 E_USAGE，不保留首发前旧 Git/Base CLI 的兼容入口。源码实验命令不是本产品契约。

`workspace create --json` 成功时在 `data` 中返回 `command="workspace create"`、`dry_run=false`、`result=created|already-ready`、`workspace_id`、`name`、`state=ready`、`source/source_hex`、`path/path_hex` 和 `materialization`。后者包含 `requested_mode`、`effective_planned_mode`、`actual_mode`、`adapter`、`outcome=succeeded`、`cow`、`fallback={used,reason}`、`failed_attempt_count`；不执行 Git 初始化或检查。`--dry-run --json` 返回 `dry_run=true`、`workspace_id=null`、`name`、`source/source_hex`、`target_parent/target_parent_hex`、`target_path_mode=id-derived-under-target-parent`、两端 Volume UUID 与 `same_volume`，以及只有请求模式、预选模式、Adapter 和 fallback 的 `materialization`；不出现 `actual_mode`、`cow` 或成功 Receipt。

`materialization` 中的模式稳定值为 `cow-clone`、`full-copy`；`adapter` 稳定值为 `apfs-file-clone`、`full-copy`。成功回执的 `cow` 为 `confirmed` 或 `not-used`。`fallback.used=false` 时 `reason=null`；为 true 时，`reason=clone-unsupported-at-preflight` 表示预检确认 clone 不支持，`reason=clone-unavailable-at-runtime` 表示实际 clone 不可用且已确认回滚后改用 Full Copy。dry-run 只能报告预检降级，不能预告运行时降级。以上取值也适用于 `workspace list/status` 展示的历史成功物化事实。

`workspace list --json` 返回 `data.command="workspace list"` 和按名称排序的 `workspaces` 数组；每项有 `workspace_id`、`name`、`state`、`source/source_hex`、`last_error_code` 和 `materialization`，不含 `git`、当前空间或未经当前核验的可用路径。已有成功最终回执时即使后来进入 Error，`materialization` 仍展示该历史成功事实，否则为 null。`workspace status --json` 返回同一记录字段、`path/path_hex`、`command="workspace status"`，以及 `git={scan_complete,state,issues,repositories}` 和 `space={state,logical_bytes,allocated_bytes_estimate}`；space.state 为 `complete` 或 `unknown`，unknown 时两个数值均为 null。每个 repository 包含 `relative_path/relative_path_hex`、`state`、`tracked_changes` 和 `issues`。非 Ready 的 `path/path_hex` 为 null；根仓库用 `.`；显示路径可能有损，无损字节在对应 hex 字段。空间字段是查询时的估算，不把创建时 Receipt 当成实时用量。

status JSON 示例：

```json
{
  "schema_version": 1,
  "ok": true,
  "data": {
    "command": "workspace status",
    "workspace_id": "ws_019...",
    "name": "auth-refresh",
    "state": "ready",
    "source": "/Volumes/data/code/my-app",
    "path": "/Volumes/data/thinws-data/workspaces/ws_019.../root",
    "space": {"state": "complete", "logical_bytes": 16384, "allocated_bytes_estimate": 8192},
    "git": {
      "scan_complete": true,
      "state": "dirty",
      "issues": [],
      "repositories": [
        {"relative_path": ".", "relative_path_hex": "2e", "state": "dirty", "tracked_changes": 3, "issues": []}
      ]
    },
    "materialization": {
      "requested_mode": "cow-clone",
      "effective_planned_mode": "cow-clone",
      "actual_mode": "cow-clone",
      "adapter": "apfs-file-clone",
      "outcome": "succeeded",
      "cow": "confirmed",
      "fallback": {"used": false, "reason": null},
      "failed_attempt_count": 0
    }
  }
}
```

示例为便于阅读省略了始终返回的 `source_hex`、`path_hex` 与 `last_error_code`；空间数字仅为示例，不表示可回收字节。整体 git.state：发现不完整或任一仓库 unknown 则 unknown；否则存在 dirty 则 dirty；至少一个仓库且均 clean 则 clean；无仓库则 not-applicable。仓库按相对位置排序；unknown 原因放在全局或逐仓库 `issues`，用稳定短名表达。tracked_changes 为去重路径数，unknown 时为 null，不以 0 代替未知。非 Ready 的 status 返回 `path/path_hex=null`、`git={"scan_complete":false,"state":"unknown","issues":["workspace-not-ready"],"repositories":[]}`、`space={"state":"unknown","logical_bytes":null,"allocated_bytes_estimate":null}`，不启动 Git 或空间扫描。

清理拒绝 JSON 示例：

```json
{
  "schema_version": 1,
  "ok": false,
  "error": {
    "code": "E_WORKSPACE_DIRTY",
    "message": "Workspace has tracked changes",
    "context": {
      "workspace_id": "ws_019...",
      "issues": [],
      "repositories": [
        {"relative_path": ".", "relative_path_hex": "2e", "state": "dirty", "tracked_changes": 3, "issues": []}
      ]
    },
    "remediation": "Preserve and commit required work, or use workspace remove <name-or-id> --force to discard the copy."
  }
}
```

JSON 模式的成功或错误 envelope 均写入 stdout，且每次只输出一个 JSON 文档；非 JSON 模式的错误诊断写入 stderr，持久异常日志写入约定文件。参数错误也保持 envelope；全局 --json 预识别在 -- 处停止。--help/--version 与 --json 互斥。脚本依据 ok、error.code 和版本化字段，不解析人类 message。首次发布前重订 schema_version=1 草案，不代表支持已撤销的旧字段。

### 10.2 退出码

| 退出码 | 名称 | 含义 |
|---:|---|---|
| 0 | OK | 操作成功；status 的检查完整性仍须看结果 |
| 2 | E_USAGE | 命令或参数错误 |
| 10 | E_NOT_INITIALIZED | 未初始化 |
| 11 | E_CAPABILITY_UNAVAILABLE | 平台能力不可用 |
| 12 | E_COW_UNAVAILABLE | 本次 clone 不支持且未允许复制 |
| 15 | E_NAME_CONFLICT | 同名活跃 Workspace 参数不同 |
| 16 | E_DATA_ROOT_CHANGE_UNSUPPORTED | 不支持切换已初始化 data root |
| 20 | E_WORKSPACE_NOT_FOUND | Workspace 不存在 |
| 21 | E_WORKSPACE_NOT_READY | path 要求 Ready |
| 22 | E_WORKSPACE_DIRTY | 已跟踪变更阻止普通清理 |
| 23 | E_WORKSPACE_BUSY | 已确认外部进程占用 |
| 25 | E_GIT_CHECK_INCOMPLETE | 无法完成普通清理的 Git 检查；可显式 force |
| 30 | E_GIT | Git 调用错误，不能映射为更具体的检查结果 |
| 31 | E_FILESYSTEM | 文件、源变化、元数据保真或日志 I/O 失败 |
| 32 | E_DATA_ROOT_UNAVAILABLE | 数据根或预期卷不可用 |
| 33 | E_DATA_ROOT_LAYOUT | 源/目标同卷、包含关系或受控路径布局不合法 |
| 35 | E_METADATA | SQLite/schema/元数据失败 |
| 36 | E_DATA_ROOT_NOT_EMPTY | 非空 data root 无有效归属 |
| 40 | E_WORKSPACE_INCOMPLETE | 工作区创建或清理未完成；只允许显式强制清理受控残留 |
| 41 | E_LOCK_TIMEOUT | 生命周期锁等待超过 5 秒，未开始修改目标 |

旧 Repository/Base/分支保护相关编号 13、14、17、18、19、24，以及旧执行包装编号 34 保留不再分配，不重新赋义。工具自身的退出码不受本表管理。

## 十一、首版验收体验

1. 指定源目录即可创建；无 Git、dirty、未跟踪或 ignored 内容不成为创建限制。
2. --dry-run 报告实际目录组合，正式执行才产生 CoW 证据。
3. 普通路径可被已有工具直接使用；不包装执行或配置语言缓存。
4. 状态和普通清理只报告已跟踪变更；对子仓库同样适用。
5. 未跟踪文件不提示、不阻塞；用户明确承担清理时丢弃这些文件的风险。
6. 普通拒绝返回非零且不删除；显式 force 可以清理并保留异常日志。
7. 不要求 commit/push/PR 证明，不把清理当成交付。
8. 外部引用、非原子快照和保真边界不被隐藏；不宣称完全 Git 隔离或安全 Sandbox。
9. 路径/卷/占用保护和未完成状态可解释；无自动或 doctor 修复。
10. 当前文档仅定义目标，全部相应自动和真实平台验收通过后才进入人工阶段放行。

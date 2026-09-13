# ThinWorkspace Phase 1 用户操作手册

**版本：v1.0（首发前修订）｜对应阶段：Phase 1 单机 CLI MVP｜性质：目标态公开契约，尚非已发布产品**

## 一、职责与非职责

本手册维护命令、参数、输出、JSON、错误码和用户使用边界。内部状态协调、物化步骤和系统 API 分别由详细设计、跨平台物化设计、技术栈管理；任务交付规则由任务流程管理，不在此重复实现。

Phase 1 只做一件事：把本机现有目录镜像为低额外磁盘占用的普通工作区，供用户或 Agent 直接使用，结束后按明确规则清理。不接管 Git 分支、提交、PR 或用户命令。

## 二、五分钟体验

以下是假设安装完成后的目标体验，不是当前仓库已经提供这些命令：

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

init 只接管新目录、空目录或属于本次实例中断初始化且可验证的目录。重复指定原路径幂等；指定另一 data root 返回 E_DATA_ROOT_CHANGE_UNSUPPORTED，非归属的非空目录返回 E_DATA_ROOT_NOT_EMPTY。无 reset/migrate。

配置固定在 `~/Library/Application Support/ThinWorkspace/config.toml`，不通过环境变量切换。数据卷身份必须与登记匹配；卷不在线时不在其他位置新建替代目录。

doctor 默认只读，报告主机、数据根与 Git 检查是否可用。只读预检不是 CoW 成功证据。显式 `doctor --repair` 才恢复已登记的中断操作；不自动扩大清理范围或增加 force。

## 四、原始目录镜像

### 4.1 创建与计划

```bash
thinws workspace create --source /Volumes/data/code/my-app --name auth-refresh
```

source 和 data-root 参数使用本机绝对目录路径；来源可以位于 data root 外，但首版要求与其同一 APFS Volume，且两者不相等、不互相包含。不支持 URL、任意目标目录、子挂载或根路径的符号链接重定向。

目标输出示例（时间仅为示意，不是性能承诺）：

```text
Workspace ready

Name:            auth-refresh
Workspace ID:    ws_019...
Source:          /Volumes/data/code/my-app
Path:            /Volumes/data/thinws-data/workspaces/ws_019.../root
Actual mode:     cow-clone
CoW:             confirmed
Git setup:       not performed
```

加 `--dry-run` 只展示当前 source/target 卷关系、目标路径模式、后端计划和降级原因，不分配 WorkspaceId、不预留名称、不保留可执行 plan token。正式创建重新检测。

名称必填，长度 1–63，匹配 `^[a-z0-9](?:[a-z0-9._-]{0,61}[a-z0-9])?$` 且不含连续 `..`；不自动归一化。活跃名称全局唯一，实际目录只由 WorkspaceId 推导。后续命令可以用名称或完整 ID；Workspace/Operation ID 分别为 `ws_`/`op_` 加标准小写 UUIDv7。示例缩写不是真实可执行 ID。

相同名称、规范源路径和创建策略，若已有 Ready 记录则返回原副本，不重新复制，也不比较源是否已变。想复制当前源的新内容必须使用新名称；同名参数不同返回 E_NAME_CONFLICT，原记录未 Ready 返回 E_RECOVERY_REQUIRED。该幂等返回不会因来源随后消失而失效。

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

## 五、直接使用与查询

```bash
thinws workspace path auth-refresh
thinws workspace list
thinws workspace status auth-refresh
```

path 成功时 stdout 只有一行绝对路径，可用于 `cd "$(thinws workspace path auth-refresh)"`。工具直接运行，不需要执行包装；终端、Agent 和工具自己管理环境、超时、输出和 Ctrl-C。

list 展示所有活跃 Workspace，按名称稳定排序，列出名称、ID、状态、源路径和实际物化模式；不为了列表自动扫描所有仓库。status 返回元数据、空间估算及当前副本内主仓库/子仓库的按需 Git 检查结果。

Git 检查只关心当前 HEAD/index 已跟踪内容，包括暂存新增、修改、删除、重命名、模式、冲突及子模块引用变化。未跟踪文件和 ignored 文件不展示、不计数、不阻塞。子仓库只有未跟踪文件时不能使父仓库误报有已跟踪修改。已从版本控制移除的历史路径不按“曾经出现过”追溯。

没有仓库显示 `not-applicable`；检查不完整显示 `unknown` 和原因，不冒称 clean。Git 不可用、外部 Git 元数据或不能安全查询时，Ready 副本仍可取得普通路径。

| 命令 | Creating | Ready | Deleting | Error |
|---|---|---|---|---|
| list/status | 只读诊断 | 只读查询 | 只读诊断 | 只读诊断 |
| path | 拒绝 | 允许 | 拒绝 | 拒绝 |
| remove | 先 repair | 检查后执行 | 原 flags 幂等继续 | 仅在归属和清理意图可证明时允许 |
| doctor | 只读 | 只读 | 只读 | 只读 |
| doctor --repair | 验证后继续或 Error | 只修复已证实不一致 | 续跑原意图 | 安全恢复或报告 |

查询不写平台状态或 Git 元数据。Git unknown 不自行将 Ready 改成 Error。物化与恢复未证明完整时不能仅因目录存在返回 Ready。

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

输出只列仓库相对位置和已跟踪变更摘要；不打印未跟踪清单、源码、完整 argv 或凭据。

### 6.2 显式强制清理

```bash
thinws workspace remove auth-refresh --force
```

force 明确授权丢弃副本内容，绕过 tracked dirty 和 Git 检查不完整；不要求填写理由、提供 commit、联网或取得主管在线批准。无额外交互确认，脚本中的显式 flag 就是清理意图。

force 不绕过实例/目标归属、卷身份、路径安全和已确认进程占用，不跟随符号链接或 Git 指针删除工作区外的内容。

```text
Workspace removed
Workspace ID: ws_019...
Forced: yes
Operation ID: op_019...
Log: /Volumes/data/thinws-data/logs/operations.jsonl
Delivery verification: not performed
```

普通拒绝后可改用 force；删除已开始后，重复命令必须携带原 force 值，否则返回 E_RECOVERY_REQUIRED。repair 不自行增加 force。

### 6.3 日志与结果边界

检查异常、普通拒绝和显式 force 写入 data root 的 `logs/operations.jsonl`。force 记录开始和结果，删除 Workspace 后日志仍保留；发生中断时可能只有开始记录，不能视为成功。日志不可写时清理尚未开始则返回 E_FILESYSTEM 并保留目录。

没有独立审计服务、提交证明数据库或防篡改承诺。日志字段在开发规范中维护，不在本手册重复。

删除成功仅表示范围内目录已清理，不表示代码已交付。平台不自动创建或保留分支，没有 `--delete-branch`。用户或 LLM 必须自行保存需要保留的内容；不能把副本内部 commit 当作副本之外的备份。

### 6.4 外部占用

尽力检查返回 `confirmed-in-use / no-evidence / scan-incomplete`。已确认占用返回 E_WORKSPACE_BUSY，force 也不能绕过；后两者分别表示无证据和不完整，不能证明绝对无人使用。不完整必须警告并记日志，但不单独阻止清理。

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

## 八、空间与 GC

```bash
thinws gc --dry-run
thinws gc
thinws gc --yes
```

dry-run 只显示可回收 staging/trash 残留及大小估算，不删除、不预留计划。实际 GC 在交互终端要求确认；非交互必须 --yes。GC 不删除活跃 Workspace（包括 Error）、未完成操作、日志、源目录或外部缓存，也不执行 Git object GC/prune。

没有 Base 缓存可回收。普通副本文件逻辑大小不等于独占磁盘大小，实际回收共享块数量只提供可获得的估算，不保证零额外空间。

## 九、故障恢复

```bash
thinws doctor
thinws doctor --repair
```

- 创建中断：普通查询仅报告；repair 验证已登记 Receipt，无法证明完整就保留 Error，不重新镜像变化后的源覆盖旧操作。
- 删除中断：按持久化原意图和剩余范围继续，不要求已删除的 Git 数据仍可读取；发现范围变化或新占用则停止。
- 数据卷缺失：E_DATA_ROOT_UNAVAILABLE，不选择替代路径。
- 归属/配置冲突：E_RECOVERY_REQUIRED；force 不覆盖身份冲突。
- 已删除 Workspace 再按原完整 ID remove：依据本实例最小删除记录返回 already-removed，不删除任何同名新对象；按已不存在名称查询仍是 E_WORKSPACE_NOT_FOUND。

## 十、命令与 JSON 契约

### 10.1 命令矩阵

| 命令 | 成功 --json | 结果内容 |
|---|---|---|
| init | 支持 | data root、卷与初始化结果 |
| doctor / doctor --repair | 支持 | 检查或逐项恢复结果 |
| workspace create / create --dry-run | 支持 | source、目标或目标模式、物化证据；dry-run ID 为 null |
| workspace list | 支持 | 所有活跃记录的稳定排序数组 |
| workspace status | 支持 | 当前状态、Git 检查完整性、逐仓库摘要和空间估算 |
| workspace remove（含 --force） | 支持 | ID、operation、forced、removed/already-removed 和日志位置 |
| gc / gc --dry-run | 支持 | 计划或实际回收结果；执行时还需 --yes |
| workspace path | 不支持 | stdout 专用一行绝对路径；--json 返回 E_USAGE |

不支持的子命令或旧参数统一 E_USAGE，不保留首发前旧 Git/Base CLI 的兼容入口。源码实验命令不是本产品契约。

status JSON 示例：

```json
{
  "schema_version": 1,
  "ok": true,
  "data": {
    "workspace_id": "ws_019...",
    "name": "auth-refresh",
    "state": "ready",
    "source": "/Volumes/data/code/my-app",
    "path": "/Volumes/data/thinws-data/workspaces/ws_019.../root",
    "git": {
      "scan_complete": true,
      "state": "dirty",
      "repositories": [
        {"relative_path": ".", "state": "dirty", "tracked_changes": 3}
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
      "failed_attempts": []
    }
  }
}
```

整体 git.state：发现不完整或任一仓库 unknown 则 unknown；否则存在 dirty 则 dirty；至少一个仓库且均 clean 则 clean；无仓库则 not-applicable。仓库按相对位置排序；unknown 附稳定原因。tracked_changes 为去重路径数，unknown 时为 null，不以 0 代替未知。

清理拒绝 JSON 示例：

```json
{
  "schema_version": 1,
  "ok": false,
  "error": {
    "code": "E_WORKSPACE_DIRTY",
    "message": "tracked changes prevent normal removal",
    "context": {
      "workspace_id": "ws_019...",
      "repositories": [
        {"relative_path": ".", "state": "dirty", "tracked_changes": 3}
      ]
    },
    "remediation": "preserve the required work, or explicitly rerun remove with --force"
  }
}
```

JSON 模式 stdout 只输出一个文档，诊断写 stderr，持久异常日志写入约定文件。参数错误也保持 envelope；全局 --json 预识别在 -- 处停止。--help/--version 与 --json 互斥。脚本依据 ok、error.code 和版本化字段，不解析人类 message。首次发布前重订 schema_version=1 草案，不代表支持已撤销的旧字段。

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
| 40 | E_RECOVERY_REQUIRED | 需要恢复或人工处理 |
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
9. 路径/卷/占用保护、错误状态和显式恢复可解释。
10. 当前文档仅定义目标，全部相应自动和真实平台验收通过后才进入人工阶段放行。

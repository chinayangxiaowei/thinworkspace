---
name: thinws-workspace
description: Use the thinws CLI from any project to create and work in a space-efficient, ordinary macOS/APFS workspace, then preserve results and clean it up safely. Trigger when asked to use ThinWorkspace or manage a thinws workspace; not for generic coding or testing thinws itself.
---

# 使用 thinws 工作区

先运行 `command -v thinws`、`thinws --version` 和 `thinws --help`，确认命令已安装且与下列格式一致。若不一致，暂停不确定的写入或清理操作，向用户说明实际版本和差异。

## 什么时候使用

- 用户明确要求用 ThinWorkspace，或任务需要从**本机现有目录**建立独立、可直接编辑的低额外空间副本，例如多个 Agent 并行处理同一源码。
- 当前首版要求 macOS、APFS，源目录与已配置 data root 在**同一个 APFS Volume**。它复制当前磁盘内容，不要求源目录是 Git 仓库。
- 不把它用于跨卷迁移、活跃数据库的原子快照、进程 Sandbox、自动 Git 分支/提交/PR、命令执行包装或构建缓存重定向。若任务只需读取现有目录，也不必创建工作区。
- 不因 Skill 被加载就自行创建或删除工作区。先确认任务所指的源目录；若有多个候选，询问用户。已存在的同名工作区先查明归属，不覆盖。

## 命令格式

`<SOURCE>` 和 `<DATA_ROOT>` 均替换为实际绝对目录路径；`<NAME>` 是本轮唯一的工作区名称（1–63 个字符，小写字母/数字及 `._-`，首尾用小写字母或数字，不含连续 `..`）。尖括号只是占位符，不能原样交给 shell。

| 目的 | 命令 |
|---|---|
| 检查实例 | `thinws doctor` |
| 首次初始化（仅在需要且用户选定位置后） | `thinws init --data-root <DATA_ROOT>` |
| 查看已有工作区 | `thinws workspace list` |
| 只预览创建计划 | `thinws workspace create --source <SOURCE> --name <NAME> --dry-run` |
| 创建普通工作区 | `thinws workspace create --source <SOURCE> --name <NAME>` |
| 显式允许同卷 clone 不支持时完整复制 | `thinws workspace create --source <SOURCE> --name <NAME> --allow-copy` |
| 获取可直接使用的路径 | `thinws workspace path <NAME>` |
| 查看当前状态、Git 摘要和空间估算 | `thinws workspace status <NAME>` |
| 正常清理 | `thinws workspace remove <NAME>` |
| 明确丢弃整个副本 | `thinws workspace remove <NAME> --force` |

除 `workspace path` 外，上述命令可在 `thinws` 后加 `--json` 取得单个版本化 JSON 文档；脚本以 `ok`、`error.code` 和结构化字段判断，不解析人类消息。`path` 的 stdout 专供原始绝对路径字节和末尾换行，不能加 `--json`。如清理目标的名称与某个 Workspace ID 有歧义，改用 `name:<NAME>` 或 `id:<完整ID>` 明确选择。

## 如何使用

1. 检查 `thinws doctor`。如果返回 `E_NOT_INITIALIZED`，请用户选择 APFS 上的 data root，再运行一次 `init`；配置绑定当前账户和卷，不通过修改 `HOME` 切换，不擅自换到另一位置。卷缺失、身份或布局错误时停止，不新建替代目录。
2. 创建前尽量停止源目录写入。运行 `create --dry-run`，核对 source、同卷关系、计划模式和任何降级原因。预览不分配 Workspace ID，也不证明 CoW 已发生。源与 data root 不得相等或相互包含；跨卷不能靠 `--allow-copy` 绕过。
3. 确认计划符合用户对空间的要求后执行 `create`。默认要求 CoW；只有用户接受完整复制的空间代价时才加 `--allow-copy`。确认结果是 Ready，记录 Workspace ID、路径、`actual_mode` 和 `cow`；失败或非 Ready 时不得仅因目录存在就开始工作。相同名称、来源和策略的 Ready 工作区可能直接返回原副本，而不会重新复制已变化的来源；需要新快照时改用新名称。
4. 用 `thinws workspace path <NAME>` 取得路径，在该**普通目录**直接运行编辑器、编译器和项目命令。常见路径可用 `cd "$(thinws workspace path <NAME>)"`；路径本身可能含换行时，改读 `thinws --json workspace status <NAME>` 的 `path_hex`，不要按行解析。不存在 `thinws workspace exec`。
5. 工作中可用 `thinws workspace status <NAME>` 查询。镜像包含 `.git`、未跟踪/ignored 文件和构建产物，但不会创建分支或自动交付；绝对符号链接、Git worktree 的外部 `.git` 指针仍可能触达原位置，工作区不是 Sandbox。status 的 Git 摘要只检查已跟踪内容；`unknown` 不是 clean，空间数字也不是删除后可释放量。
6. 结束前按任务要求保存成果。检查需要保留的已跟踪、未跟踪和 ignored 文件；如果使用 Git，按任务范围提交，并确认 commit/产物已经以用户授权的方式保存在**副本之外**。只存在于副本 `.git` 中的 commit、branch 或 stash 会随副本删除；不要因本 Skill 自动 push 或创建 PR。
7. 只有任务要求清理且成果核对完毕，才停止占用进程、离开工作区并运行普通 `remove`。它会删除整个副本，包括未跟踪和 ignored 文件，而这两类内容不会触发平台的 dirty 提示。`E_WORKSPACE_DIRTY`、`E_GIT_CHECK_INCOMPLETE` 时保留现场处理；`E_WORKSPACE_BUSY` 时停止占用进程。`--force` 仅在用户明确授权丢弃该**确切**工作区后使用；它不能绕过归属、路径、卷身份或已确认占用保护。非 Ready/中断状态不会自动续做，不能直接手工删除产品内部目录。

报告给用户：源目录、工作区名称/ID/路径、实际物化模式、成果保存位置、工作区保留或清理结果，以及任何错误码或未完成状态。`thinws gc`、自动恢复和执行包装不属于这些命令；不要凭目录存在、Git clean 或清理成功推断成果已经交付。

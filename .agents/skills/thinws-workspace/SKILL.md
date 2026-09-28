---
name: thinws-workspace
description: Use the ThinWorkspace thinws CLI to create, enter, inspect, and safely remove an ordinary project workspace on macOS/APFS. Use when asked to work through ThinWorkspace or manage a thinws workspace; not for developing or acceptance-testing ThinWorkspace itself.
---

# 使用 ThinWorkspace 工作区

本 Skill 指导 Agent **使用** `thinws` 完成用户的开发任务，不是 ThinWorkspace 项目的开发或验收流程。命令、错误码和输出的唯一权威是[Phase 1 用户操作手册](../../../docs/project/reference/ThinWorkspace_Phase1用户操作手册_v1.0.md)；运行前按当前任务阅读其对应章节。手册仍标明 Phase 1 尚未正式发布，先用 `thinws --help`、`thinws --version` 确认本机实际安装的命令，不假设目标态文档等于当前二进制能力。

## 建立工作区

1. 确认用户指定或当前任务明确指向的**源目录**，用绝对路径；不要猜测另一个仓库。确认本轮唯一的工作区名称（小写字母、数字及允许的 `._-`，不得与现有活跃名称冲突）。`thinws workspace list` 可先查看已有工作区。
2. 运行 `thinws doctor`。若未初始化，向用户确认放置数据的 APFS 目录后才运行 `thinws init --data-root <绝对目录>`；配置固定于当前 macOS 账户，不要随意换根、覆盖配置或重设 `HOME`。若数据卷缺失、身份或布局不符，停止并报告，不创建替代根。
3. 创建前让源目录停止写入；磁盘镜像不是活跃进程的原子快照。先运行：

   ```bash
   thinws workspace create --source <源目录绝对路径> --name <名称> --dry-run
   ```

   核对同一 APFS Volume、来源和 data root 不相等/不包含、计划模式及降级原因。`--dry-run` 只是预检，不分配 ID，也不证明 CoW 成功。
4. 预览符合用户的空间与复制要求后，再运行相同命令但去掉 `--dry-run`。默认要求 CoW；只有用户接受同卷 clone 不支持时的完整复制，才加 `--allow-copy`。该 flag 不是跨卷开关。创建后检查结果为 Ready，记录名称、Workspace ID、实际路径、实际模式和 CoW 结果；失败或非 Ready 时不要把已有目录当可用工作区。

## 在普通目录里工作

```bash
thinws workspace path <名称>
thinws workspace status <名称>
```

`path` 的 stdout 是普通绝对目录路径；在该目录直接编辑和运行项目自己的命令即可，不存在 `thinws workspace exec` 包装，也没有语言专属构建缓存重定向。常见路径可 `cd "$(thinws workspace path <名称>)"`；若路径含换行，改用 `thinws --json workspace status <名称>` 的 `path_hex` 无损处理，不按行解析路径。

创建会复制磁盘上的 `.git`、未跟踪、ignored 文件和构建产物，但不创建分支、不提交、不推送。绝对符号链接或 Git worktree 的外部 `.git` 指针可能仍指向来源之外；执行可能修改 Git 元数据或外部目标的命令前，先检查这些引用是否自包含。工作区也不是 Sandbox。`status` 的 Git 检查是按需、只读的 tracked 内容观察；`unknown` 不是 clean，空间值只是估算，不是删除后可释放空间。

## 保存成果并结束

1. 按用户的交付要求，在工作区内检查已跟踪、未跟踪和必要的 ignored 产物；`thinws` 的清理提示**只**覆盖已跟踪变更。若使用 Git，按任务范围显式 `git add`、commit，并分别处理需要交付的子仓库。仅存在于副本 `.git` 的 commit、branch 或 stash 会随副本删除；清理前确认成果已按用户授权的方式保存在副本之外。不要因为本 Skill 自动 push 或创建 PR。
2. 停止使用该路径的工具并离开目录。需要清理且已核对成果后，运行 `thinws workspace status <名称>`，再运行 `thinws workspace remove <名称>`。普通清理也会删除未跟踪/ignored 文件；不能把“未被拦截”当成“没有数据要保留”。清理后核对返回结果；日志保留在 data root。
3. `E_WORKSPACE_DIRTY` 或 `E_GIT_CHECK_INCOMPLETE` 时保留副本，先处理交付或检查问题。`--force` 会丢弃整个副本，仅在用户明确要求丢弃该工作区且目标 ID/名称已核实时使用；它也不能绕过归属、卷身份、路径或已确认进程占用保护。非 Ready/中断状态不会自动恢复；需要强制清理时同样先确认目标和丢弃意图，不手工删除产品内部目录。

## 给用户的结果

报告工作区名称、ID、可用路径、实际物化模式、任务成果所在位置，以及工作区是保留还是已清理。若任何步骤失败，给出原始稳定错误码和未完成状态；不要把 `dry-run`、目录存在、Git clean 或清理成功当成 CoW 成功、成果已交付或任务已验收的证明。

# ADR-0007：Phase 1 Linux/Btrfs CLI 适配

**状态：Accepted｜日期：2026-09-29｜适用范围：Phase 1 的 Linux/Btrfs 扩展**

## 职责与非职责

本文决定现有单机 CLI 是否接入 Linux/Btrfs，以及接入时不能削弱的产品边界。具体 Probe、物化和失败回执由《跨平台工作区物化设计》维护；生命周期与归属由《Phase 1 单机 CLI 详细设计》维护；公开命令及支持资格由用户手册维护。

本文不宣称当前 Linux CLI 已可运行，不定义新的命令、JSON 字段、SQLite DDL 或 Linux 系统调用封装清单。实验通过不等于产品放行。

## 背景

当前 `thinws-cli` 无条件依赖 macOS Adapter，在 Debian 上无法编译；Core、Application 和持久 Receipt 中还存在 APFS 专用候选名称与判断。真实 Debian 5.10/Btrfs 实验已证明同挂载普通文件 `FICLONE` 成功、写后隔离、目录与 `prl_fs` 输入拒绝，但尚未验证完整 Workspace 生命周期。这些证据不足以把 Linux 加入用户支持矩阵。

## 决策

1. 在 Phase 1 的同步、单机、普通目录模型内增加 Linux/Btrfs Adapter。Linux 首个资格组合仅为 Debian VM 中可重复验证的 Btrfs；不同时承诺 XFS、OverlayFS、ext4 reflink 或跨文件系统 Full Copy。
2. Linux 也使用固定的 `~/.thinws` 控制目录，可位于经身份校验的 ext4 等非 Btrfs 本机文件系统；控制目录不参与 source/target reflink 同挂载判断。source、target parent、staging、trash 必须位于本次经证据确认可执行的同一 Btrfs 挂载布局。`prl_fs` 共享源码到 Btrfs target 的组合必须拒绝，而不是暗中完整复制。
3. 保留现有七个 Port、状态机、强制清理授权和公开命令形状。Core/Application 不按操作系统分支；平台身份、文件系统特性、no-follow 访问、`FICLONE`、无覆盖发布和进程占用探测由 Linux Adapter 提供。不得复用 APFS UUID 或仅凭两个路径都是 Btrfs 推断同卷。
4. Linux 的持久归属必须有可重验的文件系统与目录身份；身份不足、挂载变化、birthtime 缺失或控制目录布局无法证明时安全失败。任何持久格式更改须明确版本化，不静默读取或迁移不兼容数据。Linux 删除必须与 macOS 一样先确认登记 target 存在、身份匹配和占用边界；`--force` 不越过这些检查。
5. 平台资格顺序为：可移植 Core/持久模型 → Linux Host/Path Probe 与控制目录/锁 → Linux Btrfs 物化 → 删除、进程与空间边界 → CLI 装配及黑盒生命周期。只有真实 Btrfs、故障注入、CLI 契约和适用质量门禁通过后，用户手册才把 Linux 组合从“未实现”改为“候选已验收”；人工阶段放行仍另行决定。

## 备选方案与代价

不采用在 Linux 上调用 `cp --reflink=auto` 或把 `FICLONE` 实验包装成 CLI：它们无法给出逐项 CoW、路径身份、失败回执和删除归属保证。不把 macOS Adapter 整体复制成一份长期平行实现；共性只在有真实调用者时抽出，平台系统调用仍各自封装。

代价是需要调整现有 APFS 专用领域命名、持久编解码与 CLI 装配，并重新验证已运行的 macOS 契约。任何为省工跳过安全检查的 Linux 版本都不能列为完成。

## 迁移与验证

现有 macOS 数据不自动迁移或重写。实现须保证旧 macOS 记录仍按已冻结格式读取；若无法做到，必须在发布前另立明确的版本化/迁移决策，而不是让新版静默拒绝或误删。Linux 首次初始化仅接管可证明为空或同一实例的新布局，不接管历史 macOS 控制目录。

真实 Linux 测试根由 `THINWS_LINUX_BTRFS_TEST_ROOT` 显式指定；跨文件系统只读对照文件另由测试环境传入，不写共享源码盘。测试覆盖同挂载成功、不同文件系统/挂载与 NOCOW 拒绝或真实失败、来源变化、部分物化、回滚不完整、缺失/替换 target、强制日志、已跟踪 Git 检查、占用探测和重复清理。测试结果与未执行项按《任务流程》记录，不把实验测试计入 CLI 验收。

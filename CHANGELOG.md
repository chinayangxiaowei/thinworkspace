# ThinWorkspace Changelog

## 职责

本文件按版本记录用户、集成方和维护者需要知道的显著能力、兼容性、安全及授权变化。

## 非职责

- 不复制 Git 提交历史、任务实施日志或审核记录；
- 不把尚未验收的计划写成已交付能力；
- 不代替 Release PR、阶段放行记录或迁移说明。

格式遵循 [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/)。项目首次正式发布前只维护 `Unreleased`；版本号、发布日期和比较链接必须在真实发布时补充，不能预先虚构。

## [Unreleased]

### Added

- 建立 Phase 0/Phase 1 项目文档基线和仓库目录骨架。
- 采用 PolyForm Noncommercial License 1.0.0 源码可用许可，并建立独立商业授权边界。
- 增加商业授权申请说明和公开联系渠道。
- 增加只读 Host/Path Probe 技术实验及固定 Rust/质量工具链，验证 APFS 卷身份、路径重验和候选后端预检；仅为 Phase 0 实验，不是已发布的 `thinws` 产品能力。
- 增加 APFS 物化技术实验、真实双卷与失败回滚测试，以及布局策略 fuzz：验证同卷 clone、跨卷拒绝、独立 Full Copy 和显式降级的证据边界；仍为 Phase 0 实验，不提供 Phase 1 CLI。
- 增加首批 Phase 1 单机 CLI：`thinws init` 在 APFS 上建立或幂等验证私有 data root，`thinws doctor` 以只读产品状态检查实例、受控布局、SQLite 元数据和未完成工作区数量，并提供稳定的人类/JSON 输出及退出码。
- 增加 Phase 1 APFS 工作区物化后端：通过路径能力探测、执行前重验、逐文件 APFS clone、清单/保真校验和身份约束回滚生成可验证 Receipt；现已由公开 `workspace create` 使用，尚未发布。
- 增加 Phase 1 Full Copy 后端与受限降级计划：独立报告字节复制候选能力，仅在同一已知 APFS 卷、用户允许且 clone 明确不可用并完成必要回滚时选择 Full Copy；回执保留降级原因、失败尝试和真实复制证据，现已接入公开创建命令。
- 增加开发中的 `workspace create/list/path/status/remove`：直接交付普通路径，按需报告已跟踪 Git 变化和当前空间估算，显式清理受控副本并在 data root 保留日志；仍须通过 Phase 1 阶段验收和人工放行。

### Changed

- 重新制定尚未正式发布的 Phase 1 布局目标：控制数据集中于 `~/.thinws`，工作区由创建命令显式指定最终 target；仅 source/target 需要同 APFS 卷。删除必须证明登记 target 存在且身份匹配，`--force` 不越过此边界。旧候选不迁移、不兼容、不自动清理；对应代码与新布局验收仍在进行中，不把本条作为已交付功能。
- 收缩首次发布前的 Phase 1 中断边界：取消 `doctor --repair`、跨进程操作重放和自动续做；未完成工作区不得冒充 Ready，只能在重新验证归属后由用户显式强制清理。
- 修订首次发布前的 Phase 1 首版设计：以本机原始目录镜像替代 Git 托管仓库/Base 创建；清理只提示已跟踪变更，允许显式强制并保留日志。提交、推送、PR 和主管验收由外部研发流程负责，不作为底层交付硬门禁。
- 收缩首次发布前的 Phase 1 范围：保留当前空间统计和显式 Workspace 清理，不发布原计划中的 `thinws gc` 或自动回收；这不是移除已发布命令。

- 修订首次发布前的 Phase 1 设计基线：移除 `workspace exec`、用户执行登记和平台构建目录重定向，以普通路径直接使用现有工具为主流程；不是已发布功能的移除，也不新增缓存导入能力。
- 项目名称统一为 `ThinWorkspace`，GitHub 仓库名为 `thinworkspace`，CLI 与技术命名空间由原暂定名 `agentws` 改为 `thinws`；首次发布前不提供旧名称兼容。

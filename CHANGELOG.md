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

### Changed

- 修订尚未实现的 Phase 1 首版设计：以本机原始目录镜像替代 Git 托管仓库/Base 创建；清理只提示已跟踪变更，允许显式强制并保留日志。提交、推送、PR 和主管验收由外部研发流程负责，不作为底层交付硬门禁；这不是已实现或已发布的能力。

- 修订尚未实现的 Phase 1 设计基线：移除 `workspace exec`、用户执行登记和平台构建目录重定向，以普通路径直接使用现有工具为主流程；不是已发布功能的移除，也不新增缓存导入能力。
- 项目名称统一为 `ThinWorkspace`，GitHub 仓库名为 `thinworkspace`，CLI 与技术命名空间由原暂定名 `agentws` 改为 `thinws`；首次发布前不提供旧名称兼容。

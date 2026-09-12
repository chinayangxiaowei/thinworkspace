# 文档索引

## 职责

本文件只维护文档分类、当前位置和导航入口，不复制任何设计、规则、计划或审核结论。

## 非职责

- 不定义产品、架构、CLI、工程规范或任务状态；
- 不充当其他文档的摘要版本；
- 不保存仅属于某次任务的实施或审核正文。

## 根目录法律与发布资料

- [正式许可证](../LICENSE.md)：PolyForm Noncommercial License 1.0.0 原文；
- [源码许可与商业授权声明](../LICENSING.md)：中文授权边界、对外称谓和待补法律信息；
- [商业授权申请说明](../COMMERCIAL-LICENSING.md)：商业使用场景、申请材料、办理流程和联系方式；
- [Changelog](../CHANGELOG.md)：按版本维护的显著变更，不代替任务记录或提交历史。

## 项目文档

项目文档描述当前有效、可跨任务复用的项目事实。

### 架构

- [产品与架构演进方案 v2.1](project/architecture/ThinWorkspace_产品与架构演进方案_v2.1.md)
- `project/architecture/adr/`：已接受的架构决策记录；草案和审核过程不放在这里。

### 设计

- [Phase 1 单机 CLI 详细设计](project/design/ThinWorkspace_Phase1单机CLI详细设计_v1.0.md)
- [跨平台工作区物化设计](project/design/ThinWorkspace_跨平台工作区物化设计_v1.0.md)

### 参考与公开契约

- [技术栈](project/reference/技术栈.md)
- [Phase 1 用户操作手册](project/reference/ThinWorkspace_Phase1用户操作手册_v1.0.md)

## 开发过程

开发过程文档描述研发如何推进，或保存有明确阶段/任务生命周期的证据。

### 流程

- [开发规范](development/process/开发规范.md)
- [任务流程](development/process/任务流程.md)

### 规划

- [Phase 1 实施计划](development/planning/ThinWorkspace_Phase1实施计划_v1.0.md)

### 实施与审核

- [P0-01 Host/Path Probe 实施记录](development/implementation/P0-01_HostPathProbe实验.md)：任务验收断言、实验结果、审核结论与未完成项；不定义产品接口。
- [P0-02 APFS clone 与跨卷实施记录](development/implementation/P0-02_APFS物化实验.md)：物化实验准入、失败补偿验收、执行证据与剩余边界。
- [P0-03 Git 与 Base 实施记录](development/implementation/P0-03_Git与Base实验.md)：Git/Base 实验准入、候选 ADR 证据与待确认的兼容边界。
- `development/implementation/`：P0 实验报告、任务实施记录、故障注入结果和小阶段收口证据；
- `development/review/`：审核记录、待处理问题和人工阶段放行材料。

实施或审核材料不能成为产品事实的唯一来源。其结论改变设计或规则时，必须回写相应权威文档。

# ThinWorkspace

面向多 Agent 并行开发的低额外磁盘占用隔离工作区。

## 职责

本文件作为仓库入口，维护项目名称，并指向 Agent 规则、文档索引和当前实施计划。

## 非职责

本文件不定义产品能力、架构、技术选型、研发流程或任务状态；这些事实由链接的权威文档维护。

## 项目标识

| 用途 | 名称 |
|---|---|
| 产品与显示名称 | `ThinWorkspace` |
| GitHub 仓库名 | `thinworkspace` |
| CLI 与技术命名空间 | `thinws` |

crate、环境变量、Git ref 和内部文件等技术标识由 `thinws` 派生；用户可见正文使用 `ThinWorkspace`。项目首次发布前不提供旧名称兼容。

## 当前范围

本仓库当前覆盖 Phase 0 关键技术验证与 Phase 1 单机 CLI MVP。

- AI Agent 的强制入口与按需加载规则：[AGENTS.md](AGENTS.md)
- 项目文档与开发过程文档索引：[docs/README.md](docs/README.md)
- P0/P1 当前实施顺序：[Phase 1 实施计划](docs/development/planning/ThinWorkspace_Phase1实施计划_v1.0.md)
- 版本显著变更：[CHANGELOG.md](CHANGELOG.md)
- 源码许可与商业授权：[LICENSING.md](LICENSING.md)、[商业授权申请说明](COMMERCIAL-LICENSING.md)及[正式许可证](LICENSE.md)

`crates/`、`tests/`、`fuzz/`、`experiments/` 和 `tools/` 中的空目录仅初始化仓库布局，不表示对应任务或能力已经完成。实现状态只以实施计划、测试证据和阶段放行结果为准。

本项目采用 `PolyForm-Noncommercial-1.0.0` 源码可用许可：非商业目的可以按许可证使用、修改和分发；商业使用必须另行取得书面授权。本项目不得宣传为 OSI 开源软件。

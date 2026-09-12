# P0-03 Git 托管拓扑与 Base 实施记录

## 职责与非职责

本文管理 P0-03 的准入、实验验收、候选 ADR 证据和未解决边界。不重新定义产品 Git 拓扑、BaseKey 字段、公开兼容范围或任务门禁；实验结论改变长期设计时，必须修改对应权威来源，不能只留在本文。

## 任务准入

| 项目 | 当前值 |
|---|---|
| 类型、阶段、小阶段、风险 | Spike/ADR；P0；P0.b；R4（Git refs、发布与精确回收） |
| 状态 | Backlog；仅准入准备，未开始 Git/Base 实验编码 |
| 负责人 | 主 Agent |
| 模型分工 | 编码拟使用 GPT-5.6 Sol / `gpt-5.6-sol` / `xhigh`；审核必须使用 GPT-6 Astra / `gpt-6-astra` / `xhigh` |
| 准备日期、环境 | 2026-09-12；实测 macOS 15.7.2（24G325）、arm64、Apple Git 2.39.5（Apple Git-154） |
| 源码基线 | 已合并并通过 CI 的 `main`：`2e29f88a1aae2ee53ce52dbc38a7c0765b58cb15` |
| 当前文档基线 | `222c76257841d5c9d90f9a0be19815b7b8fdba6d`；仅接入已审 P0-01 收口和 P0-02 准备记录，不包含未完成的物化代码 |
| 工作区、分支 | `/Volumes/data/code/thinworkspace-p0-03`；`task/p0-03-git-base`；标准 Git linked worktree，不声明 CoW |
| 前置依赖 | P0-01 已合并；公开属性兼容边界仍待确认，见下节 |
| 主要写入区域 | 计划为 `experiments/p0/git-base/` 及直接配套实验测试；当前只有本任务卡和导航引用 |
| 共享文件协调 | Cargo/lockfile、CI、质量配置及文档索引由主 Agent 集成；不与 P0-02 同时修改同一工作区的共享文件 |

权威输入为[Phase 1 详细设计](../../project/design/ThinWorkspace_Phase1单机CLI详细设计_v1.0.md)的 Repository/Git 与 Base 章节、[技术栈](../../project/reference/技术栈.md)的 Git/编码/测试章节、[用户手册](../../project/reference/ThinWorkspace_Phase1用户操作手册_v1.0.md)的仓库兼容和 checkout 可见契约，以及[开发规范](../process/开发规范.md)、[任务流程](../process/任务流程.md)。任务依赖与小阶段归[实施计划](../planning/ThinWorkspace_Phase1实施计划_v1.0.md)管理。

## 尚待确认的准入边界

只读准备已发现：目标 Git 的 `check-attr` 文本输出不能完整区分属性的布尔状态与同名字符串，例如 `-filter` 与 `filter=unset` 都可能输出 `unset`；NUL 分隔解决字段边界，不解决类型歧义。此前已向维护者提问，尚未收到答复：是否接受连显式关闭声明也拒绝的保守兼容限制，或继续验证保留关闭声明兼容性的精确方案。

这不是已接受的产品决策。确认前不得把“属性名出现即拒绝”写成用户契约或实现默认行为，也不得把未知结果当作安全放行；标准 `text/eol` 的现有支持承诺不变。最终方案必须在目标 Apple Git 上取得真实样例证据，再由规定模型审核。此边界不阻塞 P0-02。

## 实验范围与验收断言

目标是证明既定托管拓扑、独立 index 和可验证 Base 能协同工作，并形成有证据支持的实现 ADR。既有设计已经冻结的拓扑与身份字段不重新开题。

| 观察域 | 必须取得的实验结果 |
|---|---|
| 独立托管导入 | 来源与托管仓库的对象和 refs 边界符合权威契约；不依赖 alternates，不引入来源 dirty/untracked/私有 refs；不修改来源的 worktree 登记或配置 |
| fetch 与固定解析 | 来源 heads/tags 更新遵守既定原子与非 prune 语义；同名 tag 改写整次失败；ref 歧义、非 commit 与不支持的 object format 被拒绝，已解析 commit/tree 不随随后 fetch 漂移 |
| 独立 index 与 Base 构建 | Base 临时 index 与各 Workspace 原生 index 互不覆盖；从固定 tree 检出；全新受控工作目录、index 与属性查询使用一致的来源，不受用户现有 checkout 或 global/system 配置污染 |
| checkout 内容 | LF/CRLF、二进制、文件可执行模式、symlink 和嵌套目录具有独立预期值；Base manifest 核对实际 checkout 字节，不把标准行尾转换误判为损坏；不运行未批准 filter、hook、diff/textconv 或 fsmonitor |
| Base 身份与发布 | 规范编码符合详细设计与技术栈，有确定的 golden vectors；内容/策略/文件系统语义变化按契约改变身份；发布前完整校验，失败不留下可复用的假 Ready Base，复用时检测损坏 |
| 并行 Workspace | 两个不同托管分支、独立 index，初始均 Git clean；修改和提交其中一个不污染另一个或 Base；来源仓库保持不变 |
| 平台路径与不兼容输入 | 对实际 APFS 的大小写/Unicode/符号链接能力取得证据；保留项冲突、submodule、sparse 与不兼容转换明确拒绝，不产生部分可用 Workspace |
| 中途失败与精确回收 | 在分支登记、worktree 登记、Base 发布等边界制造失败，准确保留已发生事实；不通过全局 prune、GC、强制删除或递归未知目录掩盖残留 |

实验只使用本次创建的合成仓库和私有临时根，不接入用户真实仓库，不联网 fetch/push，不修改用户全局 Git 配置。命令使用 argv、固定 cwd、受控环境、有界超时，分别保存退出码和安全输出。清理只处理已核验归属与身份的实验对象；未知内容保留并报告。完整跨系统 operation/强杀恢复属于 P0-04；本任务不引入 SQLite、P1 CLI、产品 Port 或平行生命周期框架。

## 验证与交付

准入解除后先按任务流程取得可解释 RED，再实现并执行通用门禁、真实 Git debug/release 验收、受影响 crate 全量变异和纯解析/编码目标的短预算 fuzz；记录未执行项，短预算不代替阶段长预算。

产物为可重复实验、执行证据及 Git/Base 实现 ADR。候选决策与证据先保存在本记录；验证并接受后，正式 ADR 放入 `docs/project/architecture/adr/` 并更新索引。不得把草案当作已冻结设计，也不得复制完整产品契约到本记录。

## 当前进展

- 主 Agent 已读相关权威章节并建立独立工作区；此前 Sol 与 Astra 的只读准备只形成待验证命令方案和属性兼容问题，没有运行真实 Git/Base 实验。
- 主 Agent 在本工作区执行 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings` 和 `cargo test --workspace --all-targets` 均退出 0；既有 Probe 测试 49 passed、1 个需专用双卷环境的用例 ignored。`git diff --check` 通过，任务卡、索引与计划中的 24 个本地链接目标存在。这些检查只验证准备文档与 P0-01 源码基线，不证明任何 P0-03 Git/Base 行为，也不替代该任务之后的专项门禁。
- 尚无本任务代码、合成 Git 仓库、Base、实验分支或物化测试结果；工作区中的项目任务分支仅用于开发，不是被测产品的托管分支。
- GPT-6 Astra / `xhigh` 完成任务卡与两处导航的只读复核，无阻断发现，认可这三个准备文档可提交；此结论不表示 Ready、实现批准或 Go。
- 本任务不作 Go 或阶段放行声明，完整准入与规定模型审核仍待完成。

# ThinWorkspace Phase 1 实施计划

**版本：v1.0｜覆盖：P0/P1｜文档性质：可更新实施路线图**

## 一、文档职责

本文档只管理“当前要做哪些任务、依赖关系是什么、小阶段如何收口”。

- 阶段产品能力和退出控制点以《产品与架构演进方案》为准；
- 领域与生命周期以《Phase 1 单机 CLI 详细设计》为准；
- 物化与跨卷语义以《跨平台工作区物化设计》为准；
- 任务从准入到放行的固定时序以《任务流程》为准。

本计划不复制系统调用、状态迁移表、CLI 错误码和完整测试矩阵。

---

## 二、状态和更新方式

每个任务使用《任务流程》定义的状态，并在下列表格的“状态”列更新；状态值和迁移规则不在本计划重复。

`Done` 只表示对应任务验收完成，不代表 P0/P1 阶段自动放行。阶段放行仍需要架构方案的退出控制点、《任务流程》的阶段门禁和人工签字。

---

## 三、P0 关键技术验证

P0 的产物是可重复实验、ADR 和失败边界，不是可直接发布的产品代码。

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P0-01 Host/Path Probe 实验 | 可稳定识别主机、APFS Volume ID、可写性和路径身份变化 | 无 | R4 | Done |
| P0-02 APFS clone 与跨卷实验 | 证明同卷 CoW、跨卷失败、显式 Full Copy 及 partial rollback | P0-01 | R4 | Done |
| P0-03 Git 托管拓扑与 Base 实验（已取消） | 原方案不再实施；历史实验及未提交候选保留，不作为新路线通过证据 | 不再参与依赖 | —（历史 R4） | Cancelled |
| P0-04 生命周期故障注入（已取消） | 中断恢复不再属于 P0/P1；已有未合并实验仅保留为历史输入，不作为通过证据 | 不再参与依赖 | —（历史 R4） | Cancelled |
| P0-05 文件副本锁隔离验证 | 可重复的 macOS 文件身份/锁/时间戳实验，形成是否需要锁冲突处理的 Go/No-Go | P0-02 | R4 | Done |
| P0-06 原始目录镜像验证 | 非 Git/完整 .git/ignored 文件物化与首版 mtime 范围、源一致性边界及无 Git 控制项 | P0-02 | R4 | Done |
| P0-07 已跟踪检查与清理提示验证 | 主/子仓库 tracked-only、Git unknown、显式强制与持久日志的最小真实验证 | P0-06 | R4 | Done |

P0-03 因 ADR-0002 取消，旧属性/sparse 兼容选择不再阻塞新路线；不再启动其旧全量收尾批次。P0-06/P0-07 为新任务，不复用已取消编号；两项任务级验收证据见下方链接。P0-04 按 [ADR-0003](../../project/architecture/adr/ADR-0003_Phase1不实现中断恢复.md) 取消，不把未合并实验代码并入产品。

P0-05 是新增 Spike，不预设“扫描后重写”方案成立；首次执行顺序与时间预算见 §3.2，不改变已完成任务的状态。

### 3.1 P0 小阶段与当前领取

负责人为主 Agent；模型分工遵循《任务流程》§8.3。小阶段划分只用于收口，不改变上表依赖。

| 小阶段 | 包含任务 | 可观察子目标和退出条件 | 适用门禁 |
|---|---|---|---|
| P0.a 路径能力证据 | P0-01 | 实际路径的卷身份、权限和缺失目标证据可复查；路径/符号链接替换被识别；预检不产生 CoW confirmed | Rust 通用门禁、真实 APFS 和跨卷身份测试、受影响 crate 全量变异、路径 fuzz、专项审核 |
| P0.b 原始目录与清理可行性 | P0-02、P0-05、P0-06、P0-07 | 目录原样物化和首版保真范围可复查；tracked-only/子仓库检查、显式 force 和日志准确；锁隔离有实测结论，不预设修复功能 | 小阶段门禁、真实 clone/跨卷/回滚、只读 Git、强制日志和双进程锁验证 |

P0.b 收口后直接核对 P0 阶段退出控制点、执行完整阶段门禁并请求人工放行；不再设置 P0.c 或 P0-04 前置。

P0-01 从文档基线 `f7a400f` 开始，工作分支为 `task/p0-01-host-path-probe`。本次使用 FD 与卷属性 FFI，按最高影响将风险从 R3 调整为 R4。验收断言、实际命令、审核结论和未完成项统一记录在 [P0-01 实施记录](../implementation/P0-01_HostPathProbe实验.md)。P0.a 收口不代表整个 P0 放行。

P0.a 已完成任务级与小阶段验收，P0-01 已获规定模型正式 Approve 并合并；精确提交、CI、未执行边界及工作区保留原因只记录在上述实施记录。当时整个 P0 尚未放行；当前正式放行结果以 [Phase 0 阶段状态](../review/ThinWorkspace_Phase0阶段状态_v1.0.md)为准。

P0-02 已获规定模型正式 Approve，审核后真实 Verification 和远端候选 CI 通过，PR 已合并。准备、验收断言、精确提交、验证证据和保留边界只在 [P0-02 实施记录](../implementation/P0-02_APFS物化实验.md) 维护；任务 Done 不代表 P0.b 或整个 P0 放行。

旧 P0-03 的准入、实验和未完成门禁保存在 [P0-03 历史记录](../implementation/P0-03_Git与Base实验.md)。取消不等于验收通过，不删除其中代码、失败日志、scratch 或保留工作区；可复用的小型代码须在新任务中独立审查，不能带回 Git/Base 领域。

历史顺序：P0-06/P0-07/P0-05 完成 → 核验未取消的 P0 控制点与阶段门禁 → 人工确认 P0 放行 → 按依赖进入 P1。维护者已完成 Phase 0 人工放行，P1 的阶段前置依赖已解除；精确候选和签字见上述阶段状态。P0-06 的基线、保真范围、CI 修复与收口见 [P0-06 实施记录](../implementation/P0-06_原始目录镜像实验.md)。P0-07 的审核、Verification、门禁及 PR #7 合并见 [P0-07 实施记录](../implementation/P0-07_已跟踪检查与清理提示实验.md)的最终收口。P0-05 的首轮实测、精确审核、候选 CI 与 PR #8 合并见 [P0-05 实施记录](../implementation/P0-05_文件副本锁隔离实验.md)。各任务 Done 本身不扩大验证范围，也不曾替代阶段放行。

### 3.2 P0-05 排期与实施准入

**负责人：主 Agent｜类型：Spike｜风险：R4（真实文件系统/锁 FFI 与替换对照实验）｜当前状态：见 P0 任务表。**

2026-09-12 已完成[文件克隆与锁语义调研](../../project/reference/ThinWorkspace_文件克隆与锁语义调研.md)，该检查点只有文献证据，尚无本机专项实验结果。该次文档任务为 R1，基线 `0a261a7`，分支 `docs/clone-lock-research`；验收断言为关键判断有一手出处、预期与实测分开、排期不预先批准处理方案、不改变现行产品契约。该次调研期间代码实施保持暂停，不把调研或排期视为领取 P0-05。

**启动顺序按 §3.1 当前顺序执行。** P0-05 的技术前置仍仅为 P0-02；不再等待已取消的 P0-03。P0-05 必须在 P0.b 收口前完成结论评审；不把 Linux/Windows 环境就绪设为本轮阻塞依赖。

| 验证步骤 | 预计工作量 | 可交付结果 |
|---|---|---|
| 实验准入与文件身份/锁对照 | 0.5 个工作日 | 固定临时 APFS 根、测试进程握手和有界等待；已知冲突对照有效，克隆/复制锁结果可复查 |
| 时间戳、标记文件、竞态与替换风险 | 0.5 个工作日 | 完成调研矩阵首轮必需场景，分开报告隔离、一致性与元数据；不改真实缓存 |
| 门禁、规定模型审核与结论 | 0.5–1 个工作日 | 可重复命令、原始结果、未执行边界及 Go/No-Go；明确有无必要另立实现任务 |

合计预估 **1.5–2 个工作日**，从获准恢复且前述准备满足后计，不是自动启动或固定日历交付承诺。预算耗尽仍无法归因时报告已验证边界和下一项必要证据，不连续追加“解锁”机制。

准入与验收：

- 输入为调研 §7 的首轮 L1–L7、P0-02 可复用实验基线及可验证的本机 APFS 临时根；进入 Ready 前，在该任务的实施记录中固定实际运行命令、基线、临时根和回滚/清理办法，不以本表替代可执行验收断言。
- 主要写入区为 `experiments/p0/` 的独立锁验证实验；只操作新建测试数据和自有进程，不改生产 crate、真实缓存、原始目录物化契约、ProcessProbe 或公开 CLI。中断/失败时停止该场景，保留可解释证据，仅清理由实验证明归属的对象。
- 完整实验矩阵只维护在上述调研，执行时将环境、命令、结果和审核证据写入 P0-05 实施记录并更新文档索引；不新建独立 Agent 审核轮次报告。文献预期与实测不符时先通知主 Agent 核验，不改测试使其迁就预期。
- 门禁按《任务流程》执行通用检查、受影响实验的真实 APFS/双进程测试及适用安全、变异和 fuzz 验证；不适用项须有理由。原始工具样例 L8 缺失时如实标未执行，不阻塞内核锁结论，但禁止宣称具体缓存问题已修复。
- 首轮成功隔离则不安排“扫描＋重写”产品功能；确有可重复问题则由主 Agent 按调研 §8 决定局部缺陷任务或提交范围/ADR 决策，不自动扩张 P1。实验结论不能代替人工阶段放行。

后续平台安排：Linux 在对应 Adapter 正式立项且 Btrfs/XFS 真实环境具备时，先复用本矩阵做平台准入；Windows 同理在 Windows 能力立项且相应 NTFS/ReFS 环境具备时验证。当前不承诺日历日期、不新增未批准的平台实现任务，也不将两者写成“无需处理”。

历史验证（2026-09-12 锁调研排期，非当前设计候选）：GPT-6 Astra（`gpt-6-astra` / `xhigh`）正式审核最终三文件变更及实验时序补充，结论 Approve；19 份 Markdown、82 个本地链接、2 个 JSON 示例与范围断言检查通过，`git diff --check` 和《任务流程》§10.1 三项通用门禁均终态退出 0。普通测试保留 2 项额外 APFS 卷用例的既有忽略状态；该次未执行 P0-05 专项、额外跨卷、变异、fuzz 或 Linux/Windows 验证，不作为实验完成或阶段放行证据。

---

### 3.3 新路线实验边界

P0-06 只补原始目录物化所需证据：非 Git 来源、完整 .git、ignored/未跟踪内容、普通文件/目录 mtime、外部链接与源写入观察、去除 Git 控制项后的回滚。完整契约引用物化设计，不扩展缓存适配或 Git 重定位。

P0-07 只验证已跟踪检查与清理决策：真实主/子仓库状态、无仓库、外部 Git 元数据与安全查询 unknown、显式 force 和工作区外日志。验收断言来自详细设计/手册，不实现自动 commit、push、PR 或交付证明。小阶段需包含未跟踪-only 不触发父/子 dirty 的回归。

### 3.4 Issues 与当前执行优先级

维护者于 2026-09-13 要求审核后优先处理 Issues，并明确 #12、#9、#13 暂不处理。此处仅维护与现有任务的依赖、排期和范围关系；Issue 原始需求、逐项验收和执行证据保留在 GitHub，不复制第二份问题正文或完成清单，也不把 Issue 建议直接视为已接受设计。

| Issue | 当前安排与边界 |
|---|---|
| [#16](https://github.com/chinayangxiaowei/thinworkspace/issues/16) | 已通过 [PR #18](https://github.com/chinayangxiaowei/thinworkspace/pull/18) 合并并关闭；P0.b 测试可靠性修复，不重开已 Done 的 P0-07，不改变生产配置策略 |
| [#10](https://github.com/chinayangxiaowei/thinworkspace/issues/10) | 已通过 [PR #19](https://github.com/chinayangxiaowei/thinworkspace/pull/19) 合并配置窄化修复；不重开已 Done 的 P0-07，不把该修复扩大为通用 Git 配置兼容层 |
| [#15](https://github.com/chinayangxiaowei/thinworkspace/issues/15) | 维护者已取消中断恢复范围；本地计划不再据此实施 P0-04 或 P1 repair，远端 Issue 状态未在本次修改 |
| [#11](https://github.com/chinayangxiaowei/thinworkspace/issues/11) | 作为 P1 可安装纵向交付的跟踪入口，落实下文工程、创建、查询、清理和发布验收；不提前宣称 CLI 可用 |
| [#14](https://github.com/chinayangxiaowei/thinworkspace/issues/14) | 先确定可重复数据集与基准环境并测基线，再冻结 P1 发布指标；数值和大规模资源预算不得凭空填写，未验证前不增加性能承诺 |
| [#12](https://github.com/chinayangxiaowei/thinworkspace/issues/12)、[#9](https://github.com/chinayangxiaowei/thinworkspace/issues/9)、[#13](https://github.com/chinayangxiaowei/thinworkspace/issues/13) | 按维护者决定暂缓，不主动变更这些 Issue 的远端状态；不修改许可、普通清理或路径交付契约，不继续追问；仅在维护者明确恢复后再处理 |

#11 依赖 P1 交付；#15 的原中断恢复范围已撤销，不能把“所有 Issues 先关闭”反向设为基础功能的前置条件。#10 和 #14 不阻塞 P1 的最小功能链；暂缓不等于问题已修复或阶段放行豁免。

## 四、P1 实施序列

当前设计依据 ADR-0002：P1-04/P1-05/P1-08 与此前 P1-11 均取消，其他编号保留并修正依赖；P1-16 是新增只读能力，不复用 Git 托管任务。源代码和 CI 旧实验本次保持不动，实验退役的构建范围处置在新任务准入时显式审核，不能直接删测试以通过门禁。

前次文档基线修订（历史）依据 [ADR-0001](../../project/architecture/adr/ADR-0001_Phase1普通目录与无执行包装.md)：取消命令执行包装，改为普通路径直接使用原有工具。主 Agent 负责，归属 P1 范围收缩，风险 R3（影响未实现的 CLI、持久化与进程设计）；该次修订期间代码实施保持暂停。验收断言是现行文档无执行包装的正向契约、执行状态或任务依赖，内部 Git 可靠性和外部占用保护不被误删。原 P1-11 保留编号并标记 Cancelled，不复用；本次不改变 P0 任务状态或 Base 导入范围。

历史验证（ADR-0001 无执行包装修订，非当前设计候选）：GPT-6 Astra（`gpt-6-astra` / `xhigh`）正式审核结论为 Approve；18 份 Markdown 的本地链接、围栏、2 个 JSON 示例及上述残留检查通过，`git diff --check` 和《任务流程》§10.1 三项通用门禁均通过。普通测试保留 2 项额外 APFS 卷用例的既有忽略状态；该次仅改文档，不运行跨卷专项、变异、fuzz 或尚未实现的 P1 help/E2E，也不作为阶段放行证据。

### 4.1 P1.a 工程与持久化基础

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-01 Rust workspace 和 Core 类型 | crate 依赖门禁、类型 ID、错误模型与基础测试 | P0 结论 | R2 | Done |
| P1-02 BootstrapStore、SQLite schema 和双 scope 锁 | 可迁移 schema、实例身份、并发唯一性与未完成状态 | P1-01 | R4 | Done |
| P1-03 init/doctor | 完成可见初始化、只读能力诊断和不接管中断残留的边界 | P1-02 | R4 | Done |

小阶段退出：全新、幂等和冲突初始化有自动证据；中断残留不被自动接管，未经验证的 data root 不被接管。

P1-01 领取记录：P1-01 由主 Agent 于 2026-09-23 领取，基线 `e547742`，任务分支 `task/p1-01-core-foundation`，工作区 `/Volumes/data/code/worktree`。本任务修改根 workspace、`thinws-core`、依赖方向检查、名称 fuzz harness 和既有 CI 入口；验收类型化 ID、名称语法、结构化错误、禁止依赖边及其自动测试。无状态、SQLite、Git、文件系统或公开 CLI 副作用，不创建空 Port/Adapter/Application/CLI crate，不改变既有 ADR。

P1-01 完成记录（2026-09-23）：

- 实现 commit 为 `f494d59`，审核修正 commit 为 `26ec218`；交付 UUIDv7 类型 ID、冻结的 Workspace 名称语法、结构化 Core 错误、敏感上下文脱敏、Core 直接依赖白名单及自动依赖方向检查。CLI 进程退出码映射不进入 Core。
- 本轮有效 RED 只计两项行为失败：Core 仍暴露 `exit_status()` 时 `compile_fail` 契约失败；依赖抽取遗漏外部/实验/注册表直接依赖时回归测试得到空违规集。修正后两项均 GREEN。初始缺失模块或导入导致的编译失败不符合 RED 定义，未计为 TDD 证据。
- 最终普通门禁：根 workspace 与 fuzz workspace 的 fmt、Clippy 全目标/全 feature 均通过；workspace 普通测试 265 通过、0 失败、7 个既有环境用例忽略；workspace rustdoc、26 个仓库工具测试、实际 crate 依赖检查、release workspace 构建、release Core 5 项测试和 1 项 rustdoc 均通过。`cargo-deny` 和离线 `cargo-audit` 通过；仅有既有未命中许可证 allowance/exception 警告。
- 变异证据按受影响范围组合：候选 `f494d59` 的 Core 全量结果为 56＝42 caught＋14 unviable，0 missed/timeout（SHA-256 `3091b1db0863521c6838c088d5b7a88a7d489ce139ac2f9e2693f5bcd729c44c`）；修正候选 `26ec218` 对变化的 `diagnostic.rs` 补跑 22＝14 caught＋8 unviable，0 missed/timeout（SHA-256 `44cf6a8c1b52c8240fd8ad17e41e11f29086a706ce5868a4851019a04653c717`），其余 32 个未受影响变异经复审核对后复用。
- 名称解析 fuzz 使用 `cargo-fuzz 0.12.0`、`nightly-2026-08-14` 和 60 秒预算，完成 30,032,981 次运行，无 crash/hang，未新增 artifact；后续修正未改变名称解析、harness、依赖、工具或配置，证据按《任务流程》§18.1 复用。
- GPT-6 Astra（`gpt-6-astra`）/ `xhigh` 对完整候选 `26ec2186aef7558ffe8b8cda4326fbd61863469e` 独立只读复审，结论 Approve，Critical/High/Normal/Low 均为 0；随后 release Verification 的 Core 5 项测试、1 项 rustdoc、6 项依赖检查器测试及实际依赖检查再次通过。
- 未执行线上 CI、线上 PR、push、真实 APFS/Git/SQLite/CLI 验收：前两项不属于当前本地流程，后四项没有 P1-01 运行时副作用。`/Volumes/data/code/worktree` 是维护者指定的唯一持久项目 checkout，不是可删除的临时 worktree；本地快进合并后继续作为 `main` 集成目录保留，任务分支随即删除。该任务完成不表示 P1.a 小阶段或 Phase 1 放行。

当前领取：P1-02 由主 Agent 于 2026-09-23 领取，基线 `4fd9dd7`，任务分支 `task/p1-02-bootstrap-store`，继续使用维护者指定的唯一持久 checkout `/Volumes/data/code/worktree`。先由 ADR-0004 冻结 bootstrap/root marker、SQLite schema v1 与双 scope 锁协议，再实现实际 Port、Adapter、迁移和并发/未完成状态测试；P1-03 的 init/doctor、Workspace 物化、Ready Receipt、目录删除和公开 CLI 均在本任务范围外。风险 R4，文档候选须经规定模型审核后才进入代码。

P1-02 完成记录（2026-09-23）：

- 设计 commit 为 `4f93f1f`，实现及收口 commit 为 `8437e8a`、`7599a2b`、`94926ed`、`01fc5f0`、`0d89d09`。最终候选 `0d89d0948f9f55f408d955819c2a5206415b2821` 交付版本化 bootstrap/root marker、持有父目录与 marker 身份的单向 Ready 发布、SQLite schema v1、事务迁移、并发唯一性、未完成状态、bootstrap/data-root 双 scope 有界锁及对应 Port/Adapter；未实现 P1-03 CLI、物化、Receipt Ready 用例、删除或中断恢复。
- 审核修正关闭三类阻断缺口：Ready 证明现在绑定创建时 data root 的打开 FD/身份；空库判断用字面 `sqlite_` 前缀，不会漏算 `sqliteX` 等合法用户表；四张表使用 `STRICT, WITHOUT ROWID` 并以 insert-once trigger 拒绝 `INSERT OR REPLACE` 绕过不可变、状态、Receipt 和 tombstone 约束。三项均有先失败后通过的真实回归。
- 普通门禁在 macOS 15.7.2 arm64、Rust 1.97.1 上通过：根 workspace 与 fuzz workspace 的 fmt、Clippy 全目标/全 feature、workspace rustdoc、28 项仓库工具测试、实际 crate 依赖检查、debug/release 全 workspace 测试、`cargo-deny` 及离线 `cargo-audit --no-fetch` 均成功；deny 仅报告既有未命中 allowance/exception 警告。最终 `WITHOUT ROWID` 差异另行重跑 SQLite debug/release 全套 2＋10 项测试和 Clippy，全部通过。
- 真实文件系统验证使用维护者确认的 `/Volumes/data` APFS 卷（Volume UUID `1a42c888-32e3-489c-9bfa-67fd640a94e8`）及一次性独立 APFS 验证卷；bootstrap 发布、目录/marker 替换、锁竞争、普通/Release 与既有适用跨卷用例均通过。一次性卷已卸载，临时根已移入废纸篓；没有触碰真实用户 bootstrap 或 data root。
- 首次 381 个 mutant 全量批次用于诊断，结果为 198 caught、92 unviable、91 missed、0 timeout，约 20 分 36 秒；它不是通过结果。修正后只对变化和原存活所在范围重新枚举并逐轮收敛：79 个为 60 caught＋6 unviable＋13 missed，24 个为 19 caught＋4 unviable＋1 missed，6 个为 4 caught＋2 unviable＋0 missed，审核修正后的 7 个为 6 caught＋1 unviable＋0 missed/timeout。最后一批 `outcomes.json`/`mutants.json` SHA-256 分别为 `15a20d439cbf579054a10218d2f8e9b14bdc85c7902dffbfb57d34c7a15df136`、`ced46d389ef97425b545604595f42b7fab6149bd81ccbeea4d9d93edb4d24c49`；四个剩余 OR/XOR、路径组件 guard 和版本 guard 变异均由规定 Reviewer 接受为等价，不存在未处置存活变异。后续相同 7 项范围可按本次实际 22 秒重新估算等待，不再固定等待一小时。
- `thinws_workspace_state` 与 `thinws_bootstrap_document` 使用固定 cargo-fuzz/nightly 各运行 61 秒，分别完成 15,327,490 与 1,723,157 次执行，无 crash/hang，未产生需要保留的回归样本。后续审核修正未改变两个 harness 可达的状态/文档解析代码，按《任务流程》复用该证据。
- ADR 设计候选由 GPT-6 Astra（`gpt-6-astra` / `xhigh`）独立只读审核为 Approve。实现终审先后识别父目录身份、空库判断和 REPLACE/rowid 绕过并退回修正；同一规定模型对最终完整范围 `4fd9dd7..0d89d09` 审核为 Approve，Critical/High/Normal/Low 均为 0，结论绑定精确 commit `0d89d0948f9f55f408d955819c2a5206415b2821`。
- 未执行线上 CI、线上 PR 或 push，符合当前本地流程；供应链审计使用本地 1261 条 advisory 数据，未联网刷新。极端创建权限被 umask 削减或首次文件校验失败时可能留下未发布的零字节临时项，它不会发布 marker/config 或冒报成功，属于 ADR 已允许报告并显式清理的未发布残留，不扩展为自动恢复。`/Volumes/data/code/worktree` 是维护者指定的持久 checkout，不删除。P1-02 Done 不表示 P1.a 小阶段或 Phase 1 放行；P1-03 仍是小阶段剩余任务。

当前领取：P1-03 由主 Agent 于 2026-09-23 领取，基线 `dfa345b`，任务分支 `task/p1-03-init-doctor`，继续使用 `/Volumes/data/code/worktree`。范围内是现有 BootstrapStore/MetadataStore Port 的初始化构造边界、macOS 私有目录与 APFS Volume ID、Application init/doctor 编排，以及首批 `thinws init|doctor` 人类/JSON 契约；范围外是 Workspace 物化、GitInspector、修复/恢复、删除、GC 和其余 CLI。因引入最小 macOS Volume UUID FFI，风险从原 R3 调整为 R4；先审核本段及对应设计/ADR/公开契约，再进入代码。

P1-03 完成记录（2026-09-23）：

- 设计 commit 为 `2a6a564`、`06a7ce0`，实现及审核修正 commit 为 `6a77f22`、`81a5448`、`c956cce`、`354ab26`。最终候选交付 `thinws init --data-root`、只读 `thinws doctor`、人类/JSON 统一输出、macOS APFS Volume UUID 探测、私有目录布局、SQLite 初始化工厂和 Application 编排；Workspace 物化、Git 检查、修复/恢复、删除、GC 仍在后续任务，CLI 不包装用户命令。
- 审核修正均有针对性回归：每个受控目录 FD 复验实际 APFS Volume UUID，prepared root 在发布 marker 前再次核验为空；目录类型、符号链接、权限、配置/marker 替换及持有数据库 FD 后目录项被替换均按稳定错误分类拒绝；布局错误不会被 SQLite 或通用文件系统错误覆盖；Clap 人类模式参数错误统一为 `E_USAGE`。这些行为从预期失败转为通过后才进入收口。
- 普通门禁在 macOS arm64、Rust 1.97.1 上通过：根 workspace 的 fmt、Clippy 全目标/全 feature、debug/release 全 workspace 测试和 rustdoc，29 项仓库工具测试、实际 crate 依赖检查、`cargo-deny` 及离线 `cargo-audit --no-fetch` 均成功；deny 只报告既有未命中 allowance/exception 警告，审计使用本地 1261 条 advisory 数据。
- 真实平台验证使用维护者确认的 `/Volumes/data` APFS 卷（Volume UUID `1a42c888-32e3-489c-9bfa-67fd640a94e8`），显式设置受控跨卷根后，系统临时卷与该卷之间的逐目录 FD Volume UUID 不一致路径通过。尝试用 `hdiutil attach` 创建子挂载点时被本机权限拒绝，因此“受控目录树内部出现实际子挂载”的端到端场景标为未执行；这不影响已通过的同一底层跨卷拒绝入口，也不扩写为已验证场景。
- 变异门禁按最终受影响生产范围组合：九个主要模块共 366 个 mutant，结果为 274 caught＋92 unviable，0 missed/timeout，耗时约 15 分 03 秒，`outcomes.json`/`mutants.json` SHA-256 分别为 `13e7c7aa01a2a04916279ce0dae828234e64cd77b31a1f996a7e35d8626f8bb7`、`cbd968d78148bb2e3cd2860d2e8383696fea7ba1f3cf2baae90d39a7fc43d8cd`；随后对唯一漏枚举的 Adapter 入口补跑 1 个 unviable mutant，0 missed/timeout，两个文件哈希分别为 `09eb6abddc542ef1f5c60f46fb79238c1848b04e80b513fd9e4844ca86704453`、`7da25b0e06d9fe53850cf745b35a1e001dfba0f04474e6881c9e1e166b954110`。合计 367＝274 caught＋93 unviable，0 missed/timeout；两个仅重导出或声明的 Port 文件枚举为 0 mutant。首次因 cargo-mutants 临时副本使跨卷测试失去异卷条件而在 unmutated baseline 退出的批次执行 0 个 mutant，不计为通过证据。
- `thinws_init_request` 与 `thinws_bootstrap_document` 使用固定 nightly/cargo-fuzz 各运行 61 秒，分别完成 18,530,874 和 1,644,355 次执行，无 crash/hang。后续审核修正没有改变两个 harness 可达的请求转换、文档解码函数或 fuzz 配置，证据按《任务流程》§18.1 复用。
- GPT-6 Astra（`gpt-6-astra` / `xhigh`）先后指出目录卷身份、发布前竞态、布局错误分类和 CLI usage 契约缺口；修正后对完整范围复审为 Approve。最终测试专用 commit `354ab26e4ce8fdd64e8d1795a869aaa972815867` 只为跨卷测试显式绑定维护者提供的 APFS 根，Reviewer 复核后仍为 Approve，Critical/High/Normal/Low 均为 0。
- 未执行线上 CI、线上 PR 或 push，符合当前本地流程；除上述权限受限的实际子挂载外，没有把未执行项写成通过。P1-03 Done 使 P1.a 的三项任务和既定退出断言全部完成，但不表示整个 Phase 1 放行。

P1.a 小阶段完成（2026-09-23）：P1-01、P1-02、P1-03 的有效证据组合证明全新、幂等和冲突初始化已有自动测试；非空或未完成残留不会被自动接管；未验证、身份变化或布局不合法的 data root 不会被发布为 Ready。小阶段门禁与受影响范围的变异、fuzz、真实 APFS/跨卷及规定审核均已收口，以上未执行边界继续保留。P1.a 完成只解除后续任务依赖，不代表 Phase 1 阶段放行、发布可用或维护者签字。

### 4.2 原 P1.b Git 托管任务处置

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-04 托管 Repository（已取消） | 原始目录输入不注册或导入 Git 仓库 | 不再参与依赖 | —（历史 R4） | Cancelled |
| P1-05 Git Base（已取消） | 不从提交生成或缓存 Base | 不再参与依赖 | —（历史 R4） | Cancelled |

本分组不再有待验收能力，也不计为已交付。取消理由见 ADR-0002；编号保留不复用。

### 4.3 P1.c 物化与 Workspace 创建

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-06 APFS WorkspaceMaterializer | 原始目录、首版保真范围、真实 CoW Receipt | P1-03、P0-06 | R4 | Done |
| P1-07 Full Copy WorkspaceMaterializer | 独立后端、相同保真范围和受策略限制的显式降级 | P1-06 | R4 | Done |
| P1-08 Git attach/detach（已取消） | 不注册或注销 Git worktree；只读检查另由 P1-16 交付 | 不再参与依赖 | —（历史 R4） | Cancelled |
| P1-09 Workspace create | 从 source 直接镜像，只有完整物化并持久化后才成为 Ready | P1-06、P1-07 | R4 | Done |

小阶段退出：非 Git 来源、原样目录范围、默认 CoW、显式复制、路径竞态和 partial rollback 通过；未完成目录不被误报 Ready，Ready 不要求 Git clean。

当前领取：P1-06 由主 Agent 于 2026-09-23 领取，基线 `50617e9`，任务分支 `task/p1-06-apfs-materializer`，继续使用唯一持久 checkout `/Volumes/data/code/worktree`。风险 R4，主要写入区为 `thinws-core`、`thinws-ports` 和 `thinws-adapter-macos`；P0 Probe/物化实验只作为证据输入，不成为生产依赖。范围内是共同物化值、APFS 路径 Probe、`ApfsCloneMaterializer`、真实同卷 CoW、清单/保真校验、源变化检测、partial receipt 和身份约束回滚；范围外是 Full Copy/fallback、公开 CLI、SQLite 生命周期编排、Workspace Ready、删除/GC、Git 和后续平台。

验收断言：同卷真实 APFS 上，非 Git、原样 `.git`、ignored/未跟踪内容、系统可表示的原始名称、普通文件/目录/符号链接及约定权限和 mtime 均按设计物化，普通文件写入与源隔离，Receipt 的 clone 数和 CoW 事实准确；任意非 UTF-8 字节只在不接触文件系统的路径证据编码/解析测试中验证无损，APFS 以 `EILSEQ` 拒绝的名称不冒充可创建。空树与仅目录/链接树成功但不冒充 CoW confirmed。跨卷、路径/卷/挂载或源身份变化、包含关系、非空目标、特殊文件和子挂载在错误边界停止；部分创建按已登记身份逆序回滚，替换对象或无法确认回滚时保留 partial receipt 且不扩大删除。Probe 的 supported/unsupported/unknown、缺失目标父目录和四类实际路径组合有自动及真实平台证据。P1-06 不调用 Full Copy、不写产品 SQLite、不发布 Ready，也不增加 `workspace create` 命令。

P1-06 完成记录（2026-09-24）：

- 实现及审核修正 commit 为 `1a89761`、`246459f`、`0cda871`、`268f6a2`、`ec10ddd`、`dbbd58a`、`55d82a4`、`0882a64`。交付共同物化值与 Port、macOS APFS Probe 和 `ApfsCloneMaterializer`，落实 Probe → Plan → Revalidate → Execute → Receipt、逐文件 clone、清单与保真核验、源变化拒绝、部分失败证据及按已登记身份逆序回滚；不实现 Full Copy/fallback、公开 CLI、SQLite 生命周期、Ready、删除/GC 或 Git 行为。
- 真实 APFS 验收覆盖非 Git 来源、原样 `.git`、ignored/未跟踪命名内容、Unicode/空格、硬链接、外部符号链接、权限与 mtime、8 个普通文件的逐文件 clone、双向写隔离，以及删除来源目录后目标仍可读。四类计划根替换、六组路径包含关系、非空目标、特殊文件、源变化和失败回滚均有回归测试；成功 Receipt 必须绑定相同清单摘要和准确的源/目标卷身份。终审发现的 clone 成功至首次身份登记窗口已有精确 RED：替换为源快照中的普通文件硬链接曾会误报 CoW 成功；修正后在登记前拒绝与任一源普通文件重合的身份，并以未确认对象保留，不由回滚接管或删除。
- 维护者确认的 `/Volumes/data` 与系统临时目录构成真实不同 APFS 卷，P1 跨卷 Probe、Plan 拒绝及既有 store-layout 跨卷测试均通过；同卷 CoW 由真实 `clonefile` 验收。当前主机不能创建额外子挂载，因此 P1 实现层的真实子挂载端到端用例未执行；以设备身份不一致的自动测试和 P0 已有真实子挂载证据覆盖该边界，但不把它表述为本轮已实测。
- 普通门禁在 macOS arm64、Rust 1.97.1 上通过：根 workspace 的 fmt、Clippy 全目标/全 feature、debug/release 全 workspace 测试和 rustdoc，以及仓库工具、实际依赖检查、`cargo-deny` 和离线 `cargo-audit --no-fetch`；供应链检查沿用未变更依赖与配置上的同候选证据，仅有既有未命中 allowance/exception 警告。
- 变异测试按受影响范围组合收口：首批 680 个 mutant 为 512 caught、156 unviable、12 timeout，超时来自 runner 将无关 P0 ignored 清理测试带入每个变异进程，不作为通过结论；限定 P1-06 三个 package 后，42 个相关/变化范围 mutant 为 24 caught、8 unviable、10 missed，随后针对这 10 项及构造变体补跑 12 个为 10 caught、2 unviable、0 missed/timeout；终审修正对变化行精确补跑 5 个为 4 caught、1 unviable、0 missed/timeout。四批 `outcomes.json` SHA-256 依次为 `dc824dd01c08ecf24865017878fc6378ff89915947293d59d0be675148e83223`、`90c3f91f095c1bd2009eac04a6a973f338cefe70b6ea95b6a7440afc5579644d`、`8ffa7073f1f9c0fd3d5268de1ab45c8d21ab8a9f32bc1fbb716ae3dab883813b`、`93544b22093d2f9a65ccec4a612ee36dcede9a2b5c8a2b6b49f9faa167c23f37`；对应 `mutants.json` 为 `836428fec9a34fdf8f22d279a2c826355b8eed7b3dff125a21e1eb2aa17d4779`、`bd613aa6e4473bbbbd445a07a1bc295a7587b70a7b11b30d830f586679d29eb3`、`f937de5f6c96ff3858951fa80ffa36cf2b60940c65eabff2c6190476daad29bf`、`4ac0d21b5d68ecd88b504b8a44db42a00debc75b322841f19842b33e3f415d22`，组合后无未处置 missed/timeout。
- `thinws_materialization_path` 使用 `cargo-fuzz 0.12.0`、`nightly-2026-08-14` 和 60 秒预算完成 17,373,911 次执行，退出码 0，无 crash/hang；运行生成的临时 corpus 已清理，仓库保留原有 3 个种子且未产生 artifact。
- GPT-6 Astra（`gpt-6-astra` / `xhigh`）对完整候选 `24ad6fdb29ddcb3791d07bd94447a6b0d4b1d050` 首次终审发现 clone 后首次身份登记窗口的一项 High，任务退回并以 `0882a64` 的精确 RED/GREEN 修正；同一规定模型随后审核完整范围 `50617e9cdf1eaf9f8ca3f2154d7cd4b12fb8a93e..078a24b412b9bc0f59a20f89df5e5cc5e3eca2a9`，结论 Approve，Critical/High/Normal/Low 均为 0。
- 未执行线上 CI、线上 PR 或 push，符合当前本地流程。P1-06 Done 仅表示 APFS 物化后端完成，不表示 `workspace create` 可用、P1.c 小阶段或 Phase 1 已放行；下一项技术依赖仍是 P1-07。

当前领取：P1-07 由主 Agent 于 2026-09-24 领取，基线 `38a2500`，任务分支 `task/p1-07-full-copy-materializer`，继续使用唯一持久 checkout `/Volumes/data/code/worktree`。基线 fmt 与 debug 全 workspace 测试通过。风险 R4，主要写入区仍为 `thinws-core`、`thinws-ports` 和 `thinws-adapter-macos`；P0 Full Copy 与 fallback 实验只作为算法和测试输入，不成为生产依赖。范围内是 Full Copy 候选证据、Core 的受限预检/运行时降级计划、fallback 原因与失败尝试证据、`FullCopyMaterializer`、真实字节复制、共同清单/保真和身份约束回滚；范围外是 P1-09 Application/SQLite/Ready 编排、公开 `workspace create`、跨卷复制、删除/GC、Git 和后续平台。

验收断言：Full Copy 候选不因 clone 能力缺失而被误判不可用，但 Phase 1 计划仍要求 source、target、staging、trash 位于同一已知 APFS Volume；默认 Deny、clone unknown、路径/权限/卷异常、`EXDEV`、`ENOSPC`、普通 I/O、源/目标变化或未确认回滚均不得降级。只有 clone 预检明确不支持且策略为 `AllowFullCopyOnCowUnsupported`，或运行时 `CowUnavailable` 且前次目标已证明未修改/恢复基线、重新 Probe 仍满足布局时，才产生 Full Copy 有效计划；预检降级没有失败尝试，运行时降级保留前次失败及回滚证据。真实同卷 APFS 上，非 Git、原样 `.git`、ignored/未跟踪内容、硬链接目录项、普通文件/目录/链接、权限和 mtime 满足与 P1-06 相同的保真范围；普通文件以独立字节副本交付，Receipt 为 actual/effective `full-copy`、`cow=not-used`、clone count 0，并绑定相同源/目标 manifest。部分写入、注册失败、路径替换和源变化返回 partial receipt，按登记身份逆序回滚且不接管替换对象。P1-07 不发布 Ready、不写产品 SQLite、不增加或改变公开 CLI。

P1-07 定向变异首次尝试（2026-09-24 03:03 UTC）：候选为基线 `38a2500` 上的暂存代码差异 SHA-256 `4f5649dd2e9b0bfb3edb95462f70497f5c540bff9aa95b1d85e28745de5dcd88`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库 `.cargo/mutants.toml`，并发 4。枚举的 152 个变异覆盖 Core 的 Full Copy 预检/运行时决策、同卷布局与回执，macOS Probe 的 clone/Full Copy 候选及摘要，Materializer 的共享执行与字节复制路径，以及独占新建文件 FFI；验证包为 `thinws-core` 和 `thinws-adapter-macos`，输出位置为 `target/p1-07-mutants/mutants.out`。独立审核发现旧候选存在新建文件登记竞态和运行时降级连续性缺口，主 Agent 主动中止该未完成批次并退回修改；此批不能作为门禁证据，也不能以其耗时估算完整执行。修正后须按受影响范围重新取得变异证据。小阶段全量变异仍留到 P1.c 收口。

P1-07 定向变异重跑启动记录（2026-09-24 03:16 UTC）：基线 `38a2500` 上的当前代码差异 SHA-256 `727a1f59c7a0a44f423ea5af37233197b4a72f189787a4bef6528d3b6cbf5708`；主 Agent 单独执行，环境及工具配置同上，并发 4，输出为 `target/p1-07-mutants-recheck/mutants.out`。按受影响生产函数过滤列出 176 个变异，包含新建身份登记、失败后来源连续性及四路径/回执绑定；验证包为 `thinws-core` 和 `thinws-adapter-macos`，开启 gitignore 以避免忽略的 P0 `target` 测试 fixture 进入变异副本。参考 P1-03 同类 367 个约 15 分钟的完整记录，首次主动查看预计 03:35 UTC；此前不轮询或修改该代码候选。

上述第二批实际于 03:17:54–03:22:43 UTC 完成，约 4 分 49 秒；176 个变异为 124 caught、23 unviable、29 missed、0 timeout，退出码 2，不能作为通过结论。`outcomes.json` SHA-256 为 `5a633f6d4aaebd0fd56caecc3b85792117dd122d8c880af6444bdece5adcc382`，`mutants.json` 为 `346a77f648faf28ae307568c1332798c3d5893f0b383876b5bf46ff8eaa9427f`。未到预估查看时间不轮询；这次完整耗时替代先前估算。存活项中请求/计划逐角色校验、跨卷候选、源目录项即时变化、回滚证据和失败回执分类已补直接测试；位标志 `|→^`、合法 Plan 不变量下不可单独触发的守卫变体，以及不改变字节内容的 copy buffer 大小变体需在最终核对时逐项证明等价或继续补测。

P1-07 定向补跑启动记录（2026-09-24 03:40 UTC）：当前代码差异 SHA-256 `b84cb85e8e3a25fd1f40f39664c1bd3547fe98fac629d4af9bfb1de8e617790c`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，验证包仍为 `thinws-core` 和 `thinws-adapter-macos`。只覆盖补测改变语义的生产函数及先记录待确认对象的登记函数，共 78 个变异，输出为 `target/p1-07-mutants-followup/mutants.out`。按上一批 176 个约 5 分钟加上固定构建开销，首次主动查看估计 03:46 UTC；此前不轮询或修改该代码候选。

补跑实际于 03:40:37–03:42:43 UTC 完成，约 2 分 07 秒；78 个变异为 64 caught、8 unviable、6 missed、0 timeout，退出码 2，不能记作工具全绿。`outcomes.json` SHA-256 为 `dd9cc1f41405af6e9ac8f4a0034e8882d3809246756c0cfbba025f7e004cbdd3`，`mutants.json` 为 `51affcfc983cad1e4da1609cc7e81f11632ff3319620c2ca81cf4e3feafc8cdd`。相对上一完整候选的 29 个 missed，新增测试已捕获所有可达的请求路径、候选卷、目录项变化、回滚证据、失败回执判定。剩余 6 项的等价性待独立 Reviewer 接受：`validate_request_plan` 中的 Adapter 与有效模式由 Plan/Backend 构造一一配对，单独改其 `||` 不改变判定；Core runtime fallback 的前四个 `||` 在合法 Plan 下分别同为 clone 基线或被 Full Copy 的其他不变量共同拒绝，卷比较的两个布尔值又因双方均通过同卷布局检查而恒相同。另有未补跑的 4 个 FFI `|→^` 变体：本机头文件中 `O_WRONLY=0x0001`、`O_CREAT=0x00000200`、`O_EXCL=0x00000800`、`O_NOFOLLOW=0x00000100`、`O_CLOEXEC=0x01000000` 位互不重叠；一个 copy buffer `64*1024→64+1024` 变体只改变内部每次读取大小，不改变复制内容、边界或公开性能保证；一个 Full Copy 成功回执的 Adapter/有效模式 `||→&&` 变体由所有可构造 Plan 的二者一致性保证等价。将来新增 Plan 构造、修改标志位、把 chunk 大小/内部 hook 设为公开契约或允许跨卷布局时，上述证明失效并须重测；在 Reviewer 接受前，P1-07 仍不得 Done。

上述 copy buffer 的“等价”仅相对于生产 `NoopHook` 和首版公开正确性契约；它确实改变内部测试 hook 的回调次数与故障注入时点，也可能改变吞吐，不宣称内部执行轨迹或性能等价。

P1-07 本地审核修正（2026-09-24）：规定 Reviewer 对精确实现提交 `d170b205fa628aa9ac1c266dd370d2efc5d886b9` 的结论为 Changes requested（Critical 0、High 1），指出目录、符号链接及 APFS clone 在创建后首次从公开 target 名称登记身份，可能接管内容相同的替换对象。前次四项 finding 已闭合，12 项存活变异的契约等价证明获接受；本项 High 未关闭前不得 Done 或合并。主 Agent 决定让无创建 FD 的对象先在实例私有 staging 固定身份，再以 no-replace rename 发布，发布前登记已知身份、发布后复核 target；明示 staging 无外部写者的信任前提，不承诺对恶意同 UID 内部篡改隔离。目录与链接的替换反例已先 RED 后 GREEN；匹配内容的外来文件、no-replace 和真实 CoW 均补自动证据。该修正同时覆盖 P1-06 共享路径，不重开 Git 或持久化范围。

P1-07 竞态修正定向变异启动记录（2026-09-24 04:05 UTC）：代码差异相对 `d170b205` 的 SHA-256 为 `d23a1abf64bea042656bc170a66af744f77a8ca513bcf9f395f4ef37aa544765`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，验证包为 `thinws-adapter-macos`，输出为 `target/p1-07-staged-publication-mutants/mutants.out`。只选择共享物化遍历、staging 发布/清理与对应 FFI 的 23 个生产变异；前次同机 78 个完整变异耗时约 2 分 07 秒，考虑本次构建与正常波动，预计 04:09 UTC 首次主动查看；之前不轮询、不修改冻结代码。

该批实际 44 秒完成，23 个变异为 15 caught、1 unviable、7 missed、0 timeout，退出码 2，不作为通过结论；按预计时间检查时已结束，未提前轮询。`outcomes.json`/`mutants.json` SHA-256 分别为 `bfba4c8fb111ded799b5c8e07ba925d788f5b77af58ddef89f5d5206c1cfdb69`、`a762b7045074971d54c73d71db4b59f838c124209370ba777e2740b612f6cda7`。存活项集中于 staging 名称冲突与非冲突错误、发布目标已存在后的清理和错误分类；已补直接回归。来源身份排除守卫只处理旧“从公开 target 首次取身份”的竞态，staging 独占创建加预先身份绑定后已成为不可达防御分支，已删除并保留替换为源硬链接的真实回归。

P1-07 竞态修正定向补跑启动记录（2026-09-24 04:14 UTC）：代码差异相对 `d170b205` 的 SHA-256 为 `b366916663333d072ee25ca26b395e273cb1eabcfaeb95c3c7cd09d679c9116e`；执行负责人、平台、工具、配置、并发与验证包同上，输出为 `target/p1-07-staged-publication-followup/mutants.out`。仅重新枚举受补测或删减影响的 staging 发布/清理和 FFI，共 9 个生产变异；按上一批 23 个 44 秒，留构建波动余量，预计 04:16 UTC 首次主动查看；此前不轮询或修改冻结代码。

补跑实际 38 秒完成，9 个变异为 8 caught、1 unviable、0 missed/timeout，退出码 0；`outcomes.json`/`mutants.json` SHA-256 分别为 `a6e90469b30d24514e4d79bcc6f7af15df3f61f79e11576c7f43e84c1fca992f`、`126d92baa3b3916917b308d1b90cc895cee9cba247be2d78bce04486111bc317`。第一批已捕获且本轮未变语义的物化遍历变异按同一候选证据复用；本轮更改的 staging 发布/清理范围无存活变异。此后仅调整测试 hook 名称、补 Full Copy staging 无残留断言及修正信任边界注释，生产行为与变异选择不变，不重跑相同变异。P1-07 仍需精确提交审核与合并，不能因变异绿单独标 Done。

本地最终候选普通门禁（2026-09-24，macOS arm64、Rust 1.97.1）：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`cargo test --workspace --all-targets`、`cargo test --workspace --release --all-targets`、`cargo doc --workspace --no-deps`、29 项工具测试、`cargo deny check`、`cargo audit --no-fetch` 均退出 0；deny 仅有既有未命中 allowance/exception 警告，离线审计使用本地 1261 条 advisory。`THINWS_P1_CROSS_VOLUME_ROOT=/Volumes/data` 下，真实跨卷 Probe 拒绝及 store 跨卷布局用例在 Debug/Release 均通过；真实同卷 APFS clone/Full Copy 与 staging 无残留由全 workspace 集成测试覆盖。新增目录、链接、匹配内容外来文件的目标替换故障注入，以及失败发布清理/no-replace 测试均通过。未执行真实子挂载创建（本机权限边界），未新增能到达本次 staging 执行代码的 fuzz harness；既有纯路径 fuzz 证据仅覆盖未变更的解析边界，不能代替本次文件系统竞态测试。线上 CI/PR/push 均不属于当前本地交付。

P1-07 精确提交 `4714b1fff710aaf9412368e6b73cf561163b6154` 经 GPT-6 Astra（`gpt-6-astra` / `xhigh`）只读复核，前次 High 已关闭，结论 Changes requested（Critical/High 0、Normal 2）：后续发布失败漏记已成功的 `fclonefileat` 调用；staging 清理失败未在失败 Receipt 中给出相对 staging 名称与身份是否已知。任务退回 In Progress。主 Agent 已用真实 APFS RED 确认第一项；修正计数时点，增加 staging 清理未确认的独立证据和禁止降级判定，分别对首次身份观察失败、发布失败、Full Copy 链接对象做故障注入。Debug/Release 全 workspace、Clippy 与 `/Volumes/data` 真实跨卷 Probe/store 验证已通过；此处仅是修正中的验证记录，不代表 P1-07 已审核通过。

P1-07 失败回执修正定向变异启动记录（2026-09-24 23:44 UTC）：相对 `4714b1f` 的代码和相关测试差异 SHA-256 为 `ba813ae2635cca008375b479d224cc0f6bef9dd69000759fada1224d2bfc8a87`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，验证包 `thinws-core` 与 `thinws-adapter-macos`。仅选择共享遍历、staging 发布、失败回执构造与 fallback 清洁基线判断相关的 35 个变异，输出为 `target/p1-07-failure-receipt-mutants/mutants.out`。上次同机 23 个约 44 秒、9 个约 38 秒，连同构建波动，首次主动查看预计 23:47 UTC；此前不轮询、不修改冻结生产代码。

该批实际 55 秒完成：31 caught、4 unviable、0 missed/timeout，退出码 0。`outcomes.json`/`mutants.json` SHA-256 分别为 `22d3f7e9f1617f3c22151fb659ba5393192c5ce0f738e37e1406042c6cc5cf3c`、`e243a82c46278a791e1c7da234c4644f60c04019106650c87a7ae1c9c02b702f`。之后自查又发现独占 rename 拒绝发布且 staging 清理失败时，target `created` 仍包含未发布的幻影项；已以单测 RED/GREEN 修正为 target 未修改、仅 staging 清理未确认。该小改动仅触及 `stage_and_publish`，需补跑此函数的变异；前批其他函数证据可沿用。

P1-07 幻影 target 修正变异补跑启动记录（2026-09-24 23:48 UTC）：相对 `4714b1f` 的 Adapter 差异 SHA-256 `9615c423b17d3d78e79e19934809c8a32081142ef704e91c6666d4868141d685`；主 Agent 单独执行，平台/工具/配置同上，并发 4、gitignore 开启，仅针对受影响的 `stage_and_publish` 6 个变异，验证包为 `thinws-adapter-macos`，输出 `target/p1-07-target-evidence-mutants/mutants.out`。上次同机 35 个 55 秒，留构建波动余量，首次主动查看预计 23:50 UTC；此前不轮询或修改候选生产代码。

补跑实际 16 秒完成：5 caught、1 unviable、0 missed/timeout，退出码 0；`outcomes.json`/`mutants.json` SHA-256 分别为 `52ad94638e9f6ed37ecf4187e3f3d2b6fee6d67a0ab555e8b8fe7950733e6896`、`8abbe048f586988abc3276d7aafa679f597ad41086f874fefa4b9a291c7a31fb`。此后生产代码未再变化。最终候选提交前本地 `cargo fmt --all -- --check`、Clippy 全目标/全 feature、Debug/Release 全 workspace 测试、rustdoc、29 项仓库工具测试、`cargo deny check`、离线 `cargo audit --no-fetch` 均退出 0；deny 只有既有未命中配置警告。`/Volumes/data` 真实跨卷 Probe 和 store 用例在 Debug/Release 均通过，相关 Probe/store 生产代码在本次回执修正中未变。不存在与本次 syscall 失败时序对应的模糊测试 harness；不以未变更的路径解析 fuzz 证据冒充此项覆盖。仍未执行线上 CI/PR/push 或真实子挂载创建。任务在精确提交独立审核通过前保持 In Progress。

GPT-6 Astra（`gpt-6-astra` / `xhigh`）只读审核精确候选 `0b98fa9a09e4fbc9db50cbd5113f62cf56e72df6`，结论 Changes requested（Critical/High 0、Normal 1、Low 0）。先前 High、两项失败回执 Normal 和幻影 target 均确认关闭；新增 Normal 是 Core 的 `successful_apfs_clone` 未像 `successful_full_copy` 一样拒绝后端不匹配的合法 Full Copy Plan，可构造 `effective=FullCopy / actual=CowClone / cow=Confirmed` 的矛盾成功回执。真实 Adapter 入口已有防护，但 Core API 不变量仍须闭合。主 Agent 已用 Full Copy Plan 生成该矛盾回执的测试先 RED 后 GREEN，增加对称的 `AdapterMismatch` 守卫；修复后的精确候选须重新审核。

P1-07 Core 回执守卫补跑变异启动记录（2026-09-24 23:56 UTC）：相对 `0b98fa9` 的 Core 代码与测试差异 SHA-256 `4a5f8cfa492ec271ceffd5303c517b119014ca2c2368cffdc4d164fe980203d1`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，仅针对新增守卫所属的 `successful_apfs_clone` 7 个变异，验证包 `thinws-core`，输出 `target/p1-07-clone-receipt-guard-mutants/mutants.out`。前次同机 6 个完整变异耗时 16 秒，保留构建波动，预计 23:58 UTC 首次主动查看；此前不轮询或修改候选生产代码。

该批实际 12 秒完成：5 caught、1 unviable、1 missed、0 timeout，退出码 2，不能作为通过结论；`outcomes.json`/`mutants.json` SHA-256 分别为 `5544c93689be414964fdb331f0b0d3f2647d1edb717aa15d3eea17cb67e6cd75`、`81bfa8d02fac1aa2b5b85e113bddc2bba5c6b4fd6537ee157c9ea0cffd70737b`。唯一 missed 为后端与有效模式的拒绝条件 `||→&&`；当前仅有的两种合法 Plan 构造分别固定 Clone/Cow 和 FullCopy/FullCopy，故该变异在当前模型下等价。为使不变量表达更直接并避免留下无用的等价变异，主 Agent 将两项校验改为一个明确的二元组合匹配，原 RED 回归保持绿色。

P1-07 Core 组合守卫变异重跑启动记录（2026-09-24 23:59 UTC）：相对 `0b98fa9` 的 Core 代码与测试差异 SHA-256 `5a2bce7d6e2ffbe66ca5e4f2319fee177a7b84e97d11695dd7f27d4ba8f31222`；执行负责人、平台、工具与配置同上，并发 4、gitignore 开启，仅针对受影响的 `successful_apfs_clone` 5 个变异，验证包 `thinws-core`，输出 `target/p1-07-clone-receipt-pair-mutants/mutants.out`。上次同机 7 个 12 秒，留构建波动，预计 00:01 UTC 首次主动查看；此前不轮询或修改候选生产代码。

组合守卫补跑实际 8 秒完成：4 caught、1 unviable、0 missed/timeout，退出码 0；`outcomes.json`/`mutants.json` SHA-256 分别为 `df792a97b15a33b0b726b64a3acbb15924b6242f1992757ccbeae53b1ce8c0d7`、`1aa5eb7ae815b8531c0a3d564c3fe12eb4501e3d7ba5f3ceb2d831724ec17472`。之后仅按 rustfmt 调整该 `matches!` 模式换行，匹配项、控制流和测试语义不变；核对实际变异位置后复用此批证据，不重启等价范围的变异。精确提交复核前仍须完成普通门禁。

Core 守卫最终门禁（2026-09-25 UTC）：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`cargo test --workspace --all-targets`、`cargo test --workspace --release --all-targets`、`cargo doc --workspace --no-deps` 均退出 0，`git diff --check` 通过。仅 Core 回执构造和对应测试/计划有语义变化，Adapter Probe/store、依赖、工具和供应链配置均未变，沿用 `0b98fa9` 的真实 `/Volumes/data` 跨卷、29 项工具测试、`cargo deny check`、离线 `cargo audit --no-fetch` 同环境证据；不宣称本次重复运行了它们。仍未执行线上 CI/PR/push、真实子挂载创建或没有对应 harness 的 syscall 时序 fuzz。P1-07 待修正后的精确候选独立审核，不提前标 Done。

P1-07 完成记录（2026-09-25 UTC）：GPT-6 Astra（`gpt-6-astra` / `xhigh`）只读独立复核完整范围 `38a2500..1428384576192205e90bbffe9dc80d1ef10224b4`，结论 Approve，Critical/High/Normal/Low 均为 0；前次 High、失败回执两项 Normal、幻影 target 与 Core 后端错配 Normal 均已关闭。任务提交为 `d170b20`、`4714b1f`、`0b98fa9`、`1428384`，本任务的真实同卷 APFS Clone/Full Copy、显式受限 fallback、失败回执/回滚和真实 `/Volumes/data` 跨卷拒绝验收均已取得前述证据。未执行项与限制维持前述记录；后续 P1-09 才实现 Workspace create、持久状态和 Ready，因此 P1-07 Done 不表示 P1.c 小阶段或 Phase 1 放行。

当前领取：P1-09 由主 Agent 于 2026-09-25 UTC 领取，基线 `c62f381`、任务分支 `task/p1-09-workspace-create`、唯一 checkout `/Volumes/data/code/worktree`；基线 fmt 和 Debug 全 workspace 测试通过。风险 R4（真实目录写入、状态与故障边界），主要写入区为现有 Application、MetadataStore、macOS Adapter 与 CLI；不派生开发 Agent 或线上 PR，审核由 GPT-6 Astra（`xhigh`）只读独立完成。范围内是同卷 APFS 来源的 create/dry-run、Creating 与 incomplete 标记、受控空目标、现有 CoW/Full Copy 物化与获准降级、最终 Receipt 和 Ready 同一短事务、失败转 Error、同名幂等/冲突和公开命令输出；范围外是 Git 检查、path/status/list、remove/GC、自动恢复、跨卷复制和执行包装。

P1-09 验收断言：无 Git 来源可以创建可直接访问的普通目录；实际 clone 或显式 Full Copy 的结果与持久 final Receipt 一致，只有完整物化、标记清除并完成最终 SQLite 提交才返回 Ready。同名且源/策略相同的 Ready 请求返回原路径而不重复制；不同参数报冲突，非 Ready 报未完成。dry-run 不分配 ID、不预留名称、不写受控目录或产品状态。默认不静默降级，跨卷/路径变化/源变化/空间不足/回滚未确认时不误报 Ready；Creating 后任一点失败或中断仍可由 doctor 观察为非 Ready，不自动续做。并发名称争用只有一个预留成功；根、卷、WorkspaceId 和目录项归属重验，不能接管外部对象。公开 help、人类/JSON 输出、错误码与用户手册一致；真实 APFS 与 `/Volumes/data` 跨卷、故障注入、锁超时、SQLite 事务失败、受影响变异及适用 fuzz smoke 有记录化证据。P1-12 之前不以尚未实现的 remove 承诺残留可在 CLI 内清理。

P1-09 首个持久化子结果（进行中）：在现有 `MetadataStoreFactory` 增加已验证布局上的生命周期 writer 打开能力，不增加第八个 Port；实际 SQLite 打开使用 no-create/no-follow，前后重验目录与数据库身份，校验既有 schema/installation，不执行 migration。缺失数据库、未初始化空库、布局证明失败或实例错配均拒绝；真实 writer 可预留 Creating。对应测试已按预期先 RED 后 GREEN；fmt、Clippy 与 Debug 全 workspace 基线/增量门禁通过。这只是创建链路的前置边界，不等于 final Receipt/Ready 或公开命令已交付。

P1-09 生命周期 writer 定向变异启动记录（2026-09-25 00:13 UTC）：基线 `c62f381` 上代码/测试差异 SHA-256 `be0e8f8e36ef1b969c3a32e6813b80155dcbcc40e0f800040d76bbd4bd33e097`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 1、gitignore 开启，只选择 `SqliteMetadataStoreFactory::open_existing` 1 个变异，验证包 `thinws-metadata-sqlite`，输出 `target/p1-09-open-existing-mutants/mutants.out`。上一批同机 5 个约 8 秒，考虑 SQLite 测试和构建波动，首次主动查看预计 00:15 UTC；此前不轮询或修改冻结代码。

该批实际 8 秒完成（00:13:49–00:13:57 UTC），1 个变异因试图将 `Box<dyn MetadataStore>` 替换为不可构造的 `Default` 而编译失败，结果为 1 unviable、0 caught/missed/timeout，工具退出码 0 但**无有效变异测试结论**；不将其计作门禁通过。`outcomes.json`/`mutants.json` SHA-256 分别为 `30a27d456a6cec005804c7357ab1c392f7189048648a96bf1f42f852aa9cea72`、`2bd84e171fd0f49cd3f5f7f5db6223c9d699746927a06205160584831ca31049`。本边界依赖已通过的缺库、空库、错实例与布局重验回归；后续 final Receipt/Ready 的有效变异另行覆盖。

P1-09 final Receipt/Ready 定向变异启动记录（2026-09-25 00:22 UTC）：基线 `d21f9ce` 上已暂存代码/测试差异 SHA-256 `6301ad9fe06c445facd4a43b9f44e81482dafbd45e7b877781369b19e53ff318`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，只选择 `SqliteMetadataStore::complete_materialization` 枚举出的 40 个变异，验证包 `thinws-metadata-sqlite`，输出 `target/p1-09-final-receipt-mutants/mutants.out`。同机此前 35 个约 55 秒，考虑测试与构建波动，首次主动查看预计 00:25 UTC；此前不轮询、不修改冻结代码。

该批实际于 00:22:31–00:23:24 UTC 完成，约 53 秒：18 caught、1 unviable、21 missed、0 timeout，退出码 2，**不能视为通过**。`outcomes.json`/`mutants.json` SHA-256 分别为 `40eccbefe9e5865da361fa3acb5d64a3a14c27365f30002bae9b88cff62b6996`、`0cff534ca0179d4d1fec74489d8faaab8f3243c08f41c776eb2953fc8d1225bf`。存活变异集中于一个长 `||` 守卫内部替换为 `&&`；当前测试同时触发多项拒绝条件，无法区分每一项是否有效。主 Agent 将按相互独立的不变量重构守卫，补单因素错误与 fallback 许可测试，再只补跑受影响函数；不会用“看似等价”掩盖缺失测试。

P1-09 final Receipt/Ready 守卫补跑启动记录（2026-09-25 21:42 UTC）：基线 `d21f9ce` 上已暂存代码/测试差异 SHA-256 `0fc1d0516d531bdbd608feec9ecf022131141b9216f5a0aef973c227a5c5be25`；主 Agent 单独执行，平台、工具、配置、并发和验证包同前批，按函数筛选缩至 6 个变异，输出 `target/p1-09-final-receipt-followup/mutants.out`。前批 40 个约 53 秒，按 6 个与构建波动估计，首次主动查看预计 21:44 UTC；此前不轮询、不修改冻结代码。

补跑实际于 21:42:17–21:42:37 UTC 完成，约 19 秒：5 caught、1 unviable、0 missed/timeout，退出码 0；`outcomes.json`/`mutants.json` SHA-256 分别为 `bad7cda5bbffa076ea80483a474c09eb9504601ceda8f145e04049213f0bfd1c`、`5fe02ce1bc118e952ec68e58ba9221df694f3c460daa2b7d016d16f733673873`。前批长守卫已由分组事实校验替换，补测了成功回执与另一个 Probe、失败回执、回滚、单独 Full Copy 许可和无许可 CoW 成功；受影响函数此候选无存活变异。本子结果尚不包含目录创建、CLI 或 P1-09 审核。

P1-09 受控目录子结果（进行中）：在既有 `BootstrapStore` 增加由 WorkspaceId 派生的新建容器、`.state/incomplete` 与空 `root/` 的描述符绑定证明，不新增 Port；data-root lifecycle lock 和已验证布局同时在场，已存在的容器不被接管。物化器会将 `root/` 权限改为来源权限，因此只有容器及 `.state` 固定要求 `0700`，`root/` 在后续重验时检查已持有目录身份、名称、类型、属主与卷而不强制其 mode 为 `0700`。错误 lock scope、替换 marker、替换 root 为外部符号链接均有真实文件系统回归；标记清除后保留普通 root 路径。此子结果不含 Application 创建编排。

P1-09 受控目录定向变异启动记录（2026-09-25 21:51 UTC）：基线 `a4d2db0` 上已暂存代码/测试差异 SHA-256 `bd05fedce854668eec890a5a0e327421e558c8ac23f2615578222d095723d154`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，筛选受影响的准备、清标记、锁与目录身份函数共 14 个变异，验证包 `thinws-adapter-macos`，输出 `target/p1-09-workspace-layout-mutants/mutants.out`。同机前次 9 个相关变异约 38 秒，留构建波动余量，首次主动查看预计 21:54 UTC；此前不轮询、不修改冻结代码。

该批实际于 21:51:24–21:51:54 UTC 完成，约 29 秒：13 caught、1 unviable、0 missed/timeout，退出码 0；`outcomes.json`/`mutants.json` SHA-256 分别为 `7c8dde8616c0845947808778bf8cf906964653e8a6321fd5d1406e9905375e26`、`2028cc899e7d5c5bf348d1da1118e9612f83b43c1161728438a59f8c463dfeff`。按计划首次主动查看前未轮询或修改冻结候选。仍需全仓库普通门禁与本地提交，此测试不能代替创建链路 E2E。

P1-09 幂等所需回执读取子结果（进行中）：对既有 `MetadataStore` 增加类型化 final receipt 摘要读取，返回请求/实际模式、Adapter、CoW、降级原因与失败尝试数量，不让 Application/CLI 直接解释 SQLite JSON。缺少 receipt 返回 None；未知版本、畸形或与登记卷/成功状态矛盾的 receipt 拒绝，不能猜测已建副本的实际物化事实。CoW 与 Full Copy 的持久读回及畸形数据拒绝已先 RED 后 GREEN；尚未接入公开创建幂等。

P1-09 final receipt 摘要读取定向变异启动记录（2026-09-25 22:02 UTC）：基线 `852ec54` 上已暂存代码/测试差异 SHA-256 `510146a9653d5beb74337d7d80dd35a32f83c76099354b7149f2f150f39941f2`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，只选择读回与解码两函数共 18 个变异，验证包 `thinws-metadata-sqlite`，输出 `target/p1-09-receipt-read-mutants/mutants.out`。同机前次 6 个约 19 秒，考虑构建波动，首次主动查看预计 22:05 UTC；此前不轮询、不修改冻结代码。

该批实际于 22:02:34–22:02:54 UTC 完成，约 20 秒：15 caught、2 unviable、1 missed、0 timeout，退出码 2，不能视为通过。`outcomes.json`/`mutants.json` SHA-256 分别为 `db4be4a8d31565eb362ed97ae1e3a0c991c3285f1ccb02d3f170dc4568344952`、`8bead0b8186cffb8b7234e1f9f58f62f072922813e49d99445daf3d967bbf093`。唯一存活项把 runtime fallback 的失败尝试数量 `==1` 改成 `!=1`；原测试只覆盖了 clone 和预检降级，runtime 输入同时被其他不变量拒绝。将补有效 runtime 回执读取与零次尝试单因素拒绝，再定向补跑。

P1-09 runtime 摘要单变异补跑启动记录（2026-09-25 22:07 UTC）：基线 `852ec54` 上已暂存代码/测试差异 SHA-256 `6b9eae542e1c764082a3e2f2124b246d5caff555a67d06394d0f4d136608b29a`；主 Agent 单独执行，平台/工具/配置同前批，并发 1，按 `receipt_json.rs:48:81` 精确筛选唯一受新增样例影响的变异，输出 `target/p1-09-receipt-read-followup/mutants.out`。前批 18 个约 20 秒，单项含构建估计 10 秒，首次主动查看预计 22:09 UTC；此前不轮询、不修改冻结代码。

补跑实际于 22:07:08–22:07:15 UTC 完成，约 7 秒：唯一变异 caught，0 missed/timeout，退出码 0；`outcomes.json`/`mutants.json` SHA-256 分别为 `51badcceb7da3853027e27563e968ec5180ed66ef5226607c67b8ffe537c6832`、`666ad0a818a514540c55d92f0c9e48e642b2ecf062a8b298bb3f1122c701d5d2`。前批其他 15 项已捕获，2 项不可编译，候选该读取范围无存活变异；尚需全仓库普通门禁和本地提交。

P1-09 Ready 目录核验子结果：同名幂等在已持有 data-root lifecycle lock 时，除数据库 Ready 与 final receipt 外，还由既有 `BootstrapStore` 验证受控 `workspaces/<id>` 私有容器、无 incomplete 标记、普通 root 的身份/属主/卷与路径。P1-10 路径查询复用同一只读归属核验，但不取得 lifecycle lock，前后重读状态以拒绝可见的并发变化。`root/` 不强制 `0700`，因为已经继承来源目录权限；符号链接替换和残留标记由真实文件系统测试拒绝。该核验不自动修复缺损目标，也不取代 P1-10 status 的 Git 检查。

P1-09 Ready 目录核验定向变异启动记录（2026-09-25 22:12 UTC）：基线 `de78cf7` 上已暂存代码/测试差异 SHA-256 `a6628e5a32b578117a745b25ba61cbc08d57162cb64abfb83c63bb3b2619ba70`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，筛选 `validate_ready_workspace` 与 `open_owned_child_directory` 共 4 个变异，验证包 `thinws-adapter-macos`，输出 `target/p1-09-ready-path-mutants/mutants.out`。同机前次 14 个约 29 秒，预计 22:14 UTC 首次主动查看；此前不轮询、不修改冻结代码。

该批实际于 22:12:44–22:13:00 UTC 完成，约 16 秒：2 caught、2 unviable、0 missed/timeout，退出码 0；`outcomes.json`/`mutants.json` SHA-256 分别为 `f3ae078a3d10e8a0ec98ac9a3aed59eee52aa63ecb68d6b61fddd68ea6e63f9d`、`6692aa98330b30d8802c205743f23c8551fbefedc3aef62f25e9dded604e2a2e`。按计划首次主动查看前未轮询或修改候选；本子结果仍需全仓库普通门禁和本地提交。

P1-09 Application 创建编排候选（进行中）：现有 Port 完成实例核验、同名 Ready 幂等、源卷和包含关系预检、Creating 预留、受控 incomplete 容器、四路径 Probe/Plan、CoW 或受限 Full Copy 执行、清除 incomplete、最终 Receipt 与 Ready 持久化。真实 APFS 的非 Git 来源及源移走后的幂等返回、同名参数冲突、特殊文件失败后的未完成状态、跨卷及包含关系的预留前拒绝已 RED/GREEN；没有 Git 操作或命令执行包装。fmt、全目标/全 feature Clippy 和全 workspace 普通测试通过。本候选尚未加入公开 CLI create/dry-run、完整降级 E2E 或 P1-09 终审，不宣称任务完成。

P1-09 Application 定向变异启动记录（2026-09-25 22:24 UTC）：基线 `7084f6f` 上已暂存代码/测试差异 SHA-256 `d6c74d0a65bc3a8b00d2db14205aea0c4b2acf8e68f15ff0557ca6d9cc0949af`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，筛选 `ThinWorkspaceService::create/create_reserved`、路径包含与计划错误映射共 38 个变异，验证包 `thinws-application`，输出 `target/p1-09-application-mutants/mutants.out`。上一批同机 40 个约 53 秒；考虑 Application 集成测试较慢，预计 22:27 UTC 首次主动查看，在此前不轮询或修改冻结候选。

该批实际约 47 秒完成：24 caught、3 unviable、11 missed、0 timeout，退出码 2；`outcomes.json`/`mutants.json` SHA-256 分别为 `f0a79e94970c1db5d7d4df102e8eeea2e9d23097f00606c3a80930cfbec716c4`、`1d5890d553ec781216d87d25526453e7eeefd9e0e53fb89a9b11bfbd0e440e8b`。不可作为通过证据。存活项主要集中在单独错误后端、Ready 提交后错误处理、降级选择、卷双重校验、运行时 CoW 失败条件、路径包含边界与错误映射；先简化提交后返回路径并补单因素测试，再只对未处置的变异及修正影响范围补跑。

P1-09 Application 定向补跑启动记录（2026-09-25 22:31 UTC）：基线 `7084f6f` 上修正后的暂存代码/测试差异 SHA-256 `d90c9fdd5d974c3f6a177f3a0ff6d51088b02c97cb97454e1d1dd616ed963114`；主 Agent 单独执行，平台、工具、配置、验证包及并发 4 同上批，按当前 `create.rs` 的精确行号筛选前次未处置分支和新抽取的双卷不变量，输出 `target/p1-09-application-followup/mutants.out`。上批 38 个约 47 秒，本批预计不超过约 1 分钟，首次主动查看安排在 22:33 UTC；此前不轮询或修改冻结候选。

补跑实际约 32 秒完成：25 caught、0 missed/unviable/timeout，退出码 0；`outcomes.json`/`mutants.json` SHA-256 分别为 `12edd7c628ad5b15cbef0772222ed4e6245c319aa98c0f0ea061c3f16a7d7104`、`6b97a9d645aed93a967907d319f2493555b731bbcc678b9590ce5229410d4af7`。先前 11 个存活分支均被简化后的逻辑或对应单因素测试覆盖；真实 Application 测试另证明默认 CoW、同名幂等、运行时干净回滚才允许显式 Full Copy、非 CoW 失败不降级、跨卷和包含关系预留前拒绝。此结果只覆盖 Application 子结果，仍须公开 CLI、dry-run、任务终审及阶段门禁。

P1-09 只读预览与 CLI 候选（进行中）：Application `preview_create` 在已验证实例上只读探测现有 `workspaces/` 父目录及四条实际路径，不分配 ID、不取会创建文件的生命周期锁、不预留名称或持久化计划。CLI 的本机装配入口统一为 `LocalCommands`，实现公开 `workspace create [--allow-copy] [--dry-run]` 与人类/JSON 输出。真实 APFS 命令级 E2E 通过：预览无目录/锁文件写入、创建原样复制普通 `.git`/ignored/未跟踪文件和符号链接、副本写入与来源隔离、来源移走后的同名幂等、同名参数冲突与跨卷（含 `--allow-copy`）拒绝。fmt、全目标/全 feature Clippy 与全 workspace 普通测试通过；本候选尚须定向变异、供应链/发布相关门禁与独立终审，不能标记 P1-09 Done。

P1-09 预览/CLI 定向变异启动记录（2026-09-25 22:41 UTC）：基线 `3731cb9` 上已暂存代码/测试/用户手册差异 SHA-256 `90d2e0b34d0a53bc1eeaf093f1b7e2c3d403ed4c29d597b1195936f574a20226`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，精确筛选 `preview_create`、共享 `select_plan` 和 CLI `run`、人类/JSON renderer 共 11 个变异，验证包为 `thinws-application` 与 `thinws-cli`，结果放在 `target/p1-09-preview-cli-mutants/mutants.out`。前批同机 25 个约 32 秒，本批含两套测试包和重构后的 CLI，保守预计 22:43 UTC 首次主动查看；此前不轮询或改动冻结候选。

该批实际约 40 秒完成：9 caught、2 unviable、0 missed/timeout，退出码 0；`outcomes.json`/`mutants.json` SHA-256 分别为 `d20809d57dce68e60002d0ea68f9b9c9454667a38994d6604705428fd8042fc8`、`03d1a08e924c674cda697b63430587e797ef9b858d2b4241f7b0636c82388982`。候选通过的仅是预览/CLI 受影响函数；P1-09 验收断言中的并发名称、锁超时、SQLite 提交失败和适用 fuzz 等仍需单独证据，不能据此完成任务。

P1-09 并发与最终提交故障验证（进行中）：真实并发创建暴露首次同时打开 data-root lifecycle lock 时，macOS/APFS 的 `openat(O_CREAT)` 偶发 `ENOENT`；Adapter 层精确 RED 后只对该 errno、父目录身份不变且调用时限未满的情况有界重试，不改变其他 I/O 错误映射。独立 Adapter 首次并发锁测试已 GREEN，Application 双创建同名仅一份新物化、另一份幂等，二者各重复 20 轮无失败；锁超时发生在名称预留前。注入 final SQLite commit 失败时，即使克隆已完成也只保留非 Ready 行并拒绝同名自动重试。相关断言均为当前未提交候选，不能代替独立审核。

P1-09 首次并发锁精确变异启动记录（2026-09-25 23:54 UTC）：基线 `3731cb9` 上当前已暂存代码/测试/用户手册差异 SHA-256 `84e9b28e3ec56a39bf3019df8345b7c909c92b0cf9feed99f6887d3778458126`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，仅筛选 `lock.rs:104` 有界 `ENOENT` 重试守卫的 5 个变异，验证包 `thinws-adapter-macos` 和 `thinws-application`，结果 `target/p1-09-first-lock-mutants/mutants.out`。前次 11 个跨双包约 40 秒，本批有 12 轮同测并发，预计 23:57 UTC 首次主动查看；此前不轮询或修改冻结候选。

该批实际约 41 秒完成：3 caught、2 missed、0 timeout，退出码 2；`outcomes.json`/`mutants.json` SHA-256 分别为 `6d3e06a9256e294e861911fb9eba4e755843c522f7af451dcc9b9b9605984bbd`、`2e4999162f594cbab024ed8f62d55c9ecee8cd94699a6c21de66f4fafc85db05`。存活项分别把时限判断替换为恒真及 `<` 变为 `<=`；并发测试证实正常竞态恢复，却无法构造持久 `ENOENT` 或精确时限等值。须补纯时限边界测试并定向重跑，不把本批视为通过。

P1-09 首次并发锁时限补跑启动记录（2026-09-25 23:59 UTC）：基线 `3731cb9` 上新的已暂存差异 SHA-256 `3ebf132fd0a8efadc44ff601b76e39c934d4ea5d5d814dfadc7111d44a358ace`；主 Agent 单独执行，平台/工具/配置及双验证包同前批，并发 4，只选取 `lock.rs:105` 调用点和 `retry_open_before_deadline` 纯边界的 6 个变异，结果 `target/p1-09-first-lock-followup/mutants.out`。前批 5 个约 41 秒，预计 2026-09-26 00:02 UTC 首次主动查看；此前不轮询或修改冻结候选。

该批实际约 33 秒完成：5 caught、1 missed、0 timeout，退出码 2；`outcomes.json`/`mutants.json` SHA-256 分别为 `8a49493339d2e5b69f59026de9fa9734f55f33d9bd46c313782a003342a1e650`、`000b9d3689cfc2f44442994b033918dc1b970b8e994ad6d4cdea4450f5b0375c`。唯一存活项删除调用点的 `!`，使有界条件反转；并发首用随机时序在变异运行中未稳定命中这一窗口。改用更直接的“时限内明确继续，否则返回原错误”分支表达后补测其影响范围，不以第一批或本批宣称全部通过。

P1-09 首次并发锁最终定向补跑启动记录（2026-09-26 00:03 UTC）：基线 `3731cb9` 上当前已暂存差异 SHA-256 `b5aba3292b791250c1bb8d47028a8a5400a8b467342545cd7eecdb5848bf32f8`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，只选择 `retry_open_before_deadline` 5 个剩余变异，验证包 `thinws-adapter-macos`、`thinws-application`，结果 `target/p1-09-first-lock-deadline/mutants.out`。前批 6 个约 33 秒，预计 00:05 UTC 首次主动查看，期间不轮询或修改冻结候选。

最终补跑实际约 32 秒完成：5 caught、0 missed/unviable/timeout，退出码 0；`outcomes.json`/`mutants.json` SHA-256 分别为 `b7d2115968016877307e8ab905d5e16f81d23507c13a48e703139a6db2dd07ba`、`731af325d628369c76f2a0ab87e04422ee4a773db14dfb4eff3c2b5996ef8df3`。前两批未捕获的时限语义经单因素边界测试和正向分支重写后已覆盖；并发首用与 Application 双创建各 20 轮真实 APFS 重复执行无失败。该证据只针对锁变更，不代替 P1-09 全部交付门禁。

P1-09 候选补充验证（2026-09-26 UTC，macOS arm64、Rust 1.97.1）：新建 `thinws_create_request` 纯输入 fuzz target，使用 `nightly-2026-08-14` 和 libFuzzer 运行 60 秒，实际 61 秒、15,134,260 次执行、退出码 0，无 crash/hang；生成语料移出仓库暂存目录，未把随机语料作为回归样本提交。该 target 只覆盖 CreateRequest 输入边界，不声称覆盖 APFS/SQLite/锁时序。Debug/Release 全 workspace 测试、fmt、全目标/全 feature Clippy、rustdoc、30 项工具测试、crate 生产依赖方向检查、`cargo deny check`、离线 `cargo audit --no-fetch` 和 `git diff --check` 均退出 0；deny 只有既有的未命中 allowance/exception 警告，audit 使用本地 1261 条 advisory。新发现依赖检查器把 dev-dependencies 当生产依赖、遗漏已使用的 `serde_json`，已加分类回归并修正；CLI 创建结果枚举改由 Application 公共 API 暴露，避免 CLI 直接依赖 Core。真实跨卷 P1 Probe/Store 用例以 `/private/tmp` ↔ `/Volumes/data` 通过；仓库已在 `/Volumes/data`，P0 两项显式跨卷夹具改用 `/private/tmp` 后通过（首次误将 `/Volumes/data` 当异卷，仅触发夹具前置断言）。真实同卷 APFS Clone/Full Copy、CLI `.git` 原样内容、写隔离、幂等、故障注入和并发见上述测试。未执行真实子挂载创建、线上 CI/PR/push；本候选仍待精确提交后的 GPT-6 Astra `xhigh` 只读独立审核，不提前标记 Done。

P1-09 精确候选 `d0206b0c5a2ba19f0845f0df34c3285b702e701a` 经 GPT-6 Astra（`gpt-6-astra` / `xhigh`）只读独立审核，结论 Changes requested：Critical 0、High 1、Normal 3、Low 0。High 为大小写不敏感 APFS 路径别名绕过 source/data-root 包含保护；Normal 分别为终止物化失败丢弃 partial receipt、注册 data root 缺失时 create 错报 E_FILESYSTEM、缺失 source 错报 E_COW_UNAVAILABLE。任务退回 In Progress。主 Agent 已逐项先写真实 APFS/Application RED 断言，四项均因预期原因失败；当前修正用既有 Probe 目录身份链拒绝别名，物化失败在 Application 错误中保留每次 partial receipt 和安全摘要，注册根锁前只读校验并持锁后重验，缺失 source 在预留名称前分类为 E_FILESYSTEM。真实 APFS/Application/CLI 定向回归已转绿；完整门禁、受影响变异和新精确候选独立复核尚未完成。

P1-09 审核修正定向变异启动记录（2026-09-26 00:30 UTC）：相对 `d0206b0` 的 Application 生产代码与相关测试差异 SHA-256 `95bf9a2517deecf3b03164616badb9b8fe8224bb676387c4ac5d1dcf46b79b70`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，精确筛选 Application `create/preview_create/create_reserved`、目录身份重叠及失败回执保留函数的 35 个变异，验证包为 `thinws-application` 与 `thinws-cli`，输出为 `target/p1-09-review-fix-mutants/mutants.out`。上一批同机 38 个约 47 秒；本批真实 APFS 与 CLI 测试更多，预计 00:33 UTC 首次主动查看，此前不轮询或修改冻结生产候选。

该批实际约 72 秒完成：26 caught、7 unviable、2 missed、0 timeout，退出码 2；`outcomes.json`/`mutants.json` SHA-256 分别为 `ccda0f2b9dde793207d43b08bab16cee01d9e28d1a35a8ec44eceb593fff49ad`、`6c0aad04768b660c0e94eaedd33f8e5e1a51a2b5fd9c525b2c1a0229b04d3553`，不能作通过证据。两个存活项是仅在锁打开失败竞态触发的 `NotFound` 条件反转，以及身份重叠函数对“来源目录已存在”的重复判断。主 Agent 将锁失败统一重验布局以分类真实根状态；将身份函数收缩到调用方已校验的现存来源，并新增来源为 data root 祖先的大小写别名真实 APFS 断言，两个方向均须预留前拒绝。相关定向测试与 Clippy 已通过。

P1-09 身份重叠最终定向补跑启动记录（2026-09-26 00:36 UTC）：相对 `d0206b0` 的当前代码与相关测试差异 SHA-256 `338b6aef5e1a3ae8c62ed18b2344dbc46b04aea3d7e66c4a44a9eb7f3b1f447c`；平台/工具/配置同上，仅筛选更改后的 `paths_overlap_with_identity` 5 个变异，验证包为 `thinws-application`，输出为 `target/p1-09-identity-followup/mutants.out`。同机前批 35 个约 72 秒，本批预计不超过约 1 分钟，首次主动查看 00:38 UTC；此前不轮询或改动冻结生产候选。锁失败条件已改为无条件布局重验，原存活变异位置消失；前批其余 33 个结果按未变更语义复用。

补跑实际约 28 秒完成：5 caught、0 missed/unviable/timeout，退出码 0；`outcomes.json`/`mutants.json` SHA-256 分别为 `eb64611dcfd22037be603694f5965ed585a6f3fb5f4527db604114d4c6273c76`、`421f6dbb0251719288867fa1f0ecd1639569e1b6326d589440a04c6d1efc1a83`。前批存活的目录身份分支已由双向真实别名测试覆盖；锁失败现统一复查布局，不再存在只对 `NotFound` 重验的条件分支。受影响生产范围没有未处置存活变异，仍须对最终候选执行普通门禁和独立复核。

审核修正最终本地普通门禁（2026-09-26 UTC，macOS arm64、Rust 1.97.1）：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`THINWS_P1_CROSS_VOLUME_ROOT=/Volumes/data cargo test --workspace --all-targets`、相同环境的 `--release --all-targets`、`cargo doc --workspace --no-deps`、30 项工具测试、crate 生产依赖方向检查、`cargo deny check`、`cargo audit --no-fetch` 和 `git diff --check` 均退出 0。真实大小写别名从两个包含方向在名称预留前拒绝，缺失来源与注册根的 CLI 错误码回归通过。`cargo deny` 保留既有未命中 allowance/exception 警告；audit 使用本地 1261 条 advisory。`/Volumes/data` 与系统卷的 P0/P1 显式跨卷用例来自前一冻结候选且 Probe/Adapter 代码未变，本次全 workspace 仍运行 CLI/Application 真实跨卷用例；本次未重复运行 P0 显式 ignored 用例或输入解析 fuzz（本次未修改对应代码）。真实子挂载创建和线上 CI/PR/push 仍未执行。任务待修正精确提交和独立复核，不能以本地绿灯提前标 Done。

P1-09 第二次精确候选 `9bb7486a8c716d6772dbb7e609d384c5d05c8c15` 经 GPT-6 Astra（`gpt-6-astra` / `xhigh`）只读独立审核，结论 Changes requested：Critical 0、High 0、Normal 1、Low 0。Normal 为物化失败且 `record_failure` 自身失败的复合故障：Application 保留 partial receipt，却在改写成 E_METADATA 时遗漏给 CLI 的残留摘要。主 Agent 补充复合故障注入断言，先确认因缺少摘要而 RED，再通过既有 `with_partial_receipt` 在元数据错误上重建摘要，GREEN。审核者还指出来源目录不可访问的相邻分类风险；真实 APFS `0400` 来源目录预检先 RED 为 E_DATA_ROOT_UNAVAILABLE，已将来源探测与注册根布局区分并在预留前检查可读/可搜索性，默认和 `--allow-copy` 的 preview/create 均 GREEN 为 E_FILESYSTEM。未新增领域机制或公开参数。本修正仍须完整门禁、受影响变异与新精确候选独立复核，P1-09 维持 In Progress。

P1-09 第二次审核修正定向变异启动记录（2026-09-26 00:48 UTC）：相对 `9bb7486` 的 Application 生产代码与相关测试差异 SHA-256 `47ca58ad81c4d304a66d4256f4d8d2dd19b60c58319ac2ae5abd3d7838ff9dac`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，只选择新增来源错误映射、`require_existing_source`、`preview_create`/`create` 入口与失败记录守卫的 7 个变异，验证包为 `thinws-application`、`thinws-cli`，输出为 `target/p1-09-double-failure-mutants/mutants.out`。前批 5 个约 28 秒，本批双包 7 个预计 1 分钟内完成，首次主动查看安排在 00:50 UTC；此前不轮询或修改冻结生产候选。

该批实际约 40 秒完成：4 caught、3 unviable、0 missed/timeout，退出码 0；`outcomes.json`/`mutants.json` SHA-256 分别为 `dfce2d54af1d7674797fdd16e204192fda23727709dfccdb0163362896627f78`、`198c3058b69a32ecd3911a2e1a9fea4388e99ef3126b27e15910518313070eb6`。两个来源错误映射守卫和 `require_existing_source` 均被测试捕获；三个 unviable 为整函数默认返回或复合 `if let` 守卫编译不可行，不是存活变异。真实 APFS `0400`（无搜索）及 `0100`（无读取）来源的 preview/create、默认/允许复制四组合均返回 E_FILESYSTEM 且无 Workspace 预留。修正后 `cargo fmt --all -- --check`、全目标/全 feature Clippy、带 `/Volumes/data` 的 Debug/Release 全 workspace 测试、rustdoc、30 项工具测试、生产 crate 依赖方向检查、`cargo deny check`、离线 `cargo audit --no-fetch` 和 `git diff --check` 均退出 0；deny 只有既有未命中 allowance/exception 警告，audit 使用本地 1261 条 advisory。输入 fuzz、P0 显式 ignored 跨卷用例因相应输入/Probe/Adapter 实现未变而沿用前候选证据，未在本次重复；线上 CI/PR/push 与真实子挂载创建未执行。P1-09 仍待新精确 commit 的独立复核，不能提前标 Done。

P1-09 第三次精确候选 `2f298de90ed6e2756ed0ff6b26c8ac730d1c7cb1` 经 GPT-6 Astra（`gpt-6-astra` / `xhigh`）只读独立审核，结论 Changes requested：Critical 0、High 0、Normal 1、Low 0。来源在首次单路径探测后失去权限时，组合 Probe 的无路径角色 `Unavailable` 仍以布局阶段映射为 E_DATA_ROOT_UNAVAILABLE，错误对象和公开错误码不符；不会误报 Ready。主 Agent 用真实 APFS 顺序夹具确认：`0100`（可搜索但不可读）仍返回带 `source_read_unsupported` 的报告并由 Application 拒绝；`0400`（可读但不可搜索）使第二次打开 source 返回 `Unavailable`。因此在既有 PortError 增加不含路径字节的组合 Probe 失败角色，由 macOS Adapter 逐个角色标记，Application 仅将 source 的 Unavailable 映射为 E_FILESYSTEM；目标/staging/trash 仍按数据根失败处理。Port 契约、两次探测间 `0400` 变化和 Application 分类测试先因缺少角色契约 RED，再转 GREEN；不增加新 Port 或产品机制。本候选待完整门禁、定向变异和独立复核。

P1-09 组合 Probe 错误角色定向变异启动记录（2026-09-26 00:59 UTC）：相对 `2f298de` 的 Ports/Adapter/Application 生产代码及相关测试差异 SHA-256 `07c32c657776639de20bdef8483409a7c050034301d8bec27b20ff3e00d0c957`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、仓库配置、并发 4、gitignore 开启，仅选择 PortError 角色存取、Adapter 组合 Probe 入口、Application 新守卫共 8 个变异，验证包为 `thinws-ports`、`thinws-adapter-macos`、`thinws-application`、`thinws-cli`，输出 `target/p1-09-probe-role-mutants/mutants.out`。上一批 7 个跨双包约 40 秒，本批四包预计 2 分钟内完成，首次主动查看 01:01 UTC；此前不轮询、不修改冻结生产候选。

该批实际约 52 秒完成：5 caught、3 unviable、0 missed/timeout，退出码 0；`outcomes.json`/`mutants.json` SHA-256 分别为 `d7917b5d018a85b465f3008f8ea95f061b3582fd4865a067df1b14ddcf2cf32d`、`b6037cbeddb7572c8a0bfe162aea1da6f6b56b52f01210734edfb9f436dc9135`。Port 角色读取和 Application 来源角色守卫均被测试捕获，三个 unviable 为无法构造默认返回值的整函数替换；Adapter 四角色赋值不生成独立变异，由真实 APFS source 权限变化、Port 契约和分类断言覆盖。当前候选 `cargo fmt --all -- --check`、全目标/全 feature Clippy、带 `/Volumes/data` 的 Debug/Release 全 workspace 测试、rustdoc、30 项工具测试、生产 crate 依赖方向检查、`cargo deny check`、离线 `cargo audit --no-fetch` 及 `git diff --check` 均退出 0；deny 只有既有未命中 allowance/exception 警告，audit 使用本地 1261 条 advisory。P0 显式 ignored 跨卷、输入 fuzz 因实现未变沿用前候选证据，线上 CI/PR/push 和真实子挂载创建未执行。任务仍待精确提交独立复核。

P1-09 完成记录（2026-09-26 UTC）：最终代码/设计候选 `ccc426c277c0251926cff163087b39c698790781` 经 GPT-6 Astra（`gpt-6-astra` / `xhigh`）只读独立终审，结论 Approve；Critical、High、Normal、Low 均为 0。相对目标分支 `main` 的任务提交为 `d21f9ce`、`a4d2db0`、`852ec54`、`de78cf7`、`7084f6f`、`3731cb9`、`d0206b0`、`9bb7486`、`2f298de`、`ccc426c`；提供原始目录的 CoW/显式 Full Copy 创建、只读预览、Creating/Ready 状态和最终回执、同名幂等及普通路径。真实 APFS 的原样 `.git`/未跟踪内容、写隔离、跨卷拒绝、并发名称、锁超时、路径别名、权限变化、失败回执、最终 SQLite 提交故障及无 Ready 误报均有上述可重复证据。终审对本轮 Port、Adapter、Application/CLI 执行短测和差异检查，全仓 Debug/Release 与定向变异沿用主 Agent 同一候选的前述结果；未重复长时门禁。真实子挂载创建、线上 CI/PR/push 未执行。任务级 Done 不表示 P1.c 小阶段收口或 Phase 1 人工放行；该小阶段完整受影响 crate 变异和阶段门禁另按《任务流程》§18 核对。

P1-09 跨卷环境补验（2026-09-26 UTC）：经用户确认 `/Volumes/data` 可作真实 APFS 测试卷，额外执行此前按设计 ignored 的 P1 Adapter data-root 跨卷重验、P0 Probe 跨卷候选判定、P0 Materializer 跨卷 `EXDEV`/显式 Full Copy 三项测试，均 exit 0。P1 用系统卷临时目录对 `/Volumes/data`；P0 夹具源固定在仓库所在 `/Volumes/data`，另一端须为设备不同的 `/private/tmp`。P0 Probe 首次误将 `/Volumes/data` 设为另一端时，仅夹具异卷前置断言失败，纠正为 `/private/tmp` 后通过；`stat -f '%d'` 显示设备号分别为 `16777240` 与 `16777229`。所有测试仅创建受控临时目录并按测试清理，不能由此宣称真实子挂载创建已覆盖。

### 4.4 P1.d 查询与路径交付

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-10 status/list/path | 状态边界、只读检查和普通路径输出；diff 由用户直接使用 Git | P1-09、P1-16 | R3 | Done |
| P1-16 GitInspector | 主/子仓库已跟踪变更、无仓库/不完整检查报告，不写 Git | P1-03、P0-07 | R3 | Done |
| P1-11 用户命令执行包装（已取消） | 不再实施；已有外部占用检查责任归入 P1-12，无替代执行服务 | 不再参与依赖 | —（历史任务） | Cancelled |

小阶段退出：查询不写状态，非 Ready 行为与用户契约一致；返回的普通路径可供现有工具直接使用，不要求执行包装或工具链环境注入。

P1-10 领取（2026-09-26 UTC）：主 Agent 在本地 `main` 和唯一 checkout 实施，基线 `e9c9353`；不派生开发 Agent、不创建线上 PR，冻结候选只读独立审核使用 GPT-6 Astra / `xhigh`。风险 R3（普通路径的 Ready/归属证明、只读 SQLite 快照、Git unknown 的用户可见表达和 JSON 契约）。验收断言：`list` 名称排序且不触发 Git；`path` 只对完整 Receipt 与当前受控目录均验证为 Ready 的记录输出原始绝对路径字节及末尾换行，`--json` 拒绝；`status` 对 Ready 按需调用 GitInspector，真实 clean/dirty/unknown、无仓库及嵌套仓库的结果与用户手册一致，非 Ready 只诊断不冒报可用；查询不改产品状态或 Git 元数据。公开 CLI 的成功、缺失/未就绪、参数错误、JSON 与退出码均用真实临时 APFS 夹具验证。当前空间用量测量属于 P1-13，本任务不得把创建时 Receipt 的字节数冒充当前用量；P1-13 完成前如需呈现该字段，只能明确为 unknown。范围外是删除/强制日志、GC、自动提交/分支/PR 和语言工具链包装。

P1-10 初次候选 `39512f8` 自测（2026-09-26 UTC，macOS/APFS）：公开 `list/path/status` 真实 E2E 先因 E_USAGE 失败，再实现；覆盖空列表、排序、普通路径、无仓库、主/嵌套仓库 clean/dirty/unknown、未跟踪不计数、缺失/非 Ready、Ready 根符号链接替换拒绝及 SQLite 回执损坏拒绝。fmt、Clippy、全 workspace/all-targets、rustdoc、crate 依赖检查、tools 31 项、deny 与离线 audit 均通过；两条 P0 ignored 异卷实验另以 `/private/tmp` 显式通过，先前误用同卷 `/Volumes/data` 得到的环境断言失败不算产品失败。初次候选四组定向变异 43 个独特变异，补回归后 25 caught、18 unviable、0 missed，原始结果留于 `target/mutants-p1-10-{query-final,cli,cli-fix,snapshot,sqlite,sqlite-fix}/mutants.out`。独立审核发现查询锁范围和缺失 data root 的错误分类问题，该候选不用于任务放行；后续无锁修正必须重跑受影响范围。现有 fuzz harness 只覆盖未修改的名称/Git 输入解析器，故本任务无受影响 fuzz target；P1.d 小阶段门禁、线上 CI/PR/push 未执行。含换行路径的原始 stdout 不按物理行数保证。

P1-10 审核修订候选（2026-09-26 UTC，保留当时口径）：保留 `lifecycle.lock` 供创建/清理及当时仍在计划中的 GC，`path/status` 改为不取锁的只读双快照＋受控目录重验，ADR-0004、详细设计与手册已同步；非 Ready 不受锁文件异常阻塞，Ready 在核验期间转 Error 不输出可用路径。GC 后已由 ADR-0005 从 Phase 1 取消，此历史候选表述不构成当前 GC 交付承诺。新增真实 APFS 的 data root 消失与 metadata 目录缺失回归，先分别得到错误的 31/31，修订后为手册规定的 32/33；锁文件符号链接时 Ready/非 Ready 查询仍可读。`cargo fmt --all -- --check`、全目标 Clippy、以 `/Volumes/data` 为 P1 异卷端的全 workspace/all-targets 测试退出 0；真实 P0 异卷端仍是 `/private/tmp`。修订查询的 22 个定向变异为 10 caught＋12 unviable，Ready 核验方法 2 个为 1 caught＋1 unviable，均无 missed，证据在 `target/mutants-p1-10-{query-nolock,ready-validator-nolock}/mutants.out`；未改变的 CLI 渲染、SQLite/Port 回执读取沿用初次候选的定向结果，不冒称整个 P1.d 门禁。并发用户工具仍可在查询返回后改变目录；平台不维护跨进程历史 root inode，本任务只保证查询过程中可检测的归属与状态变化被拒绝。

P1-10 第二轮审核纠偏：GPT-6 Astra / `xhigh` 对 `276360c` 提出 Changes requested（P2）：status 在 Git 检查开始前已完成两次快照，Git 检查期间若 Ready 转 Error 仍会输出旧路径。新增可控 GitInspector 回归先确证 `left: Ready, right: Error` 的行为 RED；修订后 Git 检查结束再执行只读状态与归属核验，转为非 Ready 时只返回非 Ready 诊断。第一次定向变异揭示 `current != workspace || current_path != path` 中后半条件在既有“路径必须等于同一记录 target”不变量下冗余，`||→&&` 存活；删除冗余判断而非增加不可达测试，精确定向复测 2 个变异为 1 caught＋1 unviable、0 missed，见 `target/mutants-p1-10-status-post-git-final/mutants.out`。该次审核此前指出的 ADR、错误分类和非 Ready 锁阻塞问题已关闭；历史普通 root inode 替换属当前已声明边界，P1-12 破坏性清理另行核对。

P1-10 任务级完成（2026-09-26 UTC）：最终实现提交 `0f341ebba5abf8040f0d6ae076d5762672a8840b` 经 GPT-6 Astra / `xhigh` 只读独立复核，结论 Approve，未发现新可操作问题；审核者独立重跑 Application 8、CLI 契约 9、真实查询 E2E 10 项均通过，核对最后一批 1 caught＋1 unviable、0 missed/timeout，工作树干净。主 Agent 对同一修订候选的 fmt、全目标 Clippy、`THINWS_P1_CROSS_VOLUME_ROOT=/Volumes/data THINWS_P0_CROSS_VOLUME_ROOT=/private/tmp cargo test --workspace --all-targets -q` 均取得退出 0。该结论只支持 P1-10 Done；P1.d 全受影响 crate 变异、统一短预算 fuzz、阶段放行、线上 CI/PR/push 均未执行，不据此宣称 P1.d 或 Phase 1 放行。普通 root 历史 inode 替换不在本任务获得持久证明，P1-12 删除前必须单独核对安全边界。

P1-16 于 2026-09-26 UTC 由主 Agent 领取，基线 `c4a9fbc432012ff3e12d59c207bad06fea0ba94a`；在唯一 checkout `/Volumes/data/code/worktree` 的本地任务分支 `codex/p1-16-git-inspector` 实施，完成后快进合并 `main` 并删除已合并分支。基线 `cargo fmt --all -- --check`、`cargo test --workspace --all-targets` 及受影响产品 crate 的真实 APFS 全目标测试通过。风险 R3（Git 子进程、配置与外部引用），主要写入区为 Core 的纯状态解析、P1-16 才引入的 GitInspector Port、新 `thinws-adapter-git-cli` 及其测试；不派生开发 Agent，最终只读独立审核指定 GPT-6 Astra / `xhigh`。输入为已验证副本的普通目录路径，输出为发现完整性、根与嵌套仓库的相对位置、逐仓库已跟踪变更计数/unknown 原因及 aggregate；不持久化、不更改 Workspace Ready，也不参与创建。P0-07 的已验证解析、子进程和仓库预检代码可作为迁移输入，但生产实现不得依赖 `experiments/p0/`，不得同时保留两套长期分叉的同义实现。范围外是 P1-10 的 status/list/path 公开命令、P1-12 清理策略与日志、自动 commit/branch/PR、Git 修复、联网和任意命令执行包装。

P1-16 验收断言：无本地 Git 标记且扫描完整为 `not-applicable`，不是 clean；根仓库、嵌套仓库及 submodule 的当前 HEAD/index 已跟踪新增、修改、删除、重命名、模式、冲突和 gitlink 变化分别可见，未跟踪/ignored 及子仓库仅未跟踪内容不造成 dirty。外部 gitfile/commondir/worktree、损坏元数据、隐藏 index flags、sparse、partial-clone/promisor、不安全属性/配置、Git 缺失、非零退出、超时、超限或路径身份变化保守 unknown，并保留发现是否完整；不向父目录上溯、不跟随目录链接、无网络/配置/index/ref 写入或外部程序执行。失败或 kill 不写产品状态；真实 Git 临时仓库、子仓库、环境与配置故障注入、程序边界、受影响解析 fuzz 和定向变异须有可重复证据。P1-10/12 接入时再按用户手册补公开命令/清理 E2E，不能把 Port 成功冒充已交付这些能力。

P1-16 实现结果：将 P0-07 已实测的 porcelain-v2 计数与 `complete/incomplete → not-applicable/clean/dirty/unknown` 聚合迁入 Core，将原有有界 Git 子进程、no-follow 发现、配置与元数据预检整体迁入 `thinws-adapter-git-cli` 的私有模块；P0 实验只对 Core 纯解析保留重导出及自身清理策略/日志实验，产品不依赖实验 crate，内部 Git 查询不作为产品 API 公开。新增 `GitInspector` Port、完整/逐仓库报告与结构化查询失败证据；曾尝试以笼统 `QueryFailed` 映射 Port，发现会吞掉退出码/超时/回收证据，已改为单一 Port 证据类型并由 Adapter 原样返回。Core 和 Port 测试最初因缺少 API 无法编译，这只是搭建接口的编译检查，不记作行为 RED；迁移后的 Adapter 89 项单元测试中 88 通过、1 项受控子进程入口按设计 ignored，P0 状态策略 16 项及移除流程 11 项继续通过。全 workspace `fmt`、Clippy、普通测试、rustdoc 与 crate 依赖检查通过；普通测试的真实跨卷环境为夹具位于系统 Data 卷、另一端 `/Volumes/data` APFS 卷，首次误将 `/private/tmp` 指为另一卷导致仅跨卷环境断言失败，纠正后全绿。固定 nightly 的 `thinws_git_status` 纯内存 fuzz 60 秒运行 588,927 次、exit 0、无 crash/hang；内部模块收窄后通过专用 fuzz-only 重导出又运行 15 秒、182,605 次、exit 0、无 crash/hang；新增语料分别保留于 `target/p1-16-git-status-fuzz-corpus*`，不批量提交。定向变异与供应链结果见下文；没有公开 status/remove 交互，doctor 的 `git_check.available` 仍为 false 直至公开查询接入，不据此宣称 P1.c 或 Phase 1 放行。

P1-16 定向验证补记（2026-09-26 UTC）：新增独立进程、净化 Git 环境的真实 Port 集成测试，先确证 clean 与仅未跟踪仍 clean，再确证一项 tracked 修改返回 dirty 和准确计数；新增 Port 契约测试检查超时、直接子进程未确认退出及回收错误不在类型边界丢失。Core `git.rs` 仅选 100 个受影响变异，`thinws-core` 与 `thinws-p0-cleanup` 两包共同验证，四并发约 4 分钟，结果 97 caught＋3 unviable、0 missed/timeout、exit 0；`outcomes.json` SHA-256 `9269fd35ef590406cb68ac1c816d9ce69e5e011d7bcda6cb111bae37e50e9e29`，`mutants.json` SHA-256 `999d674642f0c0922322e91f22556563d4881c724f9198bc37478e9dfb0eea6c`，证据留于 `target/mutants-p1-16-core/mutants.out`。新 Port/Adapter 入口 14 个变异按历史 100 个约 4 分钟的实际耗时估计小于约 2 分钟，本次实耗见下文。新 crate 的许可例外、生产依赖白名单和 fuzz lockfile 已同步；主/模糊 workspace 的 `cargo deny check` 与 `cargo audit --no-fetch` 均退出 0（deny 仅既有未命中提示）。

P1-16 Port/Adapter 定向结果：`crates/thinws-ports/src/git.rs` 与 `crates/thinws-adapter-git-cli/src/lib.rs` 的 14 个变异，以 `thinws-ports`、`thinws-adapter-git-cli` 两包验证，82 秒完成 7 caught＋7 unviable、0 missed/timeout、exit 0；`outcomes.json` SHA-256 `439b0ac977fb1a7d3553d6a50a29a37897e45d464786baaab94ddad7068a6f9f`，`mutants.json` SHA-256 `fe1e7de2c2662879d8bb13aed72aed658817470eb77936b4aa602612398ef6d5`，证据留于 `target/mutants-p1-16-port-adapter/mutants.out`。旧 P0 Inspector 生产主体按源码差异比对仅改变数据类型来源、内部可见性、只在测试存在的 Version/run、夹具保留位置及 `finish_inspection` 调用 Port 构造器；该新调用点另选 1 个变异，25 秒完成 1 unviable、0 missed/timeout、exit 0，原因是替换为 `Default::default()` 时 `GitInspection` 没有 `Default` 实现，不是遗漏测试；`outcomes.json`/`mutants.json` SHA-256 分别为 `834e64909014881599226655bbba16fb28923f36272367a74d65a707d1935509`、`d7de1eb63f027077d1301e088ac50c94045b6942759081ea11e0cdc7dd7ab580`，证据留于 `target/mutants-p1-16-finish-inspection/mutants.out`。旧 P0 全量证据不被冒称本次新运行。新增工具白名单测试后 `tools` 31 项通过；Git Adapter 的 Release 全目标测试 88 passed、1 按设计 ignored，Port 集成测试 4 passed。三批变异复制树时对历史 P0 保留 FIFO 夹具报告“Unexpected file type”警告，未当作被测源码变异或测试失败。

P1-16 独立审核纠偏（2026-09-26 UTC）：首轮 GPT-6 Astra / `xhigh` 对提交 `49f484f` 判定 Changes requested：仓库设置 `core.filemode=false` 时，原固定 status 可能把已跟踪文件的可执行位变化误报为 Clean。新增真实 Git/Port chmod 用例后，测试确因 `left: Clean, right: Dirty` 失败，属于有效行为 RED；固定 status 随后仅在该查询中使用 `-c core.filemode=true`，不写用户配置，使该用例返回 Dirty/1。相同候选在 `THINWS_P1_CROSS_VOLUME_ROOT=/Volumes/data` 和 `THINWS_P0_CROSS_VOLUME_ROOT=/Volumes/data` 下全 workspace/all-targets 普通测试再次通过；普通命令不执行 P0 的 ignored 跨卷用例，其有效异卷参数和单独执行结果见上文 P1-09 补验。首轮审核不用于放行修正候选；本次只改变固定查询参数和对应回归，旧三批变异结果的未受影响范围沿用，新增参数由精确 argv 单测和真实 chmod 测试验证；`cargo-mutants --list` 对该字符串参数不生成独立变异，不能假报新参数的变异覆盖。修正候选 `76e784b93de890ca09589631b8b4e3d97255452e` 经同一独立审核者复核，结论 Approve，Critical/High/Normal/Low 均为 0；审核者独立复测 Git Adapter 88 项单元与 4 项集成测试，1 项受控入口 ignored，未重复全量变异、fuzz 或阶段门禁。该结论仅支持 P1-16 任务级 Done。

### 4.5 P1.e 删除与空间统计

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-12 remove 与强制清理日志 | tracked-only 普通拒绝、显式 force、普通持久日志、ProcessProbe 和受控整目录清理 | P1-09、P1-16 | R4 | Done |
| P1-13 当前空间统计 | Ready 副本的当前逻辑字节和已分配字节估算；不将估算冒充可回收量 | P1-12 | R4 | Done |

小阶段退出：未跟踪文件不提示/不阻塞；tracked/unknown 可显式 force 且日志可读；不加交付硬门禁；路径/卷/占用保护不被绕过；Ready 当前空间统计如实区分完整估算与 unknown，首版不暴露 GC 命令或候选标记。

P1-13 领取（2026-09-26 UTC）：主 Agent 在唯一 checkout 的本地 `main` 实施，基线 `18d3b653bf84272242562e9236701b07c8d4dbe2`；不派生开发 Agent、不创建线上 PR，最终冻结候选由 GPT-6 Astra / `xhigh` 只读独立审核。风险 R4（GC 潜在递归删除及路径竞态），主要写入区为 Application、macOS Adapter、既有 Port 与 CLI 的空间/GC 能力及测试。开始前 `git status` 干净，`cargo fmt --all -- --check` 与 `cargo test --workspace --all-targets` 均退出 0。验收断言：status 报告当前普通副本的逻辑字节、可取得的物理分配估算或明确 unknown，不把创建 Receipt 当成当前用量；GC dry-run 不写入或预留计划，执行仅在交互确认或 `--yes` 后进行，并在 lifecycle lock 内重验元数据快照、卷、范围与候选身份；活跃 Workspace、日志、源目录、未完成清理隔离物和缺少明确可回收标记的 staging/trash 均不删除。真实 APFS、路径替换、非交互拒绝、JSON/退出码、受影响定向变异和适用 fuzz 需留证；本任务完成不自动放行 P1.e。失败可留下全局 staging/trash 残留；运行时 clone 失败后成功回退 Full Copy，也可能留下先前回滚隔离项。现有回执及相对名都不是明确可回收标记，尚无合格的 GC 候选生产者；先厘清该设计缺口，不以扫描名称或空 GC 输出冒充实际回收能力。

P1-13 空间统计切片（2026-09-26 UTC，macOS/APFS）：真实 CLI 用例先按预期在缺少 `space.state` 时 RED，随后对 Ready 副本按当前内容测量，非 Ready 与不能完整扫描的特殊条目输出 unknown；真实 APFS 用例涵盖修改后字节、外部符号链接不跟随、硬链接分配量去重及查询锁文件被替换时仍可读。空间扫描复用 BootstrapStore 的历史目录归属证明和已有 no-follow FD 封装，不向工作区写文件，不新增 Port 或生命周期锁。全 workspace/all-targets 普通测试、fmt、全目标全 feature Clippy 和 release CLI 查询 E2E 13 项已退出 0；专用 submount、受影响变异和 P1-13 GC 尚未完成。本切片不把 GC 任务标为 Done。

P1-13 空间扫描定向变异启动记录（2026-09-26 20:21 UTC）：主 Agent 在本地 `main` 基线 `18d3b65` 加未提交切片、macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、copy 模式、4 并发、baseline skip 下，仅选择 `crates/thinws-adapter-macos/src/space.rs`；执行器报告 38 个待测变异，以 `thinws-cli` 的真实 CLI 测试作为验证包，输出 `target/p1-13-space-mutants/mutants.out`。启动前 `space.rs` SHA-256 为 `97f3170cc8d6492d0b1ad874bac6c68f37948b7ce02d23278be12f34429ae8dc`；先前同机 Adapter 33 项约数分钟，本批因多一层 CLI E2E 保守估计 20:31 UTC 首次主动查看，不在此前反复轮询或修改冻结代码候选。文件边界以外的查询状态与 CLI 渲染变异另行验证，不能用本批冒充。

首批空间扫描变异实际约 3 分钟，38 项为 18 caught、1 unviable、19 missed，退出 2；`outcomes.json`/`mutants.json` SHA-256 为 `3d98e5fba010b5e12fe345328626b1057dea748ce9aed7bad95da8043120ab41`/`c9574c71723d39c392719c6013ac794142613dd4b2b68e80c01392970dd61643`。存活项集中于深度/数量/时间边界、嵌套目录与身份、分配量累计，不据此宣称切片质量通过。修订通过纯边界断言和 256/257 层真实目录、错误身份及 inode 去重测试补强；移除递归入口重复身份判断与被总量检查覆盖的目录余量计算，不为不可达分支堆测试。4 项新 Adapter 定向测试通过，修订后 `space.rs` SHA-256 为 `609c5767769382b26891431dc20f71a612f98f9f16a12070777f947b1ecb05b5`。

P1-13 空间扫描变异复测启动记录（2026-09-26 20:33 UTC）：同机同工具和本地主 Agent，copy 模式、4 并发、baseline skip；修订影响整个 `space.rs`，对该文件 36 项重新定向验证，以真实 `thinws-cli` 测试和 `thinws-adapter-macos` 模块单测为验证包，输出 `target/p1-13-space-mutants-fix/mutants.out`。上批 38 项 3 分钟，增加 Adapter 单测后保守估计 20:38 UTC 首次主动查看；此前不反复轮询、不修改冻结代码候选。旧失败证据保留，不以复测覆盖或删除。

复测实际约 2 分钟，36 项为 34 caught、1 unviable、1 missed，退出 2；`outcomes.json`/`mutants.json` SHA-256 为 `b04937c2369323058747a6a876045172edef47529c81c9236897d15fade824e2`/`3bd6846d89d5f350d90118c30baf8240513c2458c20718e9eef8f5eb2ec34367`。唯一存活项把递归入口的“目录类型或身份不符即拒绝”改为两个条件同时成立；旧错误身份测试最终仍因收尾重验失败，不能证明遍历前拒绝。新增带目录项且已达数量边界的错误身份用例，精确要求先返回 ESTALE 而不是遍历后的 EOVERFLOW，普通定向测试通过；本次仅该测试发生变化，`space.rs` 实现保持不变，SHA-256 变为 `e050853d5921df52461c6dd8849810881cf050c3bdd6cda1f8db3cc327c4f761`。

P1-13 单项存活变异复测启动记录（2026-09-26 20:39 UTC）：同机同工具、本地主 Agent、copy 模式、1 并发、baseline skip，精确选择 `space.rs` 中 `scan_directory` 的 `||→&&` 这一项，以 `thinws-adapter-macos` 单测验证，输出 `target/p1-13-space-mutant-entry-guard/mutants.out`；预估 20:40 UTC 首次查看。前两批有效结果及其失败证据保留，不为生成一份全绿摘要重跑无关项。

该单项约 9 秒完成，1 caught、0 missed/timeout，退出 0；`outcomes.json`/`mutants.json` SHA-256 为 `fa0f5647bd77dca6703a3bd702642152cf0b4601f7699c676fa7a6b78db86b0f`/`7d3204c083dcdc21cd34fdaf2eec9cf75e0d7c8a2a689fca61455d823981eec8`。前批其余 34 caught 与 1 unviable 的生产扫描代码未变，唯一存活项由本批关闭。CLI `status --help` 同步空间能力：先以精确短语测试确认 RED，再调整帮助描述转 GREEN。

P1-13 FD 目录读取变异启动记录（2026-09-26 20:41 UTC）：主 Agent 在上述本地未提交切片、macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、copy 模式、4 并发、baseline skip 下，仅选择 `ffi.rs` 中 `read_directory_bounded`/`read_directory_with_limit` 的 9 项，验证包 `thinws-adapter-macos`，输出 `target/p1-13-space-readdir-mutants/mutants.out`。`ffi.rs` SHA-256 `42c9a0b70e5d5d0a4d527c9039f3bd986e7e783ef1d23fcbc56eb12c9d64811a`；参考上批 36 项 2 分钟，本批预计 20:43 UTC 首次主动查看，不在此前反复轮询。Application 查询与 CLI 渲染另行定向验证。

FD 目录读取实际约 25 秒，9 项全部 caught、0 missed/timeout，退出 0；`outcomes.json`/`mutants.json` SHA-256 为 `0074a1944922908af6de18a4e3080bb5d9c3ecf51e4b59efc826bce78146b958`/`ef547398029e016c35c035013dc3445105d3f9e01593ba5b4895604d60b2fc6b`。本批只覆盖新增有界目录读取及其共用循环，不声称对整个 FFI 文件完成全量变异。

P1-13 查询编排定向变异启动记录（2026-09-26 20:43 UTC）：同机同工具、本地主 Agent、copy 模式、4 并发、baseline skip，仅选择 `thinws-application/src/query.rs` 的 `WorkspaceStatus::space`、`workspace_status` 和 `measure_workspace_space` 共 6 项，验证包为 `thinws-application` 与 `thinws-cli`，输出 `target/p1-13-query-space-mutants/mutants.out`。`query.rs` SHA-256 `473668253aa77c59630a78c011bec1d323720811ceb85a337a49db2d1607858e`；参考上一批 9 项约 25 秒，本批增加真实 CLI 验证，预计 20:45 UTC 首次主动查看。CLI 渲染另测。

查询编排实际约 39 秒，6 项为 3 caught、3 unviable、0 missed/timeout，退出 0；`outcomes.json`/`mutants.json` SHA-256 为 `ea2b4cc952a23743b1a1f9eef148732b3c199f911f10bc44ee98b1c67e2d608e`/`9256713e702186efc280f108173161e721a4e7285ecdcbad1f88e5be1411f4d4`。CLI 契约再补人类输出的 `Logical bytes` 与 `Allocated bytes (estimate)` 精确行断言，契约 10 项全绿；查询生产代码未再改变。

P1-13 CLI 空间渲染定向变异启动记录（2026-09-26 20:46 UTC）：同机同工具、本地主 Agent、copy 模式、4 并发、baseline skip，仅选择 `thinws-cli/src/lib.rs` 的 `status_view`、`render_success_human`、`render_success_json` 共 5 项，验证包 `thinws-cli`，输出 `target/p1-13-cli-space-mutants/mutants.out`。CLI 生产文件 SHA-256 `36c5497e5129f07ea3db85969a1893d97f3300a8a7ba304851e8a71fd951e29c`；参考上一批 6 项约 39 秒，预计 20:48 UTC 首次主动查看，不重复轮询或扩大到无关 CLI 函数。

CLI 渲染实际约 35 秒，5 项为 3 caught、1 unviable、1 missed，退出 2；`outcomes.json`/`mutants.json` SHA-256 为 `a6acbb3b6a30355f22dc1c71ad753cc5fe2133fdd00953c9cc34ef5ab894b6bd`/`dbaf3c0eccc7d54105ab4ed34f75ee7b60bbceee2a701002726cd9ecd95c5fdf`。唯一存活项把人类状态输出的 Ready 判断反转；原非 Ready E2E 只断言 JSON，没有检查人类输出是否泄漏非 Ready 的 `Path:`。新增真实非 Ready 人类输出断言，要求不含 `Path:` 且明确显示空间未测，普通测试通过，生产 CLI 文件未改变。该项需单独复测，不能以手工阅读算通过。

P1-13 CLI 单项存活变异复测启动记录（2026-09-26 20:49 UTC）：同机同工具、本地主 Agent、copy 模式、1 并发、baseline skip，精确选择 `render_success_human` 中 Ready 判断的 `==→!=`，验证包 `thinws-cli`，输出 `target/p1-13-cli-ready-render-mutant/mutants.out`；前批 5 项约 35 秒，预计 20:50 UTC 首次查看。前批失败结果保留，其他未变生产函数的有效结果不重跑。

该单项约 14 秒完成，1 caught、0 missed/timeout，退出 0；`outcomes.json`/`mutants.json` SHA-256 为 `8521dbc139170138156b6c70f73373bb39b59ffa362b823aeccd584054d2219f`/`fc9cbc000c1902698b40f390bc61a9f53b39d764b3548992b92d60a95df760b9`。前批其余 3 caught 与 1 unviable 的生产 CLI 渲染代码未变，唯一存活项由本批关闭。空间统计切片最后一次本地复核：`cargo fmt --all -- --check`、`git diff --check`、全 workspace/all-targets 普通测试（`THINWS_P1_CROSS_VOLUME_ROOT=/Volumes/data`、`THINWS_P0_CROSS_VOLUME_ROOT=/private/tmp`）、全目标全 feature Clippy、release CLI 查询 E2E 13 项、`cargo deny check` 和 `cargo audit --no-fetch` 均退出 0；deny 保留仓库既有 unmatched-license warning。P0 ignored 异卷用例和专用 submount 未在本次切片执行；空间扫描未新增文本解析入口，不为本切片虚报 fuzz 结果。P1-13 的 GC 能力与阶段门禁仍未完成，任务保持 In Progress。

P1-13 空间切片独立审核（2026-09-26 UTC）：GPT-6 Astra / `xhigh` 对本地提交 `35581f6d2c44ccd990c619d56c491f22a53e24b2` 相对 `18d3b65` 只读审查，结论 Changes requested，仅 1 项 P3 文档澄清：详细设计 §十把“副本内部扫描不完整”和“受控根/容器归属失效”都写成 unknown；后者必须继续返回布局错误、拒绝可用路径，不能改安全实现吞掉错误。本地已在详细设计 §十分开两种边界。审核者独立运行 37 项定向测试全绿，并核对 6 批定向变异的计数与摘要及两项历史存活项的 caught 复测；未重跑全量门禁、变异、fuzz 或专用 submount。提交时 `space.rs` 全文件 SHA-256 为 `310184260b8b64157f886ab64c9c59ea215cb503a695bca94ceb8815733faa08`，与前述单项变异前的历史文件哈希不同；审查结论不将历史文件哈希误称为最终文件哈希。文案修正提交 `cc177e792a20afefc842ed70c41313eb0bbc0579` 经同一审核者只读定向复核为 Approve，原 P3 关闭，未发现新冲突；仅文档变化，未重跑代码测试。GC 范围和 P1.e 阶段门禁仍未完成，任务保持 In Progress。

P1-13 GC 入口核验（2026-09-26 UTC）：真实 Adapter 定向测试 `failed_staging_identity_and_cleanup_report_the_unconfirmed_staging_path` 退出 0，证实失败可留下 `staging/` 条目及仅随失败返回的 `unconfirmed_staging` 证据。现行 SQLite `record_failure` 只持久化 Workspace 的 Error/错误码，不持久化该条目身份；显式 `remove --force` 按已登记 WorkspaceId 清理其容器或对应 `trash/remove-<id>/`，不凭 staging 名称认领全局条目。因此强制清理后可能留下无法安全自动判定归属的 staging 残留，而当前生产路径没有“已完成、归属可证且明确标记可回收”的 GC 候选生产者。实现有效 GC 需另定持久标记的建立、撤销与清理授权；若首版不承担这套机制，须先同步调整架构方案、详细设计、用户手册和本计划的 Phase 1 GC 承诺。未获该范围决策前，不以空列表/空执行冒充 GC 已交付。

P1-13 当前范围决定（2026-09-26 UTC）：维护者明确“GC 先不实现，继续”。[ADR-0005](../../project/architecture/adr/ADR-0005_Phase1暂不实现GC.md) 已将 Phase 1 收敛为当前空间统计和显式 Workspace 清理；本节旧“GC 与空间统计”领取记录及 GC 验收断言仅是当时的过程历史，不再构成 P1-13 当前退出条件。当前还须核对 `thinws gc` 为 E_USAGE、全局残留未被 `remove` 越界删除、空间估算不冒充可释放量，并完成本任务的文档/契约复核。未为后续 GC 指定阶段或预建候选机制。P1-14 仍以本任务收口为依赖，P1-15 仍以所有活跃 P1 任务收口为依赖。

P1-13 范围收缩候选自测（2026-09-26，macOS/APFS）：仅修改权威文档、CLI 契约/E2E 测试，生产代码、依赖、fixture 和工具配置未变；既有空间扫描定向变异证据沿用上述经审核的代码候选，本次不重新运行变异或 fuzz，也不冒称 P1.e 小阶段门禁完成。`cargo fmt --all -- --check`、全目标全 feature Clippy、`THINWS_P1_CROSS_VOLUME_ROOT=/Volumes/data cargo test --workspace --all-targets`、release CLI 的 GC 拒绝契约与显式删除保留未知 staging/trash 的真实 E2E、`cargo deny check`、`cargo audit --no-fetch` 均退出 0；deny 保留既有未命中许可例外/allowance warning。全仓普通测试中的专用 P0 异卷与 submount ignored 用例、本次未运行的长预算变异/fuzz、线上 CI/PR/push 均不计为通过。一次测试环境误把 P1 异卷目标设为与源同卷的 `/private/tmp`，该用例按预期失败；修正为真实异卷 `/Volumes/data` 后整套重跑退出 0。独立审核的未提交差异预审为 Approve，但任务级 Done 须等待本地精确 commit 复核及 Verification。

P1-13 任务级收口（2026-09-26）：GPT-6 Astra / `xhigh` 对本地精确提交 `a668bfecda6aa4196480440cf1be778642768821` 相对 `f75f05043c13c6e5cc1b4a3ed9f8f2076f1a2633` 只读终审为 Approve，无可操作问题；独立重跑 release CLI 契约 11 项、真实 APFS 删除 E2E 1 项、二进制 help 与 `gc` 拒绝，并核对干净工作树，未重跑全仓/Clippy/供应链/变异/fuzz 或阶段门禁。审核后主 Agent 在相同干净提交上进入 Verification，release CLI 契约 11 项及显式删除保留全局残留的真实 E2E 1 项均再次退出 0。当前空间统计切片、GC 范围收缩和公开拒绝契约因此满足 P1-13 当前退出条件，任务标为 Done；这不宣称 P1.e 小阶段或 Phase 1 已放行。专用 submount、完整小阶段变异、长预算 fuzz 和线上 CI/PR/push 未执行，留待相应收口门禁。

P1-12 领取（2026-09-26 UTC）：主 Agent 在唯一 checkout 的本地 `main` 实施，基线 `6d9bb75`；不派生开发 Agent、不创建线上 PR，冻结候选使用 GPT-6 Astra / `xhigh` 只读独立审核。风险 R4（递归删除、跨进程路径竞态、进程占用 FFI）。先补 ADR-0004 的 Workspace 历史目录归属证明，再实现受控删除，避免仅凭当前同名普通 `root/` 推定可删除。验收断言：真实 APFS 上替换原 root/容器、缺失或损坏证明均拒绝且不触碰替换目录；普通清理对 tracked dirty/unknown 拒绝，不因 untracked-only 拒绝；显式 force 绕过前两项但不绕过身份/卷/确认占用；日志在目标删除后可读且起止事件如实；中途失败保留非 Ready 与受控残留，再次 force 只清理仍可证明归属的对象；重复 ID 由 tombstone 返回 already-removed。真实 Git、进程、CLI/JSON、错误注入、定向变异和受影响 fuzz 均需执行，任务完成不自动代表 P1.e 小阶段放行。

P1-12 首轮安全设计审核（2026-09-26 UTC）：GPT-6 Astra / `xhigh` 对 `94afe48050fc803011be8c603a69c3fcd05b3ba6` 给出 Changes requested。归属文件置于待删容器内会在删标记后中断时丢失重试证据，历史 `st_dev` 不能跨重挂载等值，预留记录却无标记且目录已不存在时必须能仅收口数据库。主 Agent 已将证据移到 `metadata/` 并在 tombstone 后保留，只持久化卷 UUID、inode 和 birthtime 上界；明确“容器不存在且显式 force”不执行路径删除，只释放记录。另经真实 APFS 测试证实把目录 mtime 调到 2000 年会把 birthtime 调早，故持久出生时间只作为上界，不能相等比较。生产 Adapter 已先补持久文件创建与 Ready 历史归属核验，并有普通目录替换先 RED 后 GREEN、缺失/损坏证明、容器替换和旧 mtime 回归；全 workspace/all-targets 普通测试及 Clippy 通过。此时删除、日志、ProcessProbe、CLI 与任务级定向变异/fuzz 均未实现或未执行，P1-12 保持 In Progress；设计修订及代码须在冻结候选后重新独立审核，不沿用对旧提交的结论。

P1-12 归属实现审核纠偏（2026-09-26 UTC）：GPT-6 Astra / `xhigh` 对 `4e827068e74e1c33209ddaae132d0bc76cf7cc4d` 给出 Changes requested：受控目录原先在公开目标名称上 `mkdirat`，再首次 `openat` 认领身份，竞争替换可被错误登记。主 Agent 改为在受控私有父目录下先建立临时目录并打开固定身份，再以 no-replace 发布到目标名称；发布前后重验目录项。真实 APFS 注入竞争目标测试确认目标在发布前不存在、竞争目标阻止发布、外来内容不变且可证明归属的临时目录被清理。普通全 workspace/all-targets 测试、fmt 和 Clippy 已通过；本修订仍待精确候选的独立复核，P1-12 继续 In Progress。

P1-12 创建链路补充纠偏（2026-09-26 UTC）：GPT-6 Astra / `xhigh` 对 `653e0bcce9def4ea39a0652ae6095853cc341b94` 给出 Changes requested（High 1）：暂存发布修正本身未见阻断问题，但既有创建链路在 `prepare_workspace` 后只按公开路径 Probe/Plan，没有把计划目标与所持 root FD 身份绑定，替换后的普通空目录可能被写入。主 Agent 以真实 APFS 注入目标替换，先观察到外来目录 mode 被修改（RED），再在初次及运行时 fallback Probe 后加入原始目录身份核对（GREEN）；另覆盖 Probe 期间替换后恢复原路径的情形，要求仍拒绝旧报告。`cargo fmt --all -- --check`、全目标 Clippy、显式真实跨卷根的全 workspace/all-targets 测试均退出 0；`require_prepared_target` 的 9 个定向变异全部 caught（`target/p1-12-target-binding-mutants/mutants.out`）。GPT-6 Astra / `xhigh` 对精确提交 `de4b4c32c1d5ce3ba8b5cfb592e4eed9a292e0f2` 只读复核为 Approve，未发现该范围新问题；P1-12 完整清理、进程、日志和 CLI 仍未完成，不能据此放行任务或阶段。

P1-12 进程占用探测（2026-09-26 UTC）：首个 macOS libproc 实现 `d181de9` 经 GPT-6 Astra / `xhigh` 只读审核为 Changes requested（High/P1 两项）：把可变化的 FD 数作为进程身份会漏掉占用，且用输入路径直接匹配会漏掉 `/System/Volumes/Data` 与 `/Volumes/data` 的同目录别名。修正为以 PID、UID、启动秒/微秒复核身份；由 no-follow 打开的容器 FD 取 `F_GETPATH`，核验目录设备号和 inode 后按内核路径匹配。真实 macOS 子进程 cwd、open-vnode 与 firmlink 别名，以及 FD 数变化和各身份字段单测通过。`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace --all-targets` 退出 0；`same_process` 定向 9 个变异全部 caught（`target/p1-12-process-identity-mutants/mutants.out`，25 秒）。GPT-6 Astra / `xhigh` 对精确生产修正提交 `537e964e47a7158c7f636f7ae9705ab9d53c6b61` 只读复核为 Approve，未重复全量门禁或变异。2 秒为合作式扫描预算，不是阻塞系统调用的硬超时；该审核不代表 `remove` 编排、删除、持久日志或 CLI 已完成。

P1-12 受控删除底层（进行中）：为避免把可由调用者构造的普通路径误当删除授权，现有 `BootstrapStore` 增量提供持 data-root 锁、布局和 WorkspaceId 的清理调用；macOS Adapter 对照创建时位于 `metadata/` 的历史归属。首版 `e8f9759` 直接按名称递归删除，经 GPT-6 Astra / `xhigh` 只读审核为 Changes requested（P1 两项）：stat 后同名对象可被替换，已打开的中间子目录移出后仍可通过旧 FD 删除其内容；审核者用候选生产代码在真实临时目录确定性复现后者，外部位置文件确实被删。主 Agent 改为先以 no-replace 同卷 rename 将整个 ID 容器隔离至私有 `trash/remove-<id>/`，核验历史身份后仅在隔离树递归删除；新显式 force 同时检查原位置与隔离位置，二者冲突拒绝。递归还逐层核验子目录挂载关系，补充移出后保留外部文件回归。真实 APFS 用例覆盖嵌套树、符号链接、原 root/容器替换、证明缺失、未知平台项、root 已缺失后的收口、删除前路径替换、隔离后部分删除重试、双位置冲突及隔离位置替换。归属文件不删除；两处均缺失才返回无路径删除结果。此次职责与隔离调整已同步 ADR-0004、详细设计、跨平台物化设计和用户手册，不新建 Port，也不提供 `destroy_materialization(&WorkspacePath)` 裸路径删除入口。修订候选的 `cargo fmt --all -- --check`、全目标全 feature Clippy、全 workspace/all-targets 测试已通过，仍待精确独立复核；Application 清理策略、持久日志、SQLite 收口、CLI、任务级变异/fuzz 均未完成，P1-12 不放行。

P1-12 隔离清理终态复核（2026-09-26 UTC）：GPT-6 Astra / `xhigh` 对精确提交 `910358b` 给出 Changes requested（P2 一项，未发现新增 P0/P1）：隔离树清空后未核对原位置与隔离位置均仍缺失。审核者以生产 `BootstrapStore` API 的真实临时目录复现：清理期间原位置重建后返回 `Removed { root_entries: 1000 }`，但原位置仍在；后续若直接写 tombstone 会错误收口。主 Agent 已在成功返回前加持锁、布局及两处 no-follow 缺失重验，任一位置重现即报错且不删除新对象；补两位置缺失/重现的受控目录检查，并把真实并发重建用例连续执行五次均得到 `InvalidLayout`、外来文件保留。`cargo fmt --all -- --check`、全目标全 feature Clippy 与全 workspace/all-targets 测试通过。审核者对精确提交 `595f6964c6e24b141a5c7341650164c358a86708` 只读复核为 Approve：重跑 1000 文件真实 API 复现及普通文件、悬空符号链接重现均被拒绝且外来对象保留；未重跑全量门禁、变异或 fuzz。此结论仅覆盖底层删除，不代表 Application/CLI 或 P1-12 放行。

P1-12 删除编排接入（进行中）：BootstrapStore 增量提供持 data-root 锁的只读容器定位，返回已验证且唯一的原位置或隔离位置供 ProcessProbe 扫描；容器不存在返回 None，两处冲突或归属不符拒绝。先写针对原位置、隔离位置、冲突及不存在的 APFS 用例并确认未实现时失败，再补实现；全工作区普通门禁通过。MetadataStore 增量提供按完整 WorkspaceId 读取最小 tombstone，供重复 remove 返回 already-removed；先验证未实现时测试因缺失 tombstone 失败，再从既有 SQLite 表读取并跨重新打开验证。仍需接入 Application 的 Git/进程决策、持久日志、SQLite 收口编排和 CLI；此阶段不把只读定位或 tombstone 读取当作完成清理。

P1-12 清理日志持久化（进行中）：在既有 BootstrapStore 边界增量实现 `logs/operations.jsonl` 的持锁、no-follow、0600、同步追加与路径身份核验，不在副本内写产品文件，不引入新 Port/审计服务。先用真实 APFS 测试确认未实现时 force 开始事件失败，再补实现；追加两条结构化事件、控制字符 JSON 转义、日志路径符号链接拒绝及不允许无结果的伪完成事件已有回归。`cargo fmt --all -- --check`、全目标全 feature Clippy 和全 workspace/all-targets 测试通过。Application 起止事件编排与异常分支仍待完成；同步调整技术栈中“tracing 诊断”和“必须 fsync 的清理 JSONL”职责，不把单条日志能力视为任务完成。

P1-12 日志部分写入审核纠偏（2026-09-26 UTC）：GPT-6 Astra / `xhigh` 对精确提交 `dd0bd5f` 给出 Changes requested（P2 一项）：已有 JSONL 尾行不完整时，下一开始事件直接粘连，fsync 成功也不构成独立可解析事件。审核者用精确提交独立构建及 `RLIMIT_FSIZE` 真实短写复现：首次 Io 留下无换行的 128 字节，第二次返回成功但有效 JSONL 事件数为零。主 Agent 补半行尾字节检测与新事件换行隔离，先写测试复现旧行为（一行而非两行）再转绿；不完整旧片段保留，不解释为成功；新事件完整同步之前不能开始删除。开发规范 §13.1 同步明确此边界。本地 `cargo fmt --all -- --check`、全目标全 feature Clippy 和全 workspace/all-targets 测试通过；独立复核仍待完成。日志写入与 Application 删除先后顺序尚未集成。

P1-12 日志纠偏复核：GPT-6 Astra / `xhigh` 对精确提交 `9e4a214ed9f291348526df7cb764973a78622b5e` 独立只读审核为 Approve，新增 P0/P1/P2 均为零。审核者用独立 target 的真实 `RLIMIT_FSIZE` 验证：128 字节旧半行后受限重试只写分隔换行仍报 Io；解除限制后新开始事件独立可解析，原半行保留且工作区未删除；`process_use=None` 与 `Some(NoEvidence)` 输出分别为 null 与 `no-evidence`，竞争 lifecycle lock 有界超时。未重跑全量门禁、变异/fuzz 或真实断电注入；该结论仅关闭日志局部问题，不代表完整删除编排放行。

P1-12 Application/CLI 接线（进行中）：在既有 Service、BootstrapStore、MetadataStore、GitInspector、ProcessProbe 边界编排受控 `workspace remove`，使用名称或完整 ID，普通模式依据 Git 已跟踪变更与确认占用拒绝，显式 force 可绕过 Git 而不能绕过确认占用、历史归属或日志开始写入；开始事件同步落盘后才进入 Deleting，受控删除后再核对两处均无容器，写 tombstone 后才记完成事件。真实 APFS/CLI 测试覆盖普通目录清理且源目录不变、已跟踪变更拒绝与 force、仅未跟踪文件不阻塞、真实 cwd 占用进程拒绝、日志写入失败前不删目录，以及完整 ID tombstone 重复请求。清理拒绝的人类输出和 JSON context 返回已跟踪仓库相对路径与变更数，不把未跟踪文件当作已跟踪变更。首次应用层测试先因未实现返回 CapabilityUnavailable 而 RED，CLI 契约先因未登记命令返回 E_USAGE 而 RED，补实现后定向测试转绿。`cargo fmt --all -- --check`、全目标全 feature Clippy 和全 workspace/all-targets 测试通过。仍需独立审核、失败注入、异常日志细节核对、定向变异/fuzz；不得把当前接线视作 P1-12 Done。

P1-12 Application/CLI 首轮独立审核与修订（2026-09-26 UTC）：GPT-6 Astra / `xhigh` 对精确提交 `a4c485b509419d2f139c649ab782748fa4b64c6c` 给出 Changes requested（P1 一项、P2 两项）。真实复现表明，一个 Workspace 名称恰好等于另一 Workspace 的完整 ID 时，无前缀 remove 会静默删除 ID 所指的错误对象；已知 ID 的归属/进程预检失败未写持久失败事件；Git 检查不完整的 remove 拒绝未展示具体原因短名。修订为双候选冲突时 E_USAGE 且不删除，并提供仅用于 remove 的 `name:`/`id:` 显式选取；已知 ID 的预检失败追加 Failed 事件且保持原错误类型；JSON 与人类拒绝输出展示仓库 issue。分别先用真实两个 Workspace、注入 ProcessProbe Io、外部 Git metadata 用例确认旧候选错误，再修正转绿；补普通模式 Error 状态拒绝、force 后根目录替换拒删外部内容并可再次显式清理、缺少归属证明拒绝并记录失败的 APFS 回归。修订候选的 `cargo fmt --all -- --check`、全目标全 feature Clippy、全 workspace/all-targets 测试均退出 0；仍待提交后的精确独立复核与适用的耗时门禁，不据此宣布 P1-12 Done。

P1-12 Application/CLI 修订复核：GPT-6 Astra / `xhigh` 对精确提交 `83e21fa0820e62540ef763842c06b7c29b9a8029` 只读审核为 Approve，新增 P0/P1/P2 为零。审核者独立运行 Application remove 11、CLI E2E remove 4、CLI contract 10 项，并复验双 Workspace 冲突、ID tombstone、正常/force 进程探测 Io 失败日志、全局及逐仓库 Git issue，均符合契约；未重复全量门禁、变异/fuzz，结论只关闭此次三项审核意见。

P1-12 底层删除首次定向变异（2026-09-26 UTC）：主 Agent 在 macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0，以 copy 模式、4 并发、baseline skip，选择 `destroy.rs` 与 `store.rs` 的受控删除相关函数 43 个变异；17:41:41 UTC 启动，按首次一小时等待于 18:41:58 UTC 主动查看，工具实际 17:44:24 UTC 已结束，耗时约 2 分 43 秒。结果 27 caught、7 unviable、9 missed，退出 2，证据在 `target/p1-12-removal-mutants/mutants.out`；不能记为通过。存活项分别涉及递归深度上限、两个由同一 inode 身份已蕴含类型的冗余条件、隔离后失败自动还原分支和 ESTALE/EXDEV/ELOOP 归类。主 Agent 已增真实 512/513 层 dirfd 递归边界用例并通过，删去冗余类型比较；隔离后失败不再自动还原，保持私有隔离残留供下一次显式 force 重新证明归属，减少自动恢复分支；底层错误归类抽成可直接验证的小函数并补三种布局错误与 EIO 对照。Adapter 单元 77 通过、1 环境要求 ignored，bootstrap 集成 27 通过；修订后仍需重新执行受影响的精确定向变异、全局普通门禁和独立审核。

P1-12 底层删除定向复测启动记录（2026-09-26 18:49 UTC）：上述生产差异相对 `83e21fa` 的 SHA-256 为 `941fc75b655bcd50faa2b7a4c8226a91734110e7bb8fa24658a0ada09a36b668`；主 Agent 单独执行，macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、copy 模式、4 并发、baseline skip，仍只选择 `destroy.rs`/`store.rs` 同一删除函数范围，结果位置 `target/p1-12-removal-mutants-fix/mutants.out`。上批同范围 43 项实际约 2 分 43 秒，考虑编译/机器波动，本批首次主动查看预计 18:54 UTC；此前不轮询、不修改冻结生产候选。修订候选在启动前的 fmt、全目标全 feature Clippy、全 workspace/all-targets 普通测试退出 0；未执行 ignored 的环境专属跨卷目标。

定向复测结果：实际 18:49:13–18:51:04 UTC，约 1 分 51 秒，按预计于 18:54 UTC 一次读取终态；36 个变异为 28 caught、8 unviable、0 missed/timeout，退出 0。`outcomes.json` 与 `mutants.json` SHA-256 分别为 `87aa71456225f38cd0a762381fe991952cc3b2c4f0bbd01daab537eb217a2800`、`b5cf24cc7a2f49fca78092a6a25db0f50484908f94e03932fa35465df2e73e11`。第一次存活的深度边界由真实 512/513 层回归捕获；同 inode 必然同文件类型的冗余条件及自动还原分支已移除；错误分类由独立断言验证。此结果只覆盖选中底层函数，不冒称 Application/CLI 变异、小阶段全受影响 crate 变异或阶段放行。

P1-12 底层修订独立复核：GPT-6 Astra / `xhigh` 对精确提交 `ff563873024fdcafd4fda2cb16b38c9b56fca8c5` 只读审核为 Approve，未发现 P0/P1/P2。审核者真实复验隔离后失去归属证明会停止删除并留下 Error/Started→Failed，修复证明后普通重试仍拒绝、新显式 force 可清理；外来同名对象保留。512/513 层在真实 Application 主线程分别成功/返回布局错误且保留隔离残留，类型替换为外部符号链接不越界；49 项定向测试含单独启用的真实异卷用例通过。审核者核对本批 28 caught、8 unviable、0 missed/timeout 的结果和哈希，但未重跑变异或全量门禁。当前结论仅关闭底层修订，不等于 P1-12 整体完成；Application/CLI 定向变异、适用 fuzz 与任务级收口仍待完成。

P1-12 remove 参数 fuzz（2026-09-26 UTC）：新增纯内存 `thinws_remove_request` harness，对原始 UTF-8 输入及文本种子的无末尾换行版本核对普通名称、完整 ID、`name:`/`id:` 显式目标的接受性，并覆盖 force 标志与负时间错误码；输入不进入文件系统或删除 API。固定 `nightly-2026-08-14`、`cargo-fuzz` 0.12.0 在 macOS arm64 构建通过，以四个仓库种子、独立 `target/p1-12-remove-fuzz-corpus.9xkTQ2` 输出目录运行 `-max_total_time=30 -timeout=5`，31 秒完成 4,573,845 次，exit 0，无 crash/hang；生成语料保留在 target 而不批量提交。fuzz lockfile 仅补齐 Application 既有传递依赖 `serde_json`，没有新增直接依赖；fuzz manifest 的 `cargo deny check` 和本地 advisory 库的离线 `cargo audit` 均退出 0，deny 只有既有未命中例外/allowance 提示。单文件 rustfmt 检查通过；独立 fuzz workspace 的全量 fmt 检查发现两个未修改旧 harness 的既有格式差异，本任务未改动它们。该 smoke 仅验证公开 remove 参数解析，不代替 Git、路径和状态的其它既有 fuzz 目标，也不代表阶段长预算。

P1-12 Application 删除编排定向变异启动记录（2026-09-26 19:06 UTC）：主 Agent 以本地精确提交 `a5171b1` 为候选，在 macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、copy 模式、4 并发、baseline skip 下选择 `crates/thinws-application/src/remove.rs` 的全部 35 个变异；验证包为 `thinws-application`，输出 `target/p1-12-app-remove-mutants/mutants.out`。上批 Adapter 36 项约 1 分 51 秒，但本批 Application 集成测试与编译不同，首次主动查看保守估计 19:11 UTC；此前不轮询、不修改冻结候选。CLI 错误渲染的独立 4 个变异随后按差异范围单独验证，不在本批冒称覆盖。

Application 首次变异结果与局部补测：实际 19:06:07–19:08:47 UTC，约 2 分 40 秒，按预计于 19:11 UTC 一次查看；35 个为 14 caught、8 unviable、13 missed、0 timeout，退出 2，`outcomes.json` SHA-256 `89f4da602112373718cd30cf548c748c0261c38e250545c18412e4fd299b45df`。存活项涉及公开 result/forced/warning、tombstone 返回、Git 检查完整性日志、force 不运行 Git 以及拒绝消息/修复建议。主 Agent 在既有 Application remove 测试补精确结果、force 免 Git 的失败型测试双桩、完整发现但仓库 unknown 的日志反例及进程扫描不完整告警；本地定向 13 项通过。尚须复测 13 个存活项；原先 14 caught、8 unviable 的位置未改生产代码，按《任务流程》§18.1 核对后保留原证据，不重跑整个 35 项。

Application 存活项复测启动记录（2026-09-26 19:14 UTC）：本次生产 `remove.rs` 未改，只改对应 Application 测试，精确筛选上次存活行及同一行的 3 个已捕获对照，共 16 个变异；主 Agent、macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、copy 模式、4 并发、baseline skip，输出 `target/p1-12-app-remove-mutants-fix/mutants.out`。上批 35 项约 2 分 40 秒，本批首次主动查看预估 19:17 UTC；此前不轮询、不修改冻结候选。

Application 存活项复测结果：实际 19:13:47–19:15:22 UTC，约 1 分 35 秒，按预计 19:17 UTC 一次读取；16 个为 15 caught、1 unviable、0 missed/timeout，退出 0。`outcomes.json` 与 `mutants.json` SHA-256 分别为 `f8923f06fe2782166d8d8e4a636b3bb1973ae6ea33bf79c1dfa2391f53b0bbcc`、`47b613c16c339d6f3bfdd12fd082dc2cc29042227e93c9754ac94090371b54fb`。与首次 35 项结果按相同生产文件/变异位置合并，全部原存活项已有捕获证据；未重跑无关 19 项，不把两批计数简单相加为独立变异数。仍需 CLI 渲染定向验证与任务级审核。

P1-12 CLI 错误渲染定向变异启动记录（2026-09-26 19:18 UTC）：主 Agent 在本地 `main`、macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、copy 模式、4 并发、baseline skip，选择 `thinws-cli/src/lib.rs` 的 `render_error` 与 `use_case_error_view` 共 4 个变异，验证包 `thinws-cli`，输出 `target/p1-12-cli-remove-mutants/mutants.out`。该范围仅衡量已有 CLI 错误输出入口，不声称自动生成新增 JSON 数组字段的变异；真实 E2E 及独立审核另覆盖该字段。根据前两批约 2–3 分钟且本批仅 4 项，首次主动查看预估 19:20 UTC；此前不轮询、不修改候选。

CLI 定向变异结果：实际 19:17:44–19:18:09 UTC，按预计 19:20 UTC 一次读取；4 个为 3 caught、1 unviable、0 missed/timeout，退出 0。`outcomes.json` 与 `mutants.json` SHA-256 分别为 `ca403f54e4159af4a5205cb5a240e4a236efea93e89b212b79dea6a0da060476`、`abc52000a3b7229f51618dc9c09a91969669c5dd9296db25f13166285c6850a3`。该证据仅覆盖选定函数变异，JSON 拒绝上下文的字段形态由真实 CLI E2E 和契约检查验证。任务级复核与通用门禁仍待完成。

P1-12 任务级候选自测（2026-09-26 UTC，macOS/APFS）：Application remove 13 项、CLI remove 真实 E2E 4 项及同 4 项 release 配置均通过；公开手册的拒绝 JSON 示例修正为现行消息、修复建议、`issues` 与无损仓库相对路径字段，并在真实 CLI E2E 精确断言，JSON 示例用 `jq` 解析通过。`cargo fmt --all -- --check`、全目标全 feature Clippy、带 `/Volumes/data` P1 异卷端的全 workspace/all-targets 普通测试及 `git diff --check` 均退出 0。相关底层、Application、CLI 定向变异与 remove 参数 30 秒 fuzz 结果见本节上方；未执行 P1.e 小阶段全受影响 crate 变异、阶段长预算 fuzz、专用 submount 和线上 CI/PR/push，这些不冒称任务级通过证据。任务风险仍为 R4，需对精确本地 commit 完成独立审核；审核通过后再进入 Verification/Done，不以本段自行放行。

P1-12 任务级首轮独立审核（2026-09-26 UTC）：GPT-6 Astra / `xhigh` 对干净 `main` 的精确提交 `e18fbc7adb84ed6b74701e339f9d55409b00ea15` 给出 Changes requested：未发现新数据误删问题，但新增 Core 删除策略及 Adapter 持久日志尚缺受影响函数的定向变异证据；归属 TOML 解码尚未进入文档 fuzz harness；普通 Git 拒绝与扫描不完整同时发生时，日志虽记录但用户错误未显示扫描警告。审核者只读运行 68 项 Adapter/Application/CLI 定向测试均通过，核对 Application 存活项闭合与公布哈希；remove 参数 fuzz 的运行量仅能引用主 Agent 记录，审核者未发现原始输出，不能冒称独立复验。已确认 `lifecycle.lock` 仅在 data root 的 `metadata/`，`path/status` 不取锁，镜像 root 不写该文件。任务返回 In Progress；主 Agent 将补上述定向门禁和组合告警，不扩大清理机制。

审核修订候选（2026-09-26 UTC）：先使普通 Git unknown＋进程扫描不完整、非 Ready＋进程扫描不完整的 Application 断言，以及拒绝时人类告警的 CLI 断言按预期失败，再复用现有诊断 context 写 `process_use=scan-incomplete`，人类错误输出该告警；手册 §6.4 同步，日志与拒绝策略不变。定向 Application 13 项与 CLI 单测转绿，全 workspace/all-targets 测试、fmt、全目标全 feature Clippy、`git diff --check` 退出 0。归属 TOML 解析接入既有纯内存 bootstrap document fuzz harness，不新增删除入口；有效与纳秒越界种子各一，Adapter 普通单测验证两者分别进入成功/拒绝分支。固定 nightly/cargo-fuzz 构建通过；首次 smoke 因目标语料输出目录不存在而未启动，不作为结果；创建目录后 30 秒 smoke 运行 563,722 次，退出 0，无 crash/hang。fuzz manifest 的 deny 与本地 advisory 库的离线 audit 退出 0，deny 仅既有未命中例外警告。单文件 fuzz rustfmt 通过，尚待 Core 清理策略、Adapter 日志及本次告警渲染的定向变异；不以目前候选关闭 P1-12。

P1-12 Core 清理策略定向变异启动记录（2026-09-26 19:39 UTC）：主 Agent 在本地精确实现提交 `8dc5fdc`、macOS arm64、Rust 1.97.1、`cargo-mutants` 27.1.0、copy 模式、4 并发、baseline skip 下选择 `thinws-core/src/removal.rs` 全部 4 个变异，验证包 `thinws-core`，输出 `target/p1-12-core-removal-mutants/mutants.out`。上次同机 Core 100 项约 4 分钟，本批仅 4 项但含固定构建开销，首次主动查看估计 19:41 UTC；此前不轮询、不修改冻结候选。Adapter 日志与 Application/CLI 告警变异分别启动、记录，不冒称在本批覆盖。

Core 结果：实际 19:38:54–19:39:18 UTC，约 24 秒，按预计 19:41 UTC 一次读取；4 个为 3 caught、1 unviable、0 missed/timeout，退出 0。`outcomes.json` 与 `mutants.json` SHA-256 分别为 `61272029799f08d34be38a2db8742ba6155c5c56cf1f95374ce17fd3b1e5b76c`、`abd99daa83dd37b4eceac9c8b364363c006e88733d5ea364203de74ec2b8160c`。copy 阶段报告了既有 `experiments/p0/cleanup/target/` 下 FIFO 类型提示，未影响测试结果；后续批次显式使用 `--gitignore true` 避免复制忽略的实验输出。

P1-12 Adapter 清理日志定向变异启动记录（2026-09-26 19:42 UTC）：同一主 Agent、平台、工具版本和实现提交 `8dc5fdc`，copy 模式、`--gitignore true`、4 并发、baseline skip；仅选择 `thinws-adapter-macos/src/operation_log.rs` 的 33 个变异，验证包 `thinws-adapter-macos`，输出 `target/p1-12-operation-log-mutants/mutants.out`。前次同机 36 项底层删除约 1 分 51 秒，日志测试范围不同且含构建开销，保守预计 19:45 UTC 首次主动查看；此前不轮询、不修改候选。

Adapter 日志首次结果：实际 19:42:03–19:45:35 UTC，约 3 分 32 秒；19:45 UTC 首次检查时仍在运行，按已见进度改为 19:47 UTC 读取终态。33 个为 17 caught、1 unviable、15 missed、0 timeout，退出 2，不能记为通过；`outcomes.json` 与 `mutants.json` SHA-256 分别为 `d9f80f0fd73cd4fd2e14203af52334cc45938459910e90b59da5280c2e6f2046`、`ac4cddb8033c7226ebc5fc7964c091f28acf155d6b647dce9ce8e8441aa8549c`。存活项限于 Refused/Started/Failed 字段组合、仓库相对路径、日志身份终验、Git/进程/拒绝原因短名和路径 hex；主 Agent 将仅补精确断言或删除已证明冗余的检查，随后只复测存活位置及同线对照，不重启无关变异。

Adapter 日志存活项复测启动记录（2026-09-26 19:50 UTC）：在真实 bootstrap 测试补非法事件字段组合、`../` 仓库相对路径、Git/进程/拒绝原因短名和路径 hex 的精确断言，30 项 bootstrap 测试通过。末尾身份核验复用已测试的 `validate_file_entry`，移除同一打开文件 FD 的恒定 inode 二次比较，不削弱目录项身份、权限或 no-follow 校验。相对提交 `8dc5fdc` 的该模块与测试差异 SHA-256 为 `919b44e8d25c0cad1727a376dd3b207289dc12f120b8b68a0802aca4988f5e1c`；主 Agent、同机同工具、copy 模式、`--gitignore true`、4 并发、baseline skip，只筛选剩余 14 个存活位置 `32/35/40/54/155/164/172/180`，输出 `target/p1-12-operation-log-mutants-fix/mutants.out`。上次 33 项约 3 分 32 秒，本批预计 19:53 UTC 首次主动查看；此前不轮询、不修改候选。

Adapter 日志复测结果：实际 19:50:11–19:51:33 UTC，约 1 分 22 秒，按预计 19:53 UTC 一次读取；14 个全部 caught、0 missed/unviable/timeout，退出 0。`outcomes.json` 与 `mutants.json` SHA-256 分别为 `b543e5c91df549947e24d5dd746e8ab3fe084cc2a4c8679fa665693f76835ea5`、`4e22182f535516401a72383984fa830836103ee7599d697db75fe9bcf664797a`。与首次 33 项按变异位置及生产差异组合，原存活的日志校验/编码缺口均已处理；原 17 caught、1 unviable 的未改位置不重复跑，不将两批计数相加为独立目标数。仍需本次 Application/CLI 告警差异的定向变异和任务级终审。

P1-12 告警差异定向变异启动记录（2026-09-26 19:54 UTC）：同机同工具、本地实现提交 `8dc5fdc` 加上仅日志模块及测试的未提交差异，copy 模式、`--gitignore true`、4 并发、baseline skip；先选 `thinws-application/src/remove.rs` 新增 `with_process_scan_warning` 的 2 个变异，输出 `target/p1-12-app-warning-mutants/mutants.out`。根据此前同包 16 项约 1 分 35 秒，预计 19:56 UTC 首次主动查看；期间不修改候选。其后单独选 CLI `render_error`/`use_case_error_view` 的 5 个变异。

Application 告警结果：实际 19:54:05–19:54:18 UTC，约 13 秒，按预计 19:56 UTC 一次读取；2 个为 1 caught、1 unviable、0 missed/timeout，退出 0。`outcomes.json` 与 `mutants.json` SHA-256 分别为 `2867a03543c73f42700091839e4e170cb207ee1b07a12885ecc918423f20d32f`、`42456ec828ed2c0d11cd2b1cb2ae3ac0316b0e4b7420af408b6bf6b0f2c1eb4b`。

P1-12 CLI 告警渲染定向变异启动记录（2026-09-26 19:56 UTC）：同机同工具、同一冻结代码候选、copy 模式、`--gitignore true`、4 并发、baseline skip；选择 `thinws-cli/src/lib.rs` 的 `render_error` 和 `use_case_error_view` 5 个变异，输出 `target/p1-12-cli-warning-mutants/mutants.out`。此前同两函数 4 项约 25 秒，本批新增一处分支，预计 19:58 UTC 首次主动查看；此前不轮询、不修改候选。

CLI 告警结果：实际 19:56:27–19:56:44 UTC，约 16 秒，按预计 19:58 UTC 一次读取；5 个为 4 caught、1 unviable、0 missed/timeout，退出 0。`outcomes.json` 与 `mutants.json` SHA-256 分别为 `0d1e85e1ae9900a9ad5bd34b0c195c08257b840577008400e42b38b208fcb411`、`bf56f90d826576fe7674833c6cca4eb17a13f7dc6ac7882cf9ecfaf14c457fdf`。本批与前述 Core/Adapter/Application 的定向证据只覆盖 P1-12 本次受影响代码，不取代 P1.e 小阶段或 Phase 1 的全范围门禁。

P1-12 审核修订候选终检（2026-09-26 UTC，macOS/APFS）：`cargo fmt --all -- --check`、`git diff --check`、全目标全 feature Clippy、带 `/Volumes/data` P1 异卷端和 `/private/tmp` P0 异卷参数的全 workspace/all-targets 普通测试均退出 0；P0 的 ignored 异卷实验不会因提供环境变量而在普通测试中自动执行。release 配置的真实 CLI remove E2E 4 项与告警渲染单测 1 项通过；bootstrap 30 项含新增日志异常组合及字段断言通过。根和 fuzz workspace 无新增直接依赖；两 workspace 的 deny 与离线 audit、所改文档 harness 单文件 rustfmt 均通过，deny 只有既有未命中例外/allowance 提示。Core、Adapter 日志、Application、CLI 的受影响变异按本节分批证据闭合，归属 TOML 文档 fuzz 短预算退出 0。专用 submount、P1.e 小阶段受影响 crate 全量变异和阶段长预算 fuzz、线上 CI/PR/push 未执行；它们不作为本任务候选已通过项。此时本地候选尚待 GPT-6 Astra / `xhigh` 精确独立复核，未进入 Verification/Done。

P1-12 任务级审核与 Verification（2026-09-26 UTC，macOS/APFS）：GPT-6 Astra / `xhigh` 对干净 `main` 的精确实现提交 `e31bf72d14f95a5d195adf12fd2a7ebe8f77f5f2` 只读独立审核为 Approve；上轮三项问题均关闭，未发现新的可操作问题。审核者独立运行 62 项定向测试通过，并核对已记录的定向变异和归属解析 fuzz 证据；未重新运行变异、fuzz 或全量门禁。主 Agent 在审核通过后进入 Verification，以 release 配置重跑真实 CLI remove E2E 4 项、Application remove 13 项和 CLI 拒绝告警单测 1 项，全部通过；此前同一候选的 fmt、Clippy、全 workspace/all-targets 普通测试及适用定向变异、短预算 fuzz 结果见上文。P1-12 已完成本地提交与任务级验收，工作树无待清理任务数据；P1-13、P1-14、P1-15 仍在 Backlog。专用 submount、P1.e 小阶段受影响 crate 全量变异、阶段长预算 fuzz 和线上 CI/PR/push 未执行，本结论不放行 P1.e 或 Phase 1。

### 4.6 P1.f 契约与发布

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-14 JSON/错误码兼容 | 新手册、help、fixture 和退出码一致，旧参数无兼容入口 | P1-03、P1-06、P1-07、P1-09、P1-10、P1-12、P1-13、P1-16 | R3 | Done |
| P1-15 Phase 1 端到端与失败边界验收 | release 二进制、真实平台、长预算质量门禁和候选发布证据；不含中断恢复 | 全部未取消的前置 P1 任务 | R4 | Done |

小阶段退出：全部公开契约与发布二进制一致，未执行门禁和剩余风险已列出，进入人工阶段放行。

维护者于 2026-09-27 明确：Phase 1 达到可供人工检查的技术候选后即停止推进，交由维护者对结构做人工测试；不得据此提前进入 Phase 2，也不得把技术候选自动写成人工放行。P1-15 收口期间不创建线上 PR 或推送；技术候选收口结果见本节末尾。

候选结构轻量核对（2026-09-27）：`python3 tools/check_crate_dependencies.py` 对 7 个产品 crate 退出 0；产品源码搜索未发现 `thinwsd`、`ExecutionBackend`、`ChangeObserver`、`WorkspaceCheckpoint`、`JobState` 或 `Placement` 的提前实现，Core/Application/CLI/Port 中未见 Rust `unsafe` 块。此静态核对不代替真实运行、完整变异或维护者的人工结构测试。`CHANGELOG.md` 仍标 `Unreleased`，候选分发包尚未生成，不能当作正式发布。

同日供应链阶段复核：主 workspace 与 `fuzz/` workspace 分别执行 `cargo deny check`、`cargo audit --no-fetch`，四项均退出 0；deny 仅报告未命中许可配置 warning，主 workspace 的 audit 因并行读取 advisory DB 短暂等待锁后仍完成扫描。两份 lockfile 本轮未修改；这是当前依赖/本机离线 advisory 数据库的检查，不冒称线上 CI 或未来数据库状态。

P1-14 领取（2026-09-26，macOS/APFS）：主 Agent 在干净的本地 `main`（基线 `142ec74d5955f79c8963a6d325317f931a9c34ca`）实施，不创建线上 PR、不派生开发 Agent；冻结候选使用 GPT-6 Astra / `xhigh` 只读独立审核。风险 R3（脚本依赖的 JSON 字段、稳定错误码和退出码漂移）；主要写入区是 CLI 契约测试与必要的 CLI help/renderer 修正。验收断言：用户手册的全部公开子命令/JSON 支持矩阵与 help 一致，成功 envelope、错误 envelope 和退出码由可重复 fixture 精确覆盖；旧 Repository/Base、`workspace exec`、`gc`、`doctor --repair` 及其参数不能重新进入 CLI，均返回 E_USAGE；`workspace path` 维持原始字节 stdout 且拒绝 JSON。无意中改变产品状态、文件物化、Git 或删除策略均超出本任务范围。先写失败的契约断言，再做最小修正；真实发布配置和必要的 APFS E2E 用于 Verification。

P1-14 候选自测（2026-09-26，macOS/APFS）：`remove --help` 缺少手册已有的 `name:`/`id:` 消歧写法，新增测试先因精确短语缺失而 RED；只修正 clap 帮助文案后 GREEN。新增固定 FakeCommands fixture 逐命令检查成功 JSON envelope/顶层字段集、全表稳定错误码/退出码及完整错误 envelope、当前 help 命令树和旧命令/参数 E_USAGE；原有真实路径字节、JSON 拒绝及成功/失败 E2E 保留。用户手册现说明命令处于开发中、尚未发布；Changelog 移除旧“create 尚未接入”的过期表述。`cargo fmt --all -- --check`、全目标全 feature Clippy、`THINWS_P1_CROSS_VOLUME_ROOT=/Volumes/data cargo test --workspace --all-targets`、同环境 release `thinws-cli --tests`（契约 15 项、真实创建 3 项、初始化/doctor 6 项、查询 13 项、删除 4 项）、`cargo deny check` 和 `cargo audit --no-fetch` 均退出 0；deny 保留既有未命中许可 warning。生产代码只改由 doc comment 生成的 help，未改变解析器、状态或文件操作，因此没有受影响的生产变异目标或 fuzz harness，本任务不运行变异/fuzz。额外执行的 `python3 tools/check_crate_dependencies.py` 退出 1：既有 `thinws-adapter-macos` 生产依赖 `serde_json` 用于清理 JSONL，但脚本旧白名单未包含它；基线 `142ec74` 即如此，不是本任务引入，须另行修正后再用于阶段收口。P0 专用异卷/submount、阶段长预算门禁及线上 CI/PR/push 未执行；候选仍待精确提交只读审核与审核后的 Verification。

P1-14 首轮独立审核纠偏（2026-09-26）：GPT-6 Astra / `xhigh` 对精确提交 `e82a83c02791ab2019ee7ab1da439de4687618b4` 给出 Changes requested，两项 P2 均为测试假阳性风险：旧 `--repo` 拒绝用例同时缺少必填 `--source`；成功 JSON fixture 只核对顶层字段，无法发现 `materialization`、`fallback`、`doctor.host`、list 条目或 Git 仓库摘要的嵌套漂移。主 Agent 已给 `--repo` 用例补齐其他合法创建参数，并对嵌套结构同时断言完整字段和值；修正后 Debug/Release 契约各 15 项、fmt、Clippy 与带真实 P1 异卷参数的全 workspace/all-targets 普通测试均退出 0。审核者对旧提交的 Changes requested 不用于放行；修正候选须重新绑定精确 commit 只读复核，未改变生产代码、依赖或工具配置。

P1-14 任务级收口（2026-09-26）：GPT-6 Astra / `xhigh` 对精确修订提交 `e2fb8e7e20d552b9d4a3cbd9dde7cd632d7b91b4` 只读复核为 Approve，两项 P2 均已关闭；独立运行 CLI 契约 15 项及差异检查通过，未重复全仓或长门禁。主 Agent 随后在同一干净提交上以 `THINWS_P1_CROSS_VOLUME_ROOT=/Volumes/data cargo test --release -p thinws-cli --tests` 进行 Verification：CLI 契约 15、真实创建 3、初始化/doctor 6、查询 13、删除 4 项全部退出 0，含旧命令 E_USAGE、原始路径与真实 APFS/Git 场景。P1-14 当前验收断言已满足，任务 Done；既有依赖白名单错误留给 P1-15 阶段收口前修正，不把本次额外失败检查写成通过，也不宣称 P1.f/Phase 1 已放行。线上 CI/PR/push、完整小阶段变异及阶段长预算 fuzz 未执行。

P1-15 领取（2026-09-26，macOS/APFS）：主 Agent 在干净的本地 `main`（基线 `5265cc4db39e6ecc153daf88f6166b5c50456f65`）实施发布前验收，不派生开发 Agent、不创建线上 PR；独立审核使用 GPT-6 Astra / `xhigh` 只读复核冻结候选。风险 R4（阶段级真实删除/路径边界及高成本门禁）；先修复已证实的依赖检查白名单滞后，再逐项核对架构控制点、P1 小阶段退出条件、release E2E、失败注入、真实 APFS/Git、供应链、变异和 fuzz 证据。不得把任务 Done、普通测试通过或 P0 结果当成 P1 人工放行；无法执行的专用环境与长预算门禁必须列为缺口，而不能降低门槛。完成发布候选证据后才请求维护者放行，不自行打 tag、push 或宣布发布。

P1-15 依赖门禁修复切片（2026-09-26）：既有 macOS Adapter 在清理 JSONL 中使用生产 `serde_json`，但 crate 依赖检查脚本白名单仍拒绝该边，导致先前 P1-14 的额外检查退出 1；技术栈早已允许该日志用途。先把该依赖加入工具的合格图与平台边单测，两项均按预期 RED；仅扩充该 Adapter 的精确外部白名单后 GREEN，未放宽 Core/其他 crate 边界。`python3 -B -m unittest` 的 31 项 CI 辅助测试、`python3 tools/check_crate_dependencies.py`（7 个产品 crate）、fmt、全目标全 feature Clippy 和带真实 P1 异卷参数的全 workspace/all-targets 普通测试均退出 0。此切片仅修复既有门禁规则，不等于完成 P1-15 阶段验收；生产 Rust、依赖清单和公开契约未变，变异/fuzz 不适用，线上 CI 未执行。

P1-15 十副本控制点切片（2026-09-26，macOS/APFS）：新增注入 bootstrap 的真实 APFS CLI 集成测试，以同一非 Git 源目录创建 10 个 `cow-clone`/`confirmed` 工作区，各自改写普通文件，再改写源文件，逐一核对 10 份副本与来源互不覆盖；本次人工复查临时夹具目录为空。Debug 与 Release 定向测试均通过；包含此用例的全 workspace/all-targets Debug/Release 普通测试和 release workspace 构建均退出 0。随后外置 `/Volumes/data` 一度未挂载，`diskutil list external` 当时未显示目标设备；卷恢复后核对 APFS Volume UUID `1A42C888-32E3-489C-9BFA-67FD640A94E8`、工作树和夹具，并重跑 Release 定向测试再次通过，但耗时 61.40 秒，明显慢于恢复前约半秒。本测试通过 `thinws_cli::run()`，并非 release `thinws` 可执行文件黑盒；本证据只证明该次内容隔离，不证明二进制入口、稳定性能或卷连接可靠。P1-15 的 release 二进制端到端、其余门禁与环境稳定性仍待核对。

P1-15 发布候选门禁盘点（2026-09-26，盘点时提交 `bfd2e2686ceca8baae7629a1b1de5efe3e6875b0`）：GPT-6 Astra / `xhigh` 对十副本证据措辞修正提交只读复核为 Approve，确认其前次 P2 已关闭；这只批准该文档修正，不等于 P1-15 终审。`cargo build --release --workspace` 退出 0，产物 `target/release/thinws` 为 arm64 Mach-O，SHA-256 为 `f27833346fa2a1e3e7e7e5fda403cf5bcb470a9d7f8fa912c3c60a390c6f9dea`；直接运行该二进制的 `--help`、`--version`、`--json gc` 分别为正常帮助、`thinws 0.1.0`、退出 2 的 `E_USAGE`。这三项仅是无产品写入的黑盒入口检查，不覆盖真实创建/删除。主 workspace 与 fuzz workspace 的 `cargo deny check`、离线 `cargo audit` 均退出 0，仅有未命中许可配置警告；固定 nightly 下全部 fuzz target 的构建通过。`thinws_remove_request` 和 `thinws_create_request` 各运行 30 秒，分别约 460 万和 662 万次输入，均退出 0、无 crash/hang；这是两个短预算目标，不是所有目标的长预算阶段门禁。本次自动生成的未跟踪 fuzz 样本已从仓库清走，既有种子保留。

P1-15 在上述盘点时不能收口：P1.c、P1.d、P1.e、P1.f 的小阶段完整门禁尚未形成有效闭合证据；阶段全量变异仅枚举出约 3029 项，未执行；既有工作流虽提供显式 `workspace` 选择，但尚无该批次的有效 CI 结果，调整后的 360 分钟上限是否足够也未经证明。当时其余 fuzz target 的阶段长预算、发布二进制真实状态变更黑盒 E2E、专用子挂载的线上 CI 重验、外置卷稳定性复核、正式分发所需签名/公证及人工放行均未完成或尚未适用；后续变化见下方记录。不得以任务级定向变异、短预算 fuzz、进程内 CLI 集成测试或二进制帮助检查代替对应门禁；当前任务状态仍为 In Progress，不打 tag、不推送、不宣称 Phase 1 发布。

P1-15 CI fuzz 清单修正（2026-09-26）：既有 `P0/P1 quality` 工作流只对 10 个已声明 fuzz target 中的 6 个安排短预算 smoke。先增加从 fuzz manifest 对照工作流实际目标及次数的辅助测试并确认因缺 4 项而 RED，再为 init/create/remove 请求和物化路径补 4 个固定 nightly、60 秒预算的步骤并转 GREEN；32 项 CI 辅助测试、工作流 YAML 语法和差异检查通过。新增目标中 create/remove 各有上文 30 秒本机结果，init/路径各有 1 秒启动 smoke，均退出 0；这些不代替工作流的 60 秒在线运行或阶段长预算。GPT-6 Astra / `xhigh` 对精确提交 `cbd97449cefaf0e2afdd1ab8cc9bfba218e4c1aa` 只读审核为 Approve，独立重跑 32 项辅助测试，并验证清单测试能拒绝缺失、重复和未知目标；未运行在线 CI 或 fuzz。本修正只关闭 CI 短预算目标遗漏，不解决 P1-15 的全量变异、长预算和发布黑盒缺口。

P1-15 发布二进制黑盒 CI 切片（2026-09-26）：现有流程已用发布配置运行进程内 CLI 集成测试，但未从真实 `target/release/thinws` 进程完成状态变更闭环。新增仅供一次性 GitHub-hosted macOS runner 使用的 CI 用例，在独立 `RUNNER_TEMP` 下以 release 二进制执行 init、doctor、非 Git 来源 CoW 创建、普通路径输出、副本写入隔离及普通删除；每次子进程有 60 秒上限，运行前拒绝已有 bootstrap config 或符号链接。初版只核对 `GITHUB_ACTIONS`，无法排除持久自托管 runner；补 `RUNNER_ENVIRONMENT=github-hosted` 守卫，回归测试先对旧实现 RED，再对修订实现 GREEN。主 Agent 本机运行 33 项 CI 辅助测试、Python 语法、工作流 YAML、fmt、Clippy 和全 workspace/all-targets 普通测试均通过；本机入口按预期拒绝运行，当前用户的 ThinWorkspace config 未创建。GPT-6 Astra / `xhigh` 对完整提交范围 `b17e093..946074f3892d903dc2e0c6374e5322df2e8f463e` 只读复核为 Approve，独立重跑 33 项辅助测试并核对进程参数、字段及守卫；未执行会写真实配置的 E2E。CI 尚未触发，本切片只是补齐可执行验收入口，不是黑盒验收通过，也不放行 P1.f 或 Phase 1。

P1-15 全工作区变异 CI 时间边界（2026-09-26）：历史 P0 批次曾因 712 项变异触及 60 分钟上限，现有 120 分钟 job 可能不足以容纳约 3029 项。仅对手动 `workflow_dispatch` 且明确选择 `mutation_scope=workspace` 的运行，将 job 上限设为 GitHub-hosted 允许的 360 分钟；普通 push/PR 及手动 modules/crates 仍为 120 分钟，触发条件、变异选择和测试预算不变。精确提交 `aa9df46375d1327ff71c59844c6631a3bf3d8e99` 经 GPT-6 Astra / `xhigh` 只读审核为 Approve；本机 YAML 解析、11 项相关 CI 辅助测试和差异检查通过。未实际启动全量变异或在线 CI，六小时包括整个 job，不能据此承诺批次完成或阶段通过。

P1-15 长预算 fuzz 入口（2026-09-26）：本地提交 `74f35630bb72df0177776f8db0cfe7782f561a25` 增加仅可手动触发的 `P1 release fuzz` 工作流，不改变普通 push/PR 的短预算门禁；目标直接从 `fuzz/Cargo.toml` 读取并拒绝空/重复清单，当前 10 个目标各使用 300 秒预算及 5 秒单输入超时，任一失败或进程超时即停止且不得把余下目标标为通过。固定 nightly 与 `cargo-fuzz` 版本沿用 `tools/quality-tools.toml`；失败时保存默认 `fuzz/artifacts/` 崩溃样本 14 天。主 Agent 本机运行 39 项辅助测试、工作流 YAML 解析、fmt、全目标全 feature Clippy、带真实 P1 异卷参数的全 workspace/all-targets 普通测试、依赖方向检查和差异检查，均退出 0。GPT-6 Astra / `xhigh` 对精确提交只读审核为 Approve，独立验证上述 39 项辅助测试及成功/失败调度路径。120 分钟 job 上限含安装、构建和 50 分钟目标预算，不保证执行完成；长预算 fuzz、线上 CI 和 artifact 上传均未实际运行，P1-15、P1.f 与 Phase 1 均未因此放行。

P1-15 真实子挂载补验（2026-09-26，macOS/APFS）：主 Agent 在 `/private/tmp` 建立一次性 128 MB APFS 镜像作为源目录唯一的 `mounted` 子目录，实际父/子设备号为 `16777229`/`16777250`；P0 的 Clone 与 Full Copy 两项真实子挂载拒绝测试通过。产品 `thinws-adapter-macos` 新增 Clone 和 Full Copy 各一项 ignored 集成测试，均从真实 PlatformProbe → Plan → Materializer 路径验证 `UnsupportedSourceEntry`，来源与目标同设备、子挂载异设备，失败回执无 created 项，目标和 staging 为空；Full Copy 只为选择后端合成 clone-unsupported 预检事实，路径证据仍来自真实 Probe。两项在 Debug/Release 配置下均通过。CI 的既有临时 APFS 镜像步骤现在同时提供 `THINWS_P1_SUBMOUNT_SOURCE`，但线上 CI 尚未执行。fmt、全目标全 feature Clippy、带 `/Volumes/data` 异卷参数的全 workspace/all-targets 普通测试和差异检查通过。GPT-6 Astra / `xhigh` 对 Clone 提交 `4619108` 只读审核为 Approve；Full Copy 首版 `7fdf88e` 因未限定来源唯一条目获 Changes requested，修订 `789de08` 补齐夹具断言后获 Approve。两次本机镜像均已先卸载再删除，仅清理本次专用临时文件；本证据关闭本机真实子挂载测试缺口，不代替小阶段全量变异、线上 CI 或 Phase 1 放行。

P1-15 阶段长预算 fuzz 本机结果（2026-09-27 UTC，候选 `0fb87a5`，macOS 15.7.2/arm64）：使用固定 `nightly-2026-08-14` 和 `tools/ci_release_fuzz.py`，于 00:12:28–01:02:47 UTC 顺序运行 manifest 中全部 10 个 target，每个 300 秒、单输入超时 5 秒；进程 exit 0，汇总为 10 passed、0 failed/timeout，日志含 10 组最终执行统计，未产生 crash artifact，工作树无新增 corpus。忽略目录中的 `target/p1-15-release-fuzz.AZm5mL/summary.md` 与 `run.log` SHA-256 分别为 `55833f8c269756fc91edbebec8c4d926e2dec3b86a6a6e85da66523bbcf0b429`、`7fcb275bbb7bbba23e4905e3b1b738e048e0d82d7922beb9fb42ed2e394ac830`；这是本机完整长预算证据，不冒称线上工作流执行。后续 P1-15 CI 测试夹具修正未改变生产代码、fuzz target、种子或工具版本；按《任务流程》§18.1 仍需在最终候选核对该证据的适用性。

P1-15 CI 异卷前置修正（2026-09-27 UTC）：只读终检发现现有工作流只导出 P0 异卷根，P1 Probe/Store 在变量缺失时静默跳过；CLI 用例则假设 checkout 与系统临时目录天然异卷。先加工作流变量联动断言并确认因缺少 P1 导出而 RED，CLI 用例在缺少明确异卷根时也按预期 RED；随后将 P1 根绑定同一专用 APFS 镜像，并使三项 P1 用例在 `--include-ignored` 下必须具备该环境，不再以缺变量返回绿色。CLI 夹具把 data root 放在规范化的系统临时根、source 放在专用卷，先断言设备不同，再核对 `--allow-copy` 仍返回 `E_DATA_ROOT_LAYOUT` 且未创建 Workspace。第一次真实运行因 `/var` 符号链接使 `init` 提前拒绝，规范化临时根后通过；本机 `/Volumes/data` 异卷定向三项在 Debug/Release 均通过。再用一次性 128 MB APFS 镜像复现 CI 布局（系统卷、checkout 卷、镜像卷设备号分别为 `16777229`、`16777240`、`16777250`），全 workspace/all-targets `--include-ignored` Debug/Release 均 exit 0、0 failed/ignored；镜像已确认卸载并清理。41 项 CI 辅助测试、fmt、全目标全 feature Clippy、普通全 workspace/all-targets 测试、YAML 语法与差异检查亦通过；两份完整平台日志保存在忽略目录 `target/p1-15-ci-mount-{debug,release}.log`，SHA-256 分别为 `dc4cfddb4eb6135934530753e84e72532496b626455692b39ef1db64eb8c2655`、`6205e0e410060d3ff31cdb0cc22b90277a8805ceb1d10fd9b86f4c44e2303d3d`。线上 CI/PR/push 仍未执行，这项修正不能代替阶段变异、二进制黑盒执行、最终独立审核或人工放行。

P1-15 本机 release 二进制黑盒补验（2026-09-27 UTC，候选 `9552d9b`，macOS/APFS）：直接调用现有 `target/release/thinws`（SHA-256 `f27833346fa2a1e3e7e7e5fda403cf5bcb470a9d7f8fa912c3c60a390c6f9dea`），只为子进程设置一次性 HOME，数据根也位于独立临时目录；来源是同一 `/Volumes/data` APFS 卷上的 `docs/project/reference`，两端设备号均为 `16777240`。实际完成 `--json init`、`--json doctor`、`--json workspace create`、普通 `workspace path`、副本新增文件与源隔离、`--json workspace remove`，逐项断言退出成功、`cow-clone/confirmed`、路径吻合、非强制删除及副本目录消失；本机闭环通过。它不冒称 GitHub-hosted CI 已运行，也不测试签名、公证或其他平台；一次性 HOME/data root 在核对后清理。

P1-15 阶段变异启动异常与诊断（2026-09-27 UTC，候选 `9552d9b`）：首次全工作区 3029 项批次在未变异基线构建时由 Apple clang 链接 `serde_derive` 触发段错误，约 66 秒终止、exit 4，`outcomes.json` 记录 0 项实际变异；该次不提供任何变异通过证据。失败结果与基线日志保存在忽略目录 `target/p1-stage-mutation-full-20260927/mutants.out`，对应 SHA-256 为 `c6d172167205108b7fdaa00aa05729b3c52178b2400e4ab6932e50f452f8ac09`、`e6510fcfd718e28e49ab74f8ac7b695d8163d24634bcf620b1ced3db79ed6515`。磁盘余量和当前内存检查未显示耗尽；随后以 `--jobserver-tasks 4 --jobs 1`、相同平台夹具运行 3 项小范围诊断，未变异基线通过，2 caught、1 unviable、0 missed/timeout、exit 0，结果 SHA-256 `fb2ac2ff4780cba1fe304b7ad4909c5540ffc94c22b08f1070d2986ee4acee80`，位于 `target/p1-stage-mutation-smoke-20260927/mutants.out`。这只能证明本次有界构建成功，不能代替阶段全量门禁；重试须另存结果且保持候选冻结。

P1-15 全量变异重试启动记录（2026-09-27 02:27:41 UTC）：主 Agent 负责，生产代码候选仍为 `9552d9b`（其后仅有任务证据文档提交）；在 macOS 15.7.2/arm64、同一专用 APFS 镜像的 P0/P1 跨卷及子挂载夹具上，对全 workspace 3029 项使用 `cargo-mutants 27.1.0`、`--test-workspace true`、`--include-ignored`、每次 cargo 命令 60 秒上限、`--jobs 2 --jobserver-tasks 4`。新结果位置为 `target/p1-stage-mutation-full-20260927-retry1/mutants.out`，标准输出和错误写入同级 `run.log`；不覆盖首次失败证据。首次主动查看暂定启动后约 1 小时，再根据该批实际进度与 P0 历史耗时调整；提前完成或失败事件可立即处理，不能把等待时间当执行时长。

P1-15 全量变异重试终止结果（2026-09-27 UTC）：03:28 UTC 首次检查时执行器仍活跃，完成 403/3029 项，346 caught、57 unviable、0 missed/timeout；据此仅预计约 10:20 UTC 再查，未按固定短间隔轮询。用户于 05:32 UTC 要求进度时，统一执行器句柄已终止，exit 101；结果文件停在 547/3029 项：458 caught、68 unviable、21 timeout、0 missed，`end_time` 为空，`run.log` 末尾是连续的 60 秒测试超时，`debug.log` 在后续 Build 阶段中断，没有完整收口摘要或可确认的单一退出原因。三个文件 SHA-256（`outcomes.json`、`run.log`、`debug.log`）依次为 `11843ab659ef397d174c1b11d829d35cc4fa41a40c0875581b74ef300c0142a3`、`6bc17072d89f493c7011cf7ada9280ea10a7fdeec169cd9b8af9993fe5366308`、`1b7f3929c7b78448abfa89a66a5d8ae04014942e951530870ccb2218d64d5b6e`，均保留在上述忽略目录。27 个父进程已退出、命令行精确指向本批两份 scratch tree 的 `tail -f` 测试辅助进程已逐个验证并发送 TERM，复查无残留；一次性 128 MB APFS 镜像经精确关联核对、卸载后删除，构建 scratch tree 已由执行器移除，日志未删。原 10:20 UTC 单次跟进已撤销。此批不是完整阶段变异证据，不复用 21 个 timeout 为通过；先检查超时是否来自对每个变异重复跑全部 workspace（含不相关 P0 真实平台测试）及执行器意外终止边界，再按受影响测试集拆分补齐，不原样重启 3029 项。

P1-15 已闭合的包级子集（同批结果，2026-09-27 UTC）：`thinws-adapter-git-cli` 当前生产源码枚举 408 个变异，与上述 `outcomes.json` 中该包 408 个变异名称排序后的 SHA-256 均为 `5a7b2c2538e2cd75c1eea234273f32a83addf93aaf24758c390645203d7a51b1`；结果为 351 caught、57 unviable、0 missed/timeout。该包在冻结源码和原整仓测试集下已覆盖全部当前枚举，不因外层批次提前中止而重做这 408 项；变更相关测试、工具或平台条件时须按《任务流程》§18.1 重新核验可复用性。其余 2621 项不能由此推断为通过。

P1-15 超时项定向诊断与回归（2026-09-27 UTC）：重试中 `thinws-adapter-macos/src/document.rs:175:9` 的 `||`→`&&` 原为 timeout；对应日志在执行与该解析器无关的 P0 `same_volume` 测试时达到 60 秒上限，因此不能把该超时当作解析器断言已覆盖。改用 `--test-package thinws-adapter-macos,thinws-application,thinws-cli`、90 秒上限、不带 `--include-ignored` 对同一变异单项诊断，基线通过，结果为 missed（exit 2，`target/p1-stage-timeout-diagnostic-20260927/mutants.out/outcomes.json` SHA-256 `cfadb78c9aeb76e1e1f1f92bb832440869d14722a609dbca0a9fc889e45ac17c`）。原有 ownership 测试没有单独覆盖 container/root 创建秒数为负的情况；在 Adapter 单测中分别加入两项断言后，原实现测试通过，同一变异、同一三包验证范围复测为 1 caught、0 missed/timeout（exit 0，`target/p1-stage-timeout-diagnostic-fixed-20260927/mutants.out/outcomes.json` SHA-256 `bd3aaf0798fd80d7029cdd2ca2c4e0a0a103dab67c52fe17bb581ad414cb9846`）。fmt、全目标全 feature Clippy、全 workspace/all-targets 普通测试和对应 bootstrap document fuzz 30 秒短预算均退出 0、未见 crash；fuzz 默认生成的 2308 个未跟踪临时语料已核对后清理，原有两份受版本控制的种子未改。此证据仅关闭这一个原超时变异，不推断其余 20 个 timeout 或剩余 2621 个变异通过；全量阶段门禁仍未完成。

P1-15 相邻 ownership 守卫补验（2026-09-27 UTC）：在上述本地提交 `47835fd` 上，使用同一三包测试集、不含 ignored 用例筛出 `document.rs:172–175` 的 12 个变异，未变异基线通过；首轮为 8 caught、4 missed、0 timeout，exit 2（`target/p1-stage-ownership-branch-diagnostic-20260927/mutants.out/outcomes.json` SHA-256 `cc224ffdaf2511bb0fefd9e7a4045f7a0fde41afb7435ea8d86d8753468d801e`）。四个 missed 分别是 container/root inode 为零时的单因素拒绝，以及对应 birth seconds 为零时的合法接受。新增逐字段普通测试并确认原实现通过后，仅补跑这四个精确变异：4 caught、0 missed/timeout，exit 0（`target/p1-stage-ownership-branch-followup-20260927/mutants.out/outcomes.json` SHA-256 `8bbbf8098ba1b103a32d8f98be40b8cb7e961c34e1619f74045d44d02b9c86f2`）。这两批按同一变异名称合并仅覆盖 12 个目标，不把重复项或其他未运行的变异记为完成；新增测试仍需通用门禁和独立审核。

P1-15 可复用的 Phase 0 实验包证据核验（2026-09-27 UTC）：`phase0` tag 至当前提交的 `experiments/p0/probe/`、`experiments/p0/materialize/`、固定 Rust 工具链和质量工具版本均无差异；`Cargo.lock` 只增加新包及依赖，未更改旧包版本，`.cargo/mutants.toml` 只迁移 Git Adapter 排除路径，原 P0 Probe/Materialize 排除项不变。产品 crate 不依赖这两个实验包，实验依赖仅 `materialize → probe`。当前 `cargo-mutants 27.1.0` 分别枚举 Probe 244 和 Materialize 313 项，排序名称 SHA-256 与 Phase 0 完整结果中的对应包精确相同：`cf381b43378d13850395347f46e00d593d708e9be4844c1746d3c4ce8b408dda`、`7c290e0aa7eedacd4f7478021a04a2b45920b05b72d77a7224d97002bac63d6b`。原完整结果对应为 Probe 177 caught＋51 unviable＋16 timeout、Materialize 267 caught＋46 unviable；原 Probe 16 个 timeout 名称与 Phase 0 补跑 16 个 Probe 名称精确一致，补跑全部 caught。沿用 [Phase 0 阶段状态](../review/ThinWorkspace_Phase0阶段状态_v1.0.md)已人工放行的本机 macOS/APFS 证据，这 557 项可按《任务流程》§18.1 复用为 460 caught＋97 unviable、0 未解决项；不据此宣布当前 3029 项全工作区变异已完成。原两个 `outcomes.json` SHA-256 为 `09d0dba9f290b0aa9c2f46b2524ba84ff04e92779f3dcf70f689576cc1e745e2`、`b01bb098aa788518443ebd7b5c78a6d0cc12fa0e0da34d94c5f4b219d6de4a16`。

P1-15 当前 P0 Cleanup 包变异（2026-09-27 UTC）：剩余实验清理包不沿用迁移前的 Phase 0 结果，而在当前源码上独立枚举并执行 23 项；`--package thinws-p0-cleanup --test-package thinws-p0-cleanup --gitignore true --timeout 90 --jobs 2 --jobserver-tasks 4`，未变异基线通过，约 23 秒完成，14 caught＋9 unviable、0 missed/timeout，exit 0。当前枚举与本批结果的变异名称排序 SHA-256 同为 `0f171101566a49eb61286ff09521f98909d10cf42e029b9894768a484fb2c1a4`；`target/p1-stage-p0-cleanup-mutation-20260927/mutants.out/outcomes.json` SHA-256 为 `0b393fb671f3a07def4503be15c29d702cc4e5fa566a950fffde0dbb7a8870fb`。此包没有 P1 产品反向依赖，验收只声称这 23 项当前包级结果；不推断其他包已通过。

P1-15 SQLite 元数据包阶段变异启动登记（2026-09-27 06:03 UTC）：候选为本地 `main` 提交 `6973d47`，执行负责人主 Agent，macOS arm64、Rust 1.97.1、`cargo-mutants 27.1.0`；当前枚举 `thinws-metadata-sqlite` 共 169 个变异。计划以 `--package thinws-metadata-sqlite --test-package thinws-metadata-sqlite,thinws-application,thinws-cli --gitignore true --timeout 90 --jobs 2 --jobserver-tasks 4` 运行，不带 `--include-ignored`；验证包覆盖该 Adapter 自身及实际反向依赖，真实平台 ignored 场景已有独立阶段测试，不在每个变异上重复跑。结果写入忽略目录 `target/p1-stage-metadata-sqlite-mutants-20260927/mutants.out`，不覆盖既有批次。参考同机同包 40 项约 53 秒但本批并发减半、验证包增加，首次主动查看保守预计启动后约 8 分钟；提前完成或失败可按事件处理，不短间隔轮询。此登记不表示批次已经通过。

P1-15 SQLite 元数据包首批结果（2026-09-27 UTC）：未变异基线通过；按计划首次查看时 88/169 项已执行、4 missed、0 timeout，依进度只再安排一次约 8 分钟后的检查。终态约 15 分钟、exit 2：169 项为 125 caught＋36 unviable＋8 missed、0 timeout，`target/p1-stage-metadata-sqlite-mutants-20260927/mutants.out/outcomes.json` SHA-256 为 `551373cb42c5ef097c83561f6a54b49de3f241db4f3d361fdcc17d8b2c9aae04`。存活项分别位于既有/未来 schema 错误分类、创建/失败回执 JSON 字段与失败类型字符串；本批不是包级通过证据。先按权威 schema 与 Receipt 契约逐项确认真实缺口或等价，再补精确测试及受影响位置，不重启已闭合的 161 项。

P1-15 SQLite 首批存活项定向补测启动登记（2026-09-27 UTC）：在首批生产源码未变的候选上，新增两条集成测试，分别核对 `inspect/open_existing` 对未来与旧 schema 的错误种类，以及真实 SQLite 中运行时 Full Copy 回退的非 UTF-8 创建对象、失败尝试与失败类型 JSON；原实现两项均通过。只筛首批 8 个 missed 的精确变异名称，以相同三包测试集、`--gitignore true --timeout 90 --jobs 2 --jobserver-tasks 4` 复测，结果独立写入 `target/p1-stage-metadata-sqlite-followup1-20260927/mutants.out`。参考 169 项约 15 分钟，本批 8 项首次主动查看保守预计约 2 分钟；如仍有等价或漏检，保留失败结果并只对未解决项再处理。

P1-15 SQLite 存活项补测结果与等价证明（2026-09-27 UTC）：8 项约 2 分钟结束，6 caught、2 missed、0 timeout/unviable，exit 2；`target/p1-stage-metadata-sqlite-followup1-20260927/mutants.out/outcomes.json` SHA-256 为 `131e075665b6a981b4e1e4935f17b7508807e1f12cae699122ce246ac1bebf62`。新集成测试使 `validate_existing_database` 的 `>→==/<` 及四个回执 JSON helper 变异被捕获。余下两项 `>→>=` 属等价变异：`open_or_initialize` 的 `(APPLICATION_ID, SCHEMA_VERSION, _)` 精确匹配分支先于 `version > SCHEMA_VERSION` 守卫，因此相等值永不进入该守卫，其他值上 `>` 与 `>=` 同值；`validate_existing_database` 的比较只在 `user_version != SCHEMA_VERSION` 分支内求值，因此相等值被排除，其他值上两式同值。该证明仅针对这两个精确变异和现有分支顺序，不把工具 exit 2 改写为全绿。fmt、全目标全 feature Clippy、全 workspace/all-targets 普通测试和差异检查退出 0；新增测试不改变生产逻辑或 fuzz harness，沿用未变条件下的阶段 fuzz 证据。GPT-6 Astra / `xhigh` 对补测代码和当时计划差异（SHA-256 `d0c8fffc0098c864aa33506b1a01e0216bab39434e5bbd018e33a952b85093f6`）只读审核为 Approve，独立运行两项新测试，确认两项等价证明；未重跑完整变异批。审核后主 Agent 的 Release 定向 Verification 为 2 passed、0 failed。169 项按 131 caught＋36 unviable＋2 经审核等价闭合；与此前已闭合的 1114 项合计覆盖 1283/3029 项，另 1746 项仍待处理，不能据此放行 Phase 1。

P1-15 Application 包阶段变异启动登记（2026-09-27 06:37 UTC）：本地 `main` 冻结候选 `c0d2483`，执行负责人主 Agent，macOS arm64、Rust 1.97.1、`cargo-mutants 27.1.0`；当前枚举 `thinws-application` 217 项。验证集为该包及唯一产品反向依赖 `thinws-cli`，参数 `--test-package thinws-application,thinws-cli --gitignore true --timeout 90 --jobs 2 --jobserver-tasks 4`，不在每个变异上重复运行已独立通过的 ignored 真实平台场景；产物写入忽略目录 `target/p1-stage-application-mutants-20260927/mutants.out`。参考 SQLite 169 项约 15 分钟并为 Application 测试规模留余量，首次主动查看预计启动后约 25 分钟；提前完成或失败才按事件处理，不短间隔轮询。本登记不是通过证据。

P1-15 macOS Adapter 后续批次预选核对（2026-09-27 UTC，尚未启动）：当前枚举 1037 项；原整仓中断批的本包 139 项为 107 caught＋11 unviable＋21 timeout，已另以定向结果捕获其中 8 项 timeout。当前加入的 `document.rs` 测试使原已闭合项中 25 个变异名称行号后移 63 行（包括未闭合 timeout 在内的原 139 项中共 26 个名称后移）；逐项映射后，118 个原已闭合项和 8 个补测项均与当前枚举名称精确对应，合并为 126 项。使用这 126 个完整名称构造精确 `--exclude-re`，当前 `cargo mutants --list --package thinws-adapter-macos --exclude-re …` 仅列出剩余 911 项；此为候选筛选校验，不是对 911 项的执行或通过声明。正式启动前须重新核对源码、测试、工具和名称集合，避免旧行号或重复项造成漏测。

P1-15 Application 包首批结果（2026-09-27 UTC）：未变异基线通过；实际 06:37:41–06:50:12 UTC，约 12 分 31 秒完成，预定约 25 分钟后首次读取时已终止，exit 2。217 项为 137 caught＋66 unviable＋14 missed、0 timeout；`target/p1-stage-application-mutants-20260927/mutants.out/outcomes.json` SHA-256 为 `a6b52942a30d45b427df3869881670520283a9ae53d1fbb1ea0ed69122cb1220`。存活项集中在初始化发布后 Ready marker 状态/身份校验、Application 错误显示与端口错误码映射，以及创建预览降级原因；本批不能记为通过。先对照权威契约与精确变异区分测试缺口、不可达或等价，再只对未解决项补测，不重启已闭合的 203 项。

P1-15 Application 首批存活项补测启动登记（2026-09-27 07:06 UTC）：仅增强 Application 单测/集成测试，不修改生产分支；按权威 init、错误码和预览契约，补发布后 marker 的状态/身份各自无效时不得发布 config、Port stage 映射及安全诊断文本、`UseCaseError` Display 和预检降级原因访问器。原实现在新增单元/集成定向测试中 7＋10 项通过，其中新测试 4 项；原首批 14 个 missed 的完整名称构造精确筛选，`cargo mutants --list` 核对恰为 14 项，不重跑同位置已 caught 的另外 2 项。继续使用 Application＋CLI 验证包、90 秒单命令上限、2 jobs/4 jobserver tasks；结果另存 `target/p1-stage-application-followup1-20260927/mutants.out`。参考同包 217 项约 12 分半，14 项首次主动查看约 3 分钟后；成功前不把这 14 项记为闭合。

P1-15 Application 存活项补测终态（2026-09-27 UTC）：实际 07:06:50–07:08:11 UTC，约 81 秒，预定约 3 分钟后首次读取已退出 0；14 caught、0 missed/timeout/unviable。补测 caught 名称与首批 14 missed 名称排序比较完全一致，没有靠重跑其它位置抵消存活项；`target/p1-stage-application-followup1-20260927/mutants.out/outcomes.json` SHA-256 为 `5433cc85b8989a277c767aac83c893b63c255ded75b149e424b99060d3887158`。两批合并按 217 个独立变异计为 151 caught＋66 unviable、0 未解决项；工具首批 exit 2 仍如实保留。补测仅加强测试，生产分支、依赖和 fuzz harness 未改；任务通用门禁、精确独立审核及审核后 Verification 尚待完成，不因本包闭合宣布小阶段或 Phase 1 放行。

P1-15 Application 候选自测与设计语义纠偏（2026-09-27 UTC）：新增测试后 `cargo fmt --all -- --check`、`git diff --check`、全目标全 feature Clippy、带 `THINWS_P1_CROSS_VOLUME_ROOT=/Volumes/data` 的全 workspace/all-targets 普通测试均退出 0。后者不带 `--include-ignored`，不将跨卷/子挂载用例冒称本轮重跑；真实平台专项仍引用前述独立结果。生产代码、依赖、fuzz target/种子未改，故本轮不重跑长预算 fuzz。审核文档时发现物化设计 §5.3 的“Plan 过期后 Application 回到 Probe/Plan”与同文件“用户下次新建才重探测”相冲突，已在原权威位置澄清：本次创建非 Ready 失败；仅已获准且回滚确认的 CoW→Full Copy 降级允许同请求重新探测。此为现有行为的文字纠偏，不新增自动重试；候选仍待 GPT-6 Astra / `xhigh` 对精确本地提交只读审核。

P1-15 CLI 包阶段变异启动登记（2026-09-27 07:15 UTC）：本地 `main` 候选 `fb0ece0`，执行负责人主 Agent，macOS arm64、Rust 1.97.1、`cargo-mutants 27.1.0`；`thinws-cli` 当前枚举 59 项，无产品反向依赖，验证包仅 `thinws-cli`。使用 `--test-package thinws-cli --gitignore true --timeout 90 --jobs 2 --jobserver-tasks 4`，不在每个变异上运行已单独通过的 ignored 平台专项；结果写入忽略目录 `target/p1-stage-cli-mutants-20260927/mutants.out`。参考同机 Application 217 项约 12 分半、但 CLI 集成测试实际耗时不同，首次主动查看预计启动后约 8 分钟；提前完成/失败事件可提前处理，不短间隔轮询。本登记不表示 59 项已执行或通过；Application 精确审核与审核后 Verification 仍并行待完成。

P1-15 Application 审核及审核后 Verification（2026-09-27 UTC）：上述补测和设计文字澄清作为本地提交 `fb0ece0a4034efb4fcba868b61983f2ff704553b` 固定。独立 GPT-6 Astra / `xhigh` 只读审核结论为 Approve、无阻塞项；审核分别核对首批及补测结果哈希、14 个 missed 与 follow-up caught 名称逐项相等、对应测试断言和物化设计的 Plan 过期语义，未把部分包结果冒称整个阶段通过。审核指出两处 P3 计划措辞：首批存活项应明确是已发布 Ready marker 的状态/身份校验；macOS 原 139 项中共 26 个名称后移，25 个仅指原已闭合项。已在本计划原位置修正，未改变代码或历史测试结果。主 Agent 随后执行 `cargo test --release -p thinws-application --lib --test init_doctor`，退出 0，7 个单测和 10 个集成测试全部通过。此 Verification 仅覆盖本次受影响的 Application 测试，不代替剩余包的变异或 Phase 1 阶段门禁。

P1-15 CLI 包首批结果（2026-09-27 UTC）：未变异基线通过；实际 07:15:30–07:18:03 UTC，约 2 分 33 秒，预定约 8 分钟后首次读取时已终止，exit 2。59 项为 42 caught＋12 unviable＋5 missed、0 timeout；`target/p1-stage-cli-mutants-20260927/mutants.out/outcomes.json` SHA-256 为 `864c2dc27761b6d6dc1b4574a6be4afcedcbb81ee47729eb214fe1b8a28ef1ae`。四项 missed 为公开 Adapter/Fallback 名称函数的返回值替换，两项一组；第五项为 `main.rs` 对系统时钟无效时的 `-1` 哨兵删除负号。原 `thinws-core::UnixMillis` 要求非负，Application 将负值映射到使用错误；正常系统时钟无法触发该异常路径，需提供可注入时间的最小测试缝隙并验证，而非宣称等价。先补精确测试，只复测未解决项及因最小重构而移动的受影响位置，不重跑已闭合的其余 54 项。

P1-15 CLI 存活项补测启动登记（2026-09-27 UTC）：新增 CLI 单测分别断言两个 Adapter 和两个 Fallback 的全部稳定名称；仅为测试系统时钟异常分支，把入口原有毫秒转换表达式原样提为 `unix_millis_or_invalid(SystemTime)`，在 binary 单测覆盖纪元前、纪元和纪元后。原实现的 3 个 lib 单测、1 个 binary 单测通过，fmt 和差异检查退出 0。由于提取使 `main.rs` 的变异名称/行号改变，复测范围为 lib 的原 4 个 missed 名称加当前 `main.rs` 全部 5 个枚举项（包括原已 caught 的 `main` 替换和新增 helper 返回替换）；`cargo mutants --list` 将精确筛选为 9 项。继续使用 CLI 验证包、90 秒单命令上限、2 jobs/4 jobserver tasks；独立结果放入 `target/p1-stage-cli-followup1-20260927/mutants.out/mutants.out`（执行器在所指定输出目录下再次创建同名子目录）。首批 59 项约 2 分半，9 项首次主动查看暂按约 2 分钟，未完成前不宣称 CLI 包闭合。

P1-15 CLI 存活项补测终态（2026-09-27 UTC）：实际 07:23:59–07:24:37 UTC，约 38 秒，预计约 2 分钟后首次读取时已退出 0；9 caught、0 missed/timeout/unviable。`target/p1-stage-cli-followup1-20260927/mutants.out/mutants.out/outcomes.json` SHA-256 为 `2df6a8a93c91e84a079b1f0e36706c530623fef010385c179071afdc6b56c530`。当前 CLI 枚举因提取入口时钟 helper 从 59 变为 62；首批原已闭合 54 名称与补测 9 名称合并后，对当前 62 名逐项取交集覆盖全部，唯一不再存在的旧名称是移动前的 `main` 替换，已由当前 `main` 替换重新捕获。按当前枚举计为 50 caught＋12 unviable、0 未解决项；原首批 exit 2 保留。相应全工作区当前总枚举由 3029 增为 3032；这只结案 CLI 包，不代表 Ports、Core 或 macOS Adapter 已通过，候选仍待通用门禁、审核和 Verification。

P1-15 CLI 补测候选自测（2026-09-27 UTC）：`cargo fmt --all -- --check`、`git diff --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings` 及 `THINWS_P1_CROSS_VOLUME_ROOT=/Volumes/data cargo test --workspace --all-targets` 均退出 0；后者未启用 ignored 真实子挂载/跨卷用例，沿用前述独立平台结果，不将本轮普通测试冒称其复跑。变更只在 CLI 名称映射测试和入口时钟最小可测化，未改依赖、fuzz harness、平台 Adapter 或物化/删除逻辑；先前阶段长预算 fuzz 仍需在最终候选按 §18.1 核验可复用性。候选待精确本地提交、独立 GPT-6 Astra / `xhigh` 只读审核和审核后 Release Verification，不据此宣布 Phase 1 放行。

P1-15 Ports 包阶段变异启动登记（2026-09-27 UTC）：本地 `main` 候选 `3f4782d`，执行负责人主 Agent，macOS arm64、Rust 1.97.1、`cargo-mutants 27.1.0`；当前 `thinws-ports` 枚举 56 项。Port 的具体类型与方法被 Application、CLI 及三个产品 Adapter 消费，验证集选这六个产品包：`thinws-ports,thinws-adapter-macos,thinws-adapter-git-cli,thinws-metadata-sqlite,thinws-application,thinws-cli`，不在每个变异上重复 ignored 的真实子挂载/跨卷场景；参数仍为 `--gitignore true --timeout 90 --jobs 2 --jobserver-tasks 4`。结果放入忽略目录 `target/p1-stage-ports-mutants-20260927/mutants.out`，不覆盖既有批次。参考 SQLite/Application 包实际用时约 15/12 分半，本批虽仅 56 项但验证包更广，首次主动查看保守预计启动后约 25 分钟；如提前完成或失败按事件处理，未读结果前不宣称 Ports 通过。

P1-15 CLI 独立审核与审核后 Verification（2026-09-27 UTC）：GPT-6 Astra / `xhigh` 对本地提交 `3f4782dec68004a57973087a28d0a7d2ecb1800f` 只读审核为 Approve，无阻塞代码或变异证据问题；独立重跑 CLI lib 3 项和 binary 1 项、核对两份 outcomes 哈希/日志、当前 62 项名称集合及全 workspace 3032 项枚举。审核提出一项 P3 文档缺口：用户手册 §10.1 原仅展示 `apfs-file-clone`，未冻结 `full-copy` Adapter 和两个 `fallback.reason` 稳定值；这不改变本次现有程序行为，但使公开 JSON 契约依据不完整。主 Agent 已仅在用户手册权威位置补充模式、Adapter、成功 CoW 与 fallback 的取值和 null/预检/运行时语义，未向设计文档复制。审核后 `cargo test --release -p thinws-cli --lib --bin thinws --test contract` 退出 0，CLI lib 3、binary 1、契约 15 项全部通过。文档补充尚待独立复核，不把此定向 Verification 代替剩余 Ports/Core/macOS 变异或 Phase 1 放行。

P1-15 CLI 公开枚举文档复核（2026-09-27 UTC）：上述用户手册与计划补充作为本地提交 `1190869c6b7110e808ecef35bec35cb2b5761fdb` 固定；同一 GPT-6 Astra / `xhigh` 审核者再次以只读方式核对精确文档差异，结论 Approve、无新 finding，前轮 P3 已关闭。审核确认成功回执的 `cow` 没有混入失败回执的 `unknown`，dry-run 只报告预检降级，运行时降级仍须确认回滚；未重复运行 Release 测试或其它长门禁。此文档复核不改变 CLI/Ports 生产源码或测试，也不表示 Phase 1 阶段审核完成。

P1-15 当前 release 二进制无状态入口检查（2026-09-27 UTC，候选 `60cd1429ed7d56d285c7eff1b7944ae8b2739311`）：`cargo build --release -p thinws-cli` 退出 0，`target/release/thinws` 为 arm64 Mach-O，SHA-256 `70a570ba1995a64742d5e37fbbca985bda59c8fa0b1afc09126d6c5a7fec1b6c`。直接运行该文件的 `--version` 返回 `thinws 0.1.0`、`--help` 返回当前命令树，`--json gc` 退出 2 且为 `E_USAGE` envelope；三项均不触发产品数据写入。`codesign -dv` 显示仅 ad hoc linker-signed、无 TeamIdentifier；它不是正式签名/公证的可分发 Release。此入口检查本身不覆盖状态变更；后续同一二进制的本机补验见下段。最终候选若代码改变还需重建、重验和记录新哈希；入口检查不替代 Ports/Core/macOS 变异、签名、公证或人工放行。

P1-15 当前 release 二进制本机状态变更黑盒补验（2026-09-27 UTC，同一二进制 SHA-256）：测试前确认 `target/p1-15-release-e2e-local-20260927` 不存在，只在该忽略目录创建一次性 HOME 和 data root；该 HOME 下先执行 `--json doctor`，确认为 E_NOT_INITIALIZED，避免触碰用户已有实例。来源 `docs/project/reference` 与测试根设备号同为 `16777240`，直接运行二进制完成 `--json init`、`--json doctor`、`--json workspace create --source ... --name release-e2e`、普通 `workspace path` 和 `--json workspace remove release-e2e`，均按各自契约成功。创建结果为 `cow-clone/apfs-file-clone/confirmed`、无 fallback；创建前源/副本 `技术栈.md` SHA-256 同为 `6b1004838cb2c761ba6246dfc192757c1433ed9e2fa57c66d1bc55dce36b64a2`，仅对副本追加测试标记后副本变为 `bcb22d36dab1c1015439e12944931f21f50923c4455de81971496de0950158c4`，来源仍为原哈希。普通 remove 返回 `forced=false`、`result=removed`，副本 root 不存在；operations 日志的 started/completed 两条事件均记录 `process_use=scan-incomplete`。`warning=process-scan-incomplete` 是手册 §6.4 允许但必须显示/记日志的尽力探测结果，不冒充无人占用证明。一次性 HOME/data root 目前保留在上述忽略目录供本机复核，未触碰用户配置；此本机闭环不冒称 GitHub-hosted CI、签名/公证或其他平台验收，最终生产代码若变还需按 §18.1 判断是否重验。

P1-15 阶段长预算 fuzz 适用性中途核验（2026-09-27 UTC）：相对 10×300 秒完整运行的候选 `0fb87a5`，当前 `Cargo.lock`、`fuzz/`、固定工具链和 `tools/quality-tools.toml` 未变；产品源文件差异除 CLI 入口时钟表达式原样提成可测 helper 外，均为 `#[cfg(test)]` 或集成测试修改，不改变 fuzz target 可达的生产路径。CLI binary 的入口 helper 不由 fuzz target 调用。因此已有无 crash/hang 的长预算结果在当前候选上仍可按《任务流程》§18.1 复用；Ports/Core/macOS 后续修正若触及生产逻辑、依赖或 fuzz harness，最终候选必须重新核对并补跑受影响目标，本项不是提前宣布阶段门禁通过。

P1-15 Ports 包首批终态及测试集核对（2026-09-27 UTC）：未变异基线通过，实际 07:30:18–07:37:46 UTC、约 7 分 28 秒结束；预定约 25 分钟后首次读取时进程已退出 0。当前 56 项枚举与结果名称逐项相等，为 23 caught＋33 unviable、0 missed/timeout；`target/p1-stage-ports-mutants-20260927/mutants.out/outcomes.json` SHA-256 `c15b6441a408c78c0d01f60ef66496ec899611fbecc37312c4f6c7e40da5a163`。执行器的未变异 baseline 只运行被变异包，不能用该 argv 判断正式变异的测试集；实际 caught 变异的 `phase_results[].argv` 包含启动登记的 Ports、两个 Adapter、Metadata、Application 和 CLI 六包，`debug.log` 亦列出同一显式集合。独立抽查 Application、SQLite 和 macOS 结果中的实际变异 argv，也确认各批所登记的逗号分隔多包范围有效，无须重跑。另用一个已知 unviable 的 Ports 变异做参数诊断，重复传入 `--test-package` 同样在正式变异的构建 argv 中展开三包；该诊断只验证工具语义，不作为新覆盖项计数。Ports 56 项已闭合，当前合计 1618/3032 项；剩余 Core 503＋macOS Adapter 911 项待执行。

P1-15 Core 包阶段变异启动登记（2026-09-27 08:01 UTC）：本地 `main` 冻结候选 `c465e7e`，执行负责人主 Agent，macOS arm64、Rust 1.97.1、`cargo-mutants 27.1.0`；`thinws-core` 当前枚举 503 项。为覆盖领域类型的全部产品消费者，本批使用 `--package thinws-core --test-package thinws-core,thinws-ports,thinws-adapter-macos,thinws-adapter-git-cli,thinws-metadata-sqlite,thinws-application,thinws-cli --gitignore true --timeout 90 --jobs 2 --jobserver-tasks 4`，不把独立通过的 ignored 真实平台场景乘入每项变异。结果另存忽略目录 `target/p1-stage-core-mutants-20260927/mutants.out`。同机 Ports 56 项、六包验证实际约 7 分半，Core 503 项和七包验证按比例约 67 分钟，留波动余量后首次主动查看预计约 80 分钟后；提前完成/失败事件可立即处理，正常运行不按短间隔轮询。本登记仅为执行计划，不表示 Core 已通过。

P1-15 发布前独立预审（2026-09-27，候选 `e0ea79b`）：GPT-6 Astra / `xhigh` 只读抽查长预算 fuzz、真实平台 Debug/Release、Ports/Application/CLI 变异证据的哈希及适用性，未发现新的高危数据安全缺陷；未提前读取正在运行的 Core 批次。预审指出手册 §6.3 将 force 的保护拒绝笼统称为 `refused`，而归属/路径预检失败实际记录 `failed`；主 Agent 已在手册原权威位置区分 Git/占用策略拒绝、预检失败和非 Ready 普通清理，不修改产品代码或第二份日志定义。预审仍将剩余 Core 503＋macOS Adapter 911 项及可重复性能基线列为阶段缺口；十副本隔离测试不等于稳定性能证明。签名、公证和人工放行另为正式发布条件，本预审不构成最终审核或阶段放行。

P1-15 性能基线准备（2026-09-27，尚未运行测量）：新增仅用于本机验收的 `tools/p1_perf_baseline.py`，在指定 APFS 卷的独立临时 HOME/data root 内构造可重复的 264 个普通文件、约 65 MiB 来源，用 release CLI 顺序创建 3 轮、每轮 10 份 CoW 工作区，并记录逐次耗时、分位数、二进制/数据集摘要、卷身份与仅供参考的空间变化；不设置未经实测的性能承诺。两项工具单测先因模块缺失按预期 RED，再 GREEN；`tools` 下 43 项 Python 单测、fmt、全目标全 feature Clippy、带 P1 异卷环境参数的全 workspace/all-targets 普通测试及差异检查均退出 0。普通测试没有启用 ignored 的专用子挂载/跨卷用例，沿用此前独立平台结果；本切片不改 Rust 生产逻辑、依赖或 fuzz 路径，已闭合的变异和长预算 fuzz 不因增加基准工具而失效。Core 变异运行期间只准备工具、不执行基准；须待 CPU/I/O 空闲后运行并核验结果及外置卷波动，再决定可否形成 P1 发布指标。本准备不将 #14 或性能门禁记为完成。

P1-15 性能基线工具与日志文案独立复核（2026-09-27）：GPT-6 Astra / `xhigh` 对冻结提交 `ef0778192fe60e8778854070ea3a7b4eff0779dc` 只读审核为 Approve、无 finding；独立运行两项新增单测和差异检查通过。审核确认临时 HOME/data root、APFS/CoW 断言、排他输出和观测统计的边界，以及手册的 `refused`/`failed` 区分；未运行正式性能测量、长测或远端操作，未读取 Core 变异结果。本审核只批准该工具与文案提交，不构成 P1-15 或 Phase 1 放行。

P1-15 首发支持信息收拢（2026-09-27）：技术栈要求发布产物带支持矩阵和已知限制；仅在公开契约权威《Phase 1 用户操作手册》末尾增加候选资格表与现有章节的限制索引，不新建或复制另一份产品事实。表中已测组合仍标“阶段放行中”，其他系统/版本不冒称已支持，明确正式签名、公证和人工放行之前不是正式发布。纯文档变更不运行 RED、变异或 fuzz；`git diff --check`、fmt、全目标全 feature Clippy 及带 P1 异卷环境参数的全 workspace/all-targets 普通测试均退出 0，ignored 的专用场景仍使用此前独立证据。本项不改变代码、依赖或长预算证据适用性，也不代替最终候选产物整理。

P1-15 首发支持信息独立复核（2026-09-27）：GPT-6 Astra / `xhigh` 对冻结提交 `fbe166de77cc8af2b38fc8819dca9516f950cdd1` 只读审核为 Approve、无 finding；独立差异检查通过。审核确认矩阵没有把实测资格、未验证组合和未实现平台混写，也没有把候选冒称正式发布；未重复运行普通测试、读取 Core 长测结果或执行远端操作。本审核只覆盖该纯文档提交，不构成最终阶段审核或人工放行。

P1-15 Core 包首次延后查看（2026-09-27 09:21 UTC）：统一执行会话仍在运行；阶段结果文件已有未变异基线和 245/503 项正式变异，其中 122 caught、61 unviable、58 missed、4 timeout，`end_time=null`，不能记为 Core 通过。约 80 分钟处理 245 项，按剩余 258 项和已观察吞吐估算还需约 84 分钟，留波动后下一次主动查看暂定 10:55 UTC，不恢复短间隔轮询。已出现的 missed 集中于 Git porcelain 解析及物化证据访问器；4 项 timeout 均为同一 Full Copy fallback 前置条件变异，日志显示测试已执行到最后的 Port 契约包，暂不能把 90 秒累计测试上限直接解释为目标函数死循环或“等价”。冻结候选的 Rust 源码/测试在本批终态前不修改；届时先读取完整摘要，再只为未闭合变异补精确测试并按真实影响范围复测，不重启已闭合项。

P1-15 Core 包中止与后续范围（2026-09-27 10:42 UTC）：用户询问进度后提前读取，结果已达 409/503 项：186 caught、112 unviable、58 missed、53 timeout，另有 94 项未执行。大量后续 timeout 的阶段日志显示七包验证接近或触及每条变异的累计 90 秒命令上限；该上限不足以区分测试慢与目标挂起。主 Agent 对本次批次先 INT、再 TERM、最后 KILL，确认无遗留变异子进程，保留 `target/p1-stage-core-mutants-20260927/mutants.out/outcomes.json`（SHA-256 `e232cb918ccd3bb5ca334acde87833efbc917a17e388b1670507ee0dfe630365`）。该文件 `end_time=null`，运行退出 137，**不是完整批次通过结果**。后续先补 58 项存活的契约测试，再对存活、超时和未执行的当前变异精确筛选，采用经基线实测足够的验证时限；298 项已闭合结果仅在逐项核对生产代码、测试集与变异位置后复用，不整批重启。

P1-15 本机性能基线实测（2026-09-27，非并发长测）：在 `/Volumes/data` 外置 USB APFS 卷，以 release 二进制 SHA-256 `70a570ba1995a64742d5e37fbbca985bda59c8fa0b1afc09126d6c5a7fec1b6c` 对 264 文件、68,157,440 逻辑字节的固定夹具顺序执行 3 轮×10 份 CoW 创建，30 次均完成；单次耗时 min 342.697 ms、median 352.009 ms、p95 441.310 ms、max 443.397 ms。夹具 manifest SHA-256 `6b6ac5c03232b2ffff92944c1b95dca5280644ce46d2415edf2ad877a9dec8c8`，结果位于忽略目录 `target/p1-15-perf-baseline-20260927.json`，文件 SHA-256 `aa7259a6260facd955c2f8cd2b161728f99a7226b1be599429db31cf16059efa`。容器余量变化只作背景观测，不能直接等同 CoW 实际占用或形成磁盘节省保证；本机一次测量不构成跨机器延迟承诺，也不代替最终候选的门禁、签名/公证或人工结构测试。

P1-15 Core 存活项定向补测启动（2026-09-27 10:51 UTC）：只增补 `thinws-core` 的 Git porcelain-v2 边界/错误测试及平台证据访问器测试，不改生产函数、依赖或 fuzz 路径；首次运行暴露两处测试夹具预期错误，修正为 Git 合法 `S.MU` 和非法 `S....` 后原实现的 Git 7 项、物化领域 8 项全部通过，fmt 与全目标 Clippy 退出 0。原 58 项 missed 已是旧测试未能拒绝错误实现的 RED 证据；用其完整名称生成精确筛选，并以 `cargo mutants --list` 确认为 58 项。验证包沿用原七包集合，只将经日志证实不足的单变异累计命令上限从 90 秒提高到 180 秒，仍为 2 jobs/4 jobserver tasks，不乘入 ignored 平台测试。结果另存 `target/p1-stage-core-missed-followup-20260927/mutants.out`，原 409 项及中止证据不覆盖。原批 409 项约 2 小时 40 分且有 53 个 90 秒超时；此批如均被新增的前置测试快速捕获可能远短于按比例的 23 分钟，暂保守约 40 分钟后首次主动查看，不频繁轮询；未得到终态前不宣称 58 项已闭合。

P1-15 Core 首轮 58 项定向终态（2026-09-27 11:15 UTC）：实际约 24 分钟，执行器 exit 2；54 caught、4 missed、0 timeout/unviable，`end_time` 有效；`outcomes.json` SHA-256 `cbe1f17fdbd3b857731059a0d12f5565372e2ab5ced3cbc55af6610a5b427f90`。四个存活项精确为重命名来源路径后偏移的 `+→*`、非跟踪记录路径校验的 `||→&&`、非跟踪记录最小长度 `<→<=`、header 最小长度 `<→<=`。其余原 58 项中 54 项已被七包测试捕获；此结果不是 Core 503 项闭合。只为四处补较长非法路径、单字符合法路径/header、重命名后仍有普通记录的解析断言，并把原硬编码来源长度的测试改为按 NUL 定界；原实现 Git 7 项、fmt、全目标 Clippy 均退出 0，不改生产函数。下一批仅对四个完整名称精确复测，原 54 项不重启；结果仍须独立保存和核对。

P1-15 Core 四项复测存储异常（2026-09-27 11:25 UTC）：同一七包测试集、180 秒单变异上限和 2 jobs 启动后，未变异基线通过，但两个 worker 在首两项的 Build/Test 日志写入处同时遇 `Input/output error (os error 5)`；执行器 exit 1、`end_time=null`、正式变异结果 0/4，`target/p1-stage-core-missed-followup2-20260927/mutants.out` 只作为失败诊断，**不是变异闭合证据**。`/Volumes/data` 和系统临时卷均仍有约 210/253 GiB 可用，外置 APFS 卷在线且非只读；本批留下一个精确指向 `cargo-mutants-worktree-ZQOr1n.tmp` 的孤儿 `fifo_probe_child`（PID 44805，PPID 1），主 Agent 已核对命令后 TERM 并复查进程消失。`diskutil verifyVolume /Volumes/data` 以只读 live mode `fsck_apfs -n -l` 完成，文件系统检查 exit 0、卷报告 appears to be OK；这不能排除瞬时 USB/设备 I/O 故障。下一步先用普通定向测试确认当前写入可用，再用更保守单 worker、内部卷结果目录对原四项复测；若再现 EIO，停止长批次并将硬件/存储问题交维护者处理，不把失败解释为代码测试红线或盲目重试。

P1-15 Core 四项复测闭合（2026-09-27 11:30 UTC）：外置卷上 `cargo test -p thinws-core --test git_status` 7 项通过后，使用系统内部卷 `/private/tmp/thinws-core4.Za6UMR` 存放结果、`--jobs 1 --jobserver-tasks 2`、原七包测试集和 180 秒上限精确重跑 4 项；未变异基线通过，约 3 分钟终态 exit 0，4 caught、0 missed/timeout/unviable。实际结果名称与前批 4 个 missed 排序完全一致；`outcomes.json` SHA-256 `d36fdfa4d5a5c07fb89aade27df9f8cdd8d6da5658007bbf42cd2ad612193f40`，复制到忽略目录 `target/p1-stage-core-missed-followup2-internal-20260927/mutants.out` 后哈希不变。首批 58 个 missed 已由 54＋4 个精确 caught 补齐，生产源码未改；原 409 项中的 53 timeout 与 94 项未执行仍未闭合，Core 503 不能记为完成。内部卷成功不证明外置卷瞬时 EIO 根因已消除；后续长批次继续先做小范围并发诊断，复发即停。

P1-15 Core 剩余批次双 worker 诊断启动（2026-09-27 11:32 UTC）：从原 53 项 timeout 和 94 项未执行中各取一个完整名称，`cargo mutants --list` 精确确认为 2 项；用原七包验证集、180 秒上限、`--jobs 2 --jobserver-tasks 4`，将日志与结果写入系统内部卷 `/private/tmp/thinws-core-smoke.GWuVkp/mutants.out`。该诊断只用于确认双 worker 在内部卷能完成并观察 90→180 秒的有效性；两个样本不能推断其余 145 项已通过。若再发生 EIO，不继续启动大批量变异。

P1-15 Core 双 worker 诊断终态（2026-09-27 11:33 UTC）：未变异基线通过，2 项约 79 秒完成，执行器 exit 2：`MaterializationReceipt::physical_bytes` 返回值变异 caught，原超时的 `materialization.rs:862:13` `||→&&` 为 missed，无 timeout/EIO。`outcomes.json` SHA-256 `a64671d6f7475864e4363e0202f97da8def95703d6c5e4bfa0fdb1afcbde7ee3`，已复制至忽略目录 `target/p1-stage-core-remaining-smoke-20260927/mutants.out` 并核对哈希。对存活项的初步结构分析：`MaterializationPlan` 私有字段仅由 `for_apfs_clone`（`fallback_reason=None`、`failed_attempts=[]`、`effective_mode=CowClone`）与 `full_copy`（`fallback_reason=Some`、`effective_mode=FullCopy`）构造；因此当前可达对象中 `fallback_reason.is_some()` 会先被同一 guard 的 `effective_mode != CowClone` 拒绝，此 `||→&&` 疑似等价。须由独立 Reviewer 核对构造路径及测试可达性后才能作为精确等价项结案；不把工具 missed 改写为 caught。内部卷双 worker 样本没有复现 EIO，但仍不足以宣称外置存储可靠；原 53 timeout 中除该项外还有 52 项、原 94 未执行中除已 caught 一项外还有 93 项，共 145 项待执行。

P1-15 Core 剩余 145 项启动登记（2026-09-27 11:38 UTC）：生产源码仍与首批 `c465e7e` 相同，仅增加 Core 测试和任务状态文档；主 Agent 从原 53 timeout＋94 未执行中扣除已诊断的 2 个完整名称，`cargo mutants --list` 精确筛为 145 项。验证仍覆盖 Core、Ports、macOS、Git、SQLite、Application、CLI 七包；`--timeout 180 --jobs 2 --jobserver-tasks 4`，不重复 ignored 真实平台专项，结果/日志写入系统内部卷 `/private/tmp/thinws-core-remaining.gbHLKy/mutants.out`，原失败与成功批次均保留。参考 58 项同类七包运行 24 分钟、且本批含更多复杂物化分支，首次主动查看暂定约启动后 65 分钟（约 12:43 UTC），提前失败或用户问询可提前处理；不短间隔轮询。该 145 项结果和上述疑似等价项终审未出前，Core 503 仍未闭合。

P1-15 Core 精确等价项独立复核（2026-09-27 UTC）：GPT-6 Astra / `xhigh` 对 `materialization.rs:862:13` 的 `||→&&` 存活项只读复核，结论为当前合法公开 API 下的等价变异，无可操作 P0–P2 finding。`MaterializationPlan` 字段私有，CoW 构造恒为 `fallback_reason=None` 且 `failed_attempts=[]`；Full Copy 构造虽可有原因/历史，却已被同一 guard 的先行 `effective_mode != CowClone` 拒绝，所以此处 `reason.is_some() || !attempts.is_empty()` 与 `reason.is_some() && !attempts.is_empty()` 对可达输入结果相同。Reviewer 同时抽查新增 Git/物化测试断言，无误。原工具结果仍保留 `missed`，以此处证明单独结案而非改写为 `caught`；若未来开放新的 plan 构造、反序列化或变更先行模式检查，必须重新证明。审核未运行新的测试或修改文件，仅覆盖该单项，不代替剩余 145 项、其他包或最终阶段审核。

P1-15 macOS Adapter 剩余范围复核（2026-09-27 UTC，Core 长测期间只读准备）：使用原 126 项已闭合名称的精确排除式，当前 `cargo mutants --list --package thinws-adapter-macos` 剩余 911 项。为使失败或存活项能分批定位，`--file` 预选验证四组互不重叠：`materializer.rs` 276 项，`ffi.rs`＋`filesystem.rs` 297 项，`store.rs`＋`probe.rs` 216 项，其余七个源码文件 122 项，合计 911；各组在正式运行时仍使用同一 126 项排除式及 Adapter/Application/CLI 反向依赖测试集。此次仅核对枚举与筛选，没有启动 macOS 变异；Core 批次未终态前不并发占用其构建资源，正式运行前须重核源码和枚举。

P1-15 Core 剩余 145 项终态（2026-09-27 12:23 UTC 完成，按约定 12:43 UTC 首次读取）：未变异基线通过；实际 11:38:24–12:23:49 UTC，约 45 分 25 秒，执行器 exit 2。结果为 103 caught＋29 unviable＋13 missed、0 timeout；`end_time` 有效，系统内部卷无本批 EIO。`outcomes.json` SHA-256 为 `3d5f39243b48bbdf8702794921d86c13e6d1db024ab4b4725cce54da654c6ea4`，已复制到忽略目录 `target/p1-stage-core-remaining-20260927/mutants.out` 且哈希相同。13 个存活项中，`RelativePath::try_from_bytes` 的四个布尔条件变异及 `MaterializationAttemptEvidence` 的四个访问器返回值变异显示具体测试缺口；另五个 Plan/Receipt 守卫变异需先独立核对可达性，不预先标为等价。当前 Core 503 项仍未闭合；只对八个明确缺口补测试并精确复测，再处理五个守卫项，不重跑已闭合项。

P1-15 Core 八项存活补测启动（2026-09-27 12:45 UTC）：在生产源码未变的候选上新增 `RelativePath` 单因素非法组件表及 `MaterializationAttemptEvidence` 非默认字段访问器断言；原实现定向测试 10 项通过。旧批这八项 `missed` 是失败实现逃过测试的 RED 证据，新增测试不改产品行为、公开契约或 fuzz harness；fmt 与差异检查均通过。用八个完整变异名称构造精确筛选，当前 `cargo mutants --list` 恰列出八项。执行负责人仍为主 Agent；沿用 Core 与六个反向依赖包、180 秒上限、2 jobs/4 jobserver tasks，内部卷结果位置 `/private/tmp/thinws-core8.7ynL7u/mutants.out`。参考刚完成的 145 项约 45 分钟及新增前置测试，本批首次主动查看预计启动后约 8 分钟，不频繁轮询；未得到终态前不把八项标为闭合。

P1-15 Core 另外五个守卫存活项独立审核（2026-09-27 UTC）：GPT-6 Astra / `xhigh` 在只读候选 `db145b9817f75fe8e9ea74ae7fe76b669f02f598` 上逐项核对 `materialization.rs:859:13`、`:860:13`、`:861:13`、`:881:13` 与 `:1608:13` 的 `||→&&`，均接受为当前合法公开 API 下等价，未发现可达反例；相关生产文件未变。前三项依赖 Plan 仅有私有字段与三条构造路径，CoW 固定为 CoW/APFS/无降级历史，Full Copy 固定为 FullCopy/FullCopy/有降级原因，完整守卫对两类对象均保持同一结果；`:881` 的新旧卷比较在四路径证据全等检查及双方单卷布局核验之后，两边差异不可能只发生一项；`:1608` 的 Adapter/有效模式成对固定，同真同假。工具原结果仍是 `missed`，不改记 `caught`；未来若开放独立修改 Plan 字段、放宽路径/同卷核验或新增模式与 Adapter 不一致的构造入口，必须重新证明。审核没有编辑、运行测试或读取八项正在执行的进度，仅关闭五个精确等价项，不替代八项复测或最终阶段审核。

P1-15 Core 八项精确复测及包级闭合（2026-09-27 UTC）：内部卷批次 12:45:42–12:49:12 UTC 约 3 分半完成，未变异基线通过，执行器 exit 0、8 caught、0 missed/timeout/unviable；按预定约 8 分钟后首次读取，没有短轮询。`outcomes.json` SHA-256 为 `265aeb64bd6726e8ccfa3d55dec756c2676b6426466c6a87fa2637e890a5faeb`，复制到忽略目录 `target/p1-stage-core8-followup-20260927/mutants.out` 后哈希不变。八个 caught 名称与原 145 项批次的八个相应 missed 完全相等。将原 409 项、58 项补测、四项补测、两项诊断、145 项剩余批及此八项复测按精确名称合并，当前 Core 503 项与结果名称集合完全相同，0 缺失、0 过期；每项取最新有效结果为 **356 caught＋141 unviable＋6 个经规定模型审核的等价 missed**，无未处置 missed/timeout。六个等价项的适用前提见上文两次独立审核；不将工具 `missed` 重写成 `caught`，也不把部分批次 exit 2 冒称单批全绿。Core 包阶段变异证据已闭合，但最终候选的通用门禁、精确提交审核及 macOS Adapter 911 项仍待完成，不能因此宣布 Phase 1 完成。

P1-15 Core 闭合提交及本地门禁（2026-09-27 UTC）：上述仅测试与计划变更已本地提交为 `75004d5`（父 `db145b9`），没有创建远端 PR 或 push。提交前 `cargo fmt --all -- --check`、全目标/全 feature Clippy、`THINWS_P1_CROSS_VOLUME_ROOT=/Volumes/data cargo test --workspace --all-targets`、7 个产品 crate 依赖方向检查及差异检查均退出 0；普通全仓测试保留需专用挂载的 ignored 场景，不能冒称本轮重跑。规定模型对精确提交的独立只读审核已派发，审核结果及审核后 Verification 尚待记录；生产源码、依赖、fuzz harness 未变，先前阶段长预算 fuzz 的适用性最终仍须核对。

P1-15 macOS Adapter 剩余第一组启动登记（2026-09-27 12:57 UTC）：在本地 `main` 候选 `75004d5` 上，当前 Adapter 枚举 1037 项，精确排除此前闭合 126 项后剩余 911 项；第一组以 `--file` 选 `lib.rs`、`document.rs`、`lock.rs`、`operation_log.rs`、`process.rs`、`space.rs`、`volume.rs`，再次列举为 122 项。执行负责人主 Agent，使用 `--package thinws-adapter-macos`、同一 126 项 `--exclude-re`、Adapter/Application/CLI 三包验证集、`--timeout 180 --jobs 2 --jobserver-tasks 4 --gitignore true`；结果放在系统内部卷 `/private/tmp/thinws-macos-remaining1.lecHCa/mutants.out`，不覆盖旧证据。与 Core 批次不并发；其余三组 297＋276＋216 项待本组完成后顺序执行。按此组复杂 OS 测试及此前批次吞吐，首次主动查看保守暂定约启动后 35 分钟，实际用时将用于重新估算下一组；不按短间隔轮询，未完成前不推断这 122 项通过。

P1-15 Core 闭合提交独立审核（2026-09-27 UTC）：GPT-6 Astra / `xhigh` 对精确本地提交 `75004d57d76d7a2383d1049d0d6c7d18ed15b4f6` 相对父 `db145b9817f75fe8e9ea74ae7fe76b669f02f598` 只读审核为 **Approve**，Critical/High/Normal/Low finding 均为 0。审核者独立核对六批结果哈希、当前 503 项集合、逐项最新结果、七包实际验证 argv 与失败日志，确认 356 caught 均因测试失败、141 unviable 均有编译诊断，六个等价 missed 的构造不变量未失效；两份新增测试文件共 17 项定向测试及差异检查通过。审核未重复变异、fuzz、全仓或阶段门禁，也未读取正在运行的 macOS 批次进度；主 Agent 的审核后 Release Verification 待 macOS 第一组不再占用构建资源时补做。此审核只批准 Core 测试与证据提交，不表示 P1-15 或 Phase 1 放行。

P1-15 macOS Adapter 第一组终态（2026-09-27 UTC）：未变异基线通过，实际 12:57:27–13:07:16 UTC 约 9 分 49 秒；按预定 13:32 UTC 首次读取时进程已退出 2。122 项为 104 caught＋12 unviable＋6 missed、0 timeout；`outcomes.json` SHA-256 `b023f1dd926a7ebac3c81c07aaed34344812472bf132c63f8d6b00879cb2a4ce`，复制到忽略目录 `target/p1-stage-macos-remaining1-20260927/mutants.out` 后哈希相同。六个存活项分别为仅在 `cfg(fuzzing)` 下编译的 ownership fuzz 桥、十六进制组合 `|→^`，以及四个外部占用安全判断/不完整状态累积条件；本批不能视为闭合。前两项需对照既有精确排除惯例和数学等价证明，后四项不得因实机竞态难复现而直接宣称等价或删除保护无关；先保持冻结生产源码运行其它未覆盖组，再对照安全语义设计可复验测试，必要最小改动须按 §18.1 重核受影响的旧变异证据。

P1-15 macOS Adapter 第二组启动登记（2026-09-27 13:33 UTC）：在同一冻结本地候选 `75004d5` 上，第一组实测 122 项约 10 分钟但包含较多简单条件；第二组仅 `ffi.rs` 与 `filesystem.rs`，原排除式＋两个精确 `--file` 重新枚举 297 项。仍由主 Agent 顺序执行，同一 Adapter/Application/CLI 三包测试集、180 秒上限、2 jobs/4 jobserver tasks、内部卷输出 `/private/tmp/thinws-macos-remaining2.Nc8oHU/mutants.out`。按第一组吞吐作基础并为 FFI/文件系统构建和波动留余量，首次主动查看暂定约启动后 35 分钟（约 14:08 UTC）；完成/失败事件或用户询问可提前处理，不短轮询。第一组六项由规定模型独立只读分析，与本组执行不并发修改生产源码。

P1-15 macOS 第一组六项独立归因（2026-09-27 UTC，尚未补测）：GPT-6 Astra / `xhigh` 只读核对精确结果及生产路径，结论为 **一项严格等价、一项 fuzz oracle 范围排除、四项真实测试缺口**，未批准本组闭合。`document.rs` 的 `decode_hex` `|→^` 因高/低四位不重叠而对全部已验证输入等价；`lib.rs` 的 ownership fuzz 桥只在 `cfg(fuzzing)` 下编译，是测试 oracle 而非普通产品构建行为，须按既有 Git fuzz 桥惯例精确排除，不能冒称跨构建数学等价，且生产解析/编码与长预算 fuzz 仍保留。`process.rs` 的根设备/inode 单因素不匹配、PID/UID 单因素不匹配和先前扫描不完整被零 FD 结果清除，均会改变 `InvalidLayout` 或 `scan-incomplete` 语义；只读审核提出仅在本模块提取现有局部判断并使用真实临时目录 metadata/单因素身份故障注入，以及直接调用既有 `inspect_process` 的非法 PID＋零 FD 分支，避免竞态或全机扫描假阳性。此处只是归因和拟补测边界；待其余冻结批次结束后由主 Agent 核验并实施，变更导致的行号/新增变异必须重新枚举，不静默复用旧名称。审核没有编辑、运行测试或查看第二组进度。

P1-15 macOS 第二组首次延后查看（2026-09-27 14:11 UTC）：统一执行会话仍在运行，结果文件 `end_time=null`，已完成 184/297 项：97 caught＋16 unviable＋71 missed、0 timeout，不能记为此组闭合。已见存活项集中于 FFI 包装、libproc 返回量、flags、路径/进程边界，需在终态后逐项区分等价、实际测试缺口及不可达条件；不以临时计数提前改写生产代码。约 37 分钟执行 184 项，剩余 113 项按当前吞吐约需 23 分钟，为尾部复杂度留余量后下一次主动查看暂定约 14:40 UTC，不短间隔轮询。

P1-15 macOS Adapter 第二组终态（2026-09-27 UTC，维护者询问进度时读取）：实际 13:34:32–14:26:10 UTC 约 51 分 38 秒，执行器 exit 2，未变异基线通过；297 项为 179 caught＋39 unviable＋79 missed、0 timeout。`outcomes.json` SHA-256 为 `61a37d1d8582f470e8b8256236d6a7766257e1ee5c4c2e179eca8cbab65dde77`，复制到忽略目录 `target/p1-stage-macos-remaining2-20260927/mutants.out` 后哈希一致。79 项主要落在 FFI 的 libproc 缓冲边界、打开 flags、系统事实与私有路径验证，另有 `filesystem.rs` 八项；不得把旗标 OR/XOR 的可能等价性推断到所有 `missed`，也不因部分 FFI 异常难通过真实内核复现而忽略安全边界。本批仅覆盖冻结候选的这 297 项；剩余 `materializer.rs` 276 项和 `store.rs`＋`probe.rs` 216 项仍未执行，第一组四个真实测试缺口也未闭合。继续顺序枚举未执行组，相关测试设计由独立只读审核辅助归因，主 Agent 不在其运行期间修改生产源码。

P1-15 macOS Adapter 第三组启动登记（2026-09-27 14:41 UTC）：冻结候选仍为本地 `75004d5`，原 126 项精确排除式＋`--file crates/thinws-adapter-macos/src/materializer.rs` 当前列举 276 项。继续由主 Agent 顺序执行 Adapter/Application/CLI 三包验证、180 秒单变异上限、2 jobs/4 jobserver tasks、`--gitignore true`，结果写系统内部卷 `/private/tmp/thinws-macos-remaining3.UbDE81/mutants.out`，不覆盖前两组。第二组 297 项实际约 52 分钟，第三组按 276 项与物化集成测试规模保守估计首次主动查看约启动后 55 分钟（约 15:36 UTC）；到时若仍运行再据实际吞吐重估，不短轮询。本批结束前不修改冻结生产源码，不以第二组的 79 个存活项阻断尚未执行范围的枚举。

P1-15 `filesystem.rs` 八个存活项独立归因（2026-09-27 UTC，尚未补测）：GPT-6 Astra / `xhigh` 在冻结候选 `75004d5` 上逐项只读审核，确认 **3 项严格等价＋5 项真实测试缺口**，无未决项。固定 Rust 1.97.1 的 Unix `Path::components()` 仅在迭代器 `StartDir` 状态产出一次 `RootDir`，随后转入 `Body`；故 `prepare_private_directory` 的 `RootDir if !saw_root`→`true`，以及 `open_absolute_directory_nofollow` 的 `RootDir if !saw_root && !saw_normal`→`true`、该守卫内 `&&`→`||` 在现有输入域上等价。若改用自定义组件流、保留标志重启遍历或平台组件模型改变，须重证这三项。其余五项不得排除：`permits_current` 的 inode 与 birthtime 单因素条件可直接用不同身份值测试；`prepare_private_directory` 的相对路径守卫变异虽仍在尾部报错，却可能在报错前于受控绝对位置创建目录，须断言无副作用；根目录终结守卫变异须核对精确拒绝操作而非仅断言 `is_err()`；`historical_directory_identity` 两个 `||`→`&&` 均由“仅 UID 不匹配、纳秒合法”的事实区分，拟从现有 `fstat` 判定提取本模块私有纯谓词，用真实 `Stat` 的单字段故障注入测试，不引入新的 Port 或通用 mock。五项补测尚未执行；待冻结批次结束后实施、重枚举受影响目标并完成精确复测。审核未编辑文件、运行测试或读取第三组进度。

P1-15 `ffi.rs` 71 个存活项独立归因（2026-09-27 UTC，尚未补测）：GPT-6 Astra / `xhigh` 只读审核绑定候选 `75004d5` 和第二组结果哈希 `61a37d1d8582f470e8b8256236d6a7766257e1ee5c4c2e179eca8cbab65dde77`，逐项分类为 **14 项严格等价的互斥 flag `|→^`、2 项仅在当前 libproc 成功返回 ABI 下差异不可达、50 项有确定性测试反例、5 项尚须边界测试或进一步证明**；未批准本组闭合。主 Agent 已在锁定的 `libc 0.2.189` Darwin 常量中复核 `O_SEARCH`、打开标志、卷属性位及 R/W/X 权限位互斥的依据。优先通过同文件局部容量/列表解码、路径与卷能力解码测试覆盖包括两项 ABI 条件项在内的实际防御判断，辅以真实 FD、权限、symlink、libproc 和独立子进程边界测试；不引入通用 syscall mock、Port 或进程模拟框架。14 项等价仅可按精确名称及现行常量集合接受，变更平台、依赖常量或运行时掩码须重新证明；其余 57 项在补测、精确复测或取得独立认可的不可达证明之前均保持未处置，不能以普通构建绿色代替。审核未修改文件、运行测试或读取第三组进度。

P1-15 macOS 前两组结果有效性补核（2026-09-27 UTC）：主 Agent 对持久副本 `outcomes.json` 的每个非 baseline Build/Test argv 逐项核对，第一组 232 个阶段调用、第二组 555 个阶段调用均包含 Adapter/Application/CLI 三包；baseline 仅运行被变异包，是 `cargo-mutants` 的单独前置检查，不代表后续变异只测一包。第一组 104 个 caught 日志均有测试失败摘要、12 个 unviable 均有 Rust 编译失败摘要；第二组对应为 179 与 39，且两组 `missed` 均为构建和测试成功、0 timeout。此检查支持结果分类未被单包误读或构建失败冒充捕获，但不改变当前 85 个存活项及剩余两组尚未闭合的状态。

P1-15 macOS Adapter 第三组终态（2026-09-27 UTC）：冻结候选 `75004d5` 的 `materializer.rs` 276 项实际运行于 14:40:52–15:28:14 UTC，约 47 分 22 秒；按预定约 15:36 UTC 单次读取执行会话，exit 2，未变异基线通过。结果为 **181 caught＋34 unviable＋61 missed、0 timeout**，`outcomes.json` SHA-256 `8db9e387c434708bdd467e6c31a0a3dba36c37a11b7247613b8751e4847851ce`，复制到忽略目录 `target/p1-stage-macos-remaining3-20260927/mutants.out` 后哈希相同。逐项核对 518 个非 baseline Build/Test argv 均包含 Adapter/Application/CLI 三包；181 个 caught 日志均有测试失败摘要、34 个 unviable 均有 Rust 编译失败摘要。61 项主要位于路径证据、清单/摘要、回滚和对象身份边界，尚未归因或补测，不能按成功处理；其余未执行的 `store.rs`＋`probe.rs` 216 项继续在冻结候选上顺序运行，不修改生产源码。

P1-15 Core 闭合提交审核后 Verification（2026-09-27 UTC）：第三组已终止、第四组尚未启动的空档，主 Agent 对已获独立审核批准的精确 Core 提交 `75004d5` 执行 `cargo test --release -p thinws-core --test git_status --test materialization_domain`，真实 Release 配置两组共 17 项通过、0 失败或忽略。该检查只完成 Core 任务的审核后定向复验，不能替代 macOS Adapter 存活项处置或 Phase 1 总门禁。

P1-15 macOS Adapter 第四组启动登记（2026-09-27 15:38 UTC）：冻结本地候选仍为 `75004d5`，原 126 项精确排除式＋`--file` 选 `store.rs` 和 `probe.rs` 当前枚举 **216 项**；仍由主 Agent 顺序执行 Adapter/Application/CLI 三包验证、180 秒单变异上限、2 jobs/4 jobserver tasks、`--gitignore true`，输出 `/private/tmp/thinws-macos-remaining4.lz9xPK/mutants.out`，不覆盖旧结果。第三组 276 项约 47 分钟、第二组 297 项约 52 分钟，本组按范围与波动保守预计启动后约 50 分钟首次主动查看（约 16:28 UTC）；提前终止事件或用户询问可处理，不按短间隔轮询。此启动登记不表示 216 项已通过；四组跑完后才修改生产源码并重新枚举受影响位置。

P1-15 `materializer.rs` 61 个存活项独立归因（2026-09-27 UTC，尚未补测）：GPT-6 Astra / `xhigh` 只读审核绑定冻结候选 `75004d5` 和第三组结果哈希 `8db9e387c434708bdd467e6c31a0a3dba36c37a11b7247613b8751e4847851ce`，逐项归为 **1 项当前合法类型值域内严格等价、11 项依赖当前调用链/私有隔离前提的条件不可达、47 项有明确反例的测试缺口、2 项只在稳定 I/O 下功能结果相同但暂不能排除**。11 项并非任意输入数学等价，新增调用者、外部报告、可变隔离目录或放宽目录 FD 前提时须重证；2 项为 64 KiB 缓冲表达式变异，会改变分块、故障点和 hook 调用序列，不能直接伪称严格等价。47 项优先补独立摘要 oracle、单字段路径/清单证据、真实 FD 隔离冲突与回滚；尤其 `rollback_created` 的末次权限恢复短路和 remaining 非空却报告 ConfirmedBaseline，必须用实际后置状态区分。审核未发现冻结生产实现自身已证实有缺陷，也未修改代码、运行测试或查询第四组；以上只是归因，后续由主 Agent 补 RED/GREEN、重枚举受影响目标并复测，不提前记为 macOS Adapter 闭合。

P1-15 上述两项缓冲区变异的独立复核结论（2026-09-27 UTC）：同一 GPT-6 Astra / `xhigh` 审核者进一步对照物化设计 §3.4、生产入口固定 `NoopHook` 与现有部分写入测试，**批准仅将 `materializer.rs:899:32` 的 `digest_file` 和 `:1130:32` 的 `copy_file_bytes` 中 `64 * 1024`→`64 + 1024` 记为当前产品契约下的等价实现参数**，不冻结 64 KiB 为新契约，也不称全部执行轨迹严格相同。两者均为正缓冲，按实际读长增量摘要或顺序 `write_all`，最终内容、长度与失败安全判断不依赖 chunk 边界；不同 syscall 数、吞吐、耗时、故障时机和部分写入长度仍是真实影响，现有契约只要求如实记录实际结果。负责人为主 Agent；最终精确排除须等测试改动后重枚举位置，绝不整函数或整文件排除。若 chunk 进入公开格式/进度/取消/断点续传、非 `NoopHook` 进入生产、明确性能/I/O 对齐门槛、块级摘要/重试、特殊流支持或缓冲循环语义/变异位置改变，立即重新审核；这项结论不豁免另外 47 项测试缺口、11 项条件前提或异常处理测试。审核没有编辑或运行测试，也未查询第四组。

P1-15 macOS Adapter 第四组终态（2026-09-27 UTC）：冻结候选 `75004d5` 的 `store.rs`＋`probe.rs` 216 项实际运行于 15:38:30–16:03:09 UTC，约 24 分 39 秒；依据 15:52 UTC 用户询问时已有 137/216 项、并参照前两组耗时，将首次后续主动查看提早至约 16:20 UTC，读取到执行器 exit 2。未变异基线通过，结果为 **126 caught＋60 unviable＋30 missed、0 timeout**；结果已复制到忽略目录 `target/p1-stage-macos-remaining4-20260927/mutants.out`，`outcomes.json` 原副本与持久副本 SHA-256 同为 `2ebd14cf58fb16295ef289b7f533fe13077a8cf1b74cdf64a6371769d3dd3eb5`。逐项核对 372 个非 baseline Build/Test argv 均包含 Adapter/Application/CLI 三包，126 个 caught 日志均有测试失败摘要、60 个 unviable 均有 Rust 编译诊断。至此冻结候选原 911 个剩余 Adapter 目标的四组均已运行；四组尚有 176 个工具 missed（其中已独立归因为严格等价、条件不可达或产品契约等价的项仍保留原结果），**不能把 Adapter 或 Phase 1 记为闭合**。第四组 30 项已派发规定模型只读独立归因；主 Agent从已明确的安全测试缺口开始定向补测，复测仅覆盖未闭合及受修改影响的目标，不重启原四组全量。

P1-15 第四组 30 项独立归因（2026-09-27 UTC）：GPT-6 Astra / `xhigh` 只读审核绑定 `75004d5` 与结果 SHA-256 `2ebd14cf58fb16295ef289b7f533fe13077a8cf1b74cdf64a6371769d3dd3eb5`，逐项分类为 **29 个确定性测试缺口＋1 个当前调用链不可达**，没有严格数学等价项，也没有已证实的冻结基线生产缺陷。唯一不可达项是 `probe.rs:539:5 require_directory→Ok(())`：当前 Darwin `O_SEARCH` 含 `O_DIRECTORY`，仅有目录 FD 可走到此处；若打开 flags、调用者或平台改变须重证，且不能称任意参数等价。Probe 的 18 个缺口需分别覆盖二次 ENOENT 查询的不同 errno、双向包含与大小写别名 ancestry、Clone 能力位的 valid/capability 组合、零 UUID 的准确 reason、EIO 不降成权限不支持及摘要枚举字节；Store 的 11 个缺口需对初始化路径、Ready/清理归属及空间二次测量做单因素失配，并断言准确错误来源和无越权副作用，不能让前置 Ready 校验或后置校验产生假阳性。Reviewer 未编辑、运行测试或变异；此结论只是补测依据，29 项尚未闭合。

P1-15 macOS 存活项第一轮定向补测（2026-09-27 UTC）：主 Agent 仅在 macOS Adapter 的物化、FFI、进程、文件系统、Probe、Store 模块提取局部已有判断并补单因素断言；未增 Port、外部依赖或公开 CLI 行为。前四组工具 `missed` 是原测试未能拒绝对应错误实现的 RED 证据；新断言的原实现 GREEN 已由当前 `THINWS_P1_CROSS_VOLUME_ROOT=/Volumes/data cargo test --workspace --all-targets` 退出 0、全目标/feature Clippy 退出 0 证明，fmt 已执行。已完成的直接测试覆盖回滚残留与根权限、独立清单字段/摘要、路径/进程/归属身份、同卷错误初始化和 FFI 列表/标志/解码边界；它们尚不是变异捕获证据。冻结补测候选为 HEAD `75004d5`＋当时 macOS Adapter 代码/测试差异 SHA-256 `f61c5ac746de3588d232142a43c507cd1febce173dad4ce824f585db3173b288`；先对 `materializer.rs` 原 61 个精确 missed 名称做补测，当前 `cargo mutants --list` 精确筛选仍为 61 项，使用 Adapter/Application/CLI 三包、180 秒、2 jobs/4 jobserver tasks、内部卷 `/private/tmp/thinws-mat-missed.M8QnrM/mutants.out`。原第三组 276 项约 47 分钟，此批 61 项预计首次主动查看启动后约 17 分钟，不短轮询；完成前不改候选源码或测试，也不宣称这 61 项闭合。

P1-15 macOS 物化存活项首轮精确复测终态（2026-09-27 UTC）：同一冻结候选的 61 项实际于 16:38:21–16:47:47 UTC 运行，约 9 分 27 秒；按约定约 16:55 UTC 首次读取时已退出 2，未变异基线通过，**32 caught＋29 missed、0 unviable/timeout**。结果保存于忽略目录 `target/p1-stage-macos-materializer-followup1-20260927/mutants.out`，`outcomes.json` SHA-256 `4e771dd25048c2c2f573e6f34b135d9a3f7a8c4bbf3c2e131cc320e324b96c95`。32 项是新测试对原存活项的实际捕获；29 项仍须区分先前独立审核认可的等价/当前调用链前提与实际测试缺口，尤其摘要后置校验、根权限/归属及隔离撤离路径，不把本批结果记为物化模块闭合。

对上述 29 项按原独立审核的前提逐名复核后，原分类中的两处“条件不可达”需要撤回：`rollback_created` 最后身份检查与 `quarantined_entry_matches` 二次打开之间都可能被同用户并发改动；普通 Workspace 和实例私有目录不是 Sandbox，不能以路径暂时稳定作安全证明。因此目前至多 9 个依赖合法构造/FD 类型的条件项、2 个已获审核认可但非全轨迹等价的缓冲参数项，另外至少 18 个仍按可达测试缺口处理。缺口集中于摘要/元数据后置核对、回滚根权限及二次绑定/恢复、隔离名碰撞、隔离项身份、目录二次观察与父目录身份。该归因不把原始 `missed` 改写为 `caught`；新增单因素断言和精确复测关闭实际缺口后，剩余条件前提仍须由最终独立审核复核。

P1-15 macOS 其余受影响目标第二批启动登记（2026-09-27 UTC）：在上述代码/测试冻结候选上，将 `ffi.rs`、`filesystem.rs`、`probe.rs`、`process.rs`、`store.rs` 的原存活项与本轮局部提取所影响函数/新 helper 的全部当前变异合并，逐名去重后精确选出 **251 项**；`cargo mutants --list` 已核对为 251 项。相对当前 Adapter 1066 项枚举，物化第一轮 61 项与这 251 项覆盖了全部因本轮改动而需重证或原先存活的目标；另外 10 个未选的旧异常项分别已有精确补测或独立认可的范围排除/等价证明，不凭旧行号推断。验证继续为 Adapter/Application/CLI 三包、180 秒上限、2 jobs/4 jobserver tasks、`--gitignore true`，结果使用内部卷独立目录 `/private/tmp/thinws-rest-followup1.ExHe9w/mutants.out`。第一轮 61 项约 9 分半，第二批包含较多 FFI/Store 实测，首次主动查看保守定为启动后约 55 分钟；执行期间不修改本批源码/测试，不短轮询，终态前不称 Adapter 闭合。

P1-15 macOS 其余受影响目标第二批终态（2026-09-27 UTC）：冻结候选的 251 项实际于 16:56:44–17:21:30 UTC 运行约 24 分 46 秒；按预估约 17:52 UTC 首次读取执行器，exit 2，未变异基线通过。结果为 **209 caught＋21 unviable＋21 missed、0 timeout**，持久副本 `target/p1-stage-macos-rest-followup1-20260927/outcomes.json` 与原结果 SHA-256 均为 `66b2f8baf77079f2e635c227cf86c4fdb5de6c5d791aac65ca4fcffbd8c6abc4`。所有非 baseline Build/Test argv 均包含 Adapter/Application/CLI 三包；209 个 caught 都因测试失败，21 个 unviable 都有编译诊断。逐名复核后，存活项精确分布为 `ffi::libproc_error` 一项、`open_root_directory` 必需 flags 的 `|→&` 一项、FFI 互斥 bit flags 的 `|→^` 十五项、`Path::components()` 的 RootDir 守卫三项及 Probe 当前 `O_SEARCH` 目录 FD 前提一项。先前“十六项都是 flag 等价”的速记不准确，已纠正；仅后 19 项按先前独立审核的等价/当前调用前提保留原始 `missed`，不得称作 caught。`libproc_error` 与 `open_root_directory` 两项真实缺口需补单因素断言和精确复测；物化第一轮余下 29 项仍单独未闭合。

P1-15 macOS 第二轮定向补测启动登记（2026-09-27 17:58 UTC）：主 Agent 补 `libproc_error` 的线程局部 errno、root FD 实际打开标志，以及物化摘要/元数据后置检查、根权限恢复、撤离前身份复核、trash 重新绑定、隔离名碰撞和父目录身份单因素断言；生产改动仅为原判断的私有局部提取和既有测试 Hook 的故障注入落点，未增 Port 或公开行为。root FD 新断言加入前，Adapter 133 项单测中 132 pass、1 项环境门禁 ignored，全目标全 feature Clippy 退出 0；随后 root FD 定向测试亦通过，完整普通门禁仍须在冻结批次后重跑。原先相应工具 `missed` 是 RED 证据，但新增断言仍须 mutation 复测确认。候选为本地 HEAD `75004d5`＋Adapter 差异 SHA-256 `27781673a21f94d90c8bf0a832fac4939a604e44796ffe1a769457650965e76f`；用 `cargo mutants --list` 将改动影响的物化函数、`ExecutionHook::restore_rollback_root_mode` 与 FFI 两个未闭合函数合并，当前精确筛选 **97 项**，使用原三包测试集、180 秒上限、2 jobs/4 jobserver tasks、`--gitignore true`，结果放系统内部卷 `/private/tmp/thinws-final-mutation.SqUf2A/mutants.out`。参照 61 项 9 分半和 251 项 24 分 46 秒，首次主动查看暂定启动后约 20 分钟（约 18:18 UTC）；执行期间冻结代码和测试，不短轮询。此批不重做已经闭合且未受影响的旧目标。

P1-15 macOS 第二轮定向复测终态及误判复核（2026-09-27 UTC）：97 项实际于 17:58:36–18:05:35 UTC 完成，约 6 分 58 秒；按预定 18:18 UTC 首次读取执行器 exit 2，未变异基线通过。工具摘要为 **88 caught＋5 unviable＋4 missed、0 timeout**，持久副本 `target/p1-stage-macos-final-followup-20260927/outcomes.json` 与原结果 SHA-256 均为 `992386285d387216e7f196ce25980467c2fa7275c1e22299fcd9962d40c04bb7`。非 baseline 的 Build/Test argv 均含 Adapter/Application/CLI 三包；88 个 caught 均有测试失败、5 个 unviable 均有编译诊断。不过逐项核对失败测试名后，`open_root_directory:333:32 |→^` 的唯一失败是无关 `real_cli_git_incomplete_refusal_exposes_the_specific_issue` 的 1 秒限制断言得到 23 而非 25，不能作为该 flag 变异的因果捕获；当前 `libc 0.2.189` 中 `O_SEARCH=O_EXEC|O_DIRECTORY`、`O_NOFOLLOW=0x100`、`O_CLOEXEC=0x01000000` 位互不重叠，故此项按等价而非有效 caught 处理。其余 87 个 caught 的失败名称均落在 FFI 或物化相关测试；四个工具 missed 为一个同类 flag `|→^`、一个已独立认可的 buffer 参数等价、两个由同一已持有根 FD 生成 baseline 的前置身份条件。对 `:333:32` 正在以相同三包范围单项复测，以排除偶发测试失败；在复测终态前不记为闭合。所有原始工具结果保持不改写，以上是负责人额外的因果审计。

`open_root_directory:333:32 |→^` 单项复核于 18:20:43–18:21:27 UTC 完成：同一冻结代码/测试、三包验证、未变异基线通过，1 项 **missed**、0 caught/timeout，执行器 exit 2；`target/p1-stage-macos-root-xor-recheck-20260927/outcomes.json` SHA-256 `5e7f7a226a62e6f9dd552f42be4b53891e4a6f821b71d91434d7342b0cde0e43`。这与位不重叠的数学证明一致，反证前批 `caught` 是无关超时用例的假阳性；不删改前批工具记录。后续对该项只用精确等价排除，不以偶发失败抬高变异捕获数。该无关 E2E 的 1 秒边界应在最终普通测试中重验；若再次失败，按产品缺陷或测试可靠性另行归因，不掩盖。

P1-15 精确变异例外更新（2026-09-27 UTC）：`.cargo/mutants.toml` 仅增加 22 个当前名称的窄排除：fuzz-only 桥 1、十六进制 nibble 严格等价 1、Darwin 互斥 flags `|→^` 15、当前 Rust `Path::components()` RootDir 单次语义 3、已由独立审核按当前产品契约认可的正缓冲参数 2。`cargo mutants --no-config --list --package thinws-adapter-macos` 为 1076 项，按当前配置为 1054 项；集合差逐名核对恰好上述 22 项，无整函数/整文件排除。条件性调用链存活项没有混入配置排除，仍需最终审核复核。若 libc flags、Rust 组件迭代语义、fuzz 构建边界、缓冲区公开契约或相关源码位置变化，须重证并更新精确正则；不将旧行号永久视为证明。

P1-15 本地普通门禁和 fuzz lint 前置修正（2026-09-27 UTC）：最终候选的全 workspace/all-targets Debug 测试以 `/Volumes/data` 为 P0/P1 指定异卷端退出 0，主 workspace `fmt`、全目标全 feature Clippy、44 项 CI/性能辅助单测、crate 依赖方向和差异检查均通过。额外按工作流执行 fuzz workspace fmt/Clippy 时，先发现 HEAD 中两处 fuzz target 排版不符当前 rustfmt，只做断言换行和 import 排序，复核 diff 无语义变化；随后直接 fuzz Clippy 因 `cfg(fuzzing)` 未开启而无法解析已有 fuzz-only 桥。新增工作流断言先 RED，实测 `RUSTFLAGS='--cfg fuzzing'` 的严格 fuzz Clippy GREEN，再仅修正 `.github/workflows/p0.yml` 的该条命令；主/fuzz fmt 检查及 44 项辅助测试复跑全绿。此修正未更改 fuzz oracle 或产品生产逻辑，既有长预算 fuzz 语义证据仍须在最终审核中核对可复用性；线上 CI 未运行，不将本机模拟称为 CI 通过。Release、真实子挂载与二进制黑盒仍待最终候选执行。

P1-15 最终候选真实平台与二进制验收（2026-09-27 UTC）：`cargo test --locked --workspace --doc`、`THINWS_P0/P1_CROSS_VOLUME_ROOT=/Volumes/data` 的全 workspace/all-targets Debug/Release 普通测试、`cargo build --locked --release --workspace` 均退出 0。随后在 `/private/tmp` 创建一次性 128 MB APFS 镜像并挂载于专用借用源下；系统临时卷、项目 `/Volumes/data`、镜像挂载点的设备号分别为 16777229、16777240、16777250，确为三卷。按工作流四项 P0/P1 环境变量运行全 workspace/all-targets `--include-ignored --nocapture` Debug/Release，均退出 0；通过精确镜像关联的 `ci_detach_image.py` 确认卸载，核对临时目录仅含该镜像与空父目录后删除，未触及项目或用户卷。Release `thinws` 以 `/Volumes/data` 上一次性 HOME/data root 和现有非 Git `docs/project/reference` 为只读来源，黑盒执行 `--json init`、`doctor`、`workspace create`、普通 `workspace path`、`--json workspace status`、`--json workspace remove`：CoW `confirmed`、内容一致、普通路径精确、副本新增文件不影响来源、普通清理非强制且目录消失，全部断言通过；测试实例核对后删除。线上 CI、签名/公证和人工结构验收未执行；上述本地门禁不能代替维护者放行。供应链结果及缓存时效见下一段。

P1-15 供应链审计边界（2026-09-27 UTC）：`cargo deny --locked check` 主/fuzz 两工作区均退出 0，未命中 license allow/exception 的提示是非阻断 warning。`cargo audit --deny warnings --file Cargo.lock` 的在线 advisory fetch 持续无响应，核对确切 PID 后仅对该审计进程发 TERM，exit 143；这次在线尝试不算通过。随后 `cargo audit --no-fetch --stale --deny warnings --file Cargo.lock` 与 `fuzz/Cargo.lock` 均退出 0，使用本机 1261 条 advisory 缓存，缓存 HEAD `17af77682cecd2afa72b217ad7c6c30585d5003f`、提交时间 2026-09-22。故仅可声明本地缓存审计通过，在线最新漏洞库未核验；不把缺少在线刷新隐藏成全新安全结论。

本机 Release 候选二进制为 arm64 Mach-O，`target/release/thinws` SHA-256 `a58382fd13456a34edd6c865c4bbbe52097f0523103a0da4eedef6ef67eaa7d2`；直接执行 `--version` 返回 `thinws 0.1.0`，`--json gc` 返回 exit 2 / `E_USAGE`，没有重新引入已取消的 GC 命令。长预算 fuzz（先前固定候选 10 target×300 秒、无 crash/hang）按《任务流程》§18.1 复用：本轮新增的 FFI/物化路径不进入现有纯输入 harness，fuzz target 差异仅 rustfmt 排版、没有 oracle/依赖/工具版本语义变化；最终 Reviewer 已核对影响边界，但复用不等于本候选重跑。

P1-15 变异证据组合（2026-09-27 UTC）：按当前 `.cargo/mutants.toml` 逐包重新枚举为 P0 Probe 244、P0 Materialize 313、P0 Cleanup 23、Git Adapter 408、SQLite 169、Application 217、CLI 62、Ports 56、Core 503、macOS Adapter 1054，合计 **3049** 项，与全 workspace 列举相等。非 macOS Adapter 的当前 1995 项沿用上文逐包闭合及 Core 精确审核；Adapter 原 1037 项由先前闭合 126 项＋四组 911 项覆盖，生产局部提取后枚举净增 39，当前无配置 1076 项，精确排除 22 项后为 1054 项。原存活项、改动函数及新增 helper 已在 61＋251＋97 项定向批次中复测；`libproc_error`、root 打开 `|→&`、回滚的身份/权限/路径/隔离错误成功分支均有直接测试捕获。剩余工具 `missed` 包括未排除的 9 个物化当前构造/FD 前提和 1 个 Probe 的 `O_SEARCH` 目录前提，已由最终独立审核逐名复核；15 个互斥 flags、nibble、RootDir、fuzz-only 桥和 2 个内部 buffer 参数已窄排除并保留证明。各原始结果仍按其真实 exit/分类保留，不把单批 exit 2 改写为全绿。

P1-15 技术候选收口（2026-09-27 UTC）：最终实现/测试提交为 `4e6ad0a`、`b9870ca`，后者只将 FD 关闭断言移入独立子进程以消除并行测试复用编号的竞态，未改变生产行为。GPT-6 Astra / `xhigh` 只读终审对 `4e6ad0a` 的唯一 P2 已在 `b9870ca` 修复后复核为 Approve，未发现剩余可操作问题。审核按函数内相对位置与实际 diff 核对 Adapter 当前 1054 项零遗漏：884 caught、160 unviable、10 个有当前构造/目录 FD 前提的条件性 missed、0 timeout；精确 `DirectoryStream::drop → ()` 修订复测为 1/1 caught。修订后主 Agent 的全仓 Debug 普通测试、严格 Clippy、fmt、Release 定向测试及差异检查均退出 0；前述同一生产候选的 Release、真实三卷/APFS、二进制黑盒、供应链缓存审计、性能基线和长预算 fuzz 证据继续有效。P1-15 与 P1.f 达到供维护者人工检查的技术候选，Phase 1 **尚未人工放行**；线上 CI、在线最新 advisory 刷新、正式签名/公证、维护者结构测试与最终确认均未执行，不打 tag、不推送、不进入 Phase 2。

### 4.7 P1 新布局修订（ADR-0006）

2026-09-28，维护者要求配置、SQLite、锁、日志和归属证据集中在固定 `~/.thinws`，创建时显式传入最终 target；每次 source 与 target 同 APFS 卷即可，控制目录可以在另一卷。删除前必须证明登记 target（或已登记隔离位置）仍存在且身份一致；缺失或不匹配时保留关联，`--force` 不绕过。旧布局不迁移、不兼容、不自动清理。此决策覆盖上述 P1.a–P1.f 旧布局技术候选的当前适用性，但不改写其历史事实；先前的 P1-15 技术候选不再具备本次布局的 Phase 1 放行资格。

| 任务 | 交付结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-17 固定控制目录与新 schema | `~/.thinws` 实例、配置/SQLite/单锁/日志/归属；不读取旧布局 | ADR-0006 | R4 | In Progress |
| P1-18 显式 target 创建与 CoW | `--target` 精确路径、同卷探测/重验、目标父目录暂存、唯一性与 Ready 归属 | P1-17 | R4 | In Progress |
| P1-19 查询与安全删除 | 任意已登记 target 的 Ready 核验、Git/空间查询、目标缺失或身份不符时关联保留，force 不绕过 | P1-18 | R4 | In Progress |
| P1-20 CLI 与工具联动 | 手册参数、JSON/错误码、help、Skill、性能与 CI 工具同步 | P1-17–P1-19 | R3 | In Progress |
| P1-21 新布局收口 | 受影响变异/fuzz、真实 APFS 多卷、Release 黑盒、供应链、独立本地审核与人工结构测试入口 | P1-17–P1-20 | R4 | In Progress |

本轮只在维护者指定的本地 checkout 实施、提交与审核，不创建线上 PR、不推送，也不使用子 Agent 开发。独立只读审核可使用 GPT-6 Astra / `xhigh`。用户已有旧控制目录、旧 data root 与已安装的旧 `thinws` 二进制不在清理或自动替换范围内。新 `init` 的测试须使用隔离 HOME，防止触碰真实 `~/.thinws`。

收口断言：两个不同 APFS 卷上各自的 source/target 同卷组合成功，source/target 跨卷（含 `--allow-copy`）拒绝；目标已存在、重复 target、路径别名/符号链接、目标卷卸载和路径替换均不误写误删；普通/force 清理在 target 缺失或身份不符时保留行、名称及日志，归属可证且内容清理完成时才写 tombstone；新命令不访问旧布局；新候选全量普通门禁与受影响专项门禁通过。完成这些仍不等于 Phase 1 人工放行。

本地候选验证进行中（2026-09-28 UTC）：首批 `git diff --unified=0` 选出的 173 个变异点，在隔离副本中以 `-j 2 --jobserver-tasks 4 --timeout 180` 执行，因随后在真实 APFS 上复现 target 大小写别名进入控制目录的缺陷而主动中止（exit 130）；中止前 54 caught、21 missed，其余未执行，原始结果留在 `target/p1-layout-v2-mutants-diff/mutants.out/`，不算通过。补了预览/创建/Adapter 双重身份保护及逐字段回归后，全 workspace 普通测试、fmt 和严格 Clippy 曾通过。18:23 UTC 启动修订候选的 176 点受影响代码变异批次；在下面两项真实 CLI 缺陷复现后主动中止（exit 130），中止前 100 caught、28 unviable、19 missed，余下 29 项未执行，原始结果位于 `target/p1-layout-v2-mutants-final/mutants.out/`，不算最终候选证据。

本地只读复核又发现预检使用固定的 `.thinws-preview-staging`/`.thinws-preview-trash` 假路径；真实 APFS 隔离 HOME 实验中，合法来源恰好叫 `.thinws-preview-staging` 时，`--dry-run` 错误返回 `E_COW_UNAVAILABLE`（加 `--allow-copy` 为 `E_FILESYSTEM`），同级普通名称来源则成功规划 `cow-clone`。这是预览路径与用户目录偶然重名导致的误拒绝。已由真实 CLI 回归先 RED 后 GREEN，预检改用本次随机 ID 对应的真实私有兄弟路径；dry-run 的 ID 不登记或返回。实验目录仅位于一次性 `/private/tmp` 根，核对无符号链接后已删除。

另一项 CLI 契约缺口：真实 APFS 隔离 HOME 下，未登记的 target 末级为既有普通文件时，`--dry-run` 返回 exit 33 / `E_TARGET_LAYOUT`，而用户手册对“目标末级已存在”规定 `E_TARGET_EXISTS`。目录情形已有测试；文件及末级符号链接的真实 CLI 回归先 RED 后 GREEN，最终路径组件已存在但不是目录时改为报告被占用，中间组件非目录仍是布局错误，原对象均不覆盖、不删除。对应一次性实验目录核对无符号链接后已删除。第二批存活项已补操作目录存在/替换/清理、错误映射与逐条件重叠的定向回归，相关定向测试和严格 Clippy 已通过；最终变异与 Release 仍待收口。

18:47 UTC 启动 57 项定向变异（`cargo mutants --workspace -F 'require_operation_directories_absent|remove_operation_directory|map_port|requested_paths_overlap|materialization_overlaps_control|provisional_materialization_paths'`；执行负责人为主 Agent，macOS/APFS，本地未提交候选，5 个相关测试包，`-j 2 --jobserver-tasks 4 --timeout 180`，结果在 `target/p1-layout-v2-targeted-mutants/mutants.out/`）。参考上一未完成批次的实际吞吐，首次查看暂定 19:05 UTC；期间不修改生产或测试候选，不轮询。完成后仍须覆盖当前差异中未跑完及新引入的变异点。

该批实际于 18:48:00–18:52:54 UTC 完成，按约定 19:05 UTC 首次读取，执行器 exit 2：52 caught、2 unviable、3 missed、0 timeout；结果 SHA-256 `f97401edc8867fffe72cfa83932b4d18319041a9ac8609c473fc832fa4950812`。其中两个 `git_query.rs` 的 `Budget.run_timeout` 存活项来自未修改的 Git 模块，因 cargo-mutants 的筛选枚举仍进入本批，不能算作本次布局代码闭合证据；它们不因本次任务静默改动 Git 实现。第三项是 `map_port` 中 `Unavailable` 的 Control 专门分支，删除该分支仍由紧随其后的通用分支返回相同的 `E_CONTROL_UNAVAILABLE`，属可直接消除的冗余判断，已删并用原实现的 4 项错误映射单测验证。当前重新枚举 `map_port` 为 38 项，按去除行号的完整变异名称核对，均包含于上批 38 caught 或 1 unviable 的旧集合；被删除的正是一个 caught 和一个 missed 的原冗余分支。其余受影响差异中的未执行变异仍待单独收口，不把这批 exit 2 写成通过。

19:09 UTC 在当前代码差异上以零上下文 Git diff 重新枚举受影响生产代码，共 184 项；按文件、函数、变异操作和重复数量与第二批已完成的 100 caught/28 unviable、上述定向批次的 52 caught/2 unviable 对照，尚有 37 项未被有效结果覆盖（Probe 8、Application 创建 13/查询 4/删除 1、CLI 9、Core 错误码 2）。用精确完整名称正则和同一 diff 双重筛选，`cargo mutants --list` 确认为恰好 37 项；后续只跑此集合，保留先前退出 130 的完整原始记录。已闭合项在最终审核中仍须逐项确认候选语义未变，不能仅凭行号复用。

19:09:57 UTC 启动上述 37 项精确未闭合变异，执行负责人为主 Agent，使用同一 macOS/APFS 本地冻结候选和 Core/Ports/macOS/Application/CLI 五包验证集、`-j 2 --jobserver-tasks 4 --timeout 180 --gitignore true`，结果目录为 `target/p1-layout-v2-unresolved-mutants/mutants.out/`。参考 57 项定向批次约 5 分钟，但本批含较大的 CLI/查询入口，保守首次主动查看暂定 19:20 UTC；此前不修改生产或测试代码、不轮询。

该批实际于 19:09:55–19:13:18 UTC 完成，按约定 19:20 UTC 首次读取，执行器 exit 2：26 caught、9 unviable、2 missed、0 timeout；结果 SHA-256 `a70335d07751e674525bab72b969d4f7f6601d443e577fb95f4c42ee70e7b43d`。两个存活项均为 `require_missing_target` 中 `||→&&`：现有真实 Probe 会把“最近存在祖先不符”与“缺失组件不是一个”一起报告，未单独验证 Port 返回相互矛盾事实时的防御。旧工具存活结果构成旧测试集的 RED 证据；主 Agent 增加只改测试的单因素 Port 报告用例，分别让最近祖先与缺失组件数单独失配，原实现的定向单测已 GREEN。当前筛选精确两项，须用同一五包测试集重新变异确认新断言确实捕获它们；不能以普通测试 GREEN 代替变异闭合。

19:22 UTC 启动两项 `require_missing_target` 精确复测，仍用 Core/Ports/macOS/Application/CLI 五包、2 jobs/4 jobserver tasks、180 秒上限，结果目录 `target/p1-layout-v2-missing-target-followup/mutants.out/`。参考上批 37 项实际约 3 分 22 秒、当前仅两项但包含基线编译，首次主动查看保守定为 19:26 UTC；期间不改生产或测试候选、不轮询。

收口范围纠偏（19:23 UTC）：上述 184 项只来自当前未提交差异；本轮新布局从 `7d74977` 技术候选之后还包含 `0d9c107`、`e55b9f5`、`b030617`、`6bc5923`、`60c5e41`、`91f4bdf` 六个本地提交，不能漏掉已提交的新布局生产代码。以 `git diff --unified=0 7d74977` 重建完整生产差异，当前 cargo-mutants 枚举为 **325 项**。对照本轮第二批、57 项定向批及 37 项补测的 caught/unviable，按文件、函数、操作和重复数量匹配后仍有 **131 项**未闭合，其中包含正在精确复测的 2 项；集中于新文档编码、文件系统路径/归属、SQLite v2 及少量应用/CLI。旧 P1-15 的全量变异属于旧布局，不用于替代这 131 项。下一批须在两项复测终态后只跑剩余精确集合，不能以 184 项未提交差异冒充整个新布局门禁。

两项 `require_missing_target` 精确复测实际于 19:22:48–19:23:29 UTC 完成，按约定 19:26 UTC 首次读取，exit 0、2 caught、0 missed/timeout/unviable；结果 SHA-256 `8d634c9704a78a5352e60d3a968e7b02614610164ec4938ea8b0cf0d2fdd3fd9`。新单因素断言已由真实 mutation RED/原实现 GREEN 闭合，旧 exit 2 记录保留；完整新布局仍有上段的 129 项待运行。

19:26 UTC 启动剩余 129 项精确变异：`--in-diff target/p1-layout-v2-all-changes.diff` 与 129 个完整名称的正则交集经 `--list` 核对为 129；执行负责人主 Agent，macOS/APFS 本地冻结候选，测试包为 Core、Ports、macOS Adapter、SQLite、Application、CLI，`-j 2 --jobserver-tasks 4 --timeout 180 --gitignore true`，结果目录 `target/p1-layout-v2-full-scope-remaining-mutants/mutants.out/`。参考 37 项实际 3 分 22 秒、57 项约 5 分钟，但本批含更多文件系统和 SQLite 分支，留正常波动后首次主动查看暂定 **19:48 UTC**；期间不改生产或测试候选、不短轮询、不并发 Release 构建。

该 129 项批次按约定于 19:48 UTC 首次读取终态：执行器 exit 2，**100 caught、27 unviable、2 missed、0 timeout**，总耗时 11 分钟；`outcomes.json` SHA-256 `62e702bfa7bb196bc7a70ac184c58a924495878dd93a69d52758c2b3beda8a29`。两项存活分别是 `validate_data_root_lock -> Ok(())` 和 `registered_target` 的父目录身份 `||→&&`；前者缺少错误 scope 的直接 Adapter 断言，后者缺少“同卷且原 target 不变，仅登记父目录被替换”的单因素用例。只增加这两条真实 APFS 测试，`explicit_target` 14 项全绿；原批的 missed 是测试修改前 RED 证据，仍需两项精确变异复测，不以普通测试 GREEN 代替。

19:50 UTC 启动上述两项精确复测：`--in-diff target/p1-layout-v2-all-changes.diff -F 'store.rs:(983|1072):9:'` 的 `--list` 恰为 2，仍用六包验证集、2 jobs/4 jobserver tasks、180 秒单项上限；结果目录 `target/p1-layout-v2-final-two-mutants/mutants.out/`。参考此前两项复测约 41 秒、此次 Adapter 集成测试更重，首次主动查看暂定 19:54 UTC；期间不修改生产或测试候选、不轮询。

该两项实际 45 秒完成，19:54 UTC 首次读取：执行器 exit 0、**2 caught、0 missed/timeout/unviable**；`outcomes.json` SHA-256 `88fc941d208ce928bac62aecd65f549983b3c6b64a00b7ad01a9755c6d31e373`。本轮旧 129 项批次的两处存活已由真实 Adapter 用例捕获；完整 325 项新布局生产差异仍须与历史有效 caught/unviable 结果逐名核对，不用单批 2/2 代替完整范围。

完整范围对账（19:56 UTC）：以 `7d74977` 到当前生产差异 `target/p1-layout-v2-all-changes.diff`（SHA-256 `6b46a5832be9ab50dce89e7cafbd4652195db8e7ca6bbf0c6bcfe3527b157fcc`）重新列举仍为 **325** 项。六批结果的 caught/unviable 按去行号的文件、函数、变异操作和重复数虽可覆盖 325/325，但其中 76 项并无当前源码坐标的同名有效结果；鉴于实现期间有行移动和局部生产修订，不能仅凭归一化多重集合宣布闭合。当前精确名称匹配已闭合 249 项；其余 **76 项**由完整名称正则与 `--in-diff` 交集复核为恰好 76，计划仅补跑这一差集。候选为本地 `91f4bdf` 加未提交新布局差异及两项仅测试补强；macOS/APFS，执行负责人主 Agent，六包测试集，2 jobs/4 jobserver tasks、180 秒单项上限；预计参考上批 129 项 11 分钟，首次主动查看暂定启动后 12 分钟，不短轮询。

精确差集批次于 **19:57 UTC** 启动，结果目录 `target/p1-layout-v2-coordinate-gap-mutants/mutants.out/`，首次主动查看定为 **20:10 UTC**；运行期间不修改生产或测试候选，也不并发执行争用同一构建资源的 Release/全仓测试。

该批实际约 8 分钟完成，20:10 UTC 首次读取：执行器 exit 0、**65 caught、11 unviable、0 missed/timeout**；`outcomes.json` SHA-256 `3252b36c918ef70be738fc298269997ce99561275932a0ac1a7bfcd3815db616`。重新枚举当前 325 项并以完整文件、行列及变异名称逐一核对七批 caught/unviable，**325/325 均有精确当前名称的有效正结果、0 缺口**；原先 aborted/exit 2 批次照实保留，只有其中已完成且与当前候选一致的单项结果参与组合，不把整批失败改写为通过。生产候选在上述三批最新补测期间未改动；后续只改诊断文字时须确认其不改变变异位置/逻辑，并重新列举对账。此为本次新布局受影响生产差异的任务级变异证据，不宣称阶段全 workspace 变异已重新执行或人工放行。

术语同步仅将旧 `data root` 诊断文字和源码注释改为 `control root`，未改符号、分支、状态或系统调用。重建完整生产差异 `target/p1-layout-v2-all-changes-final.diff`（SHA-256 `2ceb55bb02d4e201f0a047f3a85d2a3ee3e61f095f180688273248e0f489388d`）后，cargo-mutants 因函数所在行新增触及而枚举 **330** 项；原 325 项的完整名称及坐标仍逐项一致，新增 5 项函数级变异无有效旧结果。精确完整名称 `--list` 核对为 5，须仅补跑这 5 项并复核最终 330 项，不以“只有文字改动”跳过工具选中的新范围。

5 项术语联动精确变异于 **20:12 UTC** 启动：六包测试集、2 jobs/4 jobserver tasks、180 秒单项上限，结果目录 `target/p1-layout-v2-terminology-five-mutants/mutants.out/`。参考两项 45 秒和 76 项约 8 分钟，首次主动查看暂定 **20:16 UTC**；期间不再改生产或测试候选，不轮询。

20:16 UTC 首次读取发现该批在**未变异 baseline** 即失败（执行器 exit 4，0 项变异被测试；`outcomes.json` SHA-256 `348c82e3dddd11f79d79a9b12b9a7dbfa8ce48320e909257af4963782220ac21`）。唯一失败是 `bootstrap.rs` 仍断言旧操作描述 `validate prepared data root`，而生产术语已变为 `validate prepared control root`。更新该测试期望后，真实 Adapter 定向测试退出 0；旧失败批次保留，不作为变异证据，须重新运行同一 5 项并在全仓普通门禁中复核其它文字断言。

同一 5 项于 **20:17 UTC** 在更新后的测试候选上重启，结果目录 `target/p1-layout-v2-terminology-five-mutants-fixed/mutants.out/`，六包测试集与资源/超时配置不变；首次主动查看暂定 **20:21 UTC**，期间不改生产或测试候选、不轮询。

该批实际 40 秒完成，20:21 UTC 首次读取：执行器 exit 0、**1 caught、4 unviable、0 missed/timeout**；`outcomes.json` SHA-256 `07cd2a5f45fbb7d319f7c3e0740d74de20d3b5f5e47b305e1218cf607ef2a2a2`。以最终完整差异重列举 **330** 项，按文件、行列与完整变异名称交叉八批有效 caught/unviable，**330/330 均有精确结果、0 缺口**。这只证明本次新布局受影响生产差异的变异范围已闭合；未执行阶段级全 workspace 变异，也不替代后续全仓普通测试和人工放行。

当前候选的供应链离线核查（18:53 UTC）：`cargo deny --locked check`、`cargo audit --no-fetch` 和 `cargo audit --no-fetch --file fuzz/Cargo.lock` 均退出 0；deny 仅有既有未命中许可 allowance/exception 警告，audit 使用本地 1261 条 advisory，不能冒充在线最新漏洞库核查。工具单元测试 45 项退出 0；测试生成的 `tools/__pycache__/` 已移入忽略的 `target/` 验证目录。

真实目标卷离线补验（2026-09-28 UTC）：第一次在项目数据卷创建的一次性 APFS 镜像挂载被本机拒绝，确认未挂载后清理；随后按既有 CI 方式在 `/private/tmp` 创建专用 128 MB APFS 镜像，挂载为 UUID `42daf8db-3f96-497e-a854-5869fee4bcfc` 的第三卷。使用当前 Debug 二进制、隔离 HOME 与该镜像卷内 source/target 创建得到 `cow-clone/confirmed`；核对镜像关联后仅卸载这张测试镜像，`workspace remove offline-volume --force` 返回 exit 37 / `E_TARGET_MISSING`，`workspace list` 仍含 Ready 记录，SQLite 活动行/tombstone 为 `1/0`，控制目录日志有 `failed` 与该错误码。重新挂载同一镜像后，源/副本内容一致，显式清理成功，目录消失、源保留，活动行/tombstone 为 `0/1` 且日志为 `completed`。再次核对关联并卸载；两个一次性镜像及其隔离测试目录随后均已删除，未卸载或清理用户现有卷。另有目标父路径消失/恢复的真实 APFS CLI 回归通过；下文记录了冻结候选的 Release 复验。

冻结候选的完整普通/真实平台门禁（2026-09-28 UTC）：主 workspace `cargo fmt --all -- --check`、严格全目标全 feature Clippy、`cargo test --locked --workspace --all-targets`、Doc Test，fuzz workspace fmt 与带 `--cfg fuzzing` 的严格 Clippy，crate 依赖脚本、45 项工具单测和 `git diff --check` 均退出 0。用 `/private/tmp/thinws-layout-check.DNfUOA/thinws-p0-cross-volume.dmg` 的专用 APFS 卷（UUID `D4332CC3-9041-4CD3-9DC6-E87F515CBC18`）设定 P0/P1 子挂载及异卷测试根，`cargo test --quiet --locked --workspace --all-targets -- --include-ignored` 全绿；`cargo build --locked --release --workspace` 及同环境 `cargo test --quiet --locked --release --workspace --all-targets -- --include-ignored` 均退出 0。Release arm64 二进制 SHA-256 为 `40b0c682e5197452f5306db02210831263db1a07f6c6cd71e4594fe8eae078cd`。`tools/ci_release_binary_e2e.py` 用隔离 HOME 和本机模拟的 GitHub Actions 环境执行，Release CLI 的 init/create/path/remove 与旧配置不变断言全绿；这只是本机模拟，**线上 CI 未执行**。

Release 黑盒多卷与离线删除复验：控制目录在上述专用 APFS 镜像卷，source/target 在 `/Volumes/data` APFS 卷，系统 `/private/tmp` 为第三个卷，`workspace create` 返回 `cow-clone/confirmed`，精确 target 的 path/status 与正常删除均成功，镜像控制目录外无新控制文件。在另一隔离 HOME 下把 source/target 同置镜像卷，Release 创建确认 CoW 后精确卸载该镜像；`workspace remove offline-volume --force` 返回 exit 37 / `E_TARGET_MISSING`，SQLite 活动行/tombstone 仍为 `1/0`，`~/.thinws/logs/operations.jsonl` 留下 `failed` 与该码。重挂同一 UUID 后源与 target 文件仍存在，普通 `remove` 成功；行/tombstone 转为 `0/1`，日志有 `completed`，源保留、target 删除。最后再次核对镜像关联并卸载，只涉及一次性测试镜像；没有触碰用户实际 HOME 或现有卷。此次 Release 结果补足前段仅 Debug 的离线卷证据，但不等于人工阶段放行。

受影响 fuzz smoke：固定 `cargo-fuzz 0.12.0`、`nightly-2026-08-14`，隔离 `target/p1-layout-fuzz-smoke/` 语料及 artifact，以 `-max_total_time=60 -timeout=5` 分别运行 `thinws_bootstrap_document`、`thinws_create_request`、`thinws_init_request` 和 `thinws_materialization_path`；四项 exit 0，无 crash/hang，artifact 目录为空。前者约 79.8 万次执行，其后三项分别约 986 万、1305 万、1654 万次执行；变异生成的语料只留在忽略目录，不回写仓库。`thinws_remove_request` 只覆盖未变化的参数/名称解析，不覆盖此次删除归属判定；该判定由真实 APFS、故障注入和变异测试验证，不把不相关 fuzz 当成删除安全证明。

Release CoW 性能基线（同日、本机非并发 I/O）：运行 `PYTHONDONTWRITEBYTECODE=1 python3 tools/p1_perf_baseline.py --binary target/release/thinws --volume-root /Volumes/data --output target/p1-layout-perf-baseline.json`，在 UUID `1A42C888-32E3-489C-9BFA-67FD640A94E8` 的 APFS 卷上用 264 文件、约 65 MiB 逻辑数据做 3×10 次显式 target 创建，30/30 均返回 `cow-clone/confirmed` 且 target 精确匹配；中位 331.245 ms、p95 358.285 ms。输出在忽略的 `target/`，一次性 fixture 已自动清理；容量差只作参考，不将该数据表述为跨设备性能保证。

---

## 五、依赖摘要

长链只表达技术依赖，不代替 §3.1 的串行执行顺序。

```text
P0-01 → P0-02 → P0-06 → P0-07
              └→ P0-05

P0 当前未取消任务与阶段条件 → P1-01 → P1-02 → P1-03
P1-03、P0-06 → P1-06 → P1-07 → P1-09
P1-03、P0-07 → P1-16
P1-09、P1-16 → P1-10
P1-09、P1-16 → P1-12 → P1-13
全部前置活跃 P1 能力 → P1-14 → P1-15
ADR-0006 → P1-17 → P1-18 → P1-19 → P1-20 → P1-21
```

并行只允许在依赖已满足且主要写入区域不重叠时启动。具体领取、并行冲突、评审和阻塞处理遵循《任务流程》。

---

## 六、阶段放行引用

P0/P1 的正式退出条件不在本计划复制，统一使用《产品与架构演进方案》的“阶段、能力与边界控制表”。放行操作使用《任务流程》的“小阶段收口与阶段放行”，不从任务 Done 状态自动推导。

2026-09-12，维护者在当前任务中确认：“如果试验结果是预期的，确认放行”。此处保存有条件确认的原话，不预先将对象扩大为整个 P0/P1；主 Agent 已询问是整个 P0 技术验证阶段还是当前单项实验，范围待回复。这也不表示条件已经成立，不替代规定模型审核或完整阶段门禁。实际放行须绑定明确对象、候选 commit、逐项实验和门禁证据、遗留边界及结果；存在未达预期或影响退出条件的未验证项时，不得据此宣布放行。


## 七、本次首版设计修订记录

2026-09-13，维护者确认原始目录镜像、tracked-only 强提示、可强制清理并记录日志，交付由任务流程负责。主 Agent 负责本次 P0/P1 设计修订；风险 R4（改变删除与恢复设计，仅修改文档，无实际清理），基线 `1c73139`，工作区 `thinworkspace-p0-03`、分支 `task/p0-03-git-base`。既有 18 项未提交实验/配置输入保留，不混入文档提交。

验收断言：当前权威文档无旧 Git/Base 的正向首版依赖；创建不筛选文件；清理不提示未跟踪内容；force 有日志且无交付证明门槛；commit 交付只在任务流程详细定义；链接、JSON 和取消任务依赖一致。纯文档不运行行为 RED，以这些可失败的文档检查替代，不声称新产品行为已验证。

验证环境：macOS 15.7.2 / arm64，在上述工作区运行；真实 APFS 用例使用工作区内临时目录及 `target/p0-materialize-tests/` 受控测试树，不操作用户 Workspace。

- 文档检查通过：检查 21 份 Markdown、95 个本地链接目标、2 个 JSON 示例和 23 行任务表；代码围栏、已取消任务依赖和旧 CLI 正向示例检查无错误。`git diff --check` 通过。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`cargo test --workspace --all-targets` 均以退出码 0 完成；普通测试共 245 通过、0 失败、2 忽略。门禁使用保留原状的既有实验代码，包含既有物化故障注入用例，不代表新设计的实现验收。
- 未执行：2 项真实跨卷测试（未提供 `THINWS_P0_CROSS_VOLUME_ROOT`）；变异、fuzz、发布候选和远端 CI（本次仅文档任务，未进入小阶段/阶段收口）；新原始目录保真、只读 Git 和 force 日志的产品测试（尚未实现，按 P0-06、P0-07 及后续任务验证）。
- 规定审核模型 GPT-6 Astra（`gpt-6-astra`）/ `xhigh` 已完成本轮 15 份文档差异审核，结论 Approve，仅针对文档设计。审核提出的子仓库递归访问前预检、partial-clone 隐式下载边界、成果在副本外持久保存三项已修正并复核；未审核既有脏实验代码，也不将自动门禁结果冒充审核模型实测。
- 既有 18 项未提交实验/配置输入逐文件 SHA-256 比对一致；本次仅提交文档，不清理或混入这些输入。

剩余风险：原样 `.git` 外部引用、逐文件镜像的一致性和 mtime 保真仍受新设计边界及待执行实验约束；未跟踪内容与副本内独有 commit 均可能随清理删除，交付流程须在清理前落实。此次文档修订、审核通过和旧任务取消均不构成 P0/P1 放行。

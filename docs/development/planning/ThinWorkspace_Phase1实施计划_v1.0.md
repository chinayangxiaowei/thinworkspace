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

P0.a 已完成任务级与小阶段验收，P0-01 已获规定模型正式 Approve 并合并；精确提交、CI、未执行边界及工作区保留原因只记录在上述实施记录。整个 P0 尚未放行。

P0-02 已获规定模型正式 Approve，审核后真实 Verification 和远端候选 CI 通过，PR 已合并。准备、验收断言、精确提交、验证证据和保留边界只在 [P0-02 实施记录](../implementation/P0-02_APFS物化实验.md) 维护；任务 Done 不代表 P0.b 或整个 P0 放行。

旧 P0-03 的准入、实验和未完成门禁保存在 [P0-03 历史记录](../implementation/P0-03_Git与Base实验.md)。取消不等于验收通过，不删除其中代码、失败日志、scratch 或保留工作区；可复用的小型代码须在新任务中独立审查，不能带回 Git/Base 领域。

当前顺序：P0-06/P0-07/P0-05 已完成 → 核验当前未取消的 P0 控制点与阶段门禁 → 人工确认 P0 放行 → 按依赖进入 P1。P0-06 的基线、保真范围、CI 修复与收口见 [P0-06 实施记录](../implementation/P0-06_原始目录镜像实验.md)。P0-07 的审核、Verification、门禁及 PR #7 合并见 [P0-07 实施记录](../implementation/P0-07_已跟踪检查与清理提示实验.md)的最终收口。P0-05 的首轮实测、精确审核、候选 CI 与 PR #8 合并见 [P0-05 实施记录](../implementation/P0-05_文件副本锁隔离实验.md)。各任务 Done 不扩大验证范围，也不代表 P0.b 或整个 P0 放行。

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
| P1-09 Workspace create | 从 source 直接镜像，只有完整物化并持久化后才成为 Ready | P1-06、P1-07 | R4 | In Progress |

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

P1-09 Ready 目录核验子结果（进行中）：同名幂等或未来路径查询在数据库 Ready 与 final receipt 之外，还要由既有 `BootstrapStore` 在 data-root lifecycle lock 下验证受控 `workspaces/<id>` 私有容器、无 incomplete 标记、普通 root 的身份/属主/卷与路径。`root/` 不强制 `0700`，因为已经继承来源目录权限；符号链接替换和残留标记由真实文件系统测试拒绝。该核验不自动修复缺损目标，也不取代 P1-10 status 的 Git 检查。

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

### 4.4 P1.d 查询与路径交付

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-10 status/list/path | 状态边界、只读检查和普通路径输出；diff 由用户直接使用 Git | P1-09、P1-16 | R3 | Backlog |
| P1-16 GitInspector | 主/子仓库已跟踪变更、无仓库/不完整检查报告，不写 Git | P1-03、P0-07 | R3 | Backlog |
| P1-11 用户命令执行包装（已取消） | 不再实施；已有外部占用检查责任归入 P1-12，无替代执行服务 | 不再参与依赖 | —（历史任务） | Cancelled |

小阶段退出：查询不写状态，非 Ready 行为与用户契约一致；返回的普通路径可供现有工具直接使用，不要求执行包装或工具链环境注入。

### 4.5 P1.e 删除与回收

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-12 remove 与强制清理日志 | tracked-only 普通拒绝、显式 force、普通持久日志、ProcessProbe 和受控整目录清理 | P1-09、P1-16 | R4 | Backlog |
| P1-13 GC 与空间统计 | 快照计划、锁内重验和受限回收范围 | P1-12 | R4 | Backlog |

小阶段退出：未跟踪文件不提示/不阻塞；tracked/unknown 可显式 force 且日志可读；不加交付硬门禁；路径/卷/占用保护和 GC 范围不被绕过。

### 4.6 P1.f 契约与发布

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-14 JSON/错误码兼容 | 新手册、help、fixture 和退出码一致，旧参数无兼容入口 | P1-03、P1-06、P1-07、P1-09、P1-10、P1-12、P1-13、P1-16 | R3 | Backlog |
| P1-15 Phase 1 端到端与失败边界验收 | release 二进制、真实平台、长预算质量门禁和候选发布证据；不含中断恢复 | 全部未取消的前置 P1 任务 | R4 | Backlog |

小阶段退出：全部公开契约与发布二进制一致，未执行门禁和剩余风险已列出，进入人工阶段放行。

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

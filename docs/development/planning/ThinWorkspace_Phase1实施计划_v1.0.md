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
| P0-03 Git 托管拓扑与 Base 实验 | 冻结 bare repository＋linked worktree、独立 index 和 Base 发布 ADR | P0-01 | R4 | Backlog |
| P0-04 生命周期故障注入 | 证明创建/删除中断后状态可解释、可重放或安全停止 | P0-02、P0-03 | R4 | Backlog |

P0-02 和 P0-03 可在 P0-01 输出稳定后并行；P0-04 必须等两者的证据格式和 ADR 结论可用。

### 3.1 P0 小阶段与当前领取

负责人为主 Agent；模型分工遵循《任务流程》§8.3。小阶段划分只用于收口，不改变上表依赖。

| 小阶段 | 包含任务 | 可观察子目标和退出条件 | 适用门禁 |
|---|---|---|---|
| P0.a 路径能力证据 | P0-01 | 实际路径的卷身份、权限和缺失目标证据可复查；路径/符号链接替换被识别；预检不产生 CoW confirmed | Rust 通用门禁、真实 APFS 和跨卷身份测试、受影响 crate 全量变异、路径 fuzz、专项审核 |
| P0.b 物化与 Git 可行性 | P0-02、P0-03 | APFS/Full Copy 和托管 Git 拓扑可重复执行；失败边界与候选 ADR 由证据支持 | 小阶段门禁、真实 clone/跨卷/回滚、真实 Git/index 与 Base 验证 |
| P0.c 故障闭环 | P0-04 | 创建/删除中断可恢复或安全停止；P0 阶段退出控制点逐项有证据 | 完整阶段门禁和人工放行 |

P0-01 从文档基线 `f7a400f` 开始，工作分支为 `task/p0-01-host-path-probe`。本次使用 FD 与卷属性 FFI，按最高影响将风险从 R3 调整为 R4。验收断言、实际命令、审核结论和未完成项统一记录在 [P0-01 实施记录](../implementation/P0-01_HostPathProbe实验.md)。P0.a 收口不代表整个 P0 放行。

P0.a 已完成任务级与小阶段验收，P0-01 已获规定模型正式 Approve 并合并；精确提交、CI、未执行边界及工作区保留原因只记录在上述实施记录。整个 P0 尚未放行。

P0-02 已获规定模型正式 Approve，审核后真实 Verification 和远端候选 CI 通过，PR 已合并。准备、验收断言、精确提交、验证证据和保留边界只在 [P0-02 实施记录](../implementation/P0-02_APFS物化实验.md) 维护；任务 Done 不代表 P0.b 或整个 P0 放行。

---

## 四、P1 实施序列

### 4.1 P1.a 工程与持久化基础

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-01 Rust workspace 和 Core 类型 | crate 依赖门禁、类型 ID、错误模型与基础测试 | P0 结论 | R2 | Backlog |
| P1-02 BootstrapStore、SQLite schema、双 scope 锁和 operation | 可迁移 schema、实例身份、并发唯一性和恢复记录 | P1-01 | R4 | Backlog |
| P1-03 init/doctor | 完成可见初始化、能力诊断和 bootstrap 恢复边界 | P1-02 | R3 | Backlog |

小阶段退出：全新、幂等、中断和冲突初始化都有自动证据；未经验证的 data root 不被接管。

### 4.2 P1.b Repository 与 Base

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-04 托管 Repository | repo add/fetch/list，原子导入与 operation/Receipt 恢复 | P1-03、P0-03 | R4 | Backlog |
| P1-05 ResolvedBase、BaseId 与 Base builder | 固定 commit/tree，可复用、可验证的不可变 Base | P1-04 | R4 | Backlog |

小阶段退出：本机/远程来源导入、ref 更新、Base 复用/损坏隔离及中断恢复通过真实 Git 验证。

### 4.3 P1.c 物化与 Workspace 创建

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-06 APFS WorkspaceMaterializer | 实现已冻结物化 Port 和真实 CoW Receipt | P1-05、P0-02 | R4 | Backlog |
| P1-07 Full Copy WorkspaceMaterializer | 独立后端和受策略限制的显式降级 | P1-05、P0-02 | R4 | Backlog |
| P1-08 GitBackend attach/status/detach | 唯一托管分支、独立 index 和精确 detach | P1-04、P0-03 | R4 | Backlog |
| P1-09 Workspace create/reconciliation | 闭环 Creating 到 Ready/Error 及中断恢复 | P1-06、P1-07、P1-08 | R4 | Backlog |

小阶段退出：默认 CoW、显式复制、路径竞态、partial rollback、Git clean 和中断恢复均通过真实 APFS/Git 门禁。

### 4.4 P1.d 查询与前台执行

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-10 status/list/path/diff | 状态边界、只读一致性检查和稳定输出 | P1-09 | R3 | Backlog |
| P1-11 ProcessSupervisor 与 exec | 前台流转发、进程组、超时/中断和 execution 恢复 | P1-09 | R4 | Backlog |

小阶段退出：查询不写状态，非 Ready 行为与用户契约一致，exec 的终止和孤儿情形可解释。

### 4.5 P1.e 删除与回收

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-12 remove 与分支保护 | dirty、进程、保护引用、幂等删除和 tombstone | P1-09、P1-11 | R4 | Backlog |
| P1-13 GC 与空间统计 | 快照计划、锁内重验和受限回收范围 | P1-12 | R4 | Backlog |

小阶段退出：各类保护不能被无关 flag 绕过，每个破坏性步骤前重验，GC 不进入禁止范围。

### 4.6 P1.f 契约与发布

| 任务 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| P1-14 JSON/错误码兼容 | 手册、help、fixture 和退出码一致 | P1-03～P1-13 | R3 | Backlog |
| P1-15 Phase 1 端到端与故障验收 | release 二进制、真实平台、长预算质量门禁和候选发布证据 | 全部 P1 任务 | R4 | Backlog |

小阶段退出：全部公开契约与发布二进制一致，未执行门禁和剩余风险已列出，进入人工阶段放行。

---

## 五、依赖摘要

```text
P0-01 → P0-02 ─┐
      └→ P0-03 ─┴→ P0-04

P1-01 → P1-02 → P1-03 → P1-04 → P1-05
                                      ├→ P1-06 ─┐
                                      ├→ P1-07 ─┼→ P1-09
                                      └→ P1-08 ─┘
                                                   ├→ P1-10
                                                   └→ P1-11 → P1-12 → P1-13
P1-03～P1-13 → P1-14 → P1-15
```

并行只允许在依赖已满足且主要写入区域不重叠时启动。具体领取、并行冲突、评审和阻塞处理遵循《任务流程》。

---

## 六、阶段放行引用

P0/P1 的正式退出条件不在本计划复制，统一使用《产品与架构演进方案》的“阶段、能力与边界控制表”。放行操作使用《任务流程》的“小阶段收口与阶段放行”，不从任务 Done 状态自动推导。

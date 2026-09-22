# ThinWorkspace Phase 0 阶段状态

**状态：门禁执行中，尚未放行｜候选：`6f9a85d`｜负责人：主 Agent**

## 一、职责与非职责

本文是 Phase 0 小阶段收口、阶段门禁、独立审核和人工放行结果的唯一状态页。它只记录候选、控制点核对、实际门禁证据、遗留边界和最终签字，不重定义产品、架构、CLI、测试规则或任务状态。

产品退出控制点以《产品与架构演进方案》为准，任务状态和依赖以《Phase 1 实施计划》为准，门禁与放行时序以《任务流程》为准。本文不是独立 Agent 审核报告，也不能代替人工确认。

## 二、候选与范围

- 阶段：Phase 0（关键技术验证）；小阶段 P0.a、P0.b。
- 当前提交：`6f9a85d`。相对 `origin/main` 的两项本地提交仅调整首版中断边界及其文档联动；本轮生产源码、测试、依赖、fuzz harness、工具和变异配置未改变。
- 未取消任务：P0-01、P0-02、P0-05、P0-06、P0-07，均在实施计划中为 Done。P0-03、P0-04 为 Cancelled，不作为已交付能力或通过证据。
- 发布边界：Phase 0 仍是非产品实验，不发布完整 CLI，不包含 Git 托管/Base、自动交付、daemon、UI 或中断恢复。

## 三、P0.b 与阶段退出控制点

| 控制点 | 当前证据 | 状态 |
|---|---|---|
| 同卷 CoW 与跨卷限制可复查 | 当前候选 Debug/Release 真实 APFS clone、独立 APFS 镜像跨卷和子挂载用例均退出 0 | 已满足，待独立审核 |
| 原样 `.git`、未跟踪和 ignored 内容不被过滤 | P0-06 已验收；当前候选原始目录物化回归在 Debug/Release 均通过 | 已满足，待独立审核 |
| 支持范围内只读 Git 检查准确，未知如实报告 | P0-07 已验收；当前候选真实 Git、配置拒绝、unknown 与输出预算回归通过 | 已满足，待独立审核 |
| 普通拒绝不删，显式强制有日志 | P0-07 已验收；当前候选清理、日志和失败注入回归在 Debug/Release 均通过 | 已满足，待独立审核 |
| 锁隔离形成 Go/No-Go 结论 | P0-05 L1–L7 结论为不实现“扫描＋重写”；当前候选真实双进程锁回归通过 | 已满足，待独立审核 |

任务 Done 与上表技术证据都不等于阶段放行。完整变异、独立审核及人工确认仍是阻断项。

## 四、当前阶段门禁证据

环境：arm64 macOS 15.7.2（24G325）；工作区位于 APFS `/dev/disk5s1`，Volume UUID `1A42C888-32E3-489C-9BFA-67FD640A94E8`。

| 门禁 | 实际结果 |
|---|---|
| 格式、静态检查、仓库 helper | 根/fuzz rustfmt、根/fuzz Clippy、20 项 CI helper 测试均退出 0 |
| Debug/Release 与真实平台 | release workspace build 退出 0；Debug/Release 全 workspace `--include-ignored` 均退出 0；真实同卷、跨卷和子挂载场景均执行，临时镜像已卸载并清理 |
| 供应链 | 根/fuzz `cargo deny` 退出 0，仅有允许项未命中警告；根/fuzz `cargo audit --no-fetch --deny warnings` 基于本地 1243 条公告退出 0。在线公告刷新进程静止且无网络连接后已终止，不把在线刷新记为完成 |
| 长预算 fuzz | 固定 nightly `nightly-2026-08-14`、`cargo-fuzz 0.12.0`；三个目标各 `-max_total_time=600 -timeout=5 -max_len=4096`，均运行 602 秒并退出 0，artifact 均为 0。Probe 6,003,717 次，Materialize 37,770,320 次，Git status 3,673,820 次；原始日志位于忽略目录 `target/p0-stage-fuzz.ULlkPP/` |
| 文档与公开契约 | 25 份 Markdown、136 个本地链接、2 个 JSON 示例、围栏和 `git diff --check` 均通过；Phase 0 不发布产品 CLI，实验 CLI 契约测试在当前 Debug/Release 门禁通过 |

## 五、全 workspace 变异执行登记

- 候选：`6f9a85d`；执行负责人：主 Agent。
- 计划启动：2026-09-22 08:06 UTC；首次主动查看按无当前可比完整 workspace 成功批次暂定为启动后 1 小时。
- 范围：`cargo-mutants 27.1.0` 的完整 workspace 选择集；不使用模块、crate 或 diff 过滤。
- 固定参数：单变异超时 60 秒、并发 2、`--leak-dirs`、`--test-workspace true`，测试参数 `--include-ignored --nocapture`。
- 入口：`python3 -B tools/ci_mutation.py --scope workspace`；结果位置：`target/mutants.out/outcomes.json`。
- 执行期间不修改生产源码、测试、依赖、fuzz harness、工具或变异配置；文档结果回填按《任务流程》§18.1 单独核验，不冒充重新执行。
- 当前结果：待运行。

## 六、尚未完成

1. 全 workspace 变异取得完整终态，并处置所有 missed/timeout 或证据异常。
2. GPT-6 Astra / `xhigh` 对最终本地候选做只读独立审核；审核 Agent 不修改文件、不创建分支或 PR。
3. 主 Agent 在审核后完成 Verification，列明 Critical/High、Normal/Low 和开放边界。
4. 维护者对明确的 Phase 0 候选与证据人工确认放行；确认前不打 tag、不进入 P1-01。


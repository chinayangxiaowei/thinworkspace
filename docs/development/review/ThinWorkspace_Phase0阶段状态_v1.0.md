# ThinWorkspace Phase 0 阶段状态

**状态：已人工放行｜候选：`a384c51`｜本地阶段标签：`phase0`｜负责人：主 Agent**

## 一、职责与非职责

本文是 Phase 0 小阶段收口、阶段门禁、独立审核和人工放行结果的唯一状态页。它只记录候选、控制点核对、实际门禁证据、遗留边界和最终签字，不重定义产品、架构、CLI、测试规则或任务状态。

产品退出控制点以《产品与架构演进方案》为准，任务状态和依赖以《Phase 1 实施计划》为准，门禁与放行时序以《任务流程》为准。本文不是独立 Agent 审核报告，也不能代替人工确认。

## 二、候选与范围

- 阶段：Phase 0（关键技术验证）；小阶段 P0.a、P0.b。
- 当前候选：`a384c51`。相对 `origin/main` 的本地提交同步了“不实现中断恢复”的首版边界、增加本阶段状态页，并补充一个 `#[cfg(test)]` Git 子进程环境断言；另将一个仍含 `for_recovery` 的测试名改为“不做不安全回滚”。生产逻辑、依赖、fuzz harness、工具和变异配置未改变。
- 未取消任务：P0-01、P0-02、P0-05、P0-06、P0-07，均在实施计划中为 Done。P0-03、P0-04 为 Cancelled，不作为已交付能力或通过证据。
- 发布边界：Phase 0 仍是非产品实验，不发布完整 CLI，不包含 Git 托管/Base、自动交付、daemon、UI 或中断恢复。

## 三、P0.b 与阶段退出控制点

| 控制点 | 当前证据 | 状态 |
|---|---|---|
| 同卷 CoW 与跨卷限制可复查 | 当前候选 Debug/Release 真实 APFS clone、独立 APFS 镜像跨卷和子挂载用例均退出 0 | 已满足 |
| 原样 `.git`、未跟踪和 ignored 内容不被过滤 | P0-06 已验收；当前候选原始目录物化回归在 Debug/Release 均通过 | 已满足 |
| 支持范围内只读 Git 检查准确，未知如实报告 | P0-07 已验收；当前候选真实 Git、配置拒绝、unknown 与输出预算回归通过 | 已满足 |
| 普通拒绝不删，显式强制有日志 | P0-07 已验收；当前候选清理、日志和失败注入回归在 Debug/Release 均通过 | 已满足 |
| 锁隔离形成 Go/No-Go 结论 | P0-05 L1–L7 结论为不实现“扫描＋重写”；当前候选真实双进程锁回归通过 | 已满足 |

任务 Done 与上表技术证据都不等于阶段放行。完整变异及其失败项处置、最终独立审核、审核后 Verification 和维护者人工确认均已完成。

## 四、当前阶段门禁证据

环境：arm64 macOS 15.7.2（24G325）；工作区位于 APFS `/dev/disk5s1`，Volume UUID `1A42C888-32E3-489C-9BFA-67FD640A94E8`。

| 门禁 | 实际结果 |
|---|---|
| 格式、静态检查、仓库 helper | 根/fuzz rustfmt、根/fuzz Clippy、20 项 CI helper 测试均退出 0；测试补丁后再次执行两 workspace rustfmt/Clippy，均退出 0 |
| Debug/Release 与真实平台 | release workspace build 退出 0；Debug/Release 全 workspace `--include-ignored` 均退出 0；真实同卷、跨卷和子挂载场景均执行，临时镜像已卸载并清理。测试补丁后普通全 targets 为 260 passed、7 ignored，新增断言及改名用例的 release 精确测试均退出 0；补充变异的未变异基线再次执行本次选择的 Cleanup/Probe ignored Debug 场景并通过 |
| 供应链 | 根/fuzz `cargo deny` 退出 0，仅有允许项未命中警告。RustSec 缓存以普通 Git 快进至上游 `17af77682cecd2afa72b217ad7c6c30585d5003f`（2026-09-22），根/fuzz `cargo audit --no-fetch --deny warnings` 均载入 1261 条公告并退出 0；`cargo-audit` 内置刷新本次静止后被终止，不把该失败尝试冒充成功 |
| 长预算 fuzz | 固定 nightly `nightly-2026-08-14`、`cargo-fuzz 0.12.0`；三个目标各 `-max_total_time=600 -timeout=5 -max_len=4096`，均运行 602 秒并退出 0，artifact 均为 0。Probe 6,003,717 次，Materialize 37,770,320 次，Git status 3,673,820 次；原始日志位于忽略目录 `target/p0-stage-fuzz.ULlkPP/` |
| 文档与公开契约 | 26 份 Markdown、137 个真实本地链接、2 个 JSON 示例、围栏和 `git diff --check` 均通过；Phase 0 不发布产品 CLI，实验 CLI 契约测试在当前 Debug/Release 门禁通过 |
| 远端执行 | 按维护者要求，本轮不创建线上 PR、不启动远端 CI；上述结果均为本机实际门禁，不将其表述为 CI 通过 |

## 五、全 workspace 变异执行登记

- 初始完整候选：`e9ae736`；最终测试候选：`a384c51`；执行负责人：主 Agent。
- 范围：`cargo-mutants 27.1.0` 的完整 workspace 选择集；不使用模块、crate 或 diff 过滤。
- 固定参数：单变异超时 60 秒、并发 2、`--leak-dirs`、`--test-workspace true`，测试参数 `--include-ignored --nocapture`。
- 入口：`python3 -B tools/ci_mutation.py --scope workspace`；初始完整结果移至 `target/p0-stage-mutation-full-initial/outcomes.json`，补充结果位于 `target/p0-stage-mutation-unresolved-followup/mutants.out/outcomes.json`。
- 执行期间不修改生产源码、测试、依赖、fuzz harness、工具或变异配置；文档结果回填按《任务流程》§18.1 单独核验，不冒充重新执行。

首次运行于 2026-09-22 08:06:19 UTC 启动，工具枚举出 1,091 个 mutants，但 unmutated baseline 因未设置 `THINWS_P0_CROSS_VOLUME_ROOT` 而在真实跨卷 ignored 测试失败，08:06:46 UTC 以 exit 4 结束。结果为 0 caught、0 missed、0 timeout、0 unviable、0 success，明确是环境配置失败，不能作为变异证据；原始结果移入忽略目录 `target/p0-stage-mutation-baseline-env-failure/` 保留。

完整重跑保持同一 1,091 项范围和全部固定参数，在 `/private/tmp` 创建专用 APFS 镜像并同时设置 `THINWS_P0_CROSS_VOLUME_ROOT`、`THINWS_P0_SUBMOUNT_SOURCE`。工具从 2026-09-22 08:35:28 UTC 运行至 11:19:41 UTC，精确耗时约 2 小时 44 分；未变异基线通过，终态为 908 caught、166 unviable、1 missed、16 timeout，exit 3。原始 `outcomes.json` SHA-256 为 `09d0dba9f290b0aa9c2f46b2524ba84ff04e92779f3dcf70f689576cc1e745e2`；专用卷已由受控 helper 卸载，临时根已删除。

唯一 missed 是 `set_optional_env` 整体替换为 `()`：已有测试验证了环境捕获，却没有直接断言捕获值被注入固定 Git 命令。最终候选增加 `preserved_query_environment_is_forwarded_exactly`，逐项验证六个保留键；原实现无需修改。16 个 timeout 均发生在 Probe 变异的测试阶段，双 worker 下整套 workspace 测试超过固定 60 秒，原结果没有证明 caught 或 missed。

按《任务流程》§18.1 只补跑上述 17 个未解决候选；`cargo-mutants` 还自动纳入两个既有运行预算字段控制变异，因此实际选择 19 项。补跑使用独立 APFS 卷、单 worker、180 秒上限和相同完整测试集，从 11:28:01 UTC 运行至 11:33:31 UTC，未变异基线通过，19 项全部 caught，0 missed、0 timeout、0 unviable，exit 0；结果 SHA-256 为 `b01bb098aa788518443ebd7b5c78a6d0cc12fa0e0da34d94c5f4b219d6de4a16`，专用卷和临时根均已清理。最终候选的 1,091 个变异名称与初始完整列表精确相同；按唯一候选合并为 925 caught＋166 unviable，未解决项为 0，不把两个重复控制变异再次计入总数，也不为生成绿色摘要重跑已通过的 908 项。

## 六、独立审核

GPT-6 Astra / `xhigh` 已对 `6f9a85d` 及证据载体 `dd2b7c1` 完成只读独立审核并 Approve：Critical、High 和阻断性 Normal finding 均为 0。审核未修改文件、创建分支或运行耗时门禁，也没有把 mutation、在线公告刷新、Verification 或人工放行记为完成。

同一 GPT-6 Astra / `xhigh` 随后对最终测试候选 `a384c51` 和证据提交 `152b152` 完成只读独立审核并 Approve：Critical、High、Normal、开放 Low 均为 0。审核核对 1,091 项完整结果、19 项补跑、候选列表一致性、APFS/fuzz/RustSec 证据和全部本地差异；唯一发现是补跑 baseline 曾被写成覆盖全部包，主 Agent 已收窄为实际 Cleanup/Probe 范围，Reviewer 对 `152b152` 复核关闭。审核没有修改文件、运行长门禁或操作远端。

审核后主 Agent 以 release profile 精确重跑新增 Git 环境断言和改名后的未确认对象用例，各 1 passed、0 failed；`git diff --check` 通过，工作树干净。Verification 不重跑已经审核且候选未变化的全量 mutation/fuzz/APFS/供应链门禁，也不扩大为 Phase 1 产品验收。

同一 GPT-6 Astra / `xhigh` 最后对证据提交 `e478325` 完成只读文档复核并 Approve：Critical、High、Normal、Low 均为 0；确认阶段状态准确表达“仅待人工放行”，无需再修改门禁证据。该复核未修改文件、运行长门禁或操作远端。

## 七、人工放行记录

- 放行阶段：Phase 0（关键技术验证）。
- 放行候选：`a384c51`；人工确认前的最终证据提交：`e478325`。
- 退出控制点：第三节五项全部满足；P0-01、P0-02、P0-05、P0-06、P0-07 均为 Done，P0-03、P0-04 依既定范围为 Cancelled，不冒充已交付能力。
- 门禁结论：第四、五节记录的本机 macOS/APFS Debug/Release、静态检查、供应链、真实平台、全 workspace 变异和三个长预算 fuzz 门禁均已完成；独立审核及审核后 Verification 已通过。未执行线上 PR 或远端 CI，且不宣称其已通过。
- 缺陷结论：Critical、High、Normal、Low 未关闭 finding 均为 0；变异未解决项为 0。
- 开放边界：本次仅放行非产品的 Phase 0 技术验证；不发布完整 CLI，不交付 Git 托管/Base、自动交付、daemon、UI、中断恢复、Linux 或 Windows 产品能力。原始长门禁日志保留在本机忽略目录，不作为仓库内可迁移制品。这些是已声明范围边界，不是已通过的能力。
- 确认人：维护者杨小卫；确认来源：当前对话明确指令“确认放行 Phase 0”。
- 确认时间：2026-09-22T04:56:28-07:00（2026-09-22T11:56:28Z）。
- 最终标签：本地注释标签 `phase0`，指向包含本放行记录的提交；该标签不推送远端，也不表示产品语义版本发布，因此 `CHANGELOG.md` 继续保留 `Unreleased`。

人工确认完成后，P1-01 的阶段依赖已经解除；Phase 1 仍须按实施计划另行启动和验收，不能复用本记录宣称已完成。

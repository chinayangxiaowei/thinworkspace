---
name: thinws-phase1-acceptance
description: Plan or execute ThinWorkspace Phase 1 macOS/APFS acceptance with concrete CLI commands, pass/fail evidence, and stage-gate boundaries. Use for full P1 functional testing or release-candidate verification, not routine code-edit checks.
---

# ThinWorkspace Phase 1 验收

本 Skill 让其他 Agent 在当前仓库执行或编排一次可复查的 Phase 1 验收。它不定义新的产品行为、测试门槛或放行权限；这些事实仍由仓库文档维护。用户只要测试清单时输出可执行计划；用户要求执行时才运行测试。不要因调用本 Skill 自动 push、打 tag、创建 PR、修改生产代码或宣布阶段放行。

## 先确定范围

1. 从仓库根目录读取 `AGENTS.md`，按其中路由读取当前版本的[用户操作手册](../../../docs/project/reference/ThinWorkspace_Phase1用户操作手册_v1.0.md)、[任务流程 §10、§18](../../../docs/development/process/任务流程.md)、[架构方案 §13](../../../docs/project/architecture/ThinWorkspace_产品与架构演进方案_v2.1.md)及[实施计划 P1 当前状态](../../../docs/development/planning/ThinWorkspace_Phase1实施计划_v1.0.md)。内部故障注入与物化边界分别从 Phase 1 详细设计和跨平台物化设计提取；不要复制其矩阵到本 Skill。
2. 记录候选 commit、`git status --short`、`sw_vers`、`uname -m`、`rustc --version`、`git --version`、二进制版本与 SHA-256。工作树或二进制与候选不一致时，先说明实际测试对象；不得沿用另一候选的整体通过结论。
3. 区分两种范围：**功能验收**覆盖下述公开 CLI 场景和适用自动测试；**阶段放行核验**还必须按《任务流程》§18.1 补齐全量变异、全部 fuzz target 长预算、真实跨卷/子挂载、性能及供应链证据。用户仅要求人工体验或功能测试时，不擅自启动耗时阶段门禁；报告中明确“未执行”，而非“通过”。

## 环境与数据边界

- 首发目标是 macOS/APFS。先用 `diskutil info -plist <测试卷目录>` 核实 APFS 与 Volume UUID；跨卷测试必须核实两个真实不同 UUID。不要仅凭路径前缀判断同卷。
- 正式 bootstrap 配置路径固定，`init` 会影响当前账户。不在用户的日常账户运行新实例 `init`，不覆盖配置，也不手动改写 `HOME` 冒充账户隔离。新实例初始化和发布二进制完整黑盒测试使用专用一次性 macOS 账户或经用户批准的隔离运行环境；否则标为未执行，并运行无需重新初始化的场景。
- 只在经核验的测试 APFS 卷建立唯一临时来源和唯一 Workspace 名称。执行前记录来源、data root、Workspace ID 与预期删除范围；来源不得是本仓库、真实项目或用户已有工作区。失败时保留现场和证据，不猜测清理。`workspace remove --force` 只可针对本轮创建且归属已核实的测试对象；不以 `rm -rf` 清理 data root 或工作区。
- 手册示例不是可直接粘贴的测试路径。以下命令的 `THINWS_TEST_BIN`、`THINWS_TEST_SOURCE`、`THINWS_TEST_NAME` 必须先由执行 Agent 赋为已核验的绝对路径/本轮唯一名称；不要复用示例 `auth-refresh`。JSON 用 `jq` 或等价解析器检查字段，不以人类输出字符串代替稳定契约。

## 公开 CLI 功能验收

在上述变量和一次性夹具准备好后，按顺序运行；每项记录**命令、退出码、关键 JSON 字段/文件事实、是否通过**。失败不继续执行会扩大损失的后续操作。

| 场景 | 命令或动作 | 判定 |
|---|---|---|
| 主机与实例 | `"$THINWS_TEST_BIN" --json doctor` | exit 0；`ok=true`、`data.status=ready`，Volume ID 与已登记根一致。新实例 `init` 仅在上述隔离条件下验证首次 `initialized`、再次 `already-initialized` 和冲突拒绝。 |
| 预览 | `"$THINWS_TEST_BIN" --json workspace create --source "$THINWS_TEST_SOURCE" --name "$THINWS_TEST_NAME" --dry-run` | exit 0；`workspace_id=null`、无实际 `cow`/成功回执；不产生该名称的活动记录。 |
| 创建与幂等 | `"$THINWS_TEST_BIN" --json workspace create --source "$THINWS_TEST_SOURCE" --name "$THINWS_TEST_NAME"`，以完全相同的命令再运行一次 | 首次 `created`、`state=ready`、普通文件实际 clone 时 `actual_mode=cow-clone`/`cow=confirmed`；二次 `already-ready` 且 ID/路径不变。来源与副本内容一致，来源在副本写入后不变。 |
| 普通路径与状态 | `"$THINWS_TEST_BIN" workspace path "$THINWS_TEST_NAME"`；`"$THINWS_TEST_BIN" --json workspace status "$THINWS_TEST_NAME"`；`"$THINWS_TEST_BIN" --json workspace list` | path 的 stdout 仅绝对路径加换行；可不经执行包装直接访问；status 为 Ready，空间字段为当前估算或明确 unknown；list 按名称排序，不把当前 Git/空间扫描结果当持久创建回执。 |
| 无 Git / Git 只读 | 分别以非 Git 来源、含自包含 `.git` 的本轮临时来源测试；修改副本中已跟踪文件，并另测仅 untracked/ignored 修改 | 无 Git 为 `not-applicable`；tracked 修改为 `dirty`；仅 untracked/ignored 不计入 tracked_changes。主仓库、嵌套仓库、暂存/删除/冲突和外部元数据边界须结合真实 Git 集成测试核对。 |
| 普通清理与强制 | 对 dirty 副本运行 `"$THINWS_TEST_BIN" --json workspace remove "$THINWS_TEST_NAME"`；核验后才对同一测试 ID 加 `--force` | dirty 时 exit 22 / `E_WORKSPACE_DIRTY`，副本仍存在；force 成功后只删除该受控副本、来源保持不变，`logs/operations.jsonl` 留有如实结果。另测 clean 和仅 untracked 的普通清理成功；force 不绕过归属/卷/确认占用保护。 |
| 拒绝与范围外 | `"$THINWS_TEST_BIN" --json gc`、`"$THINWS_TEST_BIN" --json workspace exec`；在专用夹具上测跨卷源、源与 data root 包含关系 | 撤销的命令 exit 2 / `E_USAGE`；不支持的布局按手册稳定错误码拒绝，不创建 Ready、不静默 Full Copy。`--allow-copy` 只允许规定的同卷 clone 不支持情形，不当作跨卷开关。 |

每次运行还应逐条核对手册的命令/JSON 矩阵、退出码和 Phase 1 退出控制点，特别是 10 个副本普通文件写入互不影响、未完成状态不可 `path`、删除前归属/进程占用保护、日志在删除后可读。难以安全地从公开 CLI 制造的故障，用既有真实 APFS/Git/SQLite、并发和故障注入测试证据覆盖；不要破坏真实实例来制造错误。

## 自动门禁入口

功能验收至少在候选上运行下列命令；专用 APFS 卷和 `THINWS_P0_SUBMOUNT_SOURCE`、`THINWS_P1_SUBMOUNT_SOURCE`、`THINWS_P0_CROSS_VOLUME_ROOT`、`THINWS_P1_CROSS_VOLUME_ROOT` 的准确准备方式以当前 [P0/P1 quality 工作流](../../../.github/workflows/p0.yml)为准。没有已核实的独立挂载卷时，不执行 `--include-ignored` 的环境专属测试并报告缺口；普通 `cargo test` 跳过这些用例，不等价于完整平台验收。

```bash
cargo fmt --all -- --check
cargo fmt --manifest-path fuzz/Cargo.toml -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
RUSTFLAGS="--cfg fuzzing" cargo clippy --locked --manifest-path fuzz/Cargo.toml --all-targets --all-features -- -D warnings
python3 -B tools/check_crate_dependencies.py
cargo test --locked --workspace --all-targets
cargo test --locked --release --workspace --all-targets
cargo test --locked --workspace --doc
cargo build --locked --release --workspace
```

补跑工作流的仓库工具单测、主/fuzz 两套 `cargo deny` 与 `cargo audit`；离线 `audit --no-fetch` 只说明本地公告缓存版本，不能冒充在线最新结果。对已准备的真实独立 APFS 卷，按工作流 Debug/Release 运行 `cargo test ... -- --include-ignored --nocapture`，记录卷 UUID、被运行/忽略用例数与结果。`tools/ci_release_binary_e2e.py` 明确只允许 GitHub 托管 macOS runner；本机不得伪造 runner 环境绕过保护，本机黑盒用上节隔离夹具或已验证的既有候选证据。

只有请求阶段放行核验时，再按[任务流程 §18.1、§18.4](../../../docs/development/process/任务流程.md)和[质量工具固定版本](../../../tools/quality-tools.toml)检查全 workspace 变异及 `tools/ci_release_fuzz.py` 的全部目标长预算，使用现有 `tools/ci_mutation.py --scope workspace` 入口或可验证的既有组合证据。不要每修一个点就重启全量变异；先核对候选、代码、测试/fixture、依赖、工具、平台和未解决变异的影响范围，仅补跑失效部分。长批次按上次耗时或首次一小时估算查看时间，不短间隔轮询。性能基线使用 `tools/p1_perf_baseline.py --binary <release-binary> --volume-root <经验证的APFS测试卷目录> --output <新证据文件>`；该工具内部自建测试实例，结果只证明基准夹具，不代替独立账户的完整黑盒验收，也不是性能承诺。

## 交付给主 Agent / 用户

给出一张证据表：`场景或门禁 | 精确命令/候选 | 通过/失败/未执行 | 判定事实 | 证据位置`。区分“本轮执行”与“复用且已核验”；单列被跳过的 ignored 测试、子挂载、在线审计、二进制黑盒、mutation、fuzz 和人为结构验收。任何错误码/文档与实现冲突，先报主 Agent：位置、证据、影响和建议；未经主 Agent 决定，不擅自改规则让测试变绿。Agent 只能提出“技术证据齐全/仍有缺口”，不能替代维护者人工签字放行。

# P0-01 Host/Path Probe 实施记录

## 职责与非职责

本文记录 P0-01 的准入、实验操作、实际证据、审核结论和剩余边界；随此任务更新。本文不定义生产 Port、产品支持矩阵或阶段放行规则，也不将实验实现直接升级为 P1 产品代码。

## 任务准入

| 项目 | 当前值 |
|---|---|
| 类型、阶段、小阶段、风险 | Spike；P0；P0.a；R4（包含 FFI） |
| 状态 | In Progress |
| 负责人 | 主 Agent |
| 编码模型 | GPT-5.6 Sol / `gpt-5.6-sol` / `xhigh` |
| 规定审核模型 | GPT-6 Astra / `gpt-6-astra` / `xhigh`；已执行文档、FFI 与增量审核，最终审核待完成 |
| 开始日期、基线 | 2026-09-12；`f7a400f81e08683b242aa4950e556bc9301f1fe5` |
| 工作分支 | `task/p0-01-host-path-probe` |
| 工作区 | `/Volumes/data/code/thinworkspace-p0-01`，标准 Git linked worktree；未声明 CoW |
| 前置依赖 | 无；本机 macOS/APFS 与 Rust 工具链可用 |
| 主要写入区域 | `experiments/p0/probe/` 和直接配套的实验构建、测试配置 |

实验需要回答：给出实际目录参数时，是否能够用 FD 对应的系统事实识别卷、最近存在父目录、权限和路径替换，并向后续物化实验提供可复查证据。

权威输入为[跨平台物化设计](../../project/design/ThinWorkspace_跨平台工作区物化设计_v1.0.md)的 Probe 与路径章节、[技术栈](../../project/reference/技术栈.md)的 macOS 与测试章节、[开发规范](../process/开发规范.md)和[任务流程](../process/任务流程.md)。任务顺序由[实施计划](../planning/ThinWorkspace_Phase1实施计划_v1.0.md)维护。

P0-01 不实施文件克隆、产品初始化、SQLite、Workspace 生命周期、Git 托管或 P1 CLI。实验命令不属于 `thinws` 公开命令契约。输入检查不创建被探测目录；测试只写本次创建的受控临时目录。失败保留结构化原因，不能以错误作为“已支持”证据。

## 预先定义的验收断言

1. 主机报告包含实测系统和架构；读取 API/文件系统能力不能产生 CoW confirmed。
2. 同一 APFS 卷上不同目录的卷 UUID 一致，且与独立系统查询相符；不同卷不得只因类型相同被认作同卷。
3. 未存在目标返回最近存在父目录的证据，并明确缺失部分；探测后目标仍不存在。
4. 拒绝路径逃逸、NUL、符号链接和不合适的文件类型；符号链接包含中间组件与 dangling link。
5. 路径重验能识别已存在目录、祖先目录或符号链接的替换；原先缺失的目标或中间组件被创建也必须使原证据失效。不能仅检查路径字符串或当前卷类型。
6. 可写性证据区分内核权限预检、只读挂载及无法确认；不将权限位或成功预检视为将来执行保证。
7. 真实平台测试、纯边界测试与 JSON 实验输出一致；所有 unsafe 收敛在小型 FFI 模块并注明安全前提。
8. 先取得预期断言失败的 RED，再实施 GREEN；执行通用门禁、适用变异与有界 fuzz，按规定模型完成审核。
9. source/target/staging/trash 组合报告分别提供路径间卷关系和各后端支持三态；跨卷 APFS clone 不支持，不代表 Full Copy 底层也不支持。产品同卷限制与底层能力必须分开。

## 基线环境与当前证据

- 操作系统：macOS 15.7.2（24G325），arm64；Apple Git 2.39.5。
- Rust/Cargo：已安装固定版本 1.97.1；SDK 为 MacOSX26.1.sdk。
- `/Volumes/data` 由 `diskutil info -plist` 报告为 APFS，可写；其设备与 `/private/tmp` 所在设备不同，可用于后续跨卷身份核对。
- 开始时已安装 `cargo-mutants 27.1.0`、`cargo-fuzz 0.12.0` 和 `nightly-2026-08-14`；deny/audit 当时缺失，现已按质量工具清单补齐并验证。
- 文档基线无 Cargo workspace，开始前 Rust 门禁未执行（缺少 manifest）。本次最小 Cargo workspace 仅承载 P0 实验，不能据此标记 P1-01 完成。
- SDK 的 `sys/clonefile.h` 与 Apple 官方手册证明原设计混淆 `clonefileat`/`fclonefileat` 参数。主 Agent 已修正对应权威设计；本实验仍只探测，不调用克隆。

实验直接依赖沿用《技术栈》：`libc` 封装必要系统调用，`serde`/`serde_json` 输出可复查的实验报告，`thiserror` 保留结构化错误，`tempfile` 仅供测试建立独立临时根。实际解析版本由 `Cargo.lock` 固定；未引入异步运行时或 P1 产品 crate。

## 执行、审核与收口

### RED

本节及下方早期增量命令保留实际执行时的包名；收口前已按 README 的 `thinws` 技术命名空间将两个私有实验包统一为 `thinws-p0-probe`、`thinws-p0-probe-fuzz`，Rust 库为 `thinws_p0_probe`，实验命令为 `thinws-p0-probe`，无旧名兼容。当前重跑优先使用下方全 workspace 命令。

编码 Agent 与主 Agent 均执行 `cargo test -p thinworkspace-p0-probe --test probe -- --nocapture`：编译成功，退出码 101，两项断言因返回 `ProbeError::NotImplemented` 而失败。

- `host_report_is_measured_and_never_confirms_cow`
- `missing_target_uses_nearest_existing_ancestor_without_creation`

这些失败来自预期行为尚未实现，非构建或环境错误。

### 待完成验证

当前冻结源码已通过本地通用、release、真实跨卷、全 crate 变异、fuzz 与供应链门禁；[PR #1](https://github.com/chinayangxiaowei/thinworkspace/pull/1) 已创建。首次远端工作流校验失败，任务返回 In Progress 修正 CI，正式 Approve 暂停。P0-01/P0.a 尚未收口，未作 P0 阶段放行声明。

本次临时 APFS 卷已按精确挂载路径卸载，`hdiutil detach` 退出 0；随后删除本任务的 128 MiB 合成镜像和空临时父目录。没有删除用户数据，镜像可按已验证命令重建；失败与最终变异日志均保留于当前工作区的忽略目录。工作区暂保留用于 PR/CI 复核，合并后再记录保留或清理决定。

### 远端 CI

首个提交 `a420176` 对应[运行 34684128712](https://github.com/chinayangxiaowei/thinworkspace/actions/runs/34684128712)：completed/failure，耗时 0 秒，jobs 为空，Rust 门禁未执行。普通 YAML 解析通过不能证明 GitHub 表达式合法；工作流在 job 级 `env` 使用了不允许的 `runner.temp` 上下文，依据 [GitHub 上下文可用性表](https://docs.github.com/en/actions/reference/workflows-and-actions/contexts#context-availability)，已改为在运行步骤中从 `RUNNER_TEMP` 写入 `GITHUB_ENV`，后续步骤继续使用同一变量名。此修正不改 Rust 源码、测试范围或质量门槛，仍须以新的真实 CI 结果验证。

CI 补丁已由 GPT-6 Astra / `xhigh` 专项复核通过。主 Agent 重跑两 workspace 的 fmt/Clippy 与通用 `cargo test --workspace --all-targets`：退出 0，普通测试 49 passed、1 ignored（专用镜像已清理）；这次增量复跑不替代下表同一 Rust 源码的 50 项完整跨卷验证。新工作流结构解析与路径初始化脚本 `bash -n` 通过，远端语义校验仍待新的 CI 执行。

### 独立回归与增量结果

主 Agent 补充独立验收测试，代码仍由规定的 Astra Reviewer 审核：

- `cargo test -p thinworkspace-p0-probe --test path_revalidation`：7/7 通过。包含缺失叶子/中间目录出现、叶子替换、保留叶子 inode 的祖先替换、leaf/intermediate/dangling symlink、祖先变为 symlink 和未变路径。
- `cargo test -p thinworkspace-p0-probe --test permission_error -- --nocapture`：取得回归 RED，真实稳定目录去掉搜索权限时，原实现误报 `PathChangedDuringInspection`，应保留 `SystemCall`/`EACCES`。修复后主 Agent 已独立取得 GREEN。测试先恢复权限再断言，失败时也可回收本次临时根。
- FFI 切片由 GPT-6 Astra / `xhigh` 核对 SDK 与官方手册，未发现已证实阻断；调用者组件校验、unknown UUID 和完整运行证据仍需整体验证。这不是最终批准。

文档预审已由 GPT-6 Astra / `gpt-6-astra` / `xhigh` 执行：要求补充路径组合能力与 missing 目标变更两项断言，并明确 P0.a 运行受影响 crate 全量变异；主 Agent 已采纳。该预审不代表代码审核或最终收口通过。

### 增量审核与环境修正

- 主 Agent 采纳 Astra 审核的四项局部修正：校验报告版本/实验标记、检查 source 读取和搜索权限、保留 clone 能力查询的结构化 errno、对相同 UUID 与矛盾 fsid/类型证据返回 unknown。最后一项使用合成证据验证，不宣称本机已复现重复 UUID 挂载。
- 独立 UUID 查询原来硬编码开发机卷，无法在 CI 或变异副本运行；改为从实际测试目录以系统 `stat` 取得设备，再由 `diskutil`/`plutil` 独立查询。普通子目录不能直接传给 `diskutil info`，主 Agent 已实测其失败。
- 权限验收前置条件：本次有效 UID 为 501，`/private/tmp` 所在卷由系统查询报告 ownership enabled。权限用例不能以开发数据卷的 noowners 结果代替，也不能在 root 下跳过后算通过。
- 主 Agent 已实测创建、挂载本次独立 128 MiB APFS 镜像，ownership on；用于同时验证 CI 的独立卷准备命令和跨卷测试，测试结束必须卸载并清理本次合成文件。空白镜像创建不能使用本机 `hdiutil` 拒绝的 `-format UDRW` 组合，CI 已据实际结果修正。
- CI 显式运行标记为需跨卷环境的测试；变异入口保留 Cargo 与 libtest 的两个参数分隔符。环境准备失败即失败，不以忽略该测试获得跨卷通过证据。

### 供应链检查

质量工具版本以 [`tools/quality-tools.toml`](../../../tools/quality-tools.toml) 为准。本轮已安装缺失的 deny/audit 工具；项目依赖与 fuzz 依赖分别检查，不能用根 workspace 的结果覆盖独立 fuzz workspace。

- `cargo audit --deny warnings --file Cargo.lock` 与 `cargo audit --deny warnings --file fuzz/Cargo.lock` 均退出 0，读取 1243 条 RustSec 公告；分别扫描 27 和 25 个包。
- `cargo deny --locked check` 和 `cargo deny --locked --manifest-path fuzz/Cargo.toml --config deny.toml check` 均退出 0，advisories、bans、licenses、sources 均通过。根工作区不包含 fuzz 包，因共用配置产生未用 fuzz 许可项的警告，非许可违规。
- 许可准入由 `deny.toml` 管理：第三方宽松许可证清单，项目 PolyForm 仅按自有包名/版本允许，不跳过所有 private 包。此检查不替代正式对外发布前的授权主体与贡献权属确认。

### 已修复的验证缺口

- 首次变异副本基线因 Unix socket fixture 路径超过 `SUN_LEN` 失败，已改为受控短路径且保留真实拒绝断言。随后旧版的全量结果为 242 项：127 caught、76 missed、39 unviable、0 timeout，退出 2。原始失败清单和日志保留在 `target/mutants.out/`；该结果不能作为通过证据。
- 补齐各目的路径去写权限的拒绝、inode 不变但权限改变的重验、FD 0 和 CLOEXEC、卷属性长度/returned bit、fsid 两个 ABI word、特殊文件类型、错误映射及候选后端优先级测试。
- `openat=ENOENT` 后的 `fstatat=EIO/EACCES/ESTALE` 原来被归为不带 errno 的 race，现保留 metadata 系统错误，并有表驱动回归。
- CLI 在 stdout/stderr 读端关闭时原来 panic。主 Agent 黑盒测试取得 RED（实际退出 101，期望 1），改为可失败的输出操作后取得 GREEN；正常 JSON、带空格路径、usage 和无写入行为保持不变。
- 加强后的首次冻结快照完整运行 243 个变异：190 caught、3 missed、50 unviable、0 timeout，耗时 7 分钟、退出 2，保留在 `target/p0-01-verification/mutants.out/`。剩余三项均非等价：目录打开失败后已存在条目的错误映射、两个候选后端的成功证据 guard。现将局部错误构造直接收拢到返回既有 `ProbeError` 的私有函数，并补终态与证据正反断言；编码 Agent 通过 guard 变异分别取得 RED，恢复正确行为后 GREEN，没有新增分类枚举或注入框架。

### 当前冻结快照运行证据

下列命令由主 Agent 在本次工作区独立执行。跨卷测试实际使用本次创建的 128 MiB APFS 镜像，UUID 为 `E8F17F41-0C3C-4001-AEC1-0E4E6335A85E`，挂载于 `/private/tmp/thinws-p0-ci.7Ehld0/mounted`，作为 `THINWS_P0_CROSS_VOLUME_ROOT`；未对用户卷做格式化或权限更改。

| 验证 | 命令 | 当前结果 |
|---|---|---|
| 根与 fuzz 格式 | `cargo fmt --all -- --check`；`cargo fmt --manifest-path fuzz/Cargo.toml -- --check` | 均退出 0 |
| 根与 fuzz 静态检查 | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`；`cargo clippy --locked --manifest-path fuzz/Cargo.toml --all-targets --all-features -- -D warnings` | 均退出 0 |
| 普通与真实平台测试 | `cargo test --locked --workspace --all-targets -- --include-ignored` | 50 passed，0 failed，0 ignored；跨卷用例实际执行 |
| release 验收测试 | `cargo test --locked --release --workspace --all-targets -- --include-ignored` | 50 passed，0 failed，0 ignored |
| 全 crate 变异 | `cargo mutants --workspace --timeout 60 --jobs 2 --output target/p0-01-final -- -- --include-ignored` | 253 个候选，精确排除下述 9 个严格等价项后运行 244 项；193 caught、0 missed、51 unviable、0 timeout，耗时 6 分钟，退出 0；未变异基线通过 |
| 路径 fuzz | `cargo +nightly-2026-08-14 fuzz run thinws_probe_path_validation target/p0-fuzz-corpus -- -max_total_time=60 -timeout=5 -max_len=4096 -print_final_stats=1` | 383,411 次输入，实际 61 秒，退出 0；无 crash/hang，peak RSS 614 MiB |

fuzz 使用原始字节建立独立的接受/拒绝、规范化、非 UTF-8 保真和幂等断言，默认启用 ASan；本次新增 73 个 corpus 单元留在忽略的 `target/`，没有待固化 crash。短预算不替代 P0 阶段长预算。当前库源码 SHA-256 为 `48cce2a47e4eb736493ef3bfb41bbb86dbc07a922322d307f6c2a6343bf6aa29`，FFI 为 `1472f3c661237b5fd617ee542fce53472b0cc05a781f878728a9f14213f7e934`。

### 精确等价变异处置

负责人为主 Agent；以下证明已由 GPT-6 Astra / `xhigh` 按冻结源码核验，精确位置和变异名只在 [`.cargo/mutants.toml`](../../../.cargo/mutants.toml) 维护，不排除整个函数或文件。`cargo mutants --no-config --list --workspace` 为 253 项，加载配置后为 244 项，差集恰为以下 9 个 `| → ^`。

| 等价类 | 数量 | 证明与失效条件 |
|---|---|---|
| `decode_hex` 合并两个 nibble | 1 | 解码结果各在 0..15，高 nibble 左移 4 位后与低 nibble 位域互斥；扩大输入值域、改变移位或解码规则需重审 |
| root/child directory 的 open flags | 4 | 当前 `O_SEARCH | (O_NOFOLLOW | O_CLOEXEC)` 中外层、内层各自位域互斥；修改分组、SDK/libc 常量或平台需重审 |
| UUID/capability 属性请求 | 2 | `ATTR_VOL_INFO` 与对应 UUID/CAPABILITIES 位互斥；属性位或请求组合改变需重审 |
| source/destination 权限模式 | 2 | `R_OK=4`、`W_OK=2` 分别与 `X_OK=1` 互斥；mode 或平台定义改变需重审 |

两个实现去重复也经同一 Reviewer 核验：Darwin `O_SEARCH` 已含 `O_DIRECTORY`；对 held directory FD 的固定 `.`，`AT_SYMLINK_NOFOLLOW` 不改变解析或 errno。真实用户组件的 `openat(O_NOFOLLOW)`、`fstatat(AT_SYMLINK_NOFOLLOW)` 仍保留，`faccessat` 仍使用 `AT_EACCESS`。固定路径的限定写入 FFI `SAFETY` 注释，参数化时必须重新审核。

旧 `AT_EACCESS | AT_SYMLINK_NOFOLLOW → 0` 不属于等价变异，未排除；不同 real/effective UID 的行为场景未执行，当前普通会话无授权凭据切换环境，未提权或安装 setuid helper。动态 LLDB 启动未成功，不能记为通过；Reviewer 的静态反汇编只能证明当时构建传入 `AT_EACCESS`，不替代不同 UID 的真实行为验证。旧候选后端循环已改为相同顺序的六对显式关系，原 loop 变异已不存在，也未保留排除项。

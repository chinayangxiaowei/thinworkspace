# ThinWorkspace Linux/Btrfs 适配实施记录

## 职责与非职责

本文保留 P1-L01–L05 的逐次实施、真实平台验证、失败与补测证据，供收口核对。当前任务状态与依赖只以[Phase 1 实施计划](../planning/ThinWorkspace_Phase1实施计划_v1.0.md)为准；本文不定义产品行为、接口、错误码、质量门槛，也不代替人工阶段放行。

## 实施与验证时序

P1-L02/L04 交界实施记录（2026-09-29）：Linux Adapter 已增量加入显式 Btrfs target 的创建时目录身份与版本 3 归属文档，归属仅存于 ext4 控制根；真实 Btrfs 测试覆盖普通空 target、预占 target/操作目录拒绝、ext4 target 拒绝及 target 被替换后的失效。路径 Probe 的可写目录检测从不可用的空路径 `accessat` 调整为持有目录 FD 下的 `.` 检查，真实 Btrfs 回归确认可写证据。此能力尚未接入完整 `BootstrapStore`、Application 或 CLI；Ready 查询、空间统计、安全删除与命令黑盒验收仍未完成，不调整任务 Done 状态。
本次本地证据：Debian 11.7、ext4 控制测试根 `/home/yxw`、真实 Btrfs 测试根 `/media/yxw/thinws` 上，`cargo test -p thinws-adapter-linux --all-targets` 共 44 项通过；该包 `cargo clippy --all-targets --all-features -- -D warnings`、全仓 fmt、macOS 全仓普通测试与依赖方向检查通过。Linux `cargo check --workspace --all-targets` 仍因既有 macOS-only Adapter/P0 实验未条件编译而失败，CLI 尚不能在 Debian 构建；Linux VM 尚未安装 `cargo-mutants`，定向变异及本次归属解析 fuzz 未执行，故本记录不作为任务或小阶段放行。
P1-L04 查询入口增量（2026-09-29）：已登记 Workspace 的创建收尾现在仅删除归属可证且为空的 staging/trash；Ready 路径查询只读比对 SQLite reservation 所持字段与版本 3 归属文档、Btrfs FSID/mount ID、父目录及 target 创建身份，并拒绝未清操作目录。真实 ext4 控制根＋Btrfs target 测试覆盖非空 staging/trash 拒绝、target 缺失/替换、父目录替换、登记卷不符及归属文档变动；另以真实 Btrfs `FICLONE` 完成“准备目录 → 物化 → 清理空操作目录 → 按登记重新查询”链路，副本改写不影响源。此处尚未接入 `BootstrapStore` trait 与 CLI；空间统计、删除、日志、完整门禁和黑盒命令仍待做，因此 L04 仅为 In Progress，不视为 L03 或 L04 收口。
本次验证：Debian 11.7 的 `thinws-adapter-linux` 全目标 50 项测试与严格 Clippy、macOS 全 workspace 格式/严格 Clippy、依赖方向检查通过。macOS 全 workspace 普通测试首跑在未改动的 `real_cli_git_incomplete_refusal_exposes_the_specific_issue` 二次人工输出断言偶发得到 `E_WORKSPACE_BUSY`（退出码 23，预期 Git 检查不完整的 25）；该用例单独复跑以及全 workspace 第二次运行均通过。尚未定位该占用波动根因，不将首跑记为通过；Linux 定向变异/fuzz、Linux 全仓编译和 CLI 黑盒仍未完成。
P1-L04 空间统计增量（2026-09-29）：在已验证 Ready target 后，Linux Adapter 以持有的根目录句柄做有界只读扫描，检查每项 mount ID 和目录项身份；特殊类型、越界或内部扫描不完整给 `unknown`，根归属失效仍为错误。真实 Btrfs 测试验证文件、硬链接、符号链接的逻辑/分配字节口径以及 socket 导致的 unknown。该方法仍待接入完整 `BootstrapStore` 与 CLI `status`，不构成 L04 收口。
本次普通门禁：Debian 11.7、ext4 控制根与专用 Btrfs 测试根上，`cargo test -p thinws-adapter-linux --all-targets` 53 项全绿，Linux Adapter 严格 Clippy 通过；macOS 的全 workspace 普通测试、严格 Clippy、全仓 fmt 与依赖方向检查亦通过。本次未运行 Linux 定向变异或归属/扫描 fuzz；Linux 全仓及 CLI 仍受 macOS 专用 crate 无条件编译限制，未将单包通过冒称产品可用。
P1-L04 清理前置增量（2026-09-29）：Linux Adapter 已能在持锁状态下从持久归属证明定位原 target 或已登记的隔离目录；目标缺失、替换、双位置冲突及未登记隔离项均不授权删除。清理 JSONL 开始/结果事件现同步写入 ext4 控制目录，截断尾部先换行，日志链接拒绝；真实 Btrfs 集成测试与严格 Clippy 已覆盖这些入口。物理删除、完整 `BootstrapStore` trait 与 CLI 黑盒仍未完成，不将 P1-L04 标为 Done。此次定向变异与 fuzz 未执行，原因及收口要求仍按《任务流程》处理。
P1-L04 物理清理增量（2026-09-29）：Linux Adapter 已接入既有 `BootstrapStore`；删除先清理归属可证的操作目录，再持久登记隔离路径、无覆盖重命名目标，最后从已验证目录 FD 做同挂载/no-follow 删除。真实 Btrfs 用例验证整目录删除不跟随外部符号链接、替换或缺失目标拒绝、隔离冲突拒绝，以及特殊文件导致的部分清理保留归属并允许新的显式调用继续。此为 Adapter 层能力，Application/CLI 黑盒及专项变异、fuzz 门禁尚待完成，不改变 P1-L04 的 In Progress 状态。

P1-L05 CLI 增量（2026-09-29）：CLI 以目标平台依赖装配本机 Adapter；Linux 调用现有 Application 创建编排的 CoW-only 入口，不提供 Full Copy 执行器，Probe 对 Linux Full Copy 明确给出 unsupported。Debian 11.7 的 ext4 隔离控制根、真实 Btrfs 工作区上，包级 JSON/人类接口与实际二进制覆盖 init、doctor、dry-run、create、list、path、status、remove；`prl_fs` 来源即使带 `--allow-copy` 也拒绝。Git 已跟踪修改普通清理拒绝、显式强制清理留日志；登记 target 移走后强制清理仍返回 E_TARGET_MISSING 且保留登记；实际外部进程占用时强制清理返回 E_WORKSPACE_BUSY。Linux 产品包范围普通测试和严格 Clippy、macOS 全仓普通测试/严格 Clippy 均通过；Linux Release CLI 构建和 6 项真实 Btrfs Release E2E 通过。初次 `cargo deny check` 发现 Linux 两个仓库自有 crate 缺少本项目许可证例外，补精确例外后通过；离线 `cargo audit --no-fetch` 扫描本地 1261 条 advisory 通过。Debian 原样 `cargo check --workspace --all-targets` 仍被历史 macOS-only Adapter/P0 实验阻断，不作为 Linux 产品失败或全仓通过结论。受影响范围变异、fuzz 和阶段人工验收尚未完成，L01–L05 不标 Done。

P1-L05 Linux 归属解析 fuzz 增量（2026-09-29）：为现有 `fuzz/` 增加按目标系统选择的纯内存归属文档 harness，Linux 使用版本 3 有效语料，macOS 仍使用既有版本 2 解析器；Linux 有效 seed 的普通解析回归通过。Debian 11.7、固定 `nightly-2026-08-14`、`cargo-fuzz 0.12.0`、AddressSanitizer、60 秒预算执行 Linux harness，完成 1,143,061 次、退出码 0、无 crash/hang。生成语料仅位于 Btrfs 临时目录，核对无 crash/timeout artifact 后删除；两个固定 seed 留在仓库。macOS 的全仓普通测试/严格 Clippy、两平台 fuzz harness 编译、Debian 产品包范围普通测试/严格 Clippy 均通过。本条仅为 Linux 归属解析的任务级 smoke 证据，不替代其他受影响解析器、定向变异、小阶段或阶段门禁。

P1-L05 定向变异计划（2026-09-29）：候选冻结为本地 `main` 的 `eaf1063`，主 Agent 负责执行；Debian 11.7、`cargo-mutants 27.1.0`、真实 ext4/Btrfs 测试环境。先对本轮变更的 Linux `inspect_materialization_paths` 与 `combined_support` 精确枚举 17 个变异，验证包为 Linux Adapter 和 CLI；仅 1 job，输出 `/media/yxw/thinws/mutants-p1-l05-probe/`。Btrfs 测试盘原剩余约 1.6 GiB，已用 `cargo clean --release` 清理可重建的 685.4 MiB Release 构建缓存，为变异副本留空间；已通过的 Release 证据保留，但 VM 的 Release 二进制需重新构建才能再次使用。此为受影响函数的任务级补测，不是 Linux Adapter 或阶段全量变异；首次没有可比 Debian 同类耗时，按《任务流程》§18.4 启动后约 1 小时首次主动查看，不短轮询。

P1-L05 定向变异结果（2026-09-29）：上述首次批次完整执行 84 秒，17 个变异中 7 个 caught、4 个 unviable、6 个 missed；未捕获项集中在 source 不存在、mount ID 缺失/不一致、Btrfs 卷身份缺失/不一致及 unknown 判定。随后在 Linux Probe 增加独立的路径证据组合单元测试；Debian 复测针对 `combined_support` 的 13 个有效变异用时 50 秒，13 个全部 caught（含首次 6 个 missed），结果在 `/media/yxw/thinws/mutants-p1-l05-probe-2/`。首次失败证据保留，不把该局部复测称为 Adapter 或阶段全量变异通过；下一次同类定向批次按约 1–2 分钟实际耗时加余量估计，不再机械等待 1 小时。

P1-L03 Btrfs 文件属性增量（2026-09-29）：在真实 Btrfs 测试根通过 `chattr +C` 创建 NOCOW 来源文件，证实路径级 Probe 可预检合格但该普通文件的 `FICLONE` 在 Debian 5.10 上实际失败；物化返回 `CowUnavailable`、确认回滚至空 target，来源内容保持不变。此为实际文件属性失败边界，不扩展为全部内核或所有 NOCOW 组合的保证；对应集成测试依赖测试主机提供 `chattr` 且允许在专用 Btrfs 根设置 NOCOW。

P1-L05 创建编排定向变异计划（2026-09-29 20:56 UTC）：候选为本地 `main` 的 `73f727d`，主 Agent 在 Debian 11.7/ext4 控制根＋Btrfs 工作区执行；工具 `cargo-mutants 27.1.0`，仅 1 job，筛选 `thinws-application/src/create.rs` 中 `create_cow_only`、`create_impl`、`create_reserved` 共 23 个变异，以 Application＋CLI 测试包验证，结果写入 `/media/yxw/thinws/mutants-p1-l05-create/`。参考前一同类定向批次 50–84 秒，首次查看估计 2–3 分钟；此范围不代替小阶段或阶段全量变异。

P1-L05 创建编排首批结果及跨平台核验计划（2026-09-29）：上述 Debian 批次完整执行 67 秒，23 个变异中 6 caught、4 unviable、13 missed；主要未捕获项为创建入口后端校验、同名 Ready 复用身份以及预留后的路径/物化错误分支。现有 Application 集成测试部分只在 macOS 编译，因此先以同一候选 `73f727d` 在 macOS 15.7.2/APFS、`cargo-mutants 27.1.0`、1 job，对同一 23 个变异运行 Application 测试包，结果写入 `/Volumes/data/thinws-mutants-p1-l05-create-mac/`；参考 Debian 67 秒并考虑真实 APFS 测试，首次查看估计 3–5 分钟。两平台结果逐项比较后再决定 Linux 需补的测试，不能用 macOS 的捕获结果掩盖 Linux 特有失败边界。

P1-L05 创建编排 macOS 对照结果（2026-09-29）：完整执行 71 秒，同一 23 个变异为 17 caught、4 unviable、2 missed，输出位于上述 macOS 结果目录。共享的两个关键存活项是同名请求的 `allow_full_copy` 差异条件，以及已选择 CoW Adapter 与注入执行器种类不一致的保护。Linux 首批的另外 11 个 missed 由 macOS 专属 Application 测试捕获，但不据此宣称 Linux 各失败边界已获真实平台验证；针对共享两项先补 Linux Application 集成测试，再按影响范围定向复测。

P1-L05 创建编排补测计划（2026-09-29 21:07 UTC）：候选为本地 `main` 的 `9430fba`，已在 Debian 实际 Btrfs 工作区新增“同名但复制政策不同不复用”和“Btrfs Plan 不接受 APFS 执行器”两项 Application 集成测试；主 Agent 使用相同 Debian/ext4＋Btrfs、`cargo-mutants 27.1.0`、1 job，仅重跑首次及 macOS 对照共同存活的 `create.rs:390:17`、`create.rs:583:36` 两个变异，Application＋CLI 测试包，输出 `/media/yxw/thinws/mutants-p1-l05-create-2/`。依前一完整批次 67 秒和本次目标数，首次查看约 1 分钟；其余首次 Linux missed 保留为平台覆盖缺口，不用此局部复测冒充全量完成。

P1-L05 创建编排补测结果（2026-09-29）：上述两个共享关键存活变异在 Debian 14 秒定向复测中均 caught，未重跑第一次已 caught 的 6 项。macOS 对照已覆盖共享实现中其余 11 项，但 Linux 专项的失败路径仍按实际测试证据单独判断。新测试与既有 Linux 产品包全部目标测试、严格 Clippy，以及 macOS 全仓普通测试/严格 Clippy 均通过；新增的 Linux 内部测试依赖未引入外部包，`cargo deny check` 和离线 `cargo audit --no-fetch` 通过。

P1-L02/L03 真实跨挂载与 CLI 失败路径补测（2026-09-29）：在 Debian VM 的同一 Btrfs 文件系统内临时 bind-mount 第二目录，分别实测相同 FSID、不同 mount ID，手动运行需 `THINWS_LINUX_SECOND_BTRFS_MOUNT_ROOT` 的专用 ignored 集成测试；Linux Probe 明确给 `different_mount` 与 unsupported。测试后卸载第二挂载并移除空临时目录。另一真实 NOCOW CLI 场景证明：带 `--allow-copy` 的 `FICLONE` 失败时没有 Full Copy 后端，应返回 `E_CAPABILITY_UNAVAILABLE`，原实现误映射为 `E_FILESYSTEM`；不带参数时仍返回 `E_COW_UNAVAILABLE`。已按 RED→GREEN 修正共享 Application 错误映射、更新用户手册，并验证两种失败后均保留非 Ready 登记、空目标和来源不变，显式 `remove --force` 可安全清理。此处仍需适用的变异复测和完整回归门禁，不能单凭专项测试宣布 L05 Done。

P1-L05 Full Copy 错误映射定向变异计划（2026-09-29 21:14 UTC）：冻结候选为 `9430fba` 加本轮未提交的错误映射/测试/文档差异；主 Agent 在 Debian 11.7、真实 ext4/Btrfs 环境使用 `cargo-mutants 27.1.0`、1 job，只检验新加的 `create.rs:968:9` FullCopyUnavailable 匹配分支，运行 Application＋CLI 测试包，输出 `/media/yxw/thinws/mutants-p1-l05-fullcopy-error/`。前一两个变异批次 14 秒，本次首次查看估计约 1 分钟；不将这个单变异结果推广到其他错误映射或阶段全量。

P1-L05 Full Copy 错误映射结果与构建缓存复核（2026-09-29）：上述 1 个新分支变异用时 12 秒，结果 caught。macOS 全仓普通测试、fmt、严格 Clippy 通过。Debian 首次产品包全目标回归中，新 NOCOW CLI 测试仍拿到旧的 `E_FILESYSTEM`；单测曾拿到新结果。初步证据指向共享 `prl_fs` 源码路径下旧 Application/CLI 构建产物复用：清除 VM Btrfs 上这两个包的可重建 Cargo 缓存（约 795.8 MiB）并重新编译后，7 项 Linux CLI 黑盒用例串行全通过，随后产品包全目标普通测试与严格 Clippy 均通过；临时诊断输出已移除。这个缓存判断尚未独立复现并证明，不能把首次红灯写成通过；后续从 macOS 修改共享源码后，在 Debian 收口验证应明确观察受影响包实际重编译，必要时清理其可重建产物。此次 Linux 原样全 workspace 门禁仍受历史 macOS-only Adapter/P0 实验阻断，不以产品包结果冒称全仓通过。

P1-L05 Release 回归及仓库级门禁边界（2026-09-29）：在最新本地 `98971f1` 上，于 Debian 11.7 从清除后的 Btrfs 构建缓存重新编译 `thinws-cli` Release（50.88 秒），`cargo test -p thinws-cli --release --test e2e_linux` 的 7 项真实 ext4 控制根＋Btrfs 生命周期黑盒用例全通过，包含 NOCOW 失败分类与两次强制清理。再次执行 Debian `cargo check --workspace --all-targets` 仍失败于历史 macOS-only Adapter/P0 实验（例如 Linux `libc` 无 macOS `attribute_set_t`、`O_SEARCH`、birthtime 字段，P0 共享测试支持文件类型不匹配），不是本轮产品包编译失败。要让该仓库级命令跨平台可用需对 Mac 专用实验/Adapter 包实施系统性条件编译与测试门禁修订；这超出 Linux CLI 装配的局部修复，当前不偷改全仓结构，也不把该门禁宣称通过。

Linux 输入边界 fuzz 补充（2026-09-29）：Debian 11.7/aarch64，固定 nightly-2026-08-14、cargo-fuzz 0.12.0、AddressSanitizer，在 Btrfs 临时 corpus/artifact 根上分别运行 `thinws_materialization_path`、`thinws_create_request`、`thinws_remove_request` 各 60 秒，执行约 12,692,178 / 8,111,079 / 6,744,965 次，三者退出码均为 0，未见 crash/hang。确认临时 artifact 目录为空后删除约 1.7 MiB 生成语料；仓库固定 seed 保留。首次未切换到仓库工作目录的命令未启动 fuzz，调整工作目录后运行成功。本证据仅为任务级 smoke，不代替所有相关目标的阶段长预算。

P1-L04 删除边界定向变异计划（2026-09-29 21:35 UTC）：候选为本地 `main` 的 `388eecd`，主 Agent 在 Debian 11.7 的 ext4 控制根＋真实 Btrfs 工作区执行；`cargo-mutants 27.1.0`、1 job，限于 Linux Adapter 的 `destroy.rs` 30 个变异，以 Linux Adapter＋CLI 测试包验证，输出 `/media/yxw/thinws/mutants-p1-l04-destroy/`。参考前批 17/23 个变异的 67–84 秒，首次查看约 3 分钟；此为受影响删除模块的任务级测试，不是 1103 个 Adapter 变异的全量收口。

P1-L04 删除边界首批结果（2026-09-29）：上述 30 个变异耗时约 3 分钟，19 caught、2 unviable、9 missed；存活项涉及深度阈值、递归深度推进、目录/目录项身份重验以及同设备不同挂载识别。此批次未达标，保留结果，不将其称为删除安全门禁通过；后续以这些具体存活项为 RED 证据补真实文件系统测试并定向复测。

P1-L04 删除边界补测计划（2026-09-29）：候选为 `388eecd` 加本轮 5 个 `destroy.rs` 单元测试及本实施记录，仍在 Debian 11.7 真实 Btrfs 上以 `cargo-mutants 27.1.0`、1 job，仅重跑上述 9 个 missed，Linux Adapter＋CLI 为验证包，输出 `/media/yxw/thinws/mutants-p1-l04-destroy-2/`。其中同设备不同挂载测试由临时 Btrfs bind mount 提供，测试环境显式传入 `THINWS_LINUX_BIND_MOUNT_CHILD`；真实运行前检查设备相同、mount ID 不同。参考首批 30 个用时约 3 分钟，此批首次主动查看约 2 分钟；结束后卸载并删除临时空目录，不动持久测试根。

P1-L04 删除边界补测结果（2026-09-29）：9 个存活变异 70 秒全部 caught，含目录替换后不误删、512 层阈值及同设备不同 mount ID 拒绝。先以环境变量执行真实 bind mount 用例 1 项通过，再在同一挂载上运行 mutation baseline 与 9 项变异；`findmnt` 核对挂载源后卸载，临时目录 `rmdir` 完成。该测试未设置 `THINWS_LINUX_BIND_MOUNT_CHILD` 时不执行其特殊断言；常规测试通过不能代替 bind mount 专项证据。此补测只修复首批 9 个 missed，首批 2 个 unviable 保留原始分类，不冒充 crate 或阶段全量变异。

本轮普通回归（2026-09-29）：Debian 11.7/ext4＋Btrfs 上七个产品包（Core、Ports、SQLite、Git CLI、Linux Adapter、Application、CLI）的 `cargo test --all-targets` 与严格 Clippy 均退出 0；macOS 全仓 fmt、严格 Clippy 退出 0。macOS 首次 `cargo test --workspace --all-targets` 在未改动的 P0 Probe 实验 `closed_output_and_diagnostic_channels_return_failure_without_panic` 出现一次返回码 0/预期 1 的失败；同项独立复跑通过，完整全仓复跑（仅加 `--quiet`）退出 0。该不稳定现象仍作为质量风险保留，未修改实验生产代码或把首次失败删除。Linux 全仓门禁仍受历史 macOS-only 包阻断；本轮未运行受影响 crate 的全量变异或阶段长预算 fuzz，不宣布 L04/L05 Done。

P1-L05 CLI 查询/清理黑盒补测（2026-09-29）：在已有真实 Git 仓库的 Debian/ext4＋Btrfs 生命周期用例中，增加副本 `.git` 保留、未跟踪文件单独存在时 `status` 的完整 clean/空间结果，以及已跟踪文件修改后 dirty 与准确变更计数断言；同源创建第二个副本，证明仅有未跟踪文件时普通 `remove` 不需要 force 且删除成功。该补测验证当前公开契约，不引入 Git 托管或新命令。Debian 七个产品包的全目标普通测试与严格 Clippy、macOS 全 workspace 普通测试与严格 Clippy、全仓 fmt 均退出 0；Linux 全仓门禁的历史 macOS-only 包阻断和阶段级长预算门禁仍未解决，不宣布 L05 Done。

P1-L05 Linux 全仓构建门禁修复（2026-09-29）：在本地 `main` 的 `cb2d4de` 基线上重现 Debian 原样 `cargo check --workspace --all-targets` 失败：macOS Adapter 与 P0 APFS Probe/Materialize 实验调用 Linux `libc` 不存在的 macOS FFI，P0 清理集成测试还引用 APFS 专用共享 fixture。现将这些明确 Mac 专用的库、集成测试和锁实验条件编译为 macOS；P0 Probe 实验二进制在 Linux 直接执行时明确退出 1 并提示仅适用于 macOS，不伪报探测成功。P0 清理的纯策略测试保留在 Linux，持久日志权限测试将保留式临时目录建在 `THINWS_LINUX_BTRFS_TEST_ROOT`，避免 `prl_fs` 映射盘将 `0700` 报为 `0755`。首轮 Linux 全仓普通测试因此暴露 3 个权限断言失败；改为本机 Btrfs 测试根后，P0 清理 9 项单元测试及原样 Debian 全仓 `check/test/Clippy` 均退出 0。macOS 全仓普通测试、严格 Clippy、全仓 fmt 仍通过；Debian Release CLI 的 7 项 ext4＋真实 Btrfs 黑盒测试通过。依赖方向检查、`cargo deny check`、离线 `cargo audit --no-fetch` 通过；`cargo deny` 仍有既有未命中 allowance/exception 警告。此变更没有修改 Linux 产品行为或公开命令，解决的是跨平台全仓门禁；适用 crate 的收口级全量变异、阶段长预算 fuzz 和人工验收尚未完成，L01–L05 状态暂不改动。

P1-L05 精确变异配置审计（2026-09-29）：上述平台条件编译使三个既有例外的行号移动；在准备收口变异时，用当前源码无配置列表核对全部 39 条精确规则，还发现既有 Git Adapter 与 macOS Adapter 规则的旧行号，以及 P0 Materialize 的旧 `1000:9` 规则已误命中相邻、未经批准的 `||→&&` 变异。逐项对照函数、表达式、列号和原审核失效条件后，只迁移 18 条规则的精确行号，不新增排除类型或放宽范围。当前全 workspace 无配置列举 4385 项，配置后 4328 项；39 条规则恰好匹配并排除 57 个现存候选，未命中规则为零，集合差与规则匹配集完全一致。原先误排的相邻变异已重新进入待测集合；旧审核证明的作用域没有被扩展。此为候选枚举与配置正确性证据，不是 4328 项变异已执行或通过。

P1-L05 Linux 阶段 fuzz 入口修正（2026-09-29）：原长预算脚本按 manifest 无差别运行全部目标，其中 3 个仅依赖 macOS Adapter/P0 实验，无法在 Debian 构建。先为平台选择增加测试并确认因缺少筛选函数按预期 RED，随后在同一 fuzz manifest 明示这 3 个 macOS 专属目标；长预算脚本在 Darwin 运行 11 个、在 Linux 仅运行其余 8 个，缺失/重复/未知分类或未知平台均拒绝。工具全集测试首次暴露新 Linux 归属解析目标未接入日常 CI 短预算；补上该 smoke 后 54 项 Python 工具测试、两 workspace 的格式检查、macOS 全仓普通测试/严格 Clippy、fuzz workspace 严格 Clippy 及 `git diff --check` 通过。本项只修测试入口与平台适用性，不表示 8 个 Linux 长预算目标已执行或阶段门禁通过。

P1-L05 Linux 长预算 fuzz 启动计划（2026-09-29 22:17 UTC）：冻结产品/测试候选 `4a26362`，执行负责人为主 Agent；Debian 11.7/aarch64、固定 `nightly-2026-08-14`＋`cargo-fuzz 0.12.0`，Btrfs 上独立 `CARGO_TARGET_DIR`/`TMPDIR`，1 个顺序执行作业。由 `tools/ci_release_fuzz.py` 按 manifest 选择 Linux 适用的 8 个目标，每项 300 秒，未通过或超时即停止；结果位置为本次 Debian 命令会话输出及本记录的终态摘要，不把 macOS 专属 3 项算作 Linux 通过。已知 8×300 秒为 40 分钟，另需编译，首次主动查看预定约 23:07 UTC；若提前失败或资源不足则立即按实际结果处理，不频繁轮询。启动前 Btrfs 可用空间约 1.4 GiB；不清理 crash artifact 来取得通过结果。

首轮 Linux 长预算 fuzz 在首个 `thinws_git_status` 编译时即失败，退出码 1；原因是该 harness 使用的 P0 cleanup 纯策略/日志 crate 被 fuzz manifest 错误地限定为 macOS 依赖。没有任何目标完成 300 秒，不记录为阶段 fuzz 通过。已将此共享依赖移至普通依赖，并以工具单测固定 Linux 目标与依赖关系；Debian 使用同一固定 nightly、Btrfs 构建目录逐项预构建 8 个 Linux 适用目标，全部退出 0。随后本机 55 项工具测试、fuzz workspace fmt/严格 Clippy、`cargo deny` 通过（deny 仍有既有未命中例外警告）。这只是构建前置与修复验证，仍需重新完整运行长预算。

第二轮 Linux 长预算 fuzz 启动计划（2026-09-29 22:25 UTC）：冻结候选 `cecea25`，主 Agent 在同一 Debian/ext4＋Btrfs 环境执行，工具、8 目标×300 秒、单作业、Btrfs Cargo target/temp 与首轮一致；逐目标构建预检已全部通过。结果保留在本次命令会话输出，完成后将退出码、预算与是否有 crash/hang 写回本记录。预计已免去首次编译，首次主动查看约 23:10 UTC；首轮编译失败不作为完整运行的耗时基准。

Linux Adapter 变异收口范围预核（2026-09-29）：在 Debian 对当前源码以 `cargo mutants --list --package thinws-adapter-linux --test-package thinws-adapter-linux --test-package thinws-application --test-package thinws-cli` 列举，恰为 1103 个候选；这只是完整 Adapter 包的待执行集合，不是通过结果。后续应使用真实 Btrfs 环境和这三个验证包执行包级收口，并与 Core/Application 等共享代码的变化范围分别核对；不把 macOS 专属 P0/Adapter 候选或全 workspace 的 4328 个静态列举混算为 Linux Adapter 的执行量。长预算 fuzz 尚在运行时不启动消耗同一 VM/Btrfs 构建资源的变异批次。

Linux CLI 安全覆盖复核（2026-09-29）：现有 7 项 Debian Release CLI 用例已覆盖真实二进制隔离 HOME、普通 target 创建/查询/删除、Git 已跟踪修改、force 日志、缺失登记 target、NOCOW 失败及外部进程占用；但“新 target 嵌入活跃 Workspace target/其受控目录”的拒绝目前主要由 macOS 专属测试验证，尚无 Debian CLI 黑盒断言。当前 Linux 长预算 fuzz 不覆盖该路径状态机；待冻结批次终态后，须在真实 ext4 控制根＋Btrfs target 上补正式创建与 dry-run 的冲突回归，再对受影响判断补定向变异，不能以共享 macOS 测试冒充 Linux 平台验证。

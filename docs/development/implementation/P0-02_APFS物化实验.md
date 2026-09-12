# P0-02 APFS clone 与跨卷实施记录

## 职责与非职责

本文保存 P0-02 的任务准入、实验覆盖、验收断言、执行证据和 Go/No-Go 结论。本文不定义产品 Port、完整 Plan/Receipt schema、Git 拓扑或阶段放行规则，也不把实验代码直接认定为 Phase 1 实现。

## 任务准入

| 项目 | 当前值 |
|---|---|
| 类型、阶段、小阶段、风险 | Spike；P0；P0.b；R4（FFI、创建与回滚删除） |
| 状态 | In Review；最终候选本地门禁通过，等待正式审核与远端 CI，任务尚未完成 |
| 负责人 | 主 Agent |
| 编码模型 | GPT-5.6 Sol / `gpt-5.6-sol` / `xhigh` |
| 审核模型 | GPT-6 Astra / `gpt-6-astra` / `xhigh`；已完成准入与限定实现复核，无全任务 Approve |
| 准备日期、基线 | 2026-09-12；`7d350a91df5c1b3612cf9fc3bc3d545e8a53f690` |
| 领取日期、集成基线 | 2026-09-12；已合入 `main` 的 `2e29f88a1aae2ee53ce52dbc38a7c0765b58cb15`；本分支集成提交 `7ee03f7ae051954abfefed9ac355ba4725efae4a` |
| 工作区、分支 | `/Volumes/data/code/thinworkspace-p0-02`；`task/p0-02-apfs-materialization`；标准 Git linked worktree，不声明 CoW |
| 前置依赖 | P0-01 已完成完整 CI、正式 Approve 与合并；证据见其实施记录“最终收口” |
| 主要写入区域 | `experiments/p0/materialize/` 及直接配套的实验测试、构建和质量配置 |
| 共享文件协调 | Cargo/lockfile、CI、质量工具配置与文档索引由主 Agent 统一协调；其他任务不得同时写入 |

权威输入为[跨平台物化设计](../../project/design/ThinWorkspace_跨平台工作区物化设计_v1.0.md)、[技术栈](../../project/reference/技术栈.md)的 macOS 与测试章节、[开发规范](../process/开发规范.md)以及[任务流程](../process/任务流程.md)。依赖和小阶段由[实施计划](../planning/ThinWorkspace_Phase1实施计划_v1.0.md)管理；路径证据使用 [P0-01](P0-01_HostPathProbe实验.md) 的实验接口，不另造一套卷判断。

目标：用可重复的真实文件系统实验，证明同卷 CoW、跨卷 clone 拒绝、独立跨卷 Full Copy，以及显式降级和部分失败补偿的安全顺序。

不实施 P1 CLI、产品 Port、SQLite、Git administrative state、完整生命周期或持久化恢复框架。没有产品公开命令、错误码或支持矩阵变更。强杀后的跨进程协调归 P0-04；本任务中断只保留实验现场，不在重启时凭目录外观自动接管。

## 实验覆盖与安全前置

- 固定小树覆盖嵌套目录、非空/空普通文件、普通与可执行文件模式，以及指向实验树外受控 sentinel 的 symlink。最终 manifest 核对相对名称原始字节、类型、普通文件内容/长度/权限、目录权限和 link text，同时核对源树未变。时间戳、ACL、xattr、file flags、稀疏布局和 hardlink 拓扑不属于本轮已验证覆盖，不得因此声明完整元数据保真。
- 普通文件 clone 使用既有设计选定的源 FD 与目标 dirfd API；Full Copy 使用明确的 `read`/`write` 循环，不使用可能透明 clone 的高层复制方法。两者均排他创建，不覆盖目标已有条目。
- 父根只用于创建本次唯一私有实验子树。初始化后记录根与父目录身份，后续操作均使用已验证 dirfd、单一正常名称组件和 no-follow；实验永不删除传入父根或挂载点，不改变用户卷配置。
- 双卷用例必须使用两个真实、可写且 UUID 不同的 APFS 卷；准备失败就是环境失败，不 silent skip。镜像创建/卸载由测试环境所有者负责，实验本身不管理磁盘镜像。
- 目标允许一个预先声明的 `.git` 控制 sentinel；该对象不进入物化或回滚对象清单。未知条目、冲突名称、根/对象身份变化均停止，不扩大删除范围。
- 创建成功即进入本 attempt 的局部登记表。随后打开或读取身份失败时仍保留“已创建、身份未确认”及原始错误；不能遗漏该对象、报 clean 或仅凭名称删除。
- 回滚只逆序处理本 attempt 已登记且当前身份一致的对象。停止结论覆盖失败返回、最终 teardown 和 Drop；禁止用 `TempDir` 默认析构或 `remove_dir_all` 绕过保留现场。测试注入对象的最终回收也必须有独立 fixture 归属/身份依据，并报告精确残留。
- 私有实验树不是恶意同 UID 并发写入的安全边界；`fstatat` 后 `unlinkat` 不宣称为原子条件删除。逐项重验、源 FD 调用前后检查和源/目标 manifest 仍必须执行。

## 预先定义的验收断言

| 场景 | 必须观察到的结果 |
|---|---|
| 真实同卷 clone | 新鲜四路径 Probe、写入前重验、每个普通文件真实 clone 成功、源/目标 inode 不同、完整限定 manifest 一致；仅此时可记录 CoW confirmed。克隆后单侧写入不影响另一侧 |
| 真实跨卷 clone | 独立卷证据明确；真实调用返回 `EXDEV` 并立即保存 errno；失败文件没有新目标，attempt 不报告 confirmed，也不自动开始 Copy |
| 显式独立跨卷 Full Copy | 新 attempt、新鲜 Probe/重验、实际字节复制及 manifest 一致；记录 Full Copy/CoW not-used，失败尝试为空。仅证明底层能力，不改变 P1 跨卷布局拒绝 |
| 预检同卷 unsupported | 仅显式允许复制时选择 Copy；clone 从未调用，failed attempts 为空，保留预检原因；拒绝政策下无物化写入 |
| 运行时同卷 unsupported | 在已有成功 clone 项后注入 `ENOTSUP`，保留 errno、partial 和旧 attempt；确认回滚后才重新 Probe/Plan 并显式启动新 Copy attempt，最终树不得混用两种后端 |
| 不得降级的失败 | deny policy、`EXDEV`、`ENOSPC`、权限/一般 I/O 错误及路径/卷变化均不启动第二后端；错误保持结构化，不把未知错误改写为 unsupported |
| 集合中途失败与回滚成功 | 第 N 项失败后精确列出成功创建对象；逆序、身份核验清理后回到 baseline，受保护 sentinel 和树外链接目标未变 |
| Copy 写到一半失败 | 与 clone 单次原子失败分开验证；目标部分写入进入 partial，不能丢失创建事实，补偿按相同身份规则执行 |
| 创建成功但身份登记失败 | partial 明确保留未确认对象，不宣称清理完成；失败退出及 Drop 也不删除该未知对象 |
| 回滚失败/身份替换/未知条目 | 保留未清理对象和失败证据，停止后无第二后端、无隐式递归清理；不触碰替换对象及无关数据 |
| 源同 inode 内容变化、目标冲突及路径重验失败 | 拒绝确认成功；源/目标 manifest 或重验揭示变化，既有目标与控制 sentinel 不被覆盖 |

预检 unsupported 和运行时 `ENOTSUP` 可使用明确标记的 fault injection，但不得冒充真实 APFS 不支持。真实同卷 clone、真实跨卷 `EXDEV` 和真实跨卷 Full Copy 不能由注入或 mock 替代。显式降级只由实验/测试层做最小编排，不创建 P1 fallback 引擎。

局部 attempt 证据只服务上述断言：区分真实与注入事实、所选/实际后端、失败与已创建对象、身份是否确认、回滚结果和后续尝试顺序；不在此冻结完整产品 schema。失败证据与测试日志留在本记录关联的工作区、PR 或 CI，不创建平行审核报告。

## 验证计划

以下是计划，不是执行结果：

1. 编码前在本工作区检查基线，先取得预期行为缺失的 RED；编译或环境故障不算 RED。
2. 执行《任务流程》通用门禁；在专用双卷环境执行 debug/release 的全部适用测试，跨卷用例不得留为 ignored。
3. 对本实验 crate 运行全量 `cargo-mutants`，处理所有关键存活项；精确等价排除须由规定 Reviewer 接受，不排除整个安全函数。
4. 使用固定 nightly 和质量工具运行受影响路径 fuzz，以及新增纯内存计划/证据边界的有界 fuzz；fuzz 输入不能决定真实写入或删除路径。记录实际命令、预算、输入数与 crash/hang 结果，短预算不替代 P0 阶段长预算。
5. 由 GPT-6 Astra / `xhigh` 审查实现、FFI、失败退出和所有清理路径，再依任务流程进入 Verification/合并。未完成真实验证、失败现场仍无清理/保留说明，或显式降级顺序未验证时，不作完整 Go。

## 当前证据与剩余事项

- 2026-09-12，GPT-5.6 Sol / `xhigh` 完成只读实验准备；主 Agent 核对相关权威章节与 Apple API。
- 同日，GPT-6 Astra / `xhigh` 完成只读准入审查；主 Agent 采纳两条显式降级编排、Drop 停止边界、创建后登记失败和双侧 manifest 要求。此审查不代表实现通过或阶段放行。
- 本任务卡、索引和计划链接已由同一 Reviewer 复核通过。准备时新工作区基线 fmt、Clippy、普通测试通过（49 passed、1 个需专用双卷环境的测试 ignored），22 个相对链接检查通过；这些结果只验证文档与既有 P0-01 基线，不证明本任务物化能力。当时 P0-01 的卷卸载失败已在其后修复并完成整轮 CI、正式审核及合并。
- 已合入实际 `main`，主 Agent 重跑领取基线 fmt、Clippy、普通测试通过（49 passed、1 个专用双卷用例 ignored），准入完成后领取本任务。双卷物化验证时必须执行专用用例，不能用本次普通基线检查代替。
- 主 Agent 已创建本任务专用 128 MiB APFS 镜像 `/private/tmp/thinws-p0-materialize.OC4JPT/cross-volume.dmg`，挂载于该私有父目录的 `mounted`；Volume UUID 为 `E4C7D91A-EF7B-476B-9BEB-946B53FEE500`，与父根的 `382DF1EF-8A38-4E30-8445-613B5192C6DE` 不同。`diskutil` 和 Probe 对镜像 UUID 的观测一致，ownership enabled、写预检 allowed。环境由主 Agent 持有，实验结束后按精确挂载点普通卸载并核对镜像关联；目前仍为供测试使用的活动挂载，不声称已清理。
- 物化行为与专项门禁结果见下方执行进展；上述环境准备不证明 CoW 成功。

## RED 与开发进展

- 编码 Agent 与主 Agent 独立执行 `cargo test -p thinws-p0-materialize --test same_volume actual_same_volume_clone_materializes_regular_file -- --exact --nocapture`：crate 编译成功，测试退出 101，断言因 `MaterializationError::NotImplemented` 失败。该 RED 证明预期物化行为尚未实现，不是编译或环境故障；固定 fixture 在断言前已清理已知条目，未使用递归自动析构。后续身份替换用例执行前须补齐 fixture 自身的身份核验，不能把初始固定树清理当作完整安全边界。
- 主 Agent 在本次真实双卷环境独立重跑 P0-01 的跨卷身份测试，1 passed。首次漏传环境变量而报环境缺失，补充精确 `THINWS_P0_CROSS_VOLUME_ROOT` 后通过；该配置错误与重跑均不计为 P0-02 RED/GREEN。
- 编码 Agent 与主 Agent 均以同一首条测试命令取得 GREEN：真实 held-FD `fclonefileat` 创建一个普通文件，目标内容一致，1 passed、退出 0。这仅为单文件初始切片，没有证明最终 manifest、写入隔离、跨卷、显式 Copy 或回滚；当时仍有 6 个待接入能力函数的 dead-code warning，代码未提交，完整门禁待后续实现。
- 新包加入后，根 workspace 的 `cargo deny --locked check` 因 `thinws-p0-materialize@0.0.0` 自有许可缺少精确条目而退出 4。主 Agent 仅在 `deny.toml` 增加该包/版本的 PolyForm 例外，未放宽第三方规则；根与独立 fuzz workspace 的 deny 均重跑退出 0。共享配置中的未用许可项仍会警告，不是依赖违规。此结果不代替后续实现完整门禁和审核。
- 新 FFI 早期检查中，主 Agent 在本次私有父根用 Ruby/Fiddle 只读调用原生目录 API，确认复制的只读目录 FD 连续枚举条目计数为 4、0（含 `.`/`..`），而搜索专用 FD 直接交给 `fdopendir` 返回 `EBADF`；相对于 held directory 重新打开独立的只读目录 description 后，两次枚举均为 4，所有 `closedir` 成功。GPT-6 Astra / `xhigh` 另据本机 SDK、Apple Libc/XNU 核实相同根因。已要求编码侧局部修正并固化重复扫描回归，不允许最终 manifest 因共享偏移漏项；这不是完整物化通过结论。
- GPT-6 Astra / `xhigh` 的早期 FFI 切片还指出源项被 FIFO 替换后的阻塞风险，以及变长 `dirent` 不应创建完整 `d_name` 数组引用。主 Agent 核对 [Rust 标准库同类处理](https://github.com/rust-lang/rust/blob/master/library/std/src/sys/fs/unix.rs)，编码侧已用非阻塞打开后复核类型、raw field pointer 和独立目录 description 做局部修正，未增加平行抽象。Darwin mode_t 与 C 变参提升的编译问题也已修正；这些静态修正和首条 GREEN 不替代后续边界回归及完整实现审核。
- 固定树切片中，编码侧新增 `partial_clone_failure_rolls_back_exact_created_identity_set_in_reverse`，首次执行退出 101：第二个普通文件注入 `EIO` 后，回滚打开父目录得到 `EINVAL`，结果为 `Incomplete`。根因是调用方传入 `.`，而正常名称组件校验明确禁止该值；改为复制已验证根 FD 作锚点，不放宽名称校验。编码侧取得 GREEN 后，主 Agent 独立运行 `cargo test --locked -p thinws-p0-materialize --test same_volume partial_clone_failure_rolls_back_exact_created_identity_set_in_reverse -- --exact --nocapture`，1 passed、退出 0，已创建文件移除，`.git` 与树外 sentinel 保留。该版本只回滚一个已创建项，尚不能凭测试名证明多项逆序补偿。
- 主 Agent 独立运行 `cargo test --locked -p thinws-p0-materialize --test same_volume actual_same_volume_clone_materializes_fixed_tree_with_complete_manifests -- --exact --nocapture`，1 passed、退出 0：固定树的 3 个普通文件均真实 clone 成功，限定 manifest 一致，普通文件源/目标 inode 不同，双向单侧写入互不影响，symlink 文本和两个 sentinel 未变。上述两次测试的已登记固定 fixture 清理成功；这是当前同卷切片证据，不代表跨卷、降级或全部失败边界通过。
- 同轮 GPT-6 Astra / `xhigh` 的结构审核发现回滚锚点、创建身份贯穿、根与祖先重验、unsupported 注入绕过真实布局、注入对象归属和错误证据丢失问题。主 Agent 已核验并交编码侧局部修复；除已有回归的锚点问题外，其余尚未闭环，不是正式 Approve。主 Agent 还确认当前 fixture 虽已比较整树身份与集合，清理仍有路径式调用；在根/祖先替换测试前必须补齐 dirfd-relative/no-follow 边界，不能把现有固定树清理当作这些故障场景的安全证明。
- 后续发现 `.git` 会被仅按名称自动接纳，与“计划预先声明控制项”不符。主 Agent 确认以调用方已知 `FileIdentity` 表达固定 `.git` 声明，未声明、缺失或身份不符时在物化前失败；只调整本实验输入，不新增产品 schema。当前正在联动实现与 fixture；主 Agent 此时运行通用门禁，fmt 退出 1，Clippy/test 退出 101，分别出现格式差异、大错误类型 lint 和测试缺新增输入字段。上述为开发中未通过项，不计作行为 RED，不提交该中间态；完成联动后必须重跑。
- 接入 `blake3` 后，主 Agent 核对工具版本为固定的 cargo-deny 0.20.2 与 cargo-audit 0.22.2；根与独立 fuzz workspace 的 `cargo deny --locked check`（fuzz 显式指定 manifest/config）和 `cargo audit --deny warnings --file <对应 Cargo.lock>` 均退出 0。audit 分别扫描 35/25 个依赖；deny 仍有对应 workspace 未使用的许可配置警告，无规则违规。此时尚未新增物化 fuzz target，不能把既有 fuzz 依赖检查写成新目标执行结果。
- 主 Agent 静态复核发现目标位于源树内时，初始实现会把正在写入的目标再次作为源遍历。编码侧为 `prepare_rejects_target_nested_under_source_without_writing` 取得真实 RED：只调用 prepare，错误地接受重叠路径，目标仍为空；随后复用 Probe ancestry 的目录身份拒绝源/目标相同或互为祖先。主 Agent 独立执行 `cargo test --locked -p thinws-p0-materialize --test same_volume prepare_rejects_target_nested_under_source_without_writing -- --exact --nocapture`，1 passed、退出 0，重叠目标未被写入且固定 fixture 清理成功。此用例验证目标嵌入源树分支，其他重叠方向仍须补回归。
- GPT-6 Astra / `xhigh` 的门禁接入审查指出，fixture 通过 `#[path]` 编译同一 `src/ffi.rs` 会同时受到生产 FFI 变异影响，身份观测与清理不再独立。主 Agent 接受该问题，批准仅在测试侧使用技术栈已选且 lockfile 已包含的 `rustix`，替换同源 helper；不新增产品 API，不整文件排除生产 FFI。独立 fixture 尚待验证，在此之前不运行具有删除副作用的全量变异测试。
- 主 Agent 与同一 Reviewer 核对 [cargo-mutants 27.1.0 的 scratch 生命周期](https://github.com/sourcefrog/cargo-mutants/blob/v27.1.0/src/build_dir.rs#L25-L69)：默认析构会删除包含实验 fixture 的工作树，不能用于本任务要求保留未知现场的变异执行。主 Agent 为现有 CI 变异步骤增加 `--leak-dirs`；YAML/命令参数检查先因缺少该参数退出 1，修改后退出 0。后续本地命令同样保留 scratch 并记录实际路径，不递归兜底清理；托管 CI runner 的短期生命周期不等于已确认业务回滚。这是配置检查，不是变异测试已执行；per-crate CI 拆分待实际新 fuzz 目标与完整测试就绪后接入。
- 早期失败现场位于本工作区的 `target/p0-materialize-tests/`：`same-volume-41799-0`、`same-volume-41799-1` 来自首次 dirfd fixture 清理错误地比较可变目录 size，`same-volume-10485-0` 来自重叠目录行为 RED 的断言退出。编码侧确认进程退出后原身份登记已丢失；主 Agent 仅枚举并核对来源说明，未依据名称/PID重新认领或删除。三者按未知身份现场保留，不纳入后续 fixture 清理范围；不能将其存在或名称当作可安全删除的证明。

### 独立调试版与发布版检查点（2026-09-12）

主 Agent 在编码侧暂停写入的稳定检查点执行以下命令，均退出 0：

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
env THINWS_P0_CROSS_VOLUME_ROOT=/private/tmp/thinws-p0-materialize.OC4JPT/mounted cargo test --locked --workspace --all-targets -- --include-ignored --nocapture
env THINWS_P0_CROSS_VOLUME_ROOT=/private/tmp/thinws-p0-materialize.OC4JPT/mounted cargo test --locked --release --workspace --all-targets -- --include-ignored --nocapture
```

两种 profile 各 68 passed、0 failed、0 ignored：物化 crate 的 7 项单元测试与 11 项集成测试，加上既有 Probe 的 50 项测试。测试内部派生子进程的输出不重复计数。此前在 helper 尚未写完时两次全量测试均因 `prepare_fresh_copy_after_clone` 未定义而退出 101；那是中间编译失败，不是行为 RED，已由本检查点的完整重跑替代。

| 本检查点新增可复查证据 | 已验证范围 |
|---|---|
| `.git` 显式身份声明、重叠目录、fixture 父目录移位 | 未声明/身份错误拒绝；目标嵌入源树拒绝；私有 fixture 父目录移位时清理预检失败且不删条目 |
| 真实双卷 clone 与独立 Copy | 测试内 Probe 证明两侧均 APFS、UUID Known/非空/不同；同一跨卷布局下 injected unsupported＋Allow 仍为 0 attempts/无写入；真实 `EXDEV` 后显式独立 Copy 的限定 manifest 一致 |
| 两条显式降级 | 预检同卷 unsupported 仅 Allow 启动 Copy，Deny 无 attempt；运行时第 0/1 个 clone 后注入 `ENOTSUP`，确认 baseline 后重新准备 Copy，保留旧失败 attempt；注入 `EIO` 时无第二后端，其他禁止降级 errno 分支仍待补齐 |
| Copy 部分写入 | 实际 `copy_file_bytes` 在注入 `ENOSPC` 后独立观察目标恰为源前 5 字节；集成测试另验证已登记两项按逆序补偿 |
| 平台 FFI | held search FD 重复完整扫描及新增/长名条目、不可读目录错误、无 writer FIFO 的有界子进程非阻塞检查、255/256/257 字节 link text、clone 冲突和 unlink 类型边界 |

创建后身份登记失败的测试按契约保留现场，不按名称重新认领。此次 debug 保留 `target/p0-materialize-tests/fixture-parent-same-volume-40183-9/case`（仅观测 `target/empty`：device `16777240`、inode `34502542`）；release 保留 `fixture-parent-same-volume-41342-7/case`（device `16777240`、inode `34502977`）。观测值不是删除授权；该检查点不得写成所有现场已清理。

这个检查点仍不是完整 Go：GPT-6 Astra / `xhigh` 随后指出 fresh Copy helper 未显式比较旧/新 Volume UUID 与挂载证据，主 Agent 已核验并要求局部修正和回归；新增 FFI/Copy 单元测试 fixture 的完整清理边界仍在审核。其他故障分支、新物化 fuzz、全量变异和最终实现 Approve 尚未完成。未运行变异测试，不把普通测试绿色视为安全判断已得到充分验证。

为保留成功执行的“身份未确认”用例诊断，CI 的 debug、release 和变异测试入口均增加 `--nocapture`。主 Agent 的 YAML/argv 检查先因缺参数失败，修改后通过；首次检查脚本使用了本机 Ruby 不支持的 `filter_map`，修正为兼容写法后才取得上述有效失败结果。固定 `cargo-mutants 27.1.0` 的只读列表此时枚举本 crate 319 项候选；该数值会随后续修正变化，只用于估算门禁规模，不是变异执行结果。

### 纯内存 fuzz 接入与后续审核

新增 `thinws_materialize_layout_policy`，只编译实际布局门槛的 crate 内部模块，不开放产品 API。输入驱动固定四路径、六个关系及候选状态的合成报告，不执行 Probe、文件系统或子进程操作。它不验证完整降级编排、UUID 重验或外部报告 schema。CI 保留原路径 fuzz，并增加本目标的同等 60 秒必跑入口；YAML 检查先因缺新目标失败，补齐后通过。

主 Agent 使用 cargo-fuzz 0.12.0、nightly-2026-08-14 独立执行下列两条命令，均退出 0：

```bash
cargo +nightly-2026-08-14 fuzz run thinws_materialize_layout_policy -- -max_total_time=60 -timeout=5 -max_len=4096 -print_final_stats=1
cargo +nightly-2026-08-14 fuzz run thinws_probe_path_validation -- -max_total_time=60 -timeout=5 -max_len=4096 -print_final_stats=1
```

首轮分别为 3,224,076 / 739,847 次输入、各 61 秒，peak RSS 工具字段分别为 517 / 745 MB，slowest unit 均为 0 秒；退出成功且 artifacts 无文件。两轮从空 corpus 开始，运行产生的自动语料由主 Agent 保留在各自 corpus 目录，尚未选择为受版本控制的回归样本。运行前后布局模块 SHA-256 均为 `d88a5b5968117736c6179ce38b307475e461da6ed0a62bbd5c2437c6a15a0759`，初版 harness 均为 `06a46f945669264c83f81bd6af959380a4998d8a86781cfbce4914f304f08484`。

该首轮记录不是最终 fuzz 收口：GPT-6 Astra / `xhigh` 确认 oracle 独立且没有真实路径副作用，同时要求落实主实现的同源导入，并补实际双 candidate 组合及顺序变化；编码側正在联动，最终须对更新后版本重跑。根/fuzz 的 deny 与 audit 同轮均退出 0（35/25 个依赖），fuzz package fmt 与 Clippy 通过；整个 workspace 的新改动仍待重新冻结验证。

同轮实现审核另发现 Full Copy 的路径身份可能在创建 FD 核对前被登记为 Confirmed，导致替换对象进入回滚清单。主 Agent 已核验并要求：创建 FD 与路径身份匹配后才确认，身份贯穿单项完成回执；用独立已登记 A/B 文件做实际登记和回滚回归。失败后的故障注入也须先复用现有路径边界验证。这些修改与共享 fixture 清理迁移尚在进行，不启动具有删除副作用的全量变异，不宣布正式 Approve。

### 身份修复与定向变异复核（2026-09-12）

本节更新上方历史检查点中的待处理状态，不替代后续全量收口：

- fresh Copy 已比较旧/新四路径的 Known Volume UUID、完整 filesystem、mount 和祖先身份，以及源快照与声明的 `.git` 身份；新增 UUID 缺失/改变、fsid、文件系统类型和挂载属性回归。
- Full Copy 先登记 Unconfirmed，创建 FD 与当前路径的类型/身份一致后才 Confirmed；单文件完成回执沿用该身份。实际 A/B 替换回归证明登记失败、回滚 Incomplete、B 身份与内容保留；A/B 均由独立 fixture 预先创建登记，最终清理没有事后按名称认领。
- FFI、Copy 单元测试和集成测试统一使用测试侧 `rustix` 的独立 `ControlledTree`，不依赖被测 FFI 进行身份观测/清理，无自动递归 Drop。故障注入入口先核对既有回滚边界。GPT-6 Astra / `xhigh` 已接受该限定修复与测试清理边界，可在 `--leak-dirs` 下开展变异验证；不是全任务 Approve。
- 主 Agent 再次执行上方同一组通用与双卷 debug/release 命令，先为两种 profile 各 70 passed；下述 candidate 回归加入后，两种 profile 各 71 passed、0 failed、0 ignored（物化 10 单元＋11 集成，Probe 50）。最新检查点仍按契约保留身份未确认现场：debug 的 `fixture-parent-same-volume-35994-1/case`（观测 device `16777240`、inode `34510236`）与 release 的 `fixture-parent-same-volume-36744-7/case`（device `16777240`、inode `34510531`），均在本工作区 `target/p0-materialize-tests/` 下。这些观测值不是删除授权，也不代表先前保留现场已清理。

布局 fuzz 已扩展至 15 种 candidate 模式，包含 Clone 三态＋Copy Supported 的正反序，以及 Clone Supported＋Copy Unknown/Unsupported 的正反序。实现两处调用与 harness 均导入同一内部模块，旧本地函数已删除；规定 Reviewer 已复核同源调用与独立 oracle。模块摘要仍为上方 `d88a5b…a0759`，更新后 harness SHA-256 为 `d7283edada3f2831ebbc90479564921c62f1f355a42b53ccdd99516555e2e18c`。主 Agent 用相同 60 秒命令独立重跑：3,516,208 inputs / 61 秒、peak RSS 517 MB、slowest unit 0、0 新增语料，退出 0、无 crash/hang/artifact；已有自动 corpus 保留。此短预算不代替阶段长预算。

定向变异使用固定 cargo-mutants 27.1.0：

```bash
env THINWS_P0_CROSS_VOLUME_ROOT=/private/tmp/thinws-p0-materialize.OC4JPT/mounted cargo mutants -p thinws-p0-materialize --re 'register_created|fallback_path_evidence_matches|layout_policy' --test-workspace true --leak-dirs --timeout 60 --jobs 2 --output target/p0-02-identity-layout-mutants-verified-rerun -- -- --include-ignored --nocapture
```

首次结果在 `target/p0-02-identity-layout-mutants/mutants.out/`：23 项中 20 caught、1 unviable、2 missed，退出 2。两项存活为 `layout_policy.rs:18:32` 的 `== → !=` 和 `:19:17` 的 `&& → ||`，是普通测试未区分 Clone/Copy candidate 的真实缺口，不作等价排除。编码侧补 `clone_layout_requires_full_copy_supported_independent_of_clone_state_and_order` 后取得 GREEN。一次复跑因与普通测试并发复制源码，临时 `.tmpuv31ib` 在复制时已被测试删除，setup 退出 1；这是环境时序失败，无变异结果，不算行为 RED。待普通测试终态且冻结代码后，主 Agent 顺序执行上方命令，41 秒、退出 0：**22 caught、1 unviable、0 missed、0 timeout**，两项原存活均为 CaughtMutant。唯一 unviable 是 `register_created` 返回 `Ok(Default::default())`，`FileIdentity` 未实现 `Default`，不是新增排除。

本次结果、逐项 diff 和日志在命令所指定目录的 `mutants.out/`。两次实际运行的 scratch 均按 `--leak-dirs` 保留：位于 `/var/folders/bf/x_rpvxyn1fzc8x2fn0gb8g9r0000gn/T/` 的 `cargo-mutants-thinworkspace-p0-02-pKL1C4.tmp`、`cargo-mutants-thinworkspace-p0-02-CMqOX2.tmp`、`cargo-mutants-thinworkspace-p0-02-2B8nkS.tmp`、`cargo-mutants-thinworkspace-p0-02-k7h5yV.tmp`。具体未确认现场由各测试 `--nocapture` 日志列出，没有递归兜底删除。

此时只完成 23 项定向验证；只读 inventory 为本 crate 330 项候选，FFI 未整文件排除、测试 fixture 不在变异对象中。全量变异、剩余源变化/路径替换/回滚故障矩阵、最终实现审核和远程 CI 尚未完成，P0-02 保持 In Progress。

### 失败矩阵补齐检查点（2026-09-12）

编码侧在既有实现上补充下列确定性测试，均直接 GREEN，属于测试缺口补齐，不冒充新行为的 RED：

- 第三个普通文件 clone 注入 `EIO` 时，已创建四项依次为 `empty`、`executable.sh`、`link-to-sentinel`、`nested`；实际补偿顺序严格相反，返回 ConfirmedBaseline。
- 同卷运行时注入 `EXDEV`、`ENOSPC`、`EACCES`、`EPERM`、`EIO`，即使 Allow 也仅一条 Clone attempt、保留精确 errno；`ENOTSUP` 配合 Deny 同样不启动 Copy。真实跨卷 `EXDEV` 仍由独立真卷用例验证，不由该注入表替代。
- Clone 与 Full Copy 中途修改同 inode 源文件内容，均返回 SourceChanged，补偿回到目标 baseline。另在源变化后注入 `ENOTSUP`：fresh Copy 准备拒绝变化的源，顶层返回最新 SourceChanged，旧 attempt 保留原始 `ENOTSUP`，没有第二后端。

主 Agent 在本组代码冻结后独立运行同一组 fmt、全 workspace Clippy、真实双卷 debug/release 命令，全部退出 0；两种 profile 各 **74 passed、0 failed、0 ignored**（物化 10 单元＋14 集成，Probe 50）。身份未确认用例本轮保留 debug 的 `fixture-parent-same-volume-20423-6/case`（观测 device `16777240`、inode `34511668`）与 release 的 `fixture-parent-same-volume-20864-7/case`（device `16777240`、inode `34512062`），位置仍为上述本工作区测试父目录；不是清理完成声明。关键剩余项仍为回滚失败/未知或替换对象、四路径/祖先变化、相关故障注入自身边界、全量变异与最终审核。定向 23 项的结果和证据范围已由 GPT-6 Astra / `xhigh` 只读复核，无实质问题；不构成全任务 Approve。

随后 GPT-6 Astra / `xhigh` 的限定回滚审核发现：`apply_rollback_fault` 仅校验四根，未在注入副作用前验证目标全集、`.git` 和 Unconfirmed 清单；`verify_target_baseline` 已因未知目标返回 `UnknownTargetEntry` 时，仍可能进入 AddUnknownEntry 创建路径。ReplaceCreated 删除前也缺少既有创建父目录身份校验。主 Agent 按实际调用链核验后接受该问题，要求先取得独立 fixture 的复现 RED，再复用现有 `validate_rollback_set` / `validate_rollback_parent` 局部修复；不新增产品抽象或修改规则以绕过边界。该处尚待修复与验证，因此前述绿色检查点仍不是完整安全结论，全量变异尚未启动。

编码侧与主 Agent 独立运行 `cargo test --locked -p thinws-p0-materialize --test same_volume rollback_fault_does_not_add_another_object_after_preexisting_unknown_target -- --exact --nocapture`，均退出 101：prepare 后 fixture 独立创建登记 `preexisting-unknown`，execute 已返回 UnknownTargetEntry，却又生成第二个 `p0-02-injected-unknown`，违反停止断言。两次 RED 分别保留本工作区测试父目录下的 `fixture-parent-same-volume-95497-0/case`（观测 device `16777240`、inode `34514753`）和 `fixture-parent-same-volume-190-0/case`（device `16777240`、inode `34514778`），未事后认领第二对象。另有 special-source 用例初次因 Unix socket 地址超长而失败，属于 fixture 环境错误，不是行为 RED；其 `fixture-parent-same-volume-86033-1` 现场登记已随进程退出丢失，保留不删除。该用例改用受控短父路径后仍须重跑验证。

上述停止边界已按限定方案修复；编码侧精确回归 GREEN 后，主 Agent 在冻结代码上独立执行完整 fmt、workspace Clippy、真实双卷 debug/release 命令，均退出 0。两种 profile 各 **82 passed、0 failed、0 ignored**（物化 11 单元＋21 集成，Probe 50），未沿用编码侧日常增量运行的过滤项。新增验证包括：

| 场景 | 本检查点实际结果 |
|---|---|
| 原未知目标后的错误注入 | 保留原 UnknownTargetEntry；没有第二次创建，已登记的 fixture 目标可安全清理 |
| 回滚第 0/2 步注入 EBUSY | 立即失败或先删除 `nested`/`link-to-sentinel` 后失败；精确 remaining、无 Copy；只认领 remaining 中与独立身份吻合的条目 |
| 注入未知项/替换对象 | Incomplete、0 removed、单一 attempt；错误中的 observed identity 与独立只读观测一致，对象保留，两个 sentinel 字节未变 |
| 四根各自替换与共同祖先移位 | 执行前拒绝、创建清单为空；恢复仅针对 fixture 预登记对象，恢复后完整身份集合一致 |
| 注入自身的祖先停止边界 | 私有用例真实移动共同祖先；AddUnknownEntry 与 ReplaceCreated 均返回边界问题，held 树身份集合不变 |
| special source 与未知初始目标 | 短父路径下实际 Unix socket 被拒绝；未知目标 prepare 拒绝；无物化写入、sentinel 未变 |

本轮故意保留的现场仍位于工作区 `target/p0-materialize-tests/`：debug 为 `fixture-parent-same-volume-16721-15/case`（登记失败，观测 inode `34516429`）、`fixture-parent-same-volume-16721-17/case`（替换）、`fixture-parent-same-volume-16721-25/case`（未知项）；release 为 `fixture-parent-same-volume-17605-15/case`（登记失败，观测 inode `34517089`）、`fixture-parent-same-volume-17605-17/case`（替换）、`fixture-parent-same-volume-17605-26/case`（未知项）。登记失败观测 device 均为 `16777240`；其他现场的身份比对在测试中执行，路径由日志保留。编码侧先前对应现场 `fixture-parent-same-volume-76327-0/case`、`fixture-parent-same-volume-76371-0/case` 也未清理。

该冻结版 `src/lib.rs` SHA-256 为 `46717d53c2e50edc53fe01320581f2b4969f697d37be6a11728b956601c196a7`；只读变异 inventory 仍为 330 项（lib 230、FFI 91、layout policy 9）。GPT-6 Astra / `xhigh` 已复核关闭本次停止边界缺陷，接受独立 fixture 的精确删除、排他重命名和原登记表子路径同步；本批测试与清理方式未发现阻止 `--leak-dirs` 全 crate 变异运行的结构问题。尚未确定性复现“集合检查后、删除前父目录替换”的真实竞争窗口，不据此宣称原子条件删除，也不新造 hook 冒充该证明。主 Agent 再补原验收范围内的其他重叠方向、manifest 独立原始字节/摘要断言及空源无 CoW confirmed 用例，待再次冻结验证后启动全量变异；本次限定复核不是全任务 Approve。

### 全量变异检查点（2026-09-12）

最后一组补测覆盖相同 source/target、source 嵌套于 target、空源成功但零 clone/Cow Unknown、固定 manifest 的原始字节与独立摘要断言、`EINVAL` 不降级。原计划的非法 UTF-8 实际文件名在本机 APFS 的 fixture 源文件创建时返回 `EILSEQ`（errno 92），尚未进入物化，不是行为 RED；对应 `fixture-parent-empty-source-55734-0` 现场（观测 device `16777240`、inode `34517649`）保留不认领。主 Agent 将真实文件用例调整为固定中文名 `数据.bin`，独立 expected hex 为 `e695b0e68dae2e62696e`，并另用纯函数测试证明 `raw-\xff/tail` 的证据编码保留为 `7261772dff2f7461696c`。这分别验证真实文件操作与任意原始字节编码，不承诺本机内核拒绝的文件名能够创建，也不把本机结果泛化为全部系统版本。

主 Agent 再次独立运行完整 fmt、workspace Clippy 与双卷 debug/release，均退出 0；两种 profile 各 **86 passed、0 failed、0 ignored**（物化 12 单元＋24 集成，Probe 50）。本轮保留现场为 debug 的 `fixture-parent-same-volume-75613-18/case`（登记失败，device `16777240`、inode `34519013`）、`fixture-parent-same-volume-75613-20/case`（替换）、`fixture-parent-same-volume-75613-28/case`（未知项）；release 的 `fixture-parent-same-volume-76602-19/case`（登记失败，device `16777240`、inode `34519724`）、`fixture-parent-same-volume-76602-20/case`（替换）、`fixture-parent-same-volume-76602-29/case`（未知项），位置仍为工作区测试父目录。规定 Reviewer 已限定复核上述补测，无新增实质问题。

该版实现/测试已冻结，主 Agent 启动：

```bash
env THINWS_P0_CROSS_VOLUME_ROOT=/private/tmp/thinws-p0-materialize.OC4JPT/mounted cargo mutants -p thinws-p0-materialize --test-workspace true --leak-dirs --timeout 60 --jobs 2 --output target/p0-02-full-materialize-mutants -- -- --include-ignored --nocapture
```

覆盖本 crate 全部 330 项，生产 FFI 纳入，测试 fixture 不纳入；结果入口为 `target/p0-02-full-materialize-mutants/mutants.out/`。主 Agent 已从同一运行句柄取得终态退出码 **3**，并核对 `outcomes.json`：UTC `11:31:40` 至 `11:41:50`，**202 caught、79 missed、46 unviable、3 timeout**，未变异基线通过。本轮全量门禁未通过，不能把上方 23 项定向成功写成全量成功。本轮 scratch 保留在前述系统临时父目录下的 `cargo-mutants-thinworkspace-p0-02-nsn27C.tmp` 与 `cargo-mutants-thinworkspace-p0-02-iN5vEh.tmp`，不兜底删除。

三个超时分别是 Copy 的写偏移 `+=` 变成 `*=`、digest 的 EOF 判断反转，以及 readlink 缓冲区容量 `*=` 变成 `/=`；工具分别终止对应变异测试后继续，三者不算普通通过或等价变异。存活项已分为候选选择与降级证据、最终 manifest/身份集合、根与逐项重验、组件/错误映射，以及真实 FFI 描述符语义。主 Agent 已将 `lib.rs` 的补测与局部去重、`ffi.rs` 的真实 FD 单元测试分给互不重叠的 GPT-5.6 Sol / `xhigh` 写入区域；GPT-6 Astra / `xhigh` 独立审核精确等价项与循环整理方案。当前未新增排除项，未提高超时或降低门禁。

该轮冻结的 `src/lib.rs` SHA-256 为 `4aa82c3048ed815fb8ef125a233cac68675f433710623a54b45503f6bca77cbe`，`tests/same_volume.rs` 为 `6fc89a0ef746e8e4ddc6a257c7a19b661f8b37fff46a33c74347bdd8df663aee`，共享 fixture 为 `870a071157c4de2f4b1e4881885c4032bba4f4b36cff2f6e56e41cb83032c71f`。存活项和超时处置后的全量重跑、最终完整审核、远程 CI、任务合并与 P0.b 收口仍未完成，P0-02 保持 In Progress；无公开 CLI、产品阶段或接口契约变更。

### 变异缺口补测与局部去重（2026-09-12）

两位 GPT-5.6 Sol / `xhigh` 分别修改 `src/lib.rs` 与 `src/ffi.rs`，共享 fixture、同卷集成测试、layout helper 和 fuzz harness 保持不变。新增回归直接覆盖候选选择/证据归属、manifest 与登记集合分别不符、组件和 errno、真实 held FD 错配、受保护 `.git` 的缺失与 `ENOTDIR`、回滚父身份、源路径替换，以及 FD flags、排他创建、合法 fd 0 和目录流析构。fd 0 与 Drop 验证位于独立子测试进程；fixture 仍使用独立 rustix 登记与核验，不引入未知对象认领或自动递归清理。

局部整理遵循已获 GPT-6 Astra / `xhigh` 确认的方向：重复身份/类型检查复用既有函数，并保留各次系统调用后的即时检查及错误上下文；最终 manifest 与身份集合验证收拢到既有 ExecutionContext 方法；Copy 以剩余切片表示未写区间，保留逐次写入、WriteZero、原 errno（含 EINTR）和部分失败行为；Copy/digest 明确区分 EOF；readlink 合并重复负值检查并保持原容量增长序列。Darwin 的重复或固定 `.` 不需要的 flags 已移除，CLOEXEC、独立目录描述符、真实读权限检查和任意名称的 no-follow 边界保留。这些是保持语义的整理，不新增 Port、故障注入框架或公开契约。空文件与 131,089 字节实际 Copy/digest、65,543 字节故障前缀、真实只读 FD 的 `EBADF`、最长 1,023 字节 link text 均有回归；整理前后同组测试均通过，不能冒称行为 RED。

主 Agent 在上述冻结实现独立执行本节前述完整 fmt、workspace Clippy 及双卷 debug/release 命令，全部退出 0；每种 profile 为 **110 passed、0 failed、0 ignored**（物化 36 单元＋24 集成，Probe 50）。`src/lib.rs` SHA-256 为 `b9a396bd11db53fc33156b9aa077f6da3e3c15d431e618662153010cb6b89496`，`src/ffi.rs` 为 `ce0d1daa0e19c16fbbe3140fc914774c64b9a9c9992b8f9a6dab4ab20e3f58e9`。本轮保留现场为测试父目录下 debug 的 `fixture-parent-same-volume-51977-20/case`（登记失败，device `16777240`、inode `34527097`）、`51977-21/case`（替换）、`51977-30/case`（未知项），以及 release 的 `fixture-parent-same-volume-52569-19/case`（登记失败，device `16777240`、inode `34527791`）、`52569-20/case`（替换）、`52569-29/case`（未知项）；后四个短名与同组完整名使用相同 `fixture-parent-same-volume-` 前缀，不按名称重新认领或清理。

根与独立 fuzz workspace 的 deny/audit 再次全部退出 0，扫描 35/25 个依赖；仍只有未用许可配置警告。fuzz workspace 的 fmt/Clippy 同样通过。主 Agent 使用固定 nightly 与前述完整 60 秒命令重跑两个未改动的 harness：layout 为 **3,101,318 inputs / 61 秒 / 517 MiB / new units 0**，path 为 **801,443 inputs / 61 秒 / 598 MiB / new units 193**，均退出 0、无 crash/hang，生成语料保留未删除。此次 smoke 不替代 P0 长预算门禁。

当前 crate 未排除列表为 284 个候选；数量相较旧版减少源于上述实现去重和循环表达变化，不把消失的旧变异记为 caught。GPT-6 Astra / `xhigh` 已核对两个冻结 hash、原 330 快照差异及新增测试，未发现本轮新增阻断，同意启动全量重跑；这是限定复核，不是整个 P0-02 Approve。跨缓冲区前缀测试验证的是明确故障注入边界，不宣称强制制造了内核 short-write 或 EINTR。该检查点之后的全量结果与组合补测见下文，以上普通门禁和 smoke 本身不能作为 P0-02 收口结论。

### 精确等价变异处置

负责人为主 Agent；以下 14 项已由 GPT-6 Astra / `xhigh` 核对冻结源码与本机 Darwin SDK，主 Agent 独立核对常量与实际候选列表。最初证明误引已安装的 libc 0.2.186；主 Agent 随后发现 `Cargo.lock` 实际始终锁定 0.2.189，双方已分别读取该版本 Darwin 定义补充复核：相关常量相同，14 项结论不变，源码和排除配置无需修改。这是审核证据引用的纠正，不能声称最初已经核对了正确版本。完整路径、行列、操作符和函数名只在 [`.cargo/mutants.toml`](../../../.cargo/mutants.toml) 维护，无函数或文件级排除，也不匹配其他位置的同类操作符。

| 等价类 | 数量 | 证明与失效条件 |
|---|---|---|
| root/child directory 的 flags 合并 | 4 | 当前 SEARCH、NOFOLLOW、CLOEXEC 的相邻操作数位域互斥；SEARCH 已含的 DIRECTORY 不再重复出现在表达式中 |
| 普通文件 read/write 的 flags 合并 | 6 | RDONLY 为零；WRONLY、NONBLOCK、NOFOLLOW、CLOEXEC 对应操作数位域互斥 |
| 排他创建 flags 合并 | 4 | WRONLY、CREAT、EXCL、NOFOLLOW、CLOEXEC 对应操作数位域互斥 |

逐项按变异后的实际表达式分组验证 `a & b == 0`，故 `a | b == a ^ b`，传入系统调用的 flags 逐位相同。平台或 libc 常量、括号/分组、相邻操作数、精确源码位置改变时必须重新审核；不能把删除 CLOEXEC 的 AND 变异、改变 readlink 调用次数的容量变异或未覆盖的竞争窗口写成等价。当前机器核对 `cargo mutants --no-config --list -p thinws-p0-materialize` 与加载配置后的列表：**284 → 270**，差集恰为上述 14 项，无新增候选或其他排除；原 P0-01 精确配置未改。

主 Agent 于 UTC `2026-09-12 12:07:17` 启动当前完整重跑：

```bash
env THINWS_P0_CROSS_VOLUME_ROOT=/private/tmp/thinws-p0-materialize.OC4JPT/mounted cargo mutants -p thinws-p0-materialize --test-workspace true --leak-dirs --timeout 60 --jobs 2 --output target/p0-02-materialize-mutants-after-boundary-tests -- -- --include-ignored --nocapture
```

结果入口为 `target/p0-02-materialize-mutants-after-boundary-tests/mutants.out/`；主 Agent 从同一运行句柄取得终态退出码 **0**，并核对 `outcomes.json`：UTC `12:13:00` 完成，**270 项＝225 caught＋45 unviable，0 missed、0 timeout**，未变异基线通过（11 秒构建＋1 秒测试）。该次执行期间源码与测试保持冻结，未在原工作区并行运行会生成临时文件的测试。两个 scratch 位于前述系统临时父目录下的 `cargo-mutants-thinworkspace-p0-02-oc39hy.tmp` 和 `cargo-mutants-thinworkspace-p0-02-hCemiu.tmp`，均保留；专用 APFS 镜像仍挂载供后续验证使用，尚未卸载或清理。

全量通过后，GPT-6 Astra / `xhigh` 的任务完整性预审仍发现一项组合证据缺口：既有测试分别覆盖 fresh Copy prepare 失败和独立 Copy 部分失败，但所有 `second_attempt_faults` 均为默认值，未实际验证“旧 Clone ENOTSUP 已回滚 → 新 Copy 执行中部分失败”时两条 attempt 与最新错误同时保留。主 Agent 已授权仅用现有字段追加一条集成回归，不据此宣称已发现实现错误，也不增加注入机制。补测后的验证、正式提交/审核/CI、挂载处置和任务级结论仍待完成；P0-02、P0.b 和完整 P0/P1 目标均未完成。

### 最终候选的组合回归（2026-09-12）

GPT-5.6 Sol / `xhigh` 仅在 `tests/same_volume.rs` 增加 `runtime_clone_enotsup_then_full_copy_enospc_retains_both_attempts_and_rollbacks`：首个普通文件真实 clone 成功后注入 `ENOTSUP`，确认回滚；第二条新 Copy attempt 在第二个普通文件写入 5 字节后注入 `ENOSPC`。断言顶层保留最新 Copy 错误、旧 Clone 错误仍在第一条 attempt、两条 Partial 和精确创建/逆序移除清单完整、没有第三次尝试；目标恢复 baseline，源文件、链接文本及两个 sentinel 不变。测试沿用现有注入字段和独立 fixture，没有修改实现。原实现直接通过，因此是证据补齐，不是行为 RED。

编码侧与主 Agent 独立运行该用例的 `--exact --nocapture` 命令，均为 1 passed、退出 0。主 Agent 随后再次运行上方完整 fmt、workspace Clippy 和真实双卷 debug/release 命令，均退出 0；两种 profile 各 **111 passed、0 failed、0 ignored**（物化 36 单元＋25 集成，Probe 50）。新增后的集成测试 SHA-256 为 `805e1158b7650ed54b3a5f7722e183604598d022eef2ddc86c76f7474cb61889`，lib、FFI、layout、共享 fixture、lockfile 与变异配置均保持前述冻结值。

本轮仍按契约保留测试父目录下的六处现场：debug 为 `fixture-parent-same-volume-95909-19/case`（登记失败，device `16777240`、inode `34529654`）、`fixture-parent-same-volume-95909-20/case`（替换）、`fixture-parent-same-volume-95909-32/case`（未知项）；release 为 `fixture-parent-same-volume-96575-21/case`（登记失败，device `16777240`、inode `34530431`）、`fixture-parent-same-volume-96575-23/case`（替换）、`fixture-parent-same-volume-96575-31/case`（未知项）。观测身份不产生事后删除授权，历史现场也未被清理。

GPT-6 Astra / `xhigh` 已只读核对新增测试、三个冻结文件 hash 和上述 libc 纠正，确认组合证据缺口关闭、可以开始最终候选全量重跑；未把此前 270 项结果冒称为新增测试后的运行，也未作全任务 Approve。

主 Agent 待所有普通测试终态、编码侧确认无活动命令或写入后，运行最终候选全量门禁：

```bash
env THINWS_P0_CROSS_VOLUME_ROOT=/private/tmp/thinws-p0-materialize.OC4JPT/mounted cargo mutants -p thinws-p0-materialize --test-workspace true --leak-dirs --timeout 60 --jobs 2 --output target/p0-02-materialize-mutants-final-candidate -- -- --include-ignored --nocapture
```

同一运行句柄终态退出 **0**；`outcomes.json` 的 UTC 起止为 `2026-09-12 12:22:41` 至 `12:28:03`，**270 项＝225 caught＋45 unviable，0 missed、0 timeout**，未变异基线通过。结果入口为命令指定目录下的 `mutants.out/`。执行后再次核对实现、测试、layout、共享 fixture、两个 harness、两个 lockfile 和质量配置 hash，均未改变。两个 scratch 位于前述系统临时父目录下的 `cargo-mutants-thinworkspace-p0-02-WWTomn.tmp` 与 `cargo-mutants-thinworkspace-p0-02-GDO0kp.tmp`，明确保留。

## 候选收口结论与保留边界

- 本地技术证据支持上述预定义实验目标：真实同卷 clone 与写入隔离、真实跨卷 EXDEV 和独立字节 Copy、两条显式降级、partial/登记失败/回滚停止、路径与源变化，以及第二次 Copy 再失败均有真实平台或明确标注注入的回归。当前完整验收集为 111 项；本节不把实验结果推广为 P1 产品能力、完整元数据保真或恶意同 UID 安全隔离。
- fmt、workspace Clippy、真实双卷 debug/release、受影响 crate 全量变异均通过；根/fuzz 供应链检查与两个固定 60 秒 fuzz smoke 的命令及结果见上方最近检查点，其对应输入文件与锁定依赖未变。当前相关入口、计划与记录的 50 个本地链接目标存在，`git diff --check` 通过。新增直接依赖 `blake3` 用于限定 manifest 的内容摘要，沿用技术栈选型；`rustix` 仅在测试侧提供独立身份观测/清理，不复用被变异的生产 FFI。
- 规定 Reviewer 的可提交范围预审未发现新增阻断；实验 6 个文件和新 fuzz harness 必须完整纳入提交，自动生成 corpus 不整批提交。正式候选审核、远端 CI、审核后的 Verification 和合并仍待执行；此时不作最终任务 Go，也不标记 Done。P0.b 还依赖 P0-03，整个 P0 及 P1 目标未完成。
- 主 Agent 负责保留当前任务 worktree、变异原始日志/scratch、自动 fuzz 语料和上文登记的失败现场，用于后续审核及 P0 阶段复核。不存在要求用户立即手工删除的清理任务；保留不等于已清理，不按路径名或观测值事后认领未知对象。专用镜像目前仍挂载供审核后真实 Verification 使用，完成后只按核验的镜像关联和精确挂载点普通卸载；镜像及内部现场可继续保留，不强制删除或卸载。
- 未执行及限制：远端本候选 CI 尚未运行；P0 阶段长预算 fuzz、Git/Base、跨进程强杀恢复分别按原计划后续交付。内核 short-write/EINTR 的确定性触发、删除前瞬时竞态的原子安全证明、ACL/xattr 等未纳入本实验的元数据不冒称已验证；不通过新增机制或放宽契约掩盖这些边界。

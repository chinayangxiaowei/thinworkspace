# P0-03 Git 托管拓扑与 Base 实施记录

## 职责与非职责

本文管理 P0-03 的准入、实验验收、候选 ADR 证据和未解决边界。不重新定义产品 Git 拓扑、BaseKey 字段、公开兼容范围或任务门禁；实验结论改变长期设计时，必须修改对应权威来源，不能只留在本文。

## 任务准入

| 项目 | 当前值 |
|---|---|
| 类型、阶段、小阶段、风险 | Spike/ADR；P0；P0.b；R4（Git refs、发布与精确回收） |
| 状态 | In Progress；首个属性类型判别切片经主 Agent 准入，基线门禁通过后领取；全任务 Git/Base 验收尚未完成 |
| 负责人 | 主 Agent |
| 模型分工 | 编码实际使用 GPT-5.6 Sol / `gpt-5.6-sol` / `xhigh`；审核使用 GPT-6 Astra / `gpt-6-astra` / `xhigh`，首切片通过最终限定复核、允许提交并继续后续拓扑实验；无全任务 Approve |
| 准备日期、环境 | 2026-09-12；实测 macOS 15.7.2（24G325）、arm64、Apple Git 2.39.5（Apple Git-154） |
| 初始源码基线 | 已合并并通过 CI 的 `main`：`2e29f88a1aae2ee53ce52dbc38a7c0765b58cb15` |
| 当前集成基线 | `22d60f64f27959082913572cd80aa8b3f89ce590`；已合入 `main` 的 P0-02 收口提交 `ea9dd33ed1d4e1ab8a5b4af4921d0ef077a33517`，保留 P0-03 的 In Progress 状态及未提交实验；此轮只合并两个收口文档，不包含新实验代码 |
| 工作区、分支 | `/Volumes/data/code/thinworkspace-p0-03`；`task/p0-03-git-base`；标准 Git linked worktree，不声明 CoW |
| 前置依赖 | P0-01 已合并；属性兼容政策仍未决，但不阻塞下述不作产品接纳/拒绝决定的类型判别实验 |
| 主要写入区域 | `experiments/p0/git-base/` 及直接配套实验测试；首个切片范围见下节，不提前实现其他 Git/Base 行为 |
| 共享文件协调 | Cargo/lockfile、CI、质量配置及文档索引由主 Agent 集成；不与 P0-02 同时修改同一工作区的共享文件 |

权威输入为[Phase 1 详细设计](../../project/design/ThinWorkspace_Phase1单机CLI详细设计_v1.0.md)的 Repository/Git 与 Base 章节、[技术栈](../../project/reference/技术栈.md)的 Git/编码/测试章节、[用户手册](../../project/reference/ThinWorkspace_Phase1用户操作手册_v1.0.md)的仓库兼容和 checkout 可见契约，以及[开发规范](../process/开发规范.md)、[任务流程](../process/任务流程.md)。任务依赖与小阶段归[实施计划](../planning/ThinWorkspace_Phase1实施计划_v1.0.md)管理。

## 尚待确认的准入边界

只读准备已发现：目标 Git 的 `check-attr` 文本输出不能完整区分属性的布尔状态与同名字符串，例如 `-filter` 与 `filter=unset` 都可能输出 `unset`；NUL 分隔解决字段边界，不解决类型歧义。此前已向维护者提问，尚未收到答复：是否接受连显式关闭声明也拒绝的保守兼容限制，或继续验证保留关闭声明兼容性的精确方案。

这不是已接受的产品决策。确认前不得把“属性名出现即拒绝”写成用户契约或实现默认行为，也不得把未知结果当作安全放行；标准 `text/eol` 的现有支持承诺不变。最终方案必须在目标 Apple Git 上取得真实样例证据，再由规定模型审核。此边界不阻塞 P0-02。

### 只读核查后的最小验证候选（2026-09-12）

GPT-5.6 Sol / `xhigh` 查阅上游 Git v2.39.5 源码，主 Agent 独立核对相关实现，发现有不解析 `.gitattributes` 的候选路径：Git 的 attr pathspec 在匹配时保留内部属性类型，`:(attr:-filter)` 与 `:(attr:filter=unset)` 分别匹配布尔 Unset 和普通字符串值，不依赖 `check-attr` 的显示文本。[源码依据](https://github.com/git/git/blob/v2.39.5/pathspec.c#L725-L755)

只读准备时提出的候选是在一个合成仓库的固定 tree、临时 index 和受控空 worktree 上验证 `git ls-files --cached -z` 配合上述两个固定 pathspec 的结果集合，覆盖嵌套属性、宏和保留词碰撞。命令不执行 checkout 或转换；外部 filter 的受控 canary 必须证明未被调用，fixture 的文件内容与两个 index 身份须保持不变。**此处保存候选依据，不是已执行结果或产品实现；后续准入见下节。**

最关键的未验证前提是属性来源：`ls-files --cached` 控制文件枚举，不会自动将属性读取方向设为 INDEX；上游默认 CHECKIN 会先找工作树 `.gitattributes`，缺失时才回退 index。因此实验必须明确隔离 system/global 属性与配置、保持独立空 worktree、确认无 `info/attributes` 干扰，并用互相冲突的受控数据证明读取的是指定临时 index；不能把空工作树外观当成永久保证。[源码依据](https://github.com/git/git/blob/v2.39.5/attr.c#L797-L819)

三类属性不能笼统称为“均可显式关闭”：上游对布尔 true/false 的 `working-tree-encoding` 直接报错；`ident` 只在真正布尔 Set 时展开，裸 `ident` 与字符串 `ident=set` 的显示文本相同但转换行为不同。这些仍是上游源码结论；首个切片只验证原始类型及查询安全，不证明实际 checkout 的转换行为，也不据此自行增加或缩减支持声明。[源码依据](https://github.com/git/git/blob/v2.39.5/convert.c#L1228-L1294)

GPT-6 Astra / `xhigh` 已审核此方向，要求补清证明范围、非空独立预期集合、属性来源隔离和 canary 阳性验证；未发现首个切片必须先修改 ADR 的冲突。此审核本身不是 Ready/Go。主 Agent 核验后在下节完成切片准入；先前的保守限制提问未获批准，既不变成默认政策，也不阻塞不改变现有契约的技术验证。

### 首个切片准入与验收（2026-09-12）

本切片服务 P0-03 的独立 index/安全属性查询问题。主 Agent 负责，在 P0.b、R4 和既定工作区内执行；只有本节输入与验收已明确的实验进入实施，完整 Git/Base 结论、转换兼容政策和 ADR 均未放行。

1. 输入是本次创建的私有合成仓库、固定 tree、两份属性互相冲突的独立 index 及受控 worktree；不读取或修改用户仓库，不联网，不修改全局配置。Gitdir、worktree、index 和实际 Git 版本须在安全实验输出中可核对。
2. `ls-files --cached -z` 的完整路径集合必须与独立、非空 fixture 清单精确一致。固定 attr pathspec 查询分别覆盖 Set、Unset、Unspecified 与字符串值，包含 `set`/`unset`/`unspecified` 的显示碰撞、正反样例、嵌套属性和宏；由 Git 解释属性，不自行解析 `.gitattributes`，不把空结果或两个查询恰好互斥当作成功。
3. 用两份 index 的冲突声明，以及空 worktree 与受控冲突 `.gitattributes` 的对照，实测属性来源与优先级。system/global/info attributes、继承的 Git/pathspec 环境及配置均须隔离；显式关闭 fsmonitor 等外部程序入口。fixture 准备不用 `git add` 或 checkout，避免准备阶段触发转换。
4. 查询前后核对两份 index 的内容和文件身份，以及预先登记的 fixture 文件内容与身份；变化即失败。设置受控 filter canary，独立阳性验证观察链确实能记录调用，再证明查询阶段没有调用。除这次明确的 canary 阳性控制外，不运行外部转换程序；本切片不执行 checkout，不据此声明 `ident` 或编码转换实测通过。
5. 所有子进程采用 argv、固定 cwd、受控环境和有界超时，失败保留原始退出结果与可解释证据。私有实验根及其路径由测试明确报告并保留；本切片不实现删除/递归自动析构，不按名称认领未知对象，后续清理由所有者另行核验。
6. 首先以已知会丢失类型的显示文本判别路径取得可解释 RED，再改为 Git 内建 attr pathspec 并取得 GREEN；不得用编译错误、空 index 或失效 canary 充当 RED。实现仅提供实验所需的最小调用与结果，不引入 P1 Port、通用命令框架或第二套属性模型。
7. 执行《任务流程》通用门禁；首个有界验收入口为 `cargo test -p thinws-p0-git-base --test attribute_types -- --nocapture`，release 同名入口也必须通过。之后由 GPT-6 Astra / `xhigh` 审核切片代码、fixture 与证据，再继续本任务其他实验。全任务变异/fuzz、Git/Base 矩阵和正式 ADR 仍按下文执行，不由本切片代替。

对“声明存在但当前不发生转换”的值，最终按声明还是有效转换拒绝，仍是产品决策而非 Git 类型事实。本切片只提供判别证据，不实现接纳或拒绝策略；需要冻结公开边界时再携带实测结果请求维护者决策，标准 `text/eol` 承诺不变。

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
- 准入准备时尚无本任务代码、合成 Git 仓库、Base、实验分支或物化测试结果；该项是历史准备证据，后续实验进展见下文。工作区中的项目任务分支仅用于开发，不是被测产品的托管分支。
- GPT-6 Astra / `xhigh` 完成任务卡与两处导航的只读复核，无阻断发现，认可这三个准备文档可提交；此结论不表示 Ready、实现批准或 Go。
- 以上为准入准备时的证据，不作 Go 或阶段放行声明。2026-09-12 主 Agent 根据规定 Reviewer 的限定意见补齐首个切片验收，完成 Ready 检查并领取为 In Progress；不是把只读审核直接视为状态批准。
- 当前集成基线的 fmt、Clippy 和原样普通 workspace 测试均退出 0：109 passed、0 failed、2 ignored。两个跨卷用例因未配置本工作区专用第二卷而未执行，不替代 P0-02 的真实双卷证据；测试产生的保留现场不删除。该基线检查不包含后来新增的 Git 实验。

### 首个行为 RED

GPT-5.6 Sol / `xhigh` 与主 Agent 分别执行 `cargo test -p thinws-p0-git-base --test attribute_types -- --nocapture`，均编译成功、退出 101，1 failed。测试先确认 Apple Git `2.39.5 (Apple Git-154)` 和精确非空 cached 路径集合，再因布尔 Unset 的预期集合仅含 `boolean-unset.bin`、实际却同时包含 `string-unset.bin` 而失败。初始实现按 `check-attr --cached -z` 的显示文本分类，合成属性名为 `thinwsprobe`；此结果证明该显示分类丢失类型，不证明真实 filter、ident 或编码转换行为，也不是环境/编译失败。

主 Agent 独立重跑前后，`src/lib.rs` SHA-256 均为 `022e6d934c606bef452c687bbac7f5da4d08236ab2891213729a71a0828061ed`，`tests/attribute_types.rs` 均为 `fa05937b8162210635e3df4c518db1f2271000df285eaa0db324eddde79508f1`。两次保留根分别为本工作区 `target/p0-git-base-tests/attribute-types-41504-1789218869808512000-0` 和 `target/p0-git-base-tests/attribute-types-52781-1789218936121847000-0`；输出中的 `experiments/p0/git-base/../../../target` 是编译期路径拼接，不是用户输入，本次未删除或重新认领这些对象。

首个 pathspec GREEN 的后续结果见下节；本次 RED 不证明其余 Git/Base 任务完成，也不代替专项审核。

主 Agent 检查新包接入时，根 `cargo deny --locked check` 首次退出 4，原因仅为自有 `thinws-p0-git-base@0.0.0` 缺少精确许可条目；依赖图当时没有新增第三方包。在 `deny.toml` 增加该包/版本的 PolyForm 例外后，根检查与 `cargo deny --locked --manifest-path fuzz/Cargo.toml --config deny.toml check` 均退出 0，未放宽第三方规则。未遇到的许可配置仍给出 warning；此接入检查不代替后续代码完整门禁。

### 首个稳定 GREEN 与限定审核输入

扩展测试中曾出现两类 fixture 错误：canary 阳性控制的 Git 调用 cwd 没有指向受控冲突 worktree，导致 marker 不存在；独立预期又把声明为 `filter=canary` 的路径同时列入 Unspecified。Sol 修正 cwd 和这三处预期后重跑通过；两类失败均不计为产品行为 RED。首个扩展失败根 `attribute-types-13537-1789219304905702000-0` 及其后的实验根继续保留。

稳定实现改用 Git 内建 attr pathspec，实际受测属性为 `filter`。固定 fixture 的 17 条 cached 路径及独立类型集合通过：Set、Unset、Unspecified 与字符串 `set`/`unset`/`unspecified` 不混淆，嵌套覆盖与宏由 Git 解释；两份 index 的冲突路径分别呈现 Set/Unset，受控冲突 worktree 再覆盖成字符串 `worktree`。Git 报告为 `2.39.5 (Apple Git-154)`，两份 tree 分别为 `7f9c22840b6f54ce8ed462933adddada78d977f1` 和 `3f356626f24f939a24f96367d9fa518e8f2fc548`。这些是本机实验事实，不是产品兼容政策或完整 Base 构建结论。

canary 阳性控制明确使用受控 `/usr/bin/tee`，经不带 `-w` 的 `hash-object --stdin --path=canary.bin` 触发一次已批准的 clean 调用，验证 marker 的实际字节。随后三个查询上下文的 marker、两份 index 身份/字节及 fixture 快照均不变，空 worktree 仍为空。只有阳性控制执行了这次受控转换；查询本身未执行 checkout，未测试 `ident`/编码转换，也没有执行未知 filter。

Sol 的定向 fmt、Clippy、debug/release 均退出 0，各 profile 的同名集成入口均为 1 passed。主 Agent 随后在源码冻结且前后哈希一致时独立执行：

- 《任务流程》原样通用门禁全部退出 0，普通 workspace 测试 **110 passed、0 failed、2 ignored**；专用第二卷未配置，两个跨卷用例没有执行，不替代 P0-02 完整双卷验证。
- `cargo test -p thinws-p0-git-base --test attribute_types -- --nocapture` 和 `cargo test --release -p thinws-p0-git-base --test attribute_types -- --nocapture` 均退出 0，各 **1 passed、0 failed、0 ignored**。
- `src/lib.rs` SHA-256 为 `968ffeada945dd6b80c0022871056f5b826e14891a5c90ab054d5cdc7913a161`，测试文件为 `35651e43ff8adc67395b86170d080f92e655211692e2ebd571de45e2db92510c`；根 manifest/lock 也未在验证期间改变。

Sol 最近 debug/release 保留根为 `attribute-types-45644-1789219498383654000-0`、`attribute-types-48061-1789219512138815000-0`；主 Agent 定向重跑为 `attribute-types-63683-1789219609289062000-0`、`attribute-types-63682-1789219609286972000-0`，均位于本工作区 `target/p0-git-base-tests/`。普通测试成功输出被 Cargo 捕获，不能据此说它没有产生额外保留现场；本次未清理任何对象。

GPT-6 Astra / `xhigh` 对上述精确哈希给出 **Changes requested，仅限首切片**：类型/来源正反集合及 canary 主路径成立，但初始化失败的保留根报告太晚、异常终止丢失 kill/wait/输出及信号状态、成功退出时丢弃 stderr、NUL 解析接受缺少末尾分隔符的输入。这些失败证据与解析缺口未被前述 happy-path GREEN 覆盖。

主 Agent 已核验并授权 Sol 局部修正、各自补失败回归；同时删除库查询从不使用的 stdin 参数/写入分支，保留测试准备 helper 的真实 stdin。固定实验对成功但有 stderr 的 Git 调用也需失败并保留原始诊断；非法属性样例须由实际 Apple Git 回归确认，不把上游源码推测冒充实测。修复不得引入新 Supervisor、产品 Port、通用命令框架或兼容策略。Git 输出 fuzz、受影响 crate 变异、完整 Git/Base 矩阵、正式 ADR 和任务合并均未完成；不作全任务 Go 或阶段放行声明。

修复前新增回归取得可解释 RED：Sol 执行 `cargo test -p thinws-p0-git-base -- --nocapture`，缺少尾部 NUL 的 `a\0b` 被旧解析器接受，测试退出 101；再执行 `cargo test -p thinws-p0-git-base --test attribute_types apple_git_warning_is_not_silently_accepted -- --nocapture`，旧查询把带非法属性的结果返回为 Ok，退出 101。主 Agent 通读新增断言，并对保留根 `attribute-types-47710-1789220170851273000-0` 独立运行受控 `ls-files --cached -z` 属性查询，实际退出 0，同时取得非法属性诊断和 NUL 路径输出；这验证了“成功退出但有警告”确实存在，不是模拟或上游推测。修复后的回归还必须断言保存的状态仍为 success、stderr 非空及 stdout 保留，不能由另一种 Git 失败产生假 GREEN。

### 局部修复后的独立核验

主 Agent 对库 SHA-256 `91c1e9c39eb4ed67b5de589599527aa4ebe6c3d494ca11b67d63e1d87f454d74`、集成测试 `7975d4a5e39f086696a6df2af35693c170b0e7a7f928ebe858f1c90b8f63678c` 独立执行通用门禁，全部退出 0，普通 workspace 测试 **116 passed、0 failed、2 ignored**；专用第二卷未配置，忽略项仍未执行。定向 debug 集成入口 **3 passed**，`cargo test --release -p thinws-p0-git-base --all-targets -- --nocapture` 为库 **4 passed**、集成 **3 passed**，均无失败或忽略；验证前后哈希一致。真实 signal/timeout 测试使用受控 `/bin/sleep`，只验证已取得的退出状态及空两流，不声称实测 kill/poll 系统调用失败或任意大输出场景。

上述定向 debug 保留根为 `attribute-types-26597-1789220680546925000-0`、`attribute-types-26597-1789220680546940000-1`、`attribute-types-26597-1789220680546941000-2`；release 为 `attribute-types-28188-1789220689666027000-0`、`attribute-types-28188-1789220689666036000-1`、`attribute-types-28188-1789220689666210000-2`，均位于本工作区 `target/p0-git-base-tests/`，没有清理。

GPT-6 Astra / `xhigh` 确认查询库、初始化根报告、真实 warning 回归及 NUL 检查的原问题已关闭，但限定结论仍为 **Changes requested**：测试准备 helper 也受“所有子进程”验收约束，仍有 kill 失败后 wait、只打印 `status.code()`、reap 错误覆盖原始原因及成功 stderr 被忽略的同类遗漏。主 Agent 已授权 Sol 只修既有 helper 分支并补窄回归，不让 fixture 复用被测 runner，不引入通用框架。此记录的通过数量只对应上面的冻结输入，不证明后续修复已验证。

准备 helper 的独立失败回归 `fixture_runner_rejects_exit_zero_git_stderr` 也先编译成功后退出 101：实际 Apple Git 警告被 helper 静默接受，保留根为 `attribute-types-10140-1789221194757667000-0`。局部修复后主 Agent 通读完整 helper，并独立运行 `cargo test -p thinws-p0-git-base --all-targets -- --nocapture` 及 release 同名入口，均退出 0、各 **4 unit + 4 integration passed**。测试会有意捕获 fixture helper 的断言失败，输出中的 `panicked` 属于预期负例，不是未处理的产品 panic；实际终态和正反断言均通过。库 SHA-256 `58e3220b4854953ab4004e876a0fc2a64b5bf215f6b5398ac49cd761161c2793`、集成测试 `10b4d81296757d769ec00f2b204a4844bc44d91bfd5be45df4c550b0cf323708` 在主 Agent 验证前后未变。测试仍没有实测 OS kill/poll 失败；这些分支只取得代码检查证据，不为此新增注入框架。

接着按既有 Git 输出门禁接入纯字节 fuzz：将同一个私有 NUL 路径解析器供实验调用和 harness 使用，不扩展公开 API、转换兼容政策或 Git 命令框架。harness 不执行 Git、文件系统或子进程，只在有界输入上检查字节保真、重复项与排序、空字段和截断拒绝。变化后的组合证据见下节；此前通过数字不直接用于新的输入。

### Git 输出 fuzz 与首次全量变异

组合候选库 SHA-256 为 `b4e2c92f48bf733d4157de23136a9fb109bb6a102d41968bbf5c2bd40f50aec2`；`path_output.rs` 为 `b3e63a4e63b22561a71276c47f058072c620a34d936b19f1798b6ce7a4886500`，harness 为 `f4e262892db62444cb54f1ca3fc4d2b56da953b25b8ba7d528384cdd093a3bff`。原集成测试哈希仍为上一节的 `10b4d812…`。新模块保持 crate 私有可见性，库仍映射到既有结构化错误；无新增依赖，fuzz lockfile 未改。以下均为 2026-09-12 的本机结果，不是完整 P0-03 或阶段结论。

| 验证 | 实际结果及边界 |
|---|---|
| 主 Agent 通用与静态门禁 | 根/fuzz fmt、含 `--all-features` 的两 workspace Clippy 均退出 0；原样普通 workspace 测试 **118 passed、0 failed、2 ignored**；release Git crate **5 unit + 4 integration passed**。跨卷忽略项未在本次环境执行 |
| 主 Agent 供应链 | 根/fuzz `cargo deny --locked ... check` 与两份 lockfile 的 `cargo audit --deny warnings` 均退出 0；未命中的既有许可配置仍有 warning，不新增第三方许可例外 |
| Sol fuzz | 固定 `cargo-fuzz 0.12.0`、`nightly-2026-08-14`；build 退出 0。`fuzz run thinws_git_path_output /tmp/thinws-p0-03-git-path-output.n6kcLs -- -max_total_time=60 -timeout=5 -max_len=4096 -print_final_stats=1` 退出 0，**216690** 次、61 秒、峰值 RSS 660 MiB，无 crash/timeout |
| 主 Agent 独立 fuzz | 同版本、同参数，在独立空 corpus `/tmp/thinws-p0-03-root-git-path.Bvbtkp` 运行，退出 0，**239387** 次、61 秒、峰值 RSS 745 MiB，无 crash/timeout。仅证明该有界纯字节解析目标，不证明真实 Git 或文件系统语义 |
| 主 Agent 首次全 crate 变异 | `cargo mutants -p thinws-p0-git-base --leak-dirs --timeout 60 --jobs 2 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-mutants.9hx3VN -- -- --nocapture` 退出 **2**，成功 baseline 后 **47 = 36 caught + 6 unviable + 5 missed，0 timeout**；没有排除本任务变异 |
| CI 接入 | 既有 workflow 只增加第三个 fuzz smoke，沿用固定工具链和相同预算。主 Agent 核对两行 diff，Ruby YAML 语法解析与 `git diff --check` 退出 0；这不是远端新候选 CI 已通过 |

变异原始结果在上述目录的 `mutants.out/`，两个 `--leak-dirs` scratch 为 `/var/folders/bf/x_rpvxyn1fzc8x2fn0gb8g9r0000gn/T/cargo-mutants-thinworkspace-p0-03-cLSpfJ.tmp` 和 `…-1sDhWR.tmp`；与所有 fixture、两个临时 corpus 一并保留，不要求用户立即清理，也不声称已清理。

GPT-6 Astra / `xhigh` 独立核对源码、fixture、fuzz、CI 和原始变异结果，未发现新增阻断性实现问题，但该检查点的**门禁未通过，不能批准切片完成**。五个存活变异为两个错误类型的 `source()` 丢失、两个 `Display` 变为空，以及超时严格比较 `<` 被改成 `<=`。主 Agent 据此授权 Sol 补原始 cause/上下文/完整状态断言，并将原比较提取成实际运行与边界测试共用的私有纯判断，验证 before/equal/after；不新增 clock trait、注入框架或变异排除。`<=` 不视为等价变异。当时后续补测、重跑、组合复核及完整 Git/Base/ADR 尚未完成；补测结果见下节。

### 补测后的首切片候选

Sol 完成上述补测后，库 SHA-256 冻结为 `c9a87c48c0acb2877cb85719d447ca8d20c9db5212d0412a3b2efda0b376d175`。只有库内既有错误映射测试和私有 deadline 判断变化；运行路径与单测共用严格 `<`，并未改变比较政策。人工构造的错误值只证明 cause/诊断映射，不冒称实测 OS kill/poll 失败。解析模块、真实 Git 集成测试、fuzz harness/manifest/lock 与上述已验证输入一致。

主 Agent 通读改动并独立执行原样通用门禁及 release Git crate：全部退出 0，普通 workspace **121 passed、0 failed、2 ignored**，release 为 **8 unit + 4 integration passed**。两个跨卷用例仍因本次未配置第二卷而未执行；完整 Git/Base 任务和阶段长预算门禁未由这些结果代替。

主 Agent 再次执行相同参数的**全 crate**变异，输出目录为 `/Volumes/data/code/thinworkspace-p0-03/target/p0-03-mutants.QizjFX/mutants.out`，成功 baseline 后退出 **0**：**49 = 43 caught + 6 unviable，0 missed、0 timeout**，耗时 59 秒。没有新排除、禁用测试或缩小变异范围。新增判断使总数由 47 变为 49，不能拿原数量宣称“全部原样通过”。两个新 scratch `/var/folders/bf/x_rpvxyn1fzc8x2fn0gb8g9r0000gn/T/cargo-mutants-thinworkspace-p0-03-ebqZNY.tmp`、`…-TC9Bae.tmp` 继续保留。

GPT-6 Astra / `xhigh` 对这个精确组合候选和更新证据完成最终限定复核：未发现新的实质问题，独立核对原始变异结果及 baseline，确认无新排除，允许提交首个属性查询实验并继续既定 P0-03 后续拓扑实验。主 Agent 另检查相关四份入口/过程文档的 37 个本地链接目标存在，未把目标存在表述为锚点校验。该结论不是全任务 Approve、Git/Base ADR 冻结、P0.b 收口或阶段放行；远端新候选 CI 尚未执行，P0-03 保持 In Progress。公开 CLI 和产品兼容政策均未变化。

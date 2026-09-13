# P0-03 Git 托管拓扑与 Base 实施记录

## 职责与非职责

本文管理 P0-03 的准入、实验验收、候选 ADR 证据和未解决边界。不重新定义产品 Git 拓扑、BaseKey 字段、公开兼容范围或任务门禁；实验结论改变长期设计时，必须修改对应权威来源，不能只留在本文。

## 任务准入

| 项目 | 当前值 |
|---|---|
| 类型、阶段、小阶段、风险 | Spike/ADR；P0；P0.b；R4（Git refs、发布与精确回收） |
| 状态 | In Progress；属性查询、托管拓扑、重复 fetch、Base 发布/复用拒绝、Base/APFS/双 Workspace 组合、显式隔离重建、基线选择、路径/Gitlink 拒绝和内建转换已有实验观察。runner 有界收尾修复、75 项模块变异及 5 项历史 Base 疑点复验已取得本机证据；sparse/转换产品兼容矩阵、正式 ADR、完整候选门禁及远端 CI 尚未完成，不以局部结果标记任务完成 |
| 负责人 | 主 Agent |
| 模型分工 | 编码实际使用 GPT-5.6 Sol / `gpt-5.6-sol` / `xhigh`；审核使用 GPT-6 Astra / `gpt-6-astra` / `xhigh`，首切片通过最终限定复核、允许提交并继续后续拓扑实验；无全任务 Approve |
| 准备日期、环境 | 2026-09-12；实测 macOS 15.7.2（24G325）、arm64、Apple Git 2.39.5（Apple Git-154） |
| 初始源码基线 | 已合并并通过 CI 的 `main`：`2e29f88a1aae2ee53ce52dbc38a7c0765b58cb15` |
| P0-02 合入基线 | `22d60f64f27959082913572cd80aa8b3f89ce590`；合入 `main` 的 P0-02 收口提交 `ea9dd33ed1d4e1ab8a5b4af4921d0ef077a33517`，该次只合并两个收口文档 |
| 本轮恢复所用基线 | `f57d15d`；已合入并推送普通目录职责调整和克隆锁调研/排期；最近已提交的实验代码仍为 `5eca4077291ede2365d0ed917df9696aa02736c8`，后续拓扑/Base 候选仍在未提交工作区中，不以文档提交代表新候选验收 |
| 工作区、分支 | `/Volumes/data/code/thinworkspace-p0-03`；`task/p0-03-git-base`；标准 Git linked worktree，不声明 CoW |
| 前置依赖 | P0-01 已合并；属性兼容政策仍未决，但不阻塞下述不作产品接纳/拒绝决定的类型判别实验 |
| 主要写入区域 | `experiments/p0/git-base/` 及直接配套实验测试；各增量按下文准入，不提前实现 P1 产品接口 |
| 共享文件协调 | Cargo/lockfile、CI、质量配置及文档索引由主 Agent 集成；不与 P0-02 同时修改同一工作区的共享文件 |

权威输入为[Phase 1 详细设计](../../project/design/ThinWorkspace_Phase1单机CLI详细设计_v1.0.md)的 Repository/Git 与 Base 章节、[技术栈](../../project/reference/技术栈.md)的 Git/编码/测试章节、[用户手册](../../project/reference/ThinWorkspace_Phase1用户操作手册_v1.0.md)的仓库兼容和 checkout 可见契约，以及[开发规范](../process/开发规范.md)、[任务流程](../process/任务流程.md)。任务依赖与小阶段归[实施计划](../planning/ThinWorkspace_Phase1实施计划_v1.0.md)管理。

## 尚待确认的准入边界

只读准备已发现：目标 Git 的 `check-attr` 文本输出不能完整区分属性的布尔状态与同名字符串，例如 `-filter` 与 `filter=unset` 都可能输出 `unset`；NUL 分隔解决字段边界，不解决类型歧义。此前已向维护者提问，尚未收到答复：是否接受连显式关闭声明也拒绝的保守兼容限制，或继续验证保留关闭声明兼容性的精确方案。

这不是已接受的产品决策。确认前不得把“属性名出现即拒绝”写成用户契约或实现默认行为，也不得把未知结果当作安全放行；标准 `text/eol` 的现有支持承诺不变。最终方案必须在目标 Apple Git 上取得真实样例证据，再由规定模型审核。此边界不阻塞 P0-02。

### 只读核查后的最小验证候选（2026-09-12）

GPT-5.6 Sol / `xhigh` 查阅上游 Git v2.39.5 源码，主 Agent 独立核对相关实现，发现有不解析 `.gitattributes` 的候选路径：Git 的 attr pathspec 在匹配时保留内部属性类型，`:(attr:-filter)` 与 `:(attr:filter=unset)` 分别匹配布尔 Unset 和普通字符串值，不依赖 `check-attr` 的显示文本。[源码依据](https://github.com/git/git/blob/v2.39.5/pathspec.c#L725-L755)

只读准备时提出的候选是在一个合成仓库的固定 tree、临时 index 和受控空 worktree 上验证 `git ls-files --cached -z` 配合上述两个固定 pathspec 的结果集合，覆盖嵌套属性、宏和保留词碰撞。命令不执行 checkout 或转换；外部 filter 的受控 canary 必须证明未被调用，fixture 的文件内容与两个 index 身份须保持不变。**此处保存候选依据，不是已执行结果或产品实现；后续准入见下节。**

最关键的未验证前提是属性来源：`ls-files --cached` 控制文件枚举，不会自动将属性读取方向设为 INDEX；上游默认 CHECKIN 会先找工作树 `.gitattributes`，缺失时才回退 index。因此实验必须明确隔离 system/global 属性与配置、保持独立空 worktree、确认无 `info/attributes` 干扰，并用互相冲突的受控数据证明读取的是指定临时 index；不能把空工作树外观当成永久保证。[源码依据](https://github.com/git/git/blob/v2.39.5/attr.c#L797-L819)

此处最初将 `working-tree-encoding` 的布尔 Set/Unset 均推断为直接报错，后续真实对照已证伪 Unset 部分，主 Agent 在“内建转换假设纠正”节记录了原失败证据与原因。正确区别是：布尔 Set 报错，而 Unset 的内部字符串以 NUL 开头，先被空字符串判断当作不转换；不能只看到后面的 true/false 报错分支就推断两者都会到达。`ident` 仅真正布尔 Set 展开，裸 `ident` 与字符串 `ident=set` 的显示文本相同但转换行为不同。首个查询切片本身仍不证明实际 checkout，也不据此决定支持声明。[属性内部值](https://github.com/git/git/blob/v2.39.5/attr.c#L17-L18)、[转换判断顺序](https://github.com/git/git/blob/v2.39.5/convert.c#L1160-L1221)

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

## 执行记录（按时间追加）

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

首切片已提交为 `5eca4077291ede2365d0ed917df9696aa02736c8` 并推送到现有任务分支；完整纳入四个 crate 文件、新 harness、质量配置和现有过程记录，没有纳入 corpus、scratch 或现场数据。审核后再次执行通用门禁和 release Git crate，仍为退出 0、普通测试 121 passed/2 ignored、release 8 unit + 4 integration passed。以下继续原 P0-03 范围，不把本提交当作任务完成。

### 托管导入与双工作区实验准入

主 Agent 根据已核验的权威 Git 设计、Sol 的只读准备和 Astra 的限定准入意见，领取下一组实验；阶段仍为 P0.b、风险 R4、任务仍为 P0-03。主要写入区域为同一实验 crate 的 `managed_topology` 模块及同名集成测试，既有属性查询行为保持不变。不新增产品 Port、通用 Git runner、SQLite 或恢复框架；如需不覆盖发布，优先使用已选定 `rustix` 的窄安全调用，新增直接依赖须说明用途并完成既有供应链门禁。

1. 只使用本次独占创建、立即报告的 mode 0700 合成根。source 自证包含 heads/tags、私有 refs，以及彼此可区分的 staged、unstaged、untracked 对照；调用前保存 refs、HEAD、native index、配置、工作树内容和 worktree 登记的独立语义快照，不把时间戳变化当作内容变化。
2. 仅按权威 refspec 从受控 file transport 导入 staging bare repository；核对完整 ref 名与原始 OID、无 alternates/promisor 或继承的对象目录覆盖，以及允许 refs 可达对象的本地完整性。允许 ref 集合须精确一致；工作树内容只来自已提交的固定 tree，不复制 source 当前工作树或 index。额外 ODB 对象可如实观察，不新增“Git 协议绝不传输多余对象”的保密承诺。
3. 验证 staging 后才同卷发布，既存目标不得被覆盖；不能用“先 exists 再普通 rename”宣称原子不覆盖。负例必须在 fetch 已成功且 staging 有可核对内容后，因真实的发布前解析/验证失败而不生成 final，同时保留 staging 身份和内容；另验证既存目标及其内容保持不变。保留失败状态，不添加自动 prune/GC/递归清理。
4. 固定完整、小写 SHA-1 commit/tree；从合法固定 WorkspaceId 样例推导两个不同托管分支，创建两个 `--no-checkout` linked worktree。native 操作不得带属性实验的 `GIT_INDEX_FILE` 等覆盖；核对共同的已发布 common-dir、独立 git-dir/index 路径与文件身份、符号 HEAD 和独立 index 的 tree。
5. 用对应固定提交的中性、无 attributes 的独立预期字节建立初始工作树，按既定流程初始化 index 并取得两个 clean 状态。分别在 A stage 后及 commit 后核对：B 的 index/HEAD/tree/内容和 clean 状态、固定来源 ref、固定 commit/tree、source 快照均不变。此处不冒称实际 text/eol checkout、Base manifest、BaseId 或 CoW 验证。
6. Git 采用 argv、固定 cwd、隔离配置/环境、有限等待及原始状态/两流证据；fetch/worktree 的正常 stderr 不能套用属性查询的零 stderr 门槛，也不能丢弃。只判断有实证的终止/回收范围，不用成功 happy path 宣称进程树故障恢复已验证；若现有最小方案无法安全满足边界，先报告，不自行扩建 Supervisor。
7. 先自证 fixture，再以可编译的未实现错误取得真实行为 RED；入口固定为 `cargo test -p thinws-p0-git-base --test managed_topology -- --nocapture`，release 同名入口也必须覆盖正例和失败例。之后执行通用、变异、受影响 fuzz 与规定模型审核；本组不能代替后续 fetch 更新原子性/tag 改写、解析兼容、真实 checkout、BaseKey/manifest/发布复用、完整故障及 ADR 验收。

### 托管拓扑首个 RED（2026-09-12）

GPT-5.6 Sol / `xhigh` 的首次运行因 fixture 在快照中执行 `git write-tree`、改变 native index 的缓存扩展而退出 101；失败发生在未实现行为之前，**不计为 RED**。对应现场 `target/p0-git-base-tests/managed-topology-87018-1789222979191189000-0` 保留。修正仅将该写入移至 fixture 构造，快照使用只读 stage 记录并在查询后捕获 index/config 的身份和字节；所有查询先检查 Git status，缺失目标以 no-follow 元数据查询的 ENOENT 证明。

Sol 随后执行上述固定测试入口，主 Agent 对冻结候选独立重跑（session `40181`）：均编译成功并退出 **101**，唯一失败为 `managed_topology.rs:49` 的 `NotImplemented`。失败前已通过来源精确 refs/OID、staged/unstaged/untracked 区分、native index 条目及 worktree 登记自证；stub 前后来源快照相等，staging/final/两个工作区均不存在。主 Agent 重跑前后源码哈希一致，`git diff --check` 退出 0。

环境为 Apple Git `2.39.5 (Apple Git-154)`。main commit 为 `679a7eb57143f99a534d0e9c253d602223128b70`，main tree 为 `9c83c16c50b14e7595a3aa03b5df99e587679edc`，topic 为 `72845d0b3dc4c472093cda6fc6a6ad79b97c0436`，private 为 `aac6401b04bfedcc2245f51cf571054665259665`，staged tree 为 `5651b7b2437fa22c02ee6b63cd38a3cfdde9a989`。Sol 与主 Agent 的 mode 0700 独占现场分别是本工作区 `target/p0-git-base-tests/managed-topology-52447-1789223418063756000-0` 与 `…/managed-topology-56370-1789223443738478000-0`，均继续保留。

冻结 RED 的 SHA-256：实验模块 `19d1975eb0bedde0c35e2ef9b2d766f3120635ff9ad1d97470d0b55efb1fa204`，集成测试 `48b0356df3ffc126e8b4f561d8d54d9d6d902cd0fab3ec0c86abdc01d026e7df`，库入口 `a0b8df9a57706489632ac9fc96589b062135e6a4b08d5c8c2628bd2685d3aa63`；crate manifest 尚未变化。主 Agent 据此授权在上节七条范围内进入 GREEN；这只证明 RED 有效，未证明托管拓扑成功、未通过提交门禁或规定模型审核，未提交红色候选，未宣布任务/阶段放行。

GREEN 的直接依赖接入仅使用技术栈已选定的 `rustix` `fs` 能力，用于同卷发布的原子不覆盖；普通 `std::fs::rename` 不满足该断言，未另加 FFI 或平台框架。主 Agent 执行完整 `cargo metadata --offline --format-version 1` 退出 0，lockfile 仅为本实验包增加 `rustix` 依赖边，第三方包与版本没有变化（仍为 `rustix 1.1.4`）。该时点 manifest/lockfile SHA-256 分别为 `8926abf8dc768a0cfde333f5a746c372bb31ece2c886718113e13220bed06de8`、`2dbf4510f5a3abb4c14f1933424c509588f9d3f3c84e3bc231c3c8682fde9e6c`。

依赖接入检查 `cargo deny --locked check` 与 `cargo audit --deny warnings --file Cargo.lock` 均退出 0（分别为 session `79547`、`70259`，audit 扫描 36 个依赖）；deny 仍报告既有未命中 fuzz 包许可例外及 NCSA allowance 警告。此检查仅针对依赖接入，不替代后续完整代码、真实 Git、双 workspace、变异及 fuzz 门禁。

### BaseKey 纯编码实验准入与候选决策（2026-09-12）

本实验仍属 P0-03/P0.b、R4；与托管拓扑并行，但只写独立 `base_key` 实验源与纯输入 fuzz，公共 manifest/lockfile/入口由主 Agent 单独集成。背景是验证既定五字段能否产生确定、无歧义的 Base 身份，不生成真实 checkout policy、文件系统语义或 manifest。编码由 GPT-5.6 Sol / `xhigh` 实施；GPT-6 Astra / `xhigh` 已只读核验权威约束与下述候选，未发现需先改变公开契约的冲突，允许主 Agent 补齐本节后准入；不是代码 Approve、正式 ADR 或阶段放行。

候选采用 `frame(x) = u32_be(len(x)) || x`；先 frame ASCII magic `thinws.base-key`，随后写入 `u32_be(1)`，再按详细设计声明顺序 frame 五字段。RepositoryId 使用完整规范文本，算法固定 `sha1`，tree 使用 40 位小写十六进制文本，两个合成 digest 分别使用 64 位小写十六进制文本；候选 BaseId 为 `base_` 加普通、无密钥 `blake3::hash` 的 64 位小写十六进制摘要。不加字段名、计数、commit 字段或没有集合可排的额外排序。magic/version 的精确字节、文本表示及无密钥模式在此仅作待验证候选，最终由本任务 ADR 冻结。

备选是 UUID/OID/digest 使用原始字节；本候选选择可人工核对的规范文本，代价是 key 字节较多。P0 尚无已发布持久化 key，无现存数据迁移；未来变更编码必须升版本并按正式 ADR 处理，不能让既有 BaseId 无声漂移。验证计划如下：

1. RepositoryId 仅取预先核对的两个合法 fixture：`repo_01890f4c-7c6d-7a21-8e31-123456789abc` 与仅末尾不同的 `repo_01890f4c-7c6d-7a21-8e31-123456789abd`。实验入口不得声称实现完整 UUIDv7 解析，也不另造部分 parser；未来产品仍从既定 `uuid` 强类型接入。其余实际接收的文本须精确检查算法、长度与小写 hex，不 trim、归一化或截断；若 framing helper 接收不定长 slice，长度转换须检查。
2. 主 Agent 在被测编码器存在前，用独立 JS 计算完整预期字节，再将该 literal 交给仅哈希 stdin 的临时 helper，未调用任何 BaseKey encoder。tree 输入 `0123456789abcdef0123456789abcdef01234567`，policy digest 为字节 `00` 至 `1f` 的小写 hex，filesystem digest 为 `ff` 至 `e0` 的小写 hex：两个 ID 的 key 均为 **256 字节**，BaseId 分别为 `base_357dccc8017e408a5b944396023049c3d9ac2d6ce72d0ebace58a6ba58773b0f` 和 `base_3973aa35d456e3ac18986dbc90c13b82a21c618af89b2cf0a4bc3c80c59ad69a`。完整 bytes literal 进入实验测试，不在本文再复制；测试须同时核对完整 bytes、长度和完整 BaseId。Astra 确认这种独立于编码器的预期构造足够，不要求另建 BLAKE3 实现；其本轮未重跑摘要计算。
3. 重复输入必须得到相同结果；逐项仅改变 RepositoryId、tree、policy、filesystem digest，以及交换两个不同 digest，均须改变结果。`sha256` 属拒绝用例，不能借敏感性测试扩大 Phase 1 范围；不把不同 commit 添加为第六字段。无实际读取用例，不新增 decoder/schema。
4. 先以可编译的未实现行为取得真实 RED，主 Agent 核验后才 GREEN。候选入口为 `cargo test -p thinws-p0-git-base base_key -- --nocapture`，release 同名筛选必须覆盖完整正反断言；提交前仍运行无筛选的通用门禁、相关真实 Git 测试和全 crate 变异。编码 fuzz 只消费内存数据及上述固定 fixture 选择，不接路径、不执行 Git/文件系统操作；固定工具链 smoke 至少记录 60 秒预算及真实结果。代码与证据仍需 Astra 正式复核。

一次性计算 helper 保留在 `/tmp/thinws-p0-03-base-vector.aBob1o/`，源码 SHA-256 为 `acbf98e0fbf4184c98ed4ecd6b0fc10d4ddc8815d9578decdd38307e1f50903d`；编译和两次候选哈希命令均退出 0。它调用已编译的 `blake3 1.8.7`，不是第二套独立哈希实现，也不是产品代码。正式实验的依赖接入、RED/GREEN、fuzz、变异和提交门禁尚未执行，不能用该计算代替。

### BaseKey 首个行为 RED

Sol 只新增 `src/base_key.rs`，主 Agent 以普通私有模块接入；未加 `cfg(test)` 模块门控、warning 排除或产品接口。主 Agent 执行上述固定测试入口，编译成功、退出 **101**：实际运行 **1 项，1 failed、0 ignored、11 filtered**。两份完整 golden literal 的解码及 256 字节断言均先通过，随后唯一失败在 `base_key.rs:77`，原因为 `NotImplemented`。运行前后模块 SHA-256 均为 `1b0e34c3ba28d4bba6b9233a3e317d94ae47f86eba4d2e4d881d7b2edd496866`，库入口为 `e9f62387d56c51a3ec782beae818a1c982accaf4d5cb4ff8a43498aaec62b0bf`。

该时点正常 library 构建报告 5 项 BaseKey `dead_code` warning，反映它仍只有实验测试调用、尚未与真实 Base 组合实验接通；不能据此宣称完整 Clippy 或提交门禁通过，也不能用排除或预建平行机制隐藏。主 Agent 已授权按上节范围 GREEN，并接入技术栈已选定的 `blake3 = "1"`；完整离线 metadata 退出 0，仍锁定 `1.8.7`，根 lockfile 仅增加本实验包的直接依赖边，没有新增第三方包或升级版本。后续完整依赖与代码门禁必须覆盖这一变化，尚未提交该进行中候选。

### 托管拓扑与 BaseKey 组合验证（2026-09-12）

GREEN 中真实 Git 曾将完整 `refs/heads/thinws/<id>` worktree 参数作为 detached HEAD；Sol 改用本地短分支名注册，仍按完整 ref 核对最终符号 HEAD。失败现场 `target/p0-git-base-tests/managed-topology-82853-1789224289597400000-2` 保留。主 Agent 另补充既存空目录不覆盖断言，并要求用固定提交的独立预期字节先建立中性工作树，再初始化 native index，避免用 Git checkout 自证初始内容。

首个组合候选由主 Agent 独立执行 Debug/Release 各 **28 passed**。随后 GPT-6 Astra / `xhigh` 发现两处局部遗漏：base ref 的命名空间前缀检查仍接受 revision expression；fixture helper 丢弃成功调用的 stderr。主 Agent 在保留的真实仓库观察到 `topic~1^{commit}` 能解析为预期 main，而 `check-ref-format` 拒绝该表达式、接受合法但缺失的 ref，授权 Sol 最小修正。

Sol 的新行为回归在旧实现上编译成功、退出 101：`main^0` 被接受并完整发布，故 `expect_err` 失败；现场 `target/p0-git-base-tests/managed-topology-7383-1789225767603499000-0` 保留。GREEN 在写 staging 前通过既有受控 runner 执行完整 ref 的 `check-ref-format`，不归一化或自行解析；两个表达式均提前拒绝且源快照不变，合法 missing ref 仍在成功 fetch 后失败并保留 staging。helper 仅补记录正常 stderr，不因此拒绝命令。

本节核验快照：`managed_topology.rs` SHA-256 为 `72fdb8d259f78c9fe130e1db96ef77dfadec869f1170bfa2f61097e2f8e76fee`，同名 IT 为 `b4ff3219225ff0a3a995dbabd95a0118c0526b027576173f8ea970c20e76e46b`；`base_key.rs` 为 `17f0fad3e74d9b51cc02851e759bc6da349b5a9f3468e785d5a9b85473b0c6e2`。主 Agent 执行结果：

| 命令/范围 | 实测结果 |
|---|---|
| `cargo test --locked -p thinws-p0-git-base --all-targets` | 退出 0；19 unit＋4 attribute IT＋7 topology IT，共 30 passed、0 ignored |
| 上述命令增加 `--release` | 退出 0；相同 30 项通过 |
| `cargo test --locked --workspace --all-targets` | 退出 0；139 passed、2 ignored；两个跨卷用例因本轮未提供第二挂载卷而未执行 |
| 根与 fuzz workspace 的 fmt、`git diff --check` | 退出 0 |
| 根 workspace Clippy，`-D warnings` | **退出 101**；BaseKey 的 8 项 dead-code，真实 Base 调用尚未接通，不添加排除，不可提交或放行 |
| fuzz workspace Clippy，`-D warnings` | 退出 0 |
| 两个 workspace 的 `cargo deny --locked ... check`、两个 lockfile 的 `cargo audit --deny warnings --file ...` | 全部退出 0；audit 分别检查 36/29 个依赖；deny 仍有共用配置中未命中许可例外/allowance 警告 |

Astra 再次只读复核上述精确快照，确认两处源码修正、未见新增实质问题；不是完整任务 Approve。其指出 stderr 测试只检查原始 observation，并不能自动检出日志语句被删除。主 Agent 因此独立执行该单测的 `--nocapture`（session `64525`，退出 0），实际看到 `operation=observe successful Git stderr`、完整 `status=exit status: 0` 与原始字节；字节对应 `Preparing worktree (detached HEAD 679a7eb)\n`。这仅是实际日志观察，不冒称自动日志断言。现场 `target/p0-git-base-tests/managed-topology-64260-1789226116032420000-0` 保留。

全部事实仍限于固定私有合成拓扑，不证明任意路径、祖先替换竞态或整个后代进程树严格终止。BaseKey 的两份完整 golden、确定性、字段敏感性与拒绝矩阵已通过，但真实 checkout、Base 发布/复用、后续故障实验、全量变异、完整审核及阶段门禁尚未完成。

### BaseKey 编码 fuzz smoke

Sol 编写的 `base_key_candidate.rs` SHA-256 为 `95d00ee4eb5236633b1fe13ebc374726a355ab0074d31d8b0317bdea20dae5e2`。它只访问内存，选择两个既定 RepositoryId fixture，覆盖规范/非规范算法、hex 长度和字符、输入决定的错误位置及有界原始 UTF-8 文本；独立错误分类与固定字节偏移核对不依赖被测 framing helper，不调用 Git 或文件系统。主 Agent 集成新 bin `thinws_base_key_candidate` 及直接 `blake3` 依赖；完整离线 metadata 退出 0，fuzz lock 仅加入根 workspace 已有版本的四个依赖包，没有升级原有包。集成后的 manifest/lockfile SHA-256 分别为 `fc7afe2cf9b25d1ad2b9bb151b59118404f45c93ce6d1871d9bd8b341a2eddf9`、`cdab36a29d66f20a96da26d95af6a41db9787f01a19dc00aa7be0ac416f082b0`，供应链结果见上节。

主 Agent 核验 `cargo-fuzz 0.12.0` 与固定 `nightly-2026-08-14`，执行 `cargo +nightly-2026-08-14 fuzz build thinws_base_key_candidate` 退出 0。随后在新建、保留的空 corpus `/tmp/thinws-p0-03-base-key-corpus.AsazkG` 上执行：

```bash
cargo +nightly-2026-08-14 fuzz run thinws_base_key_candidate /tmp/thinws-p0-03-base-key-corpus.AsazkG -- -max_total_time=60 -timeout=5 -max_len=4096 -print_final_stats=1
```

session `45956` 退出 0，实际 **5,267,616 runs / 61 秒**，无 crash/timeout；libFuzzer 最终报告 84 个有效样例/5,298 字节，磁盘实际保留 83 个文件；记录 `new_units_added=217`（包含演进过程，不等于最终样例数），峰值 RSS 478 MB。corpus 和 artifact 目录均未清理、未提交。CI 已接入同一目标和短预算，但当前未提交候选的远端 CI 尚未运行；本结果不是长预算、完整任务审核或阶段放行。

Astra / `xhigh` 已只读核验这组冻结源码、两份依赖锁与 CI 增量，未发现实质问题；独立核对四个新增包的版本/checksum 与根 lock 一致。此 harness 不证明完整 UUID 解析、真实 policy/filesystem digest 的语义或超大字段边界，复用 BLAKE3 核对摘要也不等于独立验证密码算法；无需因此增建第二套实现。上述为限定增量意见，根 Clippy、全量变异和完整任务门禁仍待完成。

### Git 纯输入验证重构与 smoke

为直接测试真实解析源码，Sol 将既有 ref 输出、SHA-1 OID 和 WorkspaceId 三个纯函数原义移到私有 `topology_validation` 模块；主 Agent 接入模块和 `thinws_git_topology_validation` fuzz bin，不改产品接口、不新增依赖。错误文本、判断顺序和既有接受范围保持不变。初版测试 oracle 错误地剥除了末尾孤立 CR，主 Agent 查出后仅修正 oracle；新增普通测试固定 LF/CRLF/无尾 LF 成功、孤立 CR 留在 OID 中并被拒绝的原有行为。这是测试基准修正，不是产品行为 RED。

冻结 SHA-256：纯模块 `7e30039426c13a24d9e9817f5cece163c08ceca7f397ae72d0954958135ec982`，托管模块 `d4bb2afa5203b4f44ca9061c18f2c676172a22f82a3db7d95c408f8fe0832018`，新 harness `42210ed9997555182eb7e916c3c00c086a119857c1509b9579be5cf1e7e77e62`；托管 IT 未变。harness 消费不超过 4096 字节的原始输入和有界结构化候选，独立字节 oracle 核对完整 ref map、错误优先级、OID 与 WorkspaceId，不执行 Git/文件系统操作，也不冒称完整 Git ref-name 校验。

主 Agent 对该快照独立执行 Git crate 全部 Debug/Release 测试（session `50030`），分别 **31 passed、0 ignored**；全 workspace 普通测试（session `43886`）**140 passed、2 ignored**，两项跨卷用例仍未执行。根/fuzz fmt、fuzz Clippy 和 diff 检查退出 0；根 Clippy（session `14659`）仍因 BaseKey 未接入产生的 8 项 dead-code 退出 101。Astra / `xhigh` 限定复核确认原函数无行为漂移、oracle 边界正确、未扩公开 API；其未重跑测试，不是完整任务 Approve。

固定 nightly 的 `fuzz build thinws_git_topology_validation` 退出 0。主 Agent 在新建 corpus `/tmp/thinws-p0-03-topology-corpus.5Yvjp6` 上运行与上节相同的 60 秒/单次 5 秒/4096 字节预算，实际命令目标为 `thinws_git_topology_validation`。session `13432` 退出 0，**3,493,697 runs / 61 秒**，无 crash/timeout；libFuzzer 报告 163 个有效样例、磁盘保留 162 个文件，artifact 目录无文件，峰值 RSS 629 MB。所有现场继续保留，CI 已加入同目标 smoke，远端当前候选尚未验证。

当前结论：以上两个新增 fuzz 目标和托管拓扑切片已有真实短预算及限定审核证据；P0-03 仍 In Progress，未提交或合并本组进行中修改，公开契约未变化。下一步是原任务范围内的真实 Base 构建/发布/复用及其余 Git 实验，不用 warning 排除或测试专用假调用代替接入；完整变异、阶段长预算、P0-04 和阶段放行仍未完成。

### 真实 checkout 的独立预实验（2026-09-12）

主 Agent 在 mode 0700 新根 `target/p0-git-base-tests/checkout-preflight.EwRPPh` 内建立隔离 bare repository、树外临时 index 和空 content 目录。固定 tree 为 `1c53447bb3a3b543ccb1d93ca533fbaec902c3a7`；精确 `.gitattributes` 只包含 `lf.txt text eol=lf`、`crlf.txt text eol=crlf`、`binary.bin -text`，不涉及待确认的转换属性策略。两个文本文件共享原始 LF blob `422c2b7ab3b3c668038da977e4e93a5fc623169c`，另有包含 NUL、CRLF 和 `0xff` 的固定二进制 blob。

在清空继承环境、隔离 system/global 配置与 attributes、固定 checkout 配置并禁用外部程序入口后，真实 Apple Git `2.39.5 (Apple Git-154)` 执行 `read-tree` 和 `checkout-index --all`（无 `--force`、`--prefix`、`-u`）。首次 checkout 退出 0；对独立 literal 的三次 `cmp` 均退出 0，分别得到 LF、CRLF 和原样二进制内容。仅围绕 checkout 的 index SHA-256 前后均为 `08a0d3016b11b49109659d157c2cdf1db3530d566fa378786d0d21ced927a34d`。重复 checkout 退出 1，逐项报告文件已存在；随后内容比较和 index 哈希仍相同，全部现场保留。

该预实验只支持固定输入下的标准转换、不覆盖行为及 checkout 步骤不更新 index；不证明一般属性读取方向，不是 Base 实现的 RED，也不证明 manifest、Receipt、发布或复用。命令选型依据为 [Git checkout-index 文档](https://raw.githubusercontent.com/git/git/v2.39.5/Documentation/git-checkout-index.txt)；[Git entry.c](https://raw.githubusercontent.com/git/git/v2.39.5/entry.c) 明确允许输出 prefix 中的 symlink，因此本实验不使用 prefix 作为路径安全保证。后续 `refresh/write-tree` 的合法 index 修改应另行核对语义，不能套用本段字节不变断言。

主 Agent 同时在该私有根测得有限文件名 witness：`CaseWitness` 与 `casewitness` 查询到同一 device/inode（`16777240/34597264`）；NFC `ä`（U+00E4）与 NFD `a`＋U+0308 查询到同一对象（inode `34597265`）。另一个新建项经 no-follow 类型查询为 Symbolic Link，`readlink` 原样返回 `CaseWitness`。这些对象继续保留，只证明上述两个名字对与本次 link 行为，不代表完整 Unicode/所有挂载环境保证。将来实验 semantics descriptor 应编码稳定的观察语义及 witness/兼容版本；临时根、inode、时间和 Volume UUID 属布局/身份重验证据，不作为可复用语义摘要的易变输入。

### 固定 tree checkout 与 manifest 切片准入

主 Agent 在既有 P0-03/P0.b/R4 范围内领取下一步：先验证真实 checkout、有限 manifest 和真实策略/文件系统 witness，再把这些结果接入 BaseKey 与磁盘发布/复用。后一步必须完成后才能给出完整 Base 身份结论；本切片不发布 Base、不声明 Ready，不冻结 P1 Receipt schema 或尚待确认的转换兼容政策。

主要写入区域是实验 crate 的 `base_materialization` 模块及其真实测试。只复用现有 Git 环境构造和有界收集；如需第二实际调用者，允许将原环境构造 helper 提升为 crate-private，不新增 runner 框架。主 Agent 独占模块入口、manifest/lockfile 和文档；实现由 GPT-5.6 Sol / `xhigh` 负责，代码审核必须使用 GPT-6 Astra / `xhigh`。验收断言如下：

1. 每例只使用独占创建并立即报告的 mode 0700 合成根，bare repository、固定 commit/tree、树外临时 index、空 content staging 都在可核验的受控布局中；不接入用户仓库，不联网、不删除现场。实际 source refs/config、其他 Base/Workspace 对照在整个调用前后不变。
2. 固定 tree 的完整清单包含 `.gitattributes`、LF/CRLF 文本、含 NUL/非 UTF-8 字节的二进制、可执行普通文件、符号链接及嵌套目录/文件；精确 attributes 只使用既定支持的 `text/eol/-text`。测试先自证固定 commit/tree、非空路径/模式全集和 raw blob 内容，独立 expected 则描述实际 checkout bytes/link text，不能从被测 manifest 反推预期。
3. `read-tree` 使用独立 index；`checkout-index --all` 在新空 content root 中运行且不带 force/prefix/index 更新选项。仅此 checkout 步骤要求临时 index 身份/字节不变；后续 refresh/write-tree 允许更新其缓存，但完整 index tree 与真实固定 commit 的比较基准不变，最后取得真实 Git clean。不得在 unborn HEAD 下忽略所有新增项而称 clean。
4. scanner 仅服务本次有限 fixture：使用持有的目录 FD、no-follow 类型/身份检查、精确完整路径集和实际读取；未知项、特殊类型或预期不符均失败并保留 content staging。manifest 中普通文件的长度/摘要来自实际 checkout 字节，symlink 的长度/摘要来自原始 link text，不解引用；目录只记录路径与 `040000`，不纳入易变的 `st_size`。普通文件的 `100644/100755` 按 owner-executable bit（`0100`）分类，不能按“任一 execute bit”分类；不声称完整 Unix 权限、ACL 或 xattr 已验证。[Git canonical mode 依据](https://raw.githubusercontent.com/git/git/v2.39.5/cache.h)
5. policy descriptor 绑定本次实际 Git 版本、实际生效的显式 checkout 配置、固定 tree 的精确 attributes 及实验兼容版本；继续隔离继承环境和 global/system/info attributes，不能只哈希未经核对的配置常量。filesystem descriptor 绑定实际固定大小写/NFC-NFD pair 和 symlink witness 的明确结果及版本；witness 必须证明确属 content 所在目标文件系统，返回成功前重新确认对应身份。两者用稳定编码产生真实摘要，不用 golden 的合成 digest 代替；临时路径、inode、时间和 Volume UUID 不进入稳定语义摘要。观察失败或含糊时安全停止，不从 OS 名称猜测；有限 witness 不能升级为完整 Unicode 模型。APFS Volume/路径证据可复用已选 P0-01 Probe，新增直接依赖由主 Agent 核验后接入。
6. Git 调用保留原始 status/stdout/stderr，正常诊断不静默丢弃；只说明实际验证的进程终止范围。失败不得给出已验证 manifest 或 Base 可复用结论。至少覆盖未知目标项/manifest 内容或模式不符，并证明无外部对象被覆盖。
7. 首先运行 `cargo test --locked -p thinws-p0-git-base --lib base_materialization::tests::fixed_tree_checkout_matches_independent_manifest -- --nocapture`，在 fixture 自证后因可编译 `NotImplemented` 获得行为 RED；主 Agent 核验后才 GREEN。之后执行对应 debug/release、通用及专项门禁和规定模型审核。进入真实发布/复用前仍需明确其磁盘协议、失败断言和入口，不从本切片自动推导完成。

本切片尚未接入最终真实 Base 实验入口时，未使用代码警告如实保留，不能通过排除或假调用通过 Clippy。最终将采用与托管拓扑实验同层次的窄“执行固定 Base 实验”入口，真实消费 checkout→BaseKey→磁盘校验→发布/复用；内部 RepositoryId fixture 和编码器不为测试公开。当前仅准入本节 checkout 切片，不预建该后续入口或发布框架。

GPT-6 Astra / `xhigh` 已只读核验本切片准入，未发现阻止 fixture 自证后取得结构化 `NotImplemented` RED 的冲突；主 Agent 核验后将上述 manifest、实际 policy 来源和 witness 对应关系补清于原断言。该审核没有执行实验，不是完整 P0-03 Approve 或放行。

### 固定 checkout 首个行为 RED

GPT-5.6 Sol / `xhigh` 编写了固定 fixture 与可编译的结构化未实现入口。主 Agent 通读全部源文件后接入私有模块，作者与主 Agent 分别执行准入第 7 条的单测试命令；两次均编译成功，完成非空完整 tree、重复 commit/tree、raw blob 和调用前后现场不变自证，唯一失败发生在末尾要求 checkout 成功的断言，错误为 `NotImplemented { detail: "fixed tree checkout and manifest are not implemented" }`。主 Agent 运行结果为 0 passed、1 failed、20 filtered，退出 101；不是 Git、环境或编译失败，不作为门禁通过。

冻结 RED 源 `base_materialization.rs` SHA-256 为 `8684ec11fc6f98e2cc63c60101159cb667c2a1b1ac01e7b2f092834ca368787b`，接入时 `lib.rs` 为 `594f6218a0e54cccc1322ecedce7acdebae5de1d21aa1136fe1cd922489c012d`。作者和主 Agent 的完整现场分别保留于 `target/p0-git-base-tests/base-checkout-8152-1789228397471770000-0`、`target/p0-git-base-tests/base-checkout-13796-1789228434167522000-0`，未覆盖或清理。

主 Agent 核验 RED 后允许在本节既定边界内进入 GREEN，并接入本仓库已有 `thinws-p0-probe` 路径依赖，以复用真实 APFS Volume/路径身份查询，避免再写一套 FFI。依赖不引入新的第三方版本；原 Git 环境构造仅提升到 crate-private，供第二个实际调用者使用，不改变原拓扑命令行为。完整 A、后续 B、P0-03 及阶段门禁仍未完成。

依赖/可见性集成后，主 Agent 执行 `cargo test --locked -p thinws-p0-git-base --test managed_topology` 得到 7 passed、0 failed、退出 0，原拓扑回归未受影响；普通 library 构建仍报告 12 个未使用代码警告，不能据此称 Clippy 通过。`cargo deny --offline check` 退出 0，保留两项既有未匹配许可配置 warning；`cargo audit` 扫描 36 个依赖后退出 0（首次误加不支持的 `--locked` 参数退出 2，未完成审计，随后以有效命令重跑）。此时 `cargo fmt --all -- --check` 与 `git diff --check` 均退出 0。上述只覆盖集成时的 RED 候选和既有拓扑，不覆盖尚在编写的 GREEN，不替代完整 workspace、release、变异、fuzz 或阶段门禁。

主 Agent 对独立 RED 现场的 `content-staging` 实际运行 `thinws-p0-probe path`：现有目录、APFS、Volume UUID `1A42C888-32E3-489C-9BFA-67FD640A94E8`、fsid `[16777240,26]`，held-directory 身份为 device/inode `16777240/34597881`；退出 0。该查询验证了当前目标目录上的既有 Probe API，CoW 仍明确为 `not_executed_by_probe`，不代替 A 自身的路径/witness 重验。

### 磁盘发布与复用的最小候选与准入

此候选继续服务 P0-03 的 Base 验收；依赖 A 的真实 checkout、manifest 和 policy/fs 结果，不是新的产品设计来源。GPT-6 Astra / `xhigh` 已只读审核这个有界方向，认为不需要通用 parser、恢复框架或产品 Port；主 Agent 核验后将必要断言归入本节。本节记录编码前的候选和准入决定，不作为实验已执行证据；实际 RED/GREEN 进展按后文记录，完整任务尚无 Approve。

1. 用真实 A 的 tree/policy/fs 摘要调用既有私有 BaseKey 编码。发布单元是同卷 staging 内的一个目录，仅含 `tree/` 和树外版本化 receipt；未知同级项拒绝。receipt 以规范编码绑定 magic/version、完整 BaseKey 与排序 manifest，不新增第二套身份模型或 JSON schema。
2. expected 由固定 tree、当前真实 policy/fs descriptor 和独立验证的预期 manifest 重新形成，不从待复用 tree 或读入 receipt 反推。磁盘 receipt 以 no-follow 打开的普通文件有界读取，必须检查完整长度并拒绝截断、尾随字节；逐字节不匹配可统一返回结构化错误，无需为区分版本错误另造 decoder。随后重新扫描磁盘 tree，与可信 expected 核对完整路径、Git 模式、实际字节和原始 link text；篡改 tree 与 receipt 不能互相自证成功。
3. receipt 使用 create-new 写入并读盘回验；发布前重新核对刚校验的整个 staging 单元及父目录身份，再以同卷 held-parent `NOREPLACE` 原子发布整个单元。若 A 的 content staging 尚在单元外，先在已核验的父目录 FD 下以不覆盖方式移入单元的 `tree/`，重新检查后才写 receipt；此移动也不得覆盖未知目标，失败现场保留。
4. 复用成功必须能观察到重新读盘验证，且既有 Base 的内容和身份未被重写；不能把目录存在、保存的内存 evidence 或一份自洽但不受信的 receipt 当作可复用证明。
5. 失败实验至少包括 receipt 缺失/截断/尾随字节/版本或 owner-key 不符、manifest 内容不符、空及非空发布目标已存在、回验成功后发布前停止。损坏默认安全停止并保留。显式隔离只对本实验已登记且重新匹配身份的对象进行；坏 receipt、可疑目录名或内容吻合本身不能授权移动未知对象。quarantine 使用唯一目标及 `NOREPLACE`；冲突保留双方，隔离成功而重建失败时保留旧对象和新失败现场，不删除。
6. 窄 public P0 实验入口须实际执行上述完整流程，内部 RepositoryId fixture/编码器保持私有；该入口不成为 P1 API。证据只支持固定 expected 下的完整性、发布和受控复用，不证明无 expected 的跨进程恢复、receipt 防伪或断电持久性。完整 operation/强杀恢复仍由 P0-04 验证。

主 Agent 已核对后续组合接口：现有 `managed_topology::create_workspace` 直接写固定 `committed.txt`，尚未消费 Base；P0-02 的 `materialize_once`/`execute_prepared` 已支持源 tree、目标目录及受保护 `.git` 身份。后续须实际复用这些入口，将已验证 Base 物化到两个 linked worktree，独立核对 `.git` 内容、Gitdir/native index、初始 clean 和修改隔离；不新增复制器，不把分别通过的 Base 与拓扑实验冒充已完成的组合验收。此处只有源码核查，尚未执行该组合或增加物化 crate 依赖。

2026-09-12，主 Agent 与 GPT-6 Astra / `xhigh` 再次核对现有候选和实际 A 接口，补齐首个 RED 所需决定；仍未开始 B 编码：

- 窄入口采用 `run_fixed_base_experiment(&FixedCheckoutRequest) -> Result<FixedBaseEvidence, Box<BaseMaterializationError>>`。结果只组合原 A evidence、BaseId、实际发布路径、单元/receipt 身份和重读 manifest；签名及 public 字段实际可达的既有类型才增加文档和 re-export，A 入口及执行/扫描/helper、BaseKey 编码器与内部 RepositoryId fixture 仍不公开。不得通过假调用或提前公开无关类型消除警告。
- 实验磁盘单元的固定子项为 `tree/` 与 `receipt`。receipt 编码依次为：`frame("thinws-p0-base-receipt")`、版本 `u32BE(1)`、`frame(完整 BaseKey 规范字节)`、条目数 `u32BE`，随后按 path 原始字节升序编码每条的 `frame(path)`、Git mode `u32BE`、`frame(length)`、`frame(digest)`。frame 使用 `u32BE` 字节长度；length 的 None 为空、Some 为 `u64BE` 八字节；digest 的 None 为空、Some 为 64 字节小写 ASCII hex。目录两项均 None，普通文件和 symlink 两项均 Some；这是 P0 实验格式，不冻结 P1 Receipt schema。
- 编码 golden 使用已有独立 256 字节 BaseKey literal 与独立 manifest，比对整份固定 receipt literal。真实运行的 expected 则使用固定 `expected_manifest()` 与 A 当次真实 tree/policy/fs，经既有 BaseKey 编码形成，不能以合成 golden digest 替代实测值，或只拿 A 返回的 manifest 自证。磁盘测试的独立 expected 不调用被测 receipt encoder。
- A 的 content 仍是 private root 的直接子项，不改变 A 的路径接受范围。A 成功后才以 held-parent `NOREPLACE` 移入新 staging 单元的 `tree/`。移动前核对旧 report/FD；移动后核对同一文件身份并取得新路径证据。A 的旧路径 report 是历史 evidence，不能改写，也不能在合法搬移后继续要求该旧路径 unchanged。
- 首个行为 RED 复用已有固定 Git fixture 与独立 manifest，自证后实际调用 A，再在未实现的 B 边界返回明确结构化 `NotImplemented`；测试在核对受保护 managed.git、other-base、workspace、outside-sentinel 不变后，因预期发布/复用成功的断言失败。入口为 `cargo test --locked -p thinws-p0-git-base --lib base_materialization::tests::fixed_base_publishes_and_reuses_independent_receipt -- --nocapture`。B 不直接沿用包含待合法移动 content 目录的整个 A stable snapshot，也不扩大 A 的排除规则。
- 后续失败用例直接使用同一 prepare/publish/verify helper：分别保存损坏后的 receipt/tree 快照再验证失败不改写；空/非空 final 目标须在 staging 回验后构造并证明双方保留；发布前停止通过完成 prepare/verify 后不调用 publish 表达，随后实际 publish/verify 验证可继续，不增加故障开关或冒称强杀恢复。独立登记身份的隔离用例仍按前述第 5 条补齐，不塞进首次 RED。

以上准入准备没有运行 B 实验，不代表 A 全量门禁已过。`SuffixAbd` 仍被现有单测和实际 fuzz 使用，不能直接改成仅 `cfg(test)`；如真实 B 入口接通后它仍产生未使用警告，先核验 test/fuzz 的精确构建配置及实际 fuzz build，不扩大外部输入或增加无意义发布。

### 当前集成环境的双卷回归（2026-09-12）

主 Agent 先通过 `hdiutil info` 确认此前保留的专用镜像未挂载，并用 `imageinfo` 核对 `/private/tmp/thinws-p0-materialize.OC4JPT/cross-volume.dmg` 为 134217728 字节的 APFS 读写测试镜像；随后在新建 mode 0700 根 `/private/tmp/thinws-p0-03-cross-volume.LWPRY8` 下挂载到 `mounted`，未操作其他镜像。既有 Probe 实测该卷 UUID 为 `E4C7D91A-EF7B-476B-9BEB-946B53FEE500`，fsid `[16777252,26]`，与当前 content 所在 APFS 卷不同。

两个原 ignored 用例先分别以 `--ignored --exact --nocapture` 在 debug/release 下执行，全部退出 0：`configured_real_cross_volume_distinguishes_clone_from_full_copy` 验证跨卷能力预判，`actual_cross_volume_clone_returns_exdev_without_target_then_explicit_copy_succeeds` 验证真实 clone 返回非注入 `EXDEV`、目标未被克隆污染，随后显式 Full Copy 成功。这不放宽 Phase 1 同卷布局政策。

之后主 Agent 在同一双卷环境执行两个既有 crate 的完整普通测试：

```bash
env THINWS_P0_CROSS_VOLUME_ROOT=/private/tmp/thinws-p0-03-cross-volume.LWPRY8/mounted cargo test --locked -p thinws-p0-probe -p thinws-p0-materialize --all-targets -- --include-ignored
env THINWS_P0_CROSS_VOLUME_ROOT=/private/tmp/thinws-p0-03-cross-volume.LWPRY8/mounted cargo test --locked --release -p thinws-p0-probe -p thinws-p0-materialize --all-targets -- --include-ignored
```

两次均为 **111 passed、0 failed、0 ignored，退出 0**（物化 61，Probe 50；FD-zero 的子进程重复输出不另计用例）。这些 crate 的源码在运行前后相对当前 HEAD 无差异；本轮没有重跑其变异或 fuzz，也未把正在编写的 Git/Base A 切片包括进该结果。此前“第二卷未配置”的未执行项在本次指定范围内已补齐，不意味着整个 workspace 或 P0-03 门禁已通过。

全部测试进程结束后，主 Agent 重新核对镜像与挂载点映射，普通 `hdiutil detach /private/tmp/thinws-p0-03-cross-volume.LWPRY8/mounted` 退出 0，报告 `disk9` ejected；再次 `hdiutil info` 确认专用镜像已卸载。未使用 force detach，未删除镜像或既有证据目录；本次成功测试创建的受控 fixture 按原测试的身份校验清理逻辑处理，其他保留现场未额外清理。

### 固定 checkout 首轮 GREEN 与验证缺口（2026-09-12）

Sol / `xhigh` 冻结 A 源码为 SHA-256 `d30dc3e612dd59426b9d7ed891ec5e93d2567b8ee512660609836efd041b22b8`。主 Agent 通读全部 2449 行后，独立执行以下两个命令，分别 **4 passed、0 failed、0 ignored、20 filtered，退出 0**：

```bash
cargo test --locked -p thinws-p0-git-base --lib base_materialization::tests:: -- --nocapture
cargo test --locked --release -p thinws-p0-git-base --lib base_materialization::tests:: -- --nocapture
```

四例涵盖独立 manifest 成功、未知目标项在任何 Git/index 操作前拒绝、真实 checkout 成功后发现同长度错误字节，以及缺少 owner execute 的模式不符。失败现场保留，源仓库与其他 Base/Workspace/外部 sentinel 的对照不变；这些对照不等于任意并发修改下的原子快照。GREEN fixture 的 symlink 改为原始文本 `../outside-sentinel`，以直接核对扫描不解引用；不是扩大公开支持范围。

主 Agent debug 根前缀为 `target/p0-git-base-tests/base-checkout-29348-17892298610528`（序号 0–3），release 根前缀为 `target/p0-git-base-tests/base-checkout-29601-178922986223`（序号 0–3）。其中 debug 成功根的完整路径为 `target/p0-git-base-tests/base-checkout-29348-1789229861052850000-2`；在测试之外用独立 Git、`od`、no-follow `stat` 和 `readlink` 核对到 commit `c4b6d137b0032f31ed60ebe8c041bbe6b4bdb0ce`、tree `08d77a5fe7275ef930d8e50f00fedfc916ef94f5`、正确 LF 字节；CaseWitness/casewitness 对应同一 device/inode `16777240/34608740`，NFC/NFD 对应同一对象 `16777240/34608741`，link witness 为独立 symlink `16777240/34608742`，原始文本为 `CaseWitness`。此独立观察只证明这两个有限名字对，不能代替自动回归。

当前完整 `cargo test --workspace --all-targets` 退出 0：**144 passed、2 ignored**，其中 Git/Base 为 24 单元＋4 attributes 集成＋7 topology 集成。两项 ignored 的双卷验证已在上一节独立完成，普通 workspace 命令本身仍报告 ignored。`cargo fmt --all -- --check` 单独退出 0。通用 Clippy 命令仍退出 101，报告 A/BaseKey 尚未接入真实入口产生的 **67 项 unused-code 错误**；作者不带 `-D warnings` 的 Clippy 退出 0 不算通用门禁通过。未使用排除、假调用或扩大 API 来消除这些错误。

GPT-6 Astra / `xhigh` 完整只读审核该冻结源码，未发现新增执行语义阻断，但四例不足以关闭全部安全断言：descriptor 中 witness 关系缺少独立文件身份 oracle；0644/0755 无法区分 owner `0100` 与 any-execute `0111`；FD/report 错配、扫描阶段未知/特殊项和 witness 身份替换缺少直接拒绝回归。主 Agent 核验后安排仅在现有私有 helper 与新合成 fixture 上补测试，不新增注入框架或产品机制。本次为测试加强，不编造产品行为 RED；发现真实缺陷时必须先保留失败证据再修复。

本轮尚未运行 A 的适用变异/fuzz 验证；A 测试加强、B 发布/复用、完整 P0-03 与 P0-04、阶段门禁仍未完成。上述审核不是完整任务 Approve，不改变公开契约，不满足条件放行，也未提交、合并或打 tag。

### 固定 checkout 测试补强与专项验证（2026-09-12）

Sol / `xhigh` 仅修改 A 的测试区，最终冻结 SHA-256 `b214862a67f46c0669b45f959b479bb5f3c1b4a25f9b7dc7a3022dd6e55b390c`。主 Agent 对照确认前 1453 行生产逻辑与首轮 GREEN 一致，通读测试增量，并独立执行上一节两条 A 命令：debug/release 均 **9 passed、0 failed、0 ignored、20 filtered，退出 0**（session `38528`、`57771`）。fmt 单独退出 0。

新增自动断言核对独立 Git version、两组有限 witness 的实际 regular 类型和 device/inode；用 chmod 后独立 stat 确认仅 group/other execute、owner execute 未设置，再核对完整 manifest 仍为 `100644`。直接 helper 回归实际覆盖 FD/report 错配、成功 checkout 后加入未知项、合法固定文件名处的 Unix socket，以及同 link text 但身份已替换的 witness；不把这些用例说成流程中任意时刻的并发注入。socket 例通过 held-parent `NOREPLACE` 保留原文件，核对身份、原字节和外部 sentinel；witness 例保留新旧 symlink 并核对身份及原文本。其他直接 helper 只证明各自写明的拒绝与保留断言，不统称每例都扫描了所有 source 对照。

socket 使用独占 mode 0700 的短根，避免 Unix socket 路径长度限制。主 Agent 两次完整根分别为 `/private/tmp/tw-p0b-69540-1789230777843895000-0`、`/private/tmp/tw-p0b-69867-1789230779217981000-3`；其他八例各自保留在 `target/p0-git-base-tests/` 下对应 PID `69540`、`69867` 的独立根。作者首次尝试 `rustix::fs::mkfifoat` 因锁定版本的 Apple API 不可用产生 E0425，属于测试构造的编译失败而非行为 RED；经主 Agent 决定使用标准库真实 Unix socket，没有新增 unsafe、FFI 或依赖，也不声称 FIFO 已验证。

主 Agent 核验固定 `cargo-mutants 27.1.0`，枚举 A 全部 242 个候选；首次将测试参数 `--nocapture` 放错层级，baseline 构建成功但 Cargo 参数解析失败，退出 4，**未测试任何变异**。原始输出 `target/p0-03-checkout-mutants.n825ER/mutants.out/` 与其 leaked scratch 均保留。修正参数后重新启动以下命令，使用独立副本而非 `--in-place`，输出目录不覆盖前次：

```bash
cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/base_materialization.rs --leak-dirs --timeout 60 --jobs 2 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-checkout-mutants.ZnlAor -C=--locked -- --lib base_materialization::tests:: -- --nocapture
```

该运行的未变异 baseline 已通过（7 秒构建＋4 秒测试），两个 scratch 为 `/var/folders/bf/x_rpvxyn1fzc8x2fn0gb8g9r0000gn/T/cargo-mutants-thinworkspace-p0-03-tewZOm.tmp` 和同父目录下的 `cargo-mutants-thinworkspace-p0-03-2OQXBO.tmp`。运行尚未完成，但已实际发现存活变异，包括索引上限乘法变加法、错误 Display 变空、错误 source 丢失及其 Process/FileSystem/Probe 分支删除；这些需要补测试或经规定 Reviewer 接受的明确处置，不能报告门禁通过。它枚举整个 A 文件，但测试执行范围明确为九项 A 测试；不替代后续全 Git crate 或阶段变异门禁。没有增加排除项。

GPT-6 Astra / `xhigh` 同时完成只读 fuzz 边界核查：A 的原始组件字节与 Git 输出字节必须测试实际源码，长度前缀编码可在同一纯内存目标覆盖。主 Agent 接受最小提取 `validate_component`、`text_output`、`frame` 到私有纯模块的方向，保持原接受范围、错误分类/文字和调用语义；固定名称全集合、真实 FD/Probe、Git checkout 和 witness 仍由普通/变异/真实平台测试负责，不新增通用解析器或扫描模型。该接入尚未实现，现有相似 fuzz 目标不冒充 A 覆盖。

### Checkout 纯输入接入与 smoke 结果（2026-09-12）

Sol / `xhigh` 完成上述三个实际函数的私有提取。冻结 SHA-256：`base_materialization.rs` 为 `e6325669c815b9da609056dfde6b1c6ac7402aaaeac31cfd2a701540d8006617`，新 `checkout_validation.rs` 为 `f3f144d12a8d2c8cfb3a3a046acb70667c71a5a435c6f6f45ec81e02c66531eb`，新 harness `fuzz/fuzz_targets/checkout_validation.rs` 为 `c5f13a2141ad69bb259feb77fb5aa5213645a48eea73d06ff3c5df20d02059e5`。主 Agent 只接入私有模块声明、fuzz bin `thinws_checkout_validation` 与 CI 的 60 秒入口，没有新增/升级依赖，两份 lock 的哈希未变。

主 Agent 和 GPT-6 Astra / `xhigh` 分别通读增量并对照保留的 `b214…` 源码：接受范围、原结构化错误分类/文字和九项 A 测试均不变；A 与 harness 编译同一真实纯模块。harness 对不超过 4096 字节的原始和有界结构化输入，独立检查组件字节、UTF-8/Unicode trim 和手动大端 framing；最多 4097 个切分字段，分配有界，不访问文件系统或子进程。Astra 给出本增量无阻断的限定意见，不是整体 A/P0-03 Approve。

主 Agent 对冻结快照独立执行结果：

| 命令范围 | 结果 |
|---|---|
| 通用 `cargo test --workspace --all-targets` | session `24266` 退出 0，**152 passed、2 ignored**；其中 Git 为 32 单元＋4 attributes＋7 topology。两个 ignored 仍是已在前文另行验证的跨卷用例，本命令没有再次运行它们 |
| `cargo test --locked --release -p thinws-p0-git-base --all-targets` | session `6784` 退出 0，**43 passed、0 ignored** |
| 根与 fuzz workspace 的 fmt | 均退出 0 |
| 通用根 Clippy | session `15929` 退出 101，**70 项未使用代码错误**；真实 Base 入口尚未接入，不排除警告，也不称通用门禁通过 |
| `cargo clippy --locked --manifest-path fuzz/Cargo.toml --all-targets --all-features -- -D warnings` | session `95457` 退出 0 |

固定 nightly 的 `cargo +nightly-2026-08-14 fuzz build thinws_checkout_validation` 退出 0。新建并保留空 corpus 后执行：

```bash
cargo +nightly-2026-08-14 fuzz run thinws_checkout_validation /tmp/thinws-p0-03-checkout-corpus.ceSnRw -- -max_total_time=60 -timeout=5 -max_len=4096 -print_final_stats=1
```

session `93200` 退出 0，**1,985,335 runs / 61 秒**，无 crash/timeout；libFuzzer 报告最终 corpus 299 项/10,496 字节，磁盘实际保留 295 个文件，演进过程 `new_units_added=845`，峰值 RSS 577 MB，artifact 目录无文件。超过 `u32` 长度的字段没有动态分配和触达，其错误常量文字断言不算边界执行证据；本 smoke 也不证明真实 Git/FD/manifest 或 Base 发布。远端当前候选 CI 与阶段长预算仍未运行。

旧 `b214…` 的 A 242 项变异继续在独立副本运行，不受此次提取影响，也不覆盖此后新增测试。Astra 已逐项核验最初六个存活项，无严格等价项：索引上限改变接受范围，Display/source 丢失诊断或错误链。主 Agent 接受限定结论，安排 Sol 仅用已有私有 helper 补独立字面量的索引上下界和错误链/上下文回归；不新增 Git fixture、注入框架或排除。后续必须以新快照重跑对应变异和完整门禁，当前任务仍 In Progress，未提交、合并或放行。

### 首批六项存活变异的回归闭环（2026-09-12）

Sol / `xhigh` 仅在 A 测试区增加四项回归，冻结 `base_materialization.rs` SHA-256 为 `19cafd461ed2e804a34e5575f87d3a2747b63e789adc5abebe0e2b386ad32e62`，纯模块/harness 未变。索引样本的 1,048,575／1,048,576／1,048,577 字节长度由独立字面量规定，不从生产常量派生；前两者核对长度、真实文件身份、摘要和字节，后一者核对精确 bound 错误及原文件保留。其他用例核对外层 Display 的上下文/operation/path、Process→ExperimentError→io 的完整原因链、FileSystem 的 io kind/errno，以及实际相对路径预检产生的 ProbeError 类型；Validation 对照保持无 source。这些是包装与边界测试，不冒称真实 spawn 或权限失败。

主 Agent 通读测试增量后，独立执行完整 A debug/release 命令，分别 **13 passed、0 failed、0 ignored、23 filtered，退出 0**（session `50764`、`8253`）。新索引边界现场完整保留于 `target/p0-git-base-tests/base-checkout-63404-1789231702388706000-9` 和 `target/p0-git-base-tests/base-checkout-63770-1789231704010948000-9`；其他 A 场景仍使用各自独立根。fmt 再次退出 0。

主 Agent 先用 `cargo mutants --list` 确认筛选精确匹配最初六项，不把后来移位的行号当成另一组变异，然后运行：

```bash
cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/base_materialization.rs -F 'replace \* with \+$|BaseMaterializationError>::fmt|BaseMaterializationError>::source.*with None$|delete match arm BaseMaterializationCause::(Process|FileSystem|Probe)' --leak-dirs --timeout 60 --jobs 1 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-checkout-six-mutants.D7Pz2V -C=--locked -- --lib base_materialization::tests:: -- --nocapture
```

session `52450` 退出 **0**，成功 baseline 后 **6 tested、6 caught、0 missed、0 unviable、0 timeout**，约 2 分钟。新 scratch `/var/folders/bf/x_rpvxyn1fzc8x2fn0gb8g9r0000gn/T/cargo-mutants-thinworkspace-p0-03-7sH3mz.tmp` 与全部输出保留。没有新增排除，没有修改生产行为；此筛选只证明六项回归有效，不替代完整 A、全 crate 或阶段门禁。当前完整 A 旧快照的存活项仍需逐项处置，新测试快照尚未运行完整 workspace/全 crate 变异，不能沿用前一快照的数字称全部完成。

旧 `b214…` 的完整 A 批次（session `60088`）随后正式结束，退出 **2**：**242 = 120 caught + 102 missed + 20 unviable，0 timeout**，约 22 分钟，原始结果仍在 `target/p0-03-checkout-mutants.ZnlAor/mutants.out/`。这份原始失败结果不改写成补测后的结果；当前源码变化后必须重新验证，不能用减去若干已处置项的算术当成新快照变异成绩。

GPT-6 Astra / `xhigh` 限定核验旧快照 flags 组，指出 23 个 OR→XOR 对应两两不重叠的已选 macOS flags，严格等价；OR→AND 不等价且不能同批排除。主 Agent 尚未修改排除配置，后续源码位置/表达式改变须重新核对。主 Agent 接受最小整理方向：把 snapshot 的两次 read-open 和 scanner 的一次相同 read-open 收敛为一个真实私有 helper；已有 witness-create 由两处真实调用共用，保留各调用原错误上下文和 EXIST 分支。三处读取仍各自独立打开、读取、核对身份，不新增 Port 或故障框架。新增真实 FD 用例直接验证 CLOEXEC、NONBLOCK、访问模式，以及目录/文件 symlink 和重复创建拒绝；不得用恒真的 `contains(RDONLY)`，也不让 FIFO 阻塞来代替 flag 断言。此处记录当时的实施决定，实际结果见下节。

### 文件打开整理与纯输入变异结果（2026-09-12）

Sol / `xhigh` 完成上述两处实际共享与三项 FD 测试，冻结 A 的 SHA-256 为 `f5f03a4dfe9532cf6df812f57b201cf7aac9bf40917929e67c609ba7eccbdcb8`。主 Agent 对照保留的 `19cafd…` 源码通读完整增量，独立运行既有完整 A debug/release 命令，均 **16 passed、0 failed、0 ignored、23 filtered，退出 0**（session `3160`、`26143`）。索引边界现场分别为 `target/p0-git-base-tests/base-checkout-81082-1789232482573032000-12` 和 `target/p0-git-base-tests/base-checkout-81523-1789232485060901000-12`；socket 短根分别为 `/private/tmp/tw-p0b-81082-1789232482567129000-0` 和 `/private/tmp/tw-p0b-81523-1789232484955724000-8`。本批没有改变调用者错误上下文、读取前后身份检查或公开契约。

FD 用例直接读取 `F_GETFD/F_GETFL`，验证 CLOEXEC、NONBLOCK 和准确访问模式，并核对重复 create-new 与 symlink 拒绝后既有身份/字节不变。主 Agent 发现两个 directory helper 还应分别包含普通文件和 symlink 的对称负例，已纳入下一批补测；不把本批通过等同于所有 flags 变异已关闭。主 Agent 另对照本机 macOS 26.1 SDK 的 `sys/fcntl.h` 核验了 Astra 证明所用的八个 flag 数值；当前仍未新增等价排除。

同一真实纯模块 `checkout_validation.rs`（SHA-256 `f3f144d12a8d2c8cfb3a3a046acb70667c71a5a435c6f6f45ec81e02c66531eb`）独立执行：

```bash
cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/checkout_validation.rs --leak-dirs --timeout 60 --jobs 2 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-checkout-pure-mutants.U5gTVk -C=--locked -- --lib checkout_validation::tests:: -- --nocapture
```

枚举与终态均为 **12 项，12 caught、0 missed、0 unviable、0 timeout**；baseline 为 3 passed、36 filtered。后续查询 session `33234` 时句柄已不存在，主 Agent 没有重启实验，而是确认无对应存活进程，核对 `outcomes.json` 的完整 12 项及 `end_time=2026-09-12T17:08:27.167354Z`、逐项日志和两个恢复后的源文件哈希；不补造未取得的顶层进程退出码。全部输出及 scratch `cargo-mutants-thinworkspace-p0-03-MAERM5.tmp`、`cargo-mutants-thinworkspace-p0-03-HPDfEz.tmp`（位于前文相同系统临时父目录）保留。此结果只覆盖三个纯函数，不替代 A、Git crate 或阶段全量门禁。

Astra / `xhigh` 随后对旧快照非 flags 存活项分组核验，没有新增可直接接受的等价排除。主 Agent 领取现有 helper 的路径/布局、卷报告、读取预算、名称集合、stat、witness 和诊断反例；合成判断输入与实际 OS 观察分开记录。仅允许两处实际读调用共享小型有界读取函数、两处大小判断共享纯谓词，以及重复身份比较复用已有 `FileIdentity`，保留每个原即时检查与错误；不新增注入框架、产品抽象或兼容策略。该批仍在实施，Base 磁盘发布/复用与完整门禁尚未完成，任务仍 In Progress，未放行或打 tag。

主 Agent 另对未变的 `base_key.rs`（`17f0fad3e74d9b51cc02851e759bc6da349b5a9f3468e785d5a9b85473b0c6e2`）与 `topology_validation.rs`（`7e30039426c13a24d9e9817f5cece163c08ceca7f397ae72d0954958135ec982`）执行全部生成变异：

```bash
cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/base_key.rs -f experiments/p0/git-base/src/topology_validation.rs --leak-dirs --timeout 60 --jobs 2 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-base-topology-pure-mutants.RDUVxI -C=--locked -- --lib -- --skip base_materialization::tests:: --nocapture
```

session `78467` **退出 0，32 tested = 31 caught + 1 unviable，0 missed、0 timeout**，49 秒。baseline 为 23 passed、16 filtered；这里有意不执行 A 的真实 checkout 测试，测试范围是含两模块实际调用者的其他库内测试，不是全 Git/APFS 验收。唯一 unviable 是 `encode_base_key_v1` 被替换为 `Ok(Default::default())`，结果类型没有 `Default` 导致编译失败，不计作 caught。原始输出及 `cargo-mutants-thinworkspace-p0-03-AZXWQK.tmp`、`cargo-mutants-thinworkspace-p0-03-HxgKXx.tmp` 副本保留，主 Agent 核对恢复后的两个源码哈希与上述一致；没有新增排除。

### Checkout 边界回归与托管拓扑存活项（2026-09-12）

Sol / `xhigh` 在原 A 生产逻辑上先补 11 个 helper 反例，测试为 27 passed；首次对目录 symlink 的 errno 指定过严，修正测试为拒绝断言后通过，此项不是产品行为 RED 或实现缺陷。完成获准的局部消重并增加两项纯边界测试后，冻结 A SHA-256 为 `761f48ddafb4e85cda15fa6bb5f8e3f4fb9324b15a81eac525c49ed2992b1743`。主 Agent 通读完整增量，并独立执行完整 A debug/release，均 **29 passed、0 failed、0 ignored、23 filtered，退出 0**（session `33732`、`33687`）。现场位于 `target/p0-git-base-tests/` 下 PID `73162`、`73439` 的各自独立根，socket 根分别为 `/private/tmp/tw-p0b-73162-1789234189201875000-0`、`/private/tmp/tw-p0b-73439-1789234191277607000-0`，均保留。

新反例实际证明 owned mode `0400` 的 info 目录可打开，其 attributes 子项查询返回 `EACCES`，错误保留原 operation/path/errno；普通文件父项的 `ENOTDIR`、不覆盖改名后的路径重验、Distinct witness、超长第二名称与既存不同身份均有实际现场。卷报告字段的修改及重复名称列表明确只是合成判断输入，不冒称原生报告/readdir 产生了这些值。stat 用例用真实文件分别隔离 inode、mode、length；小预算 Cursor 测试证明最多读取上限加一字节。两个 directory opener 都补有普通文件和 symlink 反例。

GPT-6 Astra / `xhigh` 对冻结增量限定审核无阻断。`snapshot_file` 的首个 File 现保留至函数结束，跨越重开与身份比较，原实现是在链式读取语句结束时关闭；这是寿命延长，不能称关闭时点不变。scanner 仍对同一个 held File 做读后 fstat，原边界和错误映射不变。Reviewer 建议可省去 `same_identity` wrapper；主 Agent 本轮保留其四个真实调用者共享的既有值对象比较，不新增状态或规则，将其视为非阻断简化建议，不为样式修改打断冻结验证。

同一时段，旧 `managed_topology.rs` SHA-256 `49e5e3493a75a4b68247a1cecc219b4a882644e83ba86bbd6dc4eadc45bf585e` 运行：

```bash
cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/managed_topology.rs --leak-dirs --timeout 60 --jobs 2 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-managed-topology-mutants.kG3zgS -C=--locked -- --lib --test managed_topology -- --skip base_materialization::tests:: --nocapture
```

session `97322` **退出 2，92 tested = 60 caught + 21 missed + 11 unviable，0 timeout**，约 12 分钟，结束于 `2026-09-12T17:33:09.523458Z`。测试范围为非 A 库内用例及 7 项托管拓扑集成，未执行 attributes 集成或 A checkout。原始日志与两个 scratch `cargo-mutants-thinworkspace-p0-03-yz0Gy2.tmp`、`cargo-mutants-thinworkspace-p0-03-VuJbFt.tmp` 保留。21 个存活项需要按实际源码处理，不把它们归为全局得分或静默排除。

其中 Process source 分支缺少回归，Sol 仅在既有错误测试追加 29 行，冻结拓扑 SHA-256 为 `c955ae1f24c89a6ea914da24a8c0f730a15ef5a7ff74cc4ee264d6e24ae494f8`；生产逻辑未变。主 Agent 通读 diff，独立执行 `managed_topology::tests::managed_error_diagnostic_and_source_preserve_structured_cause` 精确 debug/release 单测，均 **1 passed、51 filtered、退出 0**（session `42202`）。Astra / `xhigh` 确认两级 source/downcast、errno 2 与 Display 断言正确；它只验证构造的 `Process → ExperimentError::Spawn → io` 错误链，不宣称实际 spawn 错误走了该映射。旧 92 项结果不覆盖新增断言，仍须精确重测。

B 编码前，上述 A/拓扑冻结快照的主 Agent 通用与 release 结果为：

| 命令范围 | 结果 |
|---|---|
| fmt 与 `git diff --check` | 退出 0 |
| `cargo test --workspace --all-targets` | session `93889` 退出 0，**172 passed、2 ignored**；Git 为 52 单元＋4 attributes＋7 topology，FD-zero 子进程输出不重复计数 |
| `cargo test --locked --release -p thinws-p0-git-base --all-targets` | session `14159` 退出 0，**63 passed、0 ignored** |
| 通用根 Clippy | session `63425` 退出 101，**74 项未使用代码错误**；真实 Base 入口未接入，没有用 allow、假调用或扩大无关 API 消除错误 |

两个 ignored 仍是前文另行完成双卷验证的用例，本次没有再次挂载镜像。A 新快照的完整 200 项变异在独立副本运行（session `58097`，输出 `target/p0-03-checkout-boundaries-mutants.zQZf7x/mutants.out/`），baseline 35 秒构建＋22 秒测试成功；终态为 **退出 2，168 caught、13 missed、19 unviable、0 timeout**，结束于 `2026-09-12T17:52:47.45622Z`，约 21 分钟。主 Agent 核对完整 missed 清单，以及恢复后的 `cargo-mutants-thinworkspace-p0-03-egIV26.tmp`、`cargo-mutants-thinworkspace-p0-03-PTYeLd.tmp` 两份源码 SHA-256 均为上述 `761f48…`；后续原工作区的 B 编辑不属于这次变异快照。

13 个 missed 全部对应 Astra / `xhigh` 已重新枚举、严格证明等价的四个 opener 中 OR→XOR：`open_directory` 3 项、`open_directory_at` 3 项、`open_fixed_read_at` 3 项、`create_witness_file` 4 项。主 Agent 独立核对的 macOS flag 位互不重叠，RDONLY 为零；证明限于该平台、依赖、操作数和表达式。旧 23 项不能沿用，13 项 OR→AND 不在等价结论内；源码移动后必须重新枚举位置。尚未修改排除配置，因此不把工具退出 2 改写成命令通过，也不称这些等价项已被测试杀死。

Process source 的精确重测另运行于冻结的 `c955ae…` 副本：`cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/managed_topology.rs -F 'delete match arm ManagedTopologyCause::Process' --leak-dirs --timeout 60 --jobs 1 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-topology-process-mutant.trmysa -C=--locked -- --lib managed_topology::tests::managed_error_diagnostic_and_source_preserve_structured_cause -- --exact`。session `41453` **退出 0，1 tested、1 caught**；主 Agent 确认 caught 名称恰为旧存活分支，恢复后的 `cargo-mutants-thinworkspace-p0-03-xio6yo.tmp` 源码哈希匹配。其余拓扑存活项仍在补测，不以此替代全量重测。

后续托管验证反例的准备范围由主 Agent 核验、Astra / `xhigh` 限定复核：在已有私有 root/request 与 Git 执行工具上准备最小真实 bare staging，含一个非空 blob/tree、固定作者/时间的 commit 和独立 expected ref map。它只服务 `validate_imported_repository`，不复制完整 source dirty/index fixture，也不冒称验证 source 导入。每例先验证正常仓库，再单独改变 promisor 配置、pack marker、pack 目录或 loose blob 的存在条件；移动使用唯一受控保留名及不覆盖操作，不删除对象。缺失 blob 后需实际得到非空且含 `?oid` 的成功 `rev-list` 输出；pack 错误需命中原底层错误而非提前失败。promisor 的既有“exit 1 且双流空”判断允许收进其现有唯一调用的执行方法并按实际契约更名，保留原错误及一次命令记录；先以真实 `check-ref-format` 的 exit 0/双流空取得拒绝断言的 RED，不新增 checker、runner 注入或测试公开 API。这是下一批有界准入，尚非执行结果。

### Base 发布与复用首个行为 RED（2026-09-12）

Sol / `xhigh` 首份 B 测试同时无条件要求 NotImplemented 和成功，主 Agent 识别为测试自身矛盾，未将该版本作为有效 TDD 准入。修正为互斥的 Err/Ok 分支后冻结 SHA-256 `6276dcc3dab26ac9c3d4d13d30afd83e9af82f2b5980ae0629e7e6fd1d4137af`：Err 分支核对 A 实际执行与遗留现场后明确失败，Ok 分支独立核对发布单元、receipt 身份及发布后的 tree，不再要求被合法移动的旧 content 路径存在。主 Agent 在 `lib.rs` 仅 re-export 前述真实入口与签名可达类型；BaseKey、RepositoryId fixture、A 入口和内部 helper 保持私有，不改变 P1 API。

主 Agent 通读修正后，独立执行本节准入中指定的精确测试（session `79786`），**编译成功，退出 101，0 passed、1 failed、52 filtered**。唯一失败位于第 1661 行的明确 NotImplemented 边界；此前独立 256 字节 BaseKey/502 字节 receipt literal 的字段边界、真实固定 Git fixture、20 条成功 A 命令、8 项独立 manifest 和受保护对象快照断言均通过。现场为 `target/p0-git-base-tests/base-checkout-32751-1789235589287864000-0`，保留未清理。

据此只进入同一有界 B 正常发布/读盘复用的最小 GREEN：必须把实际 receipt encoder 与上述完整独立 golden 比较，并独立核对磁盘 expected；损坏/隔离矩阵、与两个 linked worktree 的组合及完整 P0-03 门禁仍未完成。该准入不是实验成功、阶段放行或发布授权；RED 时点的工作区包含预期失败，随后实现与验证见下节。

### Base 正常流 GREEN 与构建隔离验证（2026-09-12）

Sol / `xhigh` 完成 B 正常流，冻结 `base_materialization.rs` SHA-256 `7964a0b312779b251578b1ba57ecb2388f8fb203de0b518a3c106e53d01247c7`。实际入口执行固定 checkout、形成可信 expected、移动 tree、create-new receipt、读盘回验、整个单元 `NOREPLACE` 发布及两次重新打开的复用验证；没有恢复、修复或删除动作。生产 encoder 与独立 502 字节 golden 完整相等；真实磁盘 receipt 由测试独立编码核对。作者完整模块 debug/release 均 **30 passed、26 filtered**。

主 Agent 通读对 A `761f48…` 的全部增量，独立 workspace 普通测试 **176 passed、2 ignored**（session `69263`），Git crate release **67 passed、0 ignored**（session `72905`），两者退出 0。Clippy 此时只剩测试/fuzz 专用的 `SuffixAbd` 未构造这一项，未以 allow 或假调用压制。主 Agent 只对该 variant 及其 match arm 加 `cfg(any(test, fuzzing))`，并在实际编译该源文件的 Git 实验包与 fuzz 包声明 `check-cfg`；BaseKey SHA-256 变为 `c359cdceecf0bc172fa2cb8ff214cc7585ec5c937e2839a9991662d3a9d84f5d`，编码规则和函数体不变。本机固定 `cargo-fuzz 0.12.0` 源码确认正常构建传入 `--cfg fuzzing`，没有用单纯 `cfg(test)` 丢掉 fuzz 的第二 owner 样例。

构建隔离后，主 Agent 再次运行完整通用与 Git release 门禁：fmt 通过，Clippy（session `21589`）退出 0；`cargo test --workspace --all-targets` 为 **176 passed、2 ignored**，`cargo test --locked --release -p thinws-p0-git-base --all-targets` 为 **67 passed、0 ignored**（session `12654`，两组全部通过，最终退出 0）。源码核对仍为 B `7964a0…` 与拓扑 `c051dd…`，无 warning。两个 ignored 仍是已有独立双卷证据的用例，本轮未重挂镜像。该轮 Base 正常流现场分别为 `target/p0-git-base-tests/base-checkout-61344-1789236355092488000-9`、`base-checkout-66655-1789236382383500000-6`，均保留。

真实 `cargo +nightly-2026-08-14 fuzz build thinws_base_key_candidate`（session `16223`）退出 0。随后沿用前文 60 秒、timeout 5 秒、max_len 4096 的命令预算，在新 corpus `/tmp/thinws-p0-03-base-key-cfg-corpus.j7aeAW` 执行 smoke（session `86055`）：**退出 0，5,765,971 runs / 61 秒**，无 crash/timeout，峰值 RSS 499 MB；libFuzzer 报告 75 个有效样例/4,759 字节，磁盘保留 74 个 corpus 文件，artifact 为 0。harness SHA-256 仍为 `95d00ee4eb5236633b1fe13ebc374726a355ab0074d31d8b0317bdea20dae5e2`；旧 corpus/现场均未改写或删除。本结果只证明构建隔离保留实际 fuzz 路径，不是阶段长预算。

同一冻结拓扑的三项新测试由主 Agent 通读，并包含于上述独立 debug/release 中。另对原相对路径、UUID、publish flags、require_missing 行的全部 16 个变异执行 `cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/managed_topology.rs -F 'managed_topology.rs:(457|458|459|520|761|1073|1074):' --leak-dirs --timeout 60 --jobs 2 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-topology-boundaries-mutants.HRamW5 -C=--locked -- --lib managed_topology::tests:: -- --nocapture`。session `93291` **退出 2，13 caught、3 missed、0 unviable、0 timeout**，结束于 `2026-09-12T18:04:20.05957Z`；3 个 missed 恰为已证明等价的 publish OR→XOR，未新增排除。主 Agent 核对恢复后的 `cargo-mutants-thinworkspace-p0-03-yuuCxe.tmp`、`cargo-mutants-thinworkspace-p0-03-iI9lY7.tmp` 源码均为 `c051dd…`，不把精确重测替代全量拓扑门禁。

GPT-6 Astra / `xhigh` 完整读取 B 增量、上述拓扑纯测试增量和 BaseKey 构建隔离，限定结论无阻断。Reviewer 明确这不是完整 B 或 P0-03 Approve；publish-parent 反例不证明竞态或独立 CLOEXEC，Base 复用不证明无 expected 恢复或断电持久性。当前继续既定损坏拒绝用例和托管验证反例，之后仍需发布冲突/发布前停止、受控隔离、双 worktree 组合、剩余 Git 矩阵及完整收口门禁。当前未提交候选未运行远端 CI，未放行、合并或打 tag。

### Base 损坏拒绝与托管验证闭环（2026-09-12）

Sol / `xhigh` 在既有 B 正常流上增加 7 个独立 fixture 用例：receipt 缺失、截断、尾随字节、版本错误、不同 RepositoryId、tree 内容损坏，以及 tree 与 receipt 同时被改为自洽但不可信的内容。每例先真实发布，再只改变目标条件；验证失败前后比较损坏现场与受保护对象，确认不改写。快照覆盖根下条目的类型、身份、模式、文件字节和原始 link text，不包括根自身元数据或 atime/mtime，不声称抗同 UID 并发攻击。缺失 receipt 通过不覆盖移动保留原文件，不删除证据。

首份测试源码 `25b122883059901d9de97851798f1e27d7b5dcd6da11d5d4017082f2c9976f1f` 的完整模块由主 Agent 独立 debug/release 执行，均 **37 passed、32 filtered、0 ignored，退出 0**（session `52880`、`60298`）。主 Agent 随后发现 owner-key 用例实际修改的是 filesystem digest，不是 RepositoryId，要求只纠正该断言。冻结源码成为 `89ada08d3c24403020cb1fbaf9dc5e557eced8eedf2ebbc46b39d85e812e2c7c`：明确核对 BaseKey `27..68`、receipt `61..102` 的 41 字节合法 RepositoryId，仅将后缀 `abc` 改为 `abd`，其余字节保持不变。主 Agent 精确 debug/release 重测均 **1 passed、68 filtered，退出 0**（session `20345`）。该修正是测试目标纠正，不是产品行为 RED；整个增量没有改变生产逻辑。各现场保留，精确 owner 用例根为 `base-checkout-93983-1789237144525752000-0`、`base-checkout-95072-1789237149174933000-0`。

托管验证按前文准入完成最小真实 bare fixture 和 6 个测试，Sol / `xhigh` 冻结 `managed_topology.rs` 为 `f80ee84e7f7b8b935be5215e6e0b71e338117cac3facbbfda35eea4bc6d8e6ac`。生产增量仅把既有 promisor 查询的“exit 1 且双流为空”判断收进原执行方法并更名，保留原错误上下文及恰好一次命令记录。其有效 RED 是真实 `check-ref-format` 返回 exit 0、双流为空后，旧私有执行方法仍接受；这不是公开流程曾接受 promisor 仓库的证据。其余 5 个 bare 场景覆盖 promisor config、pack marker、pack 目录 ENOENT/ENOTDIR 和 loose blob 缺失后的非空 `?oid` 输出；其中空 pack 目录缺失是允许 ENOENT 的正向边界，不称为拒绝反例。promisor fixture 首次自证失败来自测试窗口长度错误，修正测试后通过，不计作产品 RED。

主 Agent 通读生产与测试增量，执行 `cargo test --locked -p thinws-p0-git-base --lib --test managed_topology -- --skip base_materialization::tests:: --nocapture` 及其 `--release` 版本：均 **32 个非 B 单元＋7 个托管拓扑集成通过，37 filtered，退出 0**（session `44838`、`51571`）。测试只证明该私有验证器与受控仓库的真实行为，不代替完整来源导入或 Base 组合场景。

同一 `f80ee8…` 快照全量枚举该模块变异，命令沿用前文拓扑全量入口，仅输出目录改为 `target/p0-03-managed-validation-mutants.Iaxb49`。session `79975` **退出 2，92 tested = 79 caught + 3 missed + 10 unviable，0 timeout**，结束于 `2026-09-12T18:23:51.349479Z`；baseline 成功。3 个 missed 位于当前源码 `publish_no_replace` 第 759 行，均为 OR→XOR；尚未设置排除，也未把退出 2 写成通过。主 Agent 核对保留的 `cargo-mutants-thinworkspace-p0-03-aa2HrE.tmp`、`cargo-mutants-thinworkspace-p0-03-Avbs6Z.tmp` 两份恢复源码哈希均匹配。GPT-6 Astra / `xhigh` 已只读核对这两组增量，限定无阻断；对当前表达式再次核对冻结依赖和 macOS SDK：`RDONLY=0`、`DIRECTORY=0x00100000`、`NOFOLLOW=0x100`、`CLOEXEC=0x01000000` 无重复非零位，三个 OR→XOR（列 63/44/24）严格等价，三个 OR→AND 均已实际 caught。此说明只在当前平台、依赖映射和表达式不变时成立；unviable 不计 caught。完整 P0-03 仍未收口。

在上述原始结果保留之后，主 Agent 将这 3 个经规定 Reviewer 接受的等价项，以路径、行、列、操作及函数名完全锚定的表达式加入 `.cargo/mutants.toml`；责任人为主 Agent，失效条件为源码位置/表达式、平台或依赖映射变化。再次 `cargo mutants --list --json` 实际枚举为 **89 项**，三个 OR→AND 仍在，未增加函数/文件级排除。这是排除范围核对，不是又执行了一轮 89 项变异；先前退出 2 的历史结果不改写。A/B 的排除尚未迁移，仍待最终源码逐项核对。

该冻结组合随后通过主 Agent 通用门禁和 Git release（session `57522`）：fmt、根 workspace Clippy、全 workspace 普通测试、Git release **69 unit＋4 attributes＋7 topology** 及 `git diff --check` 全部退出 0。两个专用跨卷用例在普通 workspace 命令中仍 ignored，本次未挂载第二卷，原有双卷证据不被冒充为本次执行。

另核对 CI 原样 fuzz Clippy 命令时发现构建模式遗漏：不带 `fuzzing` cfg 的 fuzz binary 编译在 harness 第 25/132 行引用 `SuffixAbd` 时返回 E0599、退出 101。此前实际 cargo-fuzz 通过不代表该独立 lint 入口通过。主 Agent 只在 CI 的 fuzz Clippy 末尾传入 `--cfg fuzzing`，不删除第二 owner、增加 allow 或新 feature；实际 `cargo clippy --locked --manifest-path fuzz/Cargo.toml --all-targets --all-features -- --cfg fuzzing -D warnings` 退出 0，fuzz fmt 通过。该项为构建配置纠正，不计产品行为 RED，远端候选 CI 尚未执行。两工作区 `cargo deny --offline --locked check`（fuzz 另指定 manifest 与根 deny.toml）均通过，保留未匹配许可配置 warning，不称零警告。

GPT-6 Astra / `xhigh` 对上述单行 CI 修正只读复核无阻断：目标集、锁定依赖和 `-D warnings` 均保留，只对齐 cargo-fuzz 默认的 harness 条件编译，不声称 sanitizer/依赖构建参数完全一致。主 Agent 另执行两份 lockfile 的 `cargo audit --deny warnings --file <lockfile>`（session `58871`），实际更新公告库并分别扫描 36/29 个依赖，均退出 0。

后续仍只落实既定“发布目标冲突与发布前停止”断言：主 Agent 接受 Astra / `xhigh` 的限定设计意见，将现有 B 单个函数拆为私有 prepare 与按值消费资源的 publish，公开实验入口仍直接组合两者。资源束只持有实际所需 FD、路径报告、可信 expected、单元身份和受控根/BaseId；不引入新 Port、状态机、Clone/Drop 机制或故障开关。阶段间允许停留后，publish 必须先再次调用已有完整 staging 验证器并核对身份，不能用只验证路径的 Probe 代替后代内容校验；随后保留 FD/路径重验与真实 NOREPLACE。空/非空 final 在 prepare 完整回验后才独立创建，失败必须精确命中发布冲突并保留双方。停止/继续用例消费同一个 prepared，不声称 SIGKILL、跨进程恢复或断电持久性。该段为准入决定，不是这些测试已经执行的证据。

机械拆分后的原 37 项模块测试由 Sol 执行保持通过。随后新增阶段间 receipt 改变的回归并冻结源码 `3529c9aafc3846ea20cbd358e9468db875e195ea323c3612944decc839b247e0`；主 Agent 通读测试、prepare helper 与生产拆分，独立运行 `cargo test --locked -p thinws-p0-git-base --lib base_materialization::tests::prepared_fixed_base_rejects_receipt_change_before_atomic_publish -- --exact --nocapture`（session `63767`），编译成功后 **退出 101，0 passed、1 failed、69 filtered**。独立 expected 和完整 staging 验证先通过；仅改变已登记 receipt 最后一字节后，publish 虽返回正确 receipt mismatch，却已把 staging 改名为 final，最终缺失断言因此失败。真实根 `base-checkout-28839-1789237918084297000-0` 保留，源码前后哈希一致。这证明阶段间内容重验不可省略，不是编译或测试准备错误；主 Agent 据此进入上述最小 GREEN，尚不称修复验证完成。

GREEN 冻结源码为 `0e747febe04d8d5a2604a1985096ebb20804f6c914ff883e692a836c576e90a1`。除私有阶段拆分外，生产增量只有发布入口的完整 staging 回验及保存身份比较、staging report 与 held FD 匹配；其余原检查和错误包装保留。Sol 新增空/非空目标冲突和同一 prepared 停止后继续，加上上述损坏回归共 4 项。主 Agent 通读相对 `89ada08…` 的完整 diff，独立执行模块 debug/release（session `3671`），均 **41 passed、32 filtered、0 ignored，退出 0**；现场分别为 PID `76349`、`79679` 的报告根，均保留。Astra / `xhigh` 限定复核无阻断，确认双方目录根元数据另外比较，未用不含自身根的子树快照冒充根身份验证；结论仍限于同进程、FD 持续持有的暂停/继续。

主 Agent 对本次 B 的 8 个实际函数枚举 56 项变异。首个命令漏写测试参数前的第二个 `--`，unmutated build 成功、Cargo test 参数解析失败，session `51250` 退出 4，**0 个变异执行**；原输出 `target/p0-03-base-publish-mutants.LWZIhK` 和 scratch `cargo-mutants-thinworkspace-p0-03-XPWQcI.tmp` 保留，不计行为 RED。只纠正参数分隔符后，以相同 B 源码重新运行：

```bash
cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/base_materialization.rs -F 'run_fixed_base_experiment|publish_and_reuse_fixed_base|prepare_fixed_base|publish_prepared_fixed_base|encode_fixed_receipt|append_receipt_frame|write_new_fixed_receipt|verify_fixed_base_unit' --leak-dirs --timeout 60 --jobs 2 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-base-publish-mutants.q7H25G -C=--locked -- --lib base_materialization::tests:: -- --nocapture
```

session `70613` 的 baseline 成功（14 秒构建＋13 秒测试），终态 **退出 2，56 tested = 32 caught + 17 missed + 7 unviable，0 timeout**，结束于 `2026-09-12T18:49:13.83166Z`。scratch `cargo-mutants-thinworkspace-p0-03-my3OSD.tmp`、`cargo-mutants-thinworkspace-p0-03-btAa1D.tmp` 保留，主 Agent 核对恢复源码均为 `0e747fe…`。存活项包括 prepare/publish 的复合校验、receipt 创建 flags、写后 stat 比较和读取长度检查，正在由规定 Reviewer 按精确源码判别；尚未增加 B 排除或修正这些分支，不能宣称变异门禁通过，也不把存活数量直接等同于已确认产品缺陷。本次范围不替代原 A 或整个 crate 的全量收口门禁。

### 来源 refs 重复 fetch 实验准入（2026-09-12）

主 Agent 在原任务“fetch 与固定解析”范围内领取两项命令语义验证，由 Sol / `xhigh` 只在现有拓扑集成测试文件复用 Fixture 和受控 Git runner 实施。先完成真实托管导入和两个 clean worktree，再仅用带预期旧 OID 的 Git 操作改变本次合成来源 refs。成功例验证来源 main 推进、新 tag 导入和已删除来源 topic 不被 prune；同名 tag 改写例验证整次 fetch 失败，managed main、新 tag、旧 tag 和完整 ref map 均不发生部分更新。两个用例均独立比较 fetch 前后的 source 快照、Workspace 原生 index/HEAD/内容以及固定 commit/tree。

调用使用《技术栈》既定 argv 和本机合成来源，不新增 P1 `repo fetch` API。允许失败 fetch 已接收 objects，不把 ref 原子性扩大为整个 bare repository 字节事务。该批是既有 Git 命令的实验补测，没有产品行为修改，不人为制造 RED；若真实语义违背预期则停止并报告，不改兼容政策或静默降级。此处仅记录准入，执行结果待补。

Sol / `xhigh` 完成两项测试，冻结 `tests/managed_topology.rs` 为 `51bb36d1bb20c3faf180ab4f32ccc8d13ec44ee4e47332a3e98b1557273ff8a4`；生产 topology 仍为 `f80ee84…`。作者精确 debug/release 两项均通过，完整拓扑集成 debug/release 均 **9 passed**。成功例完整 ref map 确认推进/新增/保留，拒绝例完整 ref map 保持原样；双方 Workspace 的 index/HEAD/tree、工作文件及 `.git`/commondir/gitdir 均有独立身份和字节对照。Astra / `xhigh` 读完增量后限定无阻断。测试固化的是正常非零退出、stdout 空、stderr 含 `[rejected]` 与 `v1` 以及完整 ref map 不变；作者在保留现场另观察到的精确 exit 1 和完整 clobber/atomic 诊断不冒充全部已固化断言，也不称主 Agent 已独立逐字验证该诊断。

### 组合门禁的既有 CLI 失败（2026-09-12）

主 Agent 对 B `0e747fe…`、生产 topology `f80ee84…`、集成测试 `51bb36d…` 运行组合门禁（session `55395`）。根/fuzz fmt、根/fuzz Clippy 均通过；Git debug **73 unit＋4 attributes＋9 topology** 通过。但后续 `thinws-p0-probe --test cli_contract` 的 `closed_output_and_diagnostic_channels_return_failure_without_panic` 在第 120 行失败：期望进程退出 1，实际退出 0；该目标 **3 passed、1 failed**，整组命令退出 101。原 Probe 生产/测试文件没有本次修改；此事实不足以判断是测试夹具竞态还是生产错误，主 Agent 已安排受控只读诊断和最小复现，不以重复执行偶然成功代替解释。串联命令中的 release 段因此尚未开始，不能称整个 workspace 门禁通过或放行。

主 Agent 随后单独执行 `cargo test --locked --release -p thinws-p0-git-base --all-targets`（session `62654`），同一冻结组合 **73 unit＋4 attributes＋9 topology 全部通过，退出 0**。它覆盖本次 Git/Base 的 release 行为，不消除上述 workspace 门禁失败。

诊断只在 mode 0700 的 `/private/tmp/thinws-p0-probe-channel.wHAlWG` 中创建保留式临时实验，未改 Probe 生产代码。Sol / `xhigh` 编写 `socket_peer.rs`（SHA-256 `7190caa66e68a852e6f0bc61eceaac890db7ee2b1e027ff4a3f853bd53e1ef10`）；主 Agent 通读后独立编译为同目录 `socket_peer-root-check`，以实际现有 Probe binary 运行（session `14960`，退出 0），5 个场景各 32 次结果一致：

| 输出夹具条件 | 实际 Probe 退出结果 |
|---|---|
| 唯一 reader 被 drop | 32 次均为 1 |
| 无 reader 副本，reader shutdown Both 后 drop | 32 次均为 1 |
| 保留 reader 副本，仅 drop 原 reader | 32 次均为 0 |
| 保留 reader 副本，原 reader shutdown Both 后 drop | 32 次均为 0 |
| 原 reader 和副本都存活，发送端 shutdown Write | 独立发送端 clone 实写均为 BrokenPipe/errno 32；Probe 32 次均为 1 |

这证明旧 fixture 的“drop 原 reader 即保证输出不可写”前提不自足，也证明已有 CLI 在明确的 EPIPE 下正确返回失败；没有实际捕获原失败那一次的并发继承事件，因此不把理论 FD 窗口写成确定根因。主 Agent 只授权修正原测试的故障构造：发送端 shutdown Write、保留 reader 副本至子进程结束，并在调用前独立实写断言 EPIPE；不改变 CLI 生产逻辑，不添加 sleep、重试或全局锁。原始 `main.rs` SHA-256 为 `3c6f8c20b03d7c0f6e192c4256fa42a409b3b6dc3b4f7d27f34e54448bd28ba4`，修正前测试为 `44829741f1f020f140598c958bfb9417de3aa887e32e006cd9cda4e15708f6c5`；后续 fixture 修正与全 workspace 重跑仍待验证。

修正后测试 SHA-256 为 `db3ab3a591ff75b702cd5a8f02d46bba772d4384a74a96d2e25116c0a91ddcce`，仅增加必要 imports 与原用例的故障前提，main 哈希不变。Sol 精确 debug/release 各 1 passed、完整 CLI 契约目标各 4 passed；主 Agent 通读 17 行测试增量，并独立运行根/fuzz fmt、根/fuzz Clippy（后者显式 `--cfg fuzzing`）、`cargo test --workspace --all-targets`、`cargo test --locked --release --workspace --all-targets` 和 `git diff --check`（session `53492`）：全部退出 0，**debug/release 各 195 passed、2 ignored**。FD-zero 子进程的嵌套 1 passed 不重复计数。两个 ignored 仍为未在本轮配置第二卷的专用用例，已有双卷记录不能冒充本轮执行。Astra / `xhigh` 对该 fixture 修正限定复核无阻断；公开 CLI 契约未变化。

### B 存活变异处置范围（2026-09-12）

规定 Reviewer 对 17 项终态逐组核验后，主 Agent 接受以下局部整理，不扩建故障注入框架或新 Port：prepare/publish 中，已有完整验证器成功即保证 manifest 相等，删除重复的 manifest 比较但保留登记单元身份检查；receipt 的 create-new 实际调用共用既有文件创建 helper，其 flags 取 receipt 当前全集，并补既有真实 FD 测试中的 NONBLOCK 断言（witness 会新增此标志，不称位级行为完全未变）；receipt 写后 named stat 比较复用已有纯 `require_same_stat` 并保留原 caller 错误上下文；同一个未关闭/未替换的 owned FD 从创建到写完的重复 identity 比较按所有权不变量去除，精确写后长度检查保留；读取时负 size 已被转 u64 后不等于固定非空 expected 长度的检查包含，只去掉重复负值项，保留精确长度和完整 bytes 校验。

五个 receipt 位运算 `| → ^` 经当前 SDK/依赖及操作数核对为等价；三个 `| → &` 不等价。prepare/publish 的两个逻辑运算 `|| → &&` 变异会屏蔽身份条件，也不是等价，以上整理不能表述为它们原来安全。`publish_prepared_fixed_base` 当前第 581 行的 `first_read != reused || reused.0 != staging_identity` 两个条件互不包含：它仍有**非等价覆盖缺口**，本次保持原检查，不通过改写布尔形式、增加测试专用 hook 或降低门禁消除报告。主 Agent 将其保留在当前任务未完成项中，后续结合已有复用/受控隔离范围核验是否有自然可测试边界；未解决前不作任务/小阶段放行。上述整理尚待实施、重测和规定模型核验，当前候选仍未提交、合并或打 tag。

NONBLOCK 回归先冻结为 `785fce4536ae330551edeb03900ec3e2d476f367584accd45103290e465fed34`，生产区仍与 `0e747fe…` 相同。主 Agent 独立执行 `cargo test --locked -p thinws-p0-git-base --lib base_materialization::tests::exclusive_witness_create_helper_enforces_flags_and_never_replaces -- --exact --nocapture`，编译成功后退出 **101，0 passed、1 failed、72 filtered**；唯一失败是第 3697 行真实 `fcntl_getfl` 不含 NONBLOCK。原 CLOEXEC、WRONLY、既存普通文件和符号链接不得覆盖的断言保留；后续断言在该失败运行中未执行，不冒称 RED 覆盖全部用例。作者现场 `base-checkout-10448-1789239458616414000-0` 与主 Agent 现场 `base-checkout-57129-1789239767032914000-0` 均保留在本工作区 `target/p0-git-base-tests/`。主 Agent 据此批准仅在前述五点范围进入 GREEN，尚未取得修正后结果。

五点整理后的冻结 SHA-256 为 `b03ef59e236894a9c79f0f8e4f7fdbc43e7271b0047a2c2375ec848c2ec39ae8`。原 helper 按 receipt/witness 的实际共用职责更名 `create_fixed_file`；没有增加 flags 参数或新的抽象层。主 Agent 通读与 `0e747fe…` 的完整 diff，独立执行完整 Base 模块 debug/release（session `53265`），均 **41 passed、32 filtered、0 ignored、退出 0**；新 NONBLOCK 断言及其后的不覆盖断言完整通过。根 fmt/Clippy（session `23874`）也退出 0。主 Agent 的 debug/release 现场分别为 `base-checkout-88213-*`、`base-checkout-90916-*`，以及实际报告的短路径现场，均保留；此处前缀仅供索引，不作清理选择器。前述 workspace 各 195 passed 的结果仍属于旧 `0e747fe…`，不能直接转写为新候选的全量结果。

新变异范围保留原八个 B 函数，并加入这次真正复用的 `create_fixed_file`、`require_same_stat`，枚举 **48 项**，不新增排除。主 Agent 以相同 `--timeout 60 --jobs 2 -C=--locked -- --lib base_materialization::tests:: -- --nocapture` 参数启动执行（session `29853`），输出 `target/p0-03-base-shared-mutants.bBdojj/mutants.out/`；枚举数不是测试终态，实际结果待补。

该次终态为**退出 2，48 = 34 caught + 6 missed + 8 unviable，0 timeout**；开始于 `2026-09-12T19:07:07.11031Z`，结束于 `2026-09-12T19:15:15.770549Z`。六项实际存活只剩 `publish_prepared_fixed_base:581:29` 的 OR→AND 和 `create_fixed_file:1605–1609:13` 的五个 OR→XOR；后五项是前述互不重叠的固定 flags 等价情况，未新增排除，不把退出 2 改称通过。新的实际共享创建/stat helper 已纳入测试，原 receipt 其他非等价存活没有在本快照中再次出现。主 Agent 核对完整 `missed.txt`、原始结果及恢复后的 `cargo-mutants-thinworkspace-p0-03-mjyIMM.tmp`、`cargo-mutants-thinworkspace-p0-03-b40CSf.tmp` 两份源码，SHA-256 均为 `b03ef59…`；运行期主工作区后续发布/复用重构不属于该结果。

GPT-6 Astra / `xhigh` 对 `b03ef59…` 相对 `0e747fe…` 的完整五点增量限定复核无阻断；该次审核只读，未自行运行测试，也不表示完整 B 批准。

主 Agent 随后在 `b03ef59…` 前后哈希一致时完成全 workspace debug/release、fuzz fmt/Clippy 与 `git diff --check`（session `56518`）：全部退出 0，**各 195 passed、2 ignored**，嵌套 FD-zero 子进程不重复计数；与 session `23874` 的根 fmt/Clippy 合计覆盖通用命令。两个跨卷用例本次仍未配置第二卷，不能称本轮跨卷已执行。此结果对应五点整理版本，不证明下面尚待实施的发布/复用拆分或完整组合验收。

### 发布后磁盘复用边界准入（2026-09-12）

主 Agent 依据详细设计 §6.2 的发布完成与复用前验证职责，并经 GPT-6 Astra / `xhigh` 核验，批准以下行为不变的局部重构与覆盖补强。仍只修改 B 实验模块，由原 Sol / `xhigh` 编写；不新增产品接口、Published/Recovery 类型或测试专用 hook。

1. `publish_prepared_fixed_base` 借用既有 Prepared 资源束，执行原发布流程至首次读盘，返回原读盘 evidence tuple；新的私有 `reuse_published_fixed_base` 借用同一资源束和首次事实，实际调用完整验证器，然后执行原来的两个独立条件。原公开实验入口必须真实串联两步，保持原输出和失败语义、全部 FD 的覆盖寿命；不得在发布后重验已成为历史的 staging 路径报告。
2. 在两步之间，用不覆盖操作保留受控原 receipt，create-new 同字节 receipt，再调用真正的复用操作。必须独立证明 unit 身份未变、receipt 身份已变、新旧字节相同；原检查应拒绝，所有保留对象及受保护现场不被自动重写或删除。该例能够使旧复合检查的条件分别为 true/false，实际区分 OR 与 AND。
3. 另构造已发布单元被同内容、不同身份单元替换的受控反例，保留原单元及登记 FD。真实复用仍须拒绝；此时两个条件通常均为 true，不称该例单独杀死 OR→AND，也不得用新的观察覆盖原登记身份或授权隔离未知对象。
4. 先机械拆分且保持旧测试通过，再补新反例并验证正确实现通过；随后精确重测原复合条件的变异，取得错误实现真实失败的覆盖证据。它不是功能修复 RED，不人为删除安全检查来制造 RED。仍需完整 B debug/release、通用门禁和规定模型核验；原 `b03ef59…` 的在跑变异结果不得转记给重构后的源码。

后续组合验收的只读准备选择复用 B 私有 Fixture、真实 `run_fixed_base_experiment` 和 P0-02 `materialize_once`，在测试内按既定顺序登记两个 linked worktree、物化 Base、初始化各自 native index 并验证隔离。预计仅需现有实验包的测试依赖，不修改中性托管拓扑实验，不宣称同一链重新验证来源导入；尚未开始组合测试实施。

发布/复用机械拆分的旧 41 项测试先通过，作者记录的基线为 `3eb99a62ed92c556f6583262b2569c273ce70e4c10885c96b540700923689a99`；最初两次引用解构导致的编译错误不计作 RED。新增两项真实反例后冻结为 `d9529091562d9b030bc49840b9c1fba972f125bcc080976fa7613baa4764893f`。主 Agent 通读相对 `b03ef59…` 的完整 diff，并独立运行完整 Base 模块 debug/release（session `22510`），均 **43 passed、32 filtered、0 ignored**。随后根 fmt 通过，但根 Clippy 因借用解构后遗留的 16 项冗余引用退出 101，整个串联命令失败；主 Agent 仅授权移除这些多余的 `&`，不添加 allow、不改变检查逻辑，修正待核验。

主 Agent 对 `d952909…` 单独执行原检查的精确变异：`cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/base_materialization.rs -F 'replace \|\| with && in reuse_published_fixed_base' --leak-dirs --timeout 60 --jobs 1 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-reuse-identity-mutant.xzp3hw -C=--locked -- --lib base_materialization::tests::published_fixed_base_reuse_rejects_same_bytes_replacement_receipt -- --exact --nocapture`。session `59312` **退出 0，1 tested、1 caught，0 missed/unviable/timeout**，结束于 `2026-09-12T19:21:18.075914Z`。主 Agent 核对原始日志：正常 baseline 通过，变异版编译成功后在本例的实际复用拒绝断言失败（错误地返回新 receipt 的成功 evidence），不是更早的 fixture 或编译错误。恢复后的 `cargo-mutants-thinworkspace-p0-03-Iuov6p.tmp` 哈希匹配；该结果证明原覆盖缺口被这项真实反例检出，不代表整个新候选变异门禁已通过。

### Git 格式与非 commit 基线补测准入（2026-09-12）

主 Agent 按既有格式/解析验收领取两项原行为覆盖补强，由另一个 Sol / `xhigh` 仅修改 `tests/managed_topology.rs`，不改生产入口或扩大短名解析能力，与 B 的写入区域分离。

- 在现有独占私有 root 内真实初始化 SHA-256 来源，独立确认系统 Git 报告的格式，再调用现有实验入口；必须在读取来源格式后拒绝，来源快照不变且所有新目标缺失。不得通过只改配置伪造格式；系统不支持初始化时报告实际失败，不能跳过后称验证通过。
- 在独立 SHA-1 Fixture 中，用预期零 OID 的 ref 更新新增受控 tag，指向实际 `cat-file` 已证明的 tree/blob。明确扩充本例完整预期 ref map，快照从故意新增 tag 之后开始；选择该完整 tag 时须因不能解析为 commit 而拒绝，并保留实际 Git 阶段、状态、两流及 staging，且不产生 final 或 Workspace。若被测流程在更早阶段错误拒绝了本可导入的非 commit tag，先报告实际证据，不用任意 `expect_err` 宣称目标断言满足，也不自行修改生产代码。

这些用例不证明尚未接入该有界 API 的短名歧义、任意完整 OID 输入或完整产品兼容矩阵。补测不人为制造功能 RED；必须精确 debug/release 与全部拓扑集成测试通过，再交规定模型核验。当前仅准入，未取得执行结论。

Sol 完成后的测试 SHA-256 为 `f8d9b288e71d1dd687d9f4f7b2d7c18b1ee1f9b18773f338855e96cdea3a3a67`，生产 topology 仍为 `f80ee84e…`。真实 SHA-256 初始化成功后仅有 version/format 两条被测 Git 命令，准确命中格式预检拒绝；tree/blob 各例先通过导入和可达对象检查，再准确命中 `resolve-base-commit` 的 GitExit。作者精确 debug/release 均通过，完整 topology IT 各 **11 passed**。主 Agent 通读全部 229 行测试增量；GPT-6 Astra / `xhigh` 限定复核无阻断。快照只证明声明观察的 refs、HEAD、文件、index/config 和 worktree 登记等字段，不扩称所有底层元数据不变。

上述测试与仅清理冗余引用后的 B `9b6cc074834128bb010e5b20d755501eea3b9f1b95dbb6b22b1623d21fcbb505` 一同冻结。主 Agent 验证前后两文件哈希一致，完整根/fuzz fmt、根/fuzz Clippy、workspace debug/release 与 `git diff --check`（session `30267`）全部退出 0，**debug/release 各 199 passed、2 ignored**（Git 为 75 unit＋4 attributes＋11 topology；FD-zero 嵌套子进程不重复计数）。两个跨卷用例仍未在本次配置第二卷。Astra 也核对 B 相对 `d952909…` 仅为借用清理，限定无阻断。

主 Agent 在 `9b6cc074…` 再精确执行同一 receipt 替换测试所对应的 OR→AND 变异（session `48938`，输出 `target/p0-03-reuse-identity-clean-mutant.C4YApz/mutants.out/`）：**退出 0，1 tested、1 caught**，0 missed/unviable/timeout；结束于 `2026-09-12T19:26:54.076676Z`。该版本检查位置为 `reuse_published_fixed_base:597:30`；正常 baseline 通过，变异编译成功并在实际复用的拒绝断言失败。恢复后的 `cargo-mutants-thinworkspace-p0-03-fXntQw.tmp` 哈希匹配。以上仍不代替受影响 crate 全量变异、完整组合验收或任务放行。

### Base/APFS/双 Workspace 组合验收准入（2026-09-12）

主 Agent 根据前述已接受的最小接线准备，领取既有组合验收范围；仍为 P0.b、R4。主要写入区域仅为 B 私有测试模块；主 Agent 在实验包中接入已有 P0-02 包的测试依赖。生产模块、公开 API、中性拓扑 Fixture 和其导入测试不变。

1. 只使用既有 B 私有 Fixture。先取得真实 Base 发布/复用证据，再把实际 `published_unit/tree` 交给 P0-02 `materialize_once`，明确使用 APFS clone；不启用 fallback，不用模拟 clone 或只复制预期字节代替真实物化。
2. 两个规范 WorkspaceId 对应不同托管分支；测试以既有受控 Git helper 执行分支登记、`worktree add --no-checkout`、物化、native `read-tree/refresh/status`。原临时 index 与两个 native index 分开，native 调用不得继承 Base 的 index 覆盖。登记后的 `.git` 及 admin/index 初始状态须独立观测，不能假定 `--no-checkout` 一定未创建 index。
3. 物化只允许保护已实测身份的 `.git`；每次 Receipt 均需真实 clone 成功、CoW Confirmed、六个普通文件成功物化、source/target manifest 相符且 created 不含控制项。分别核对物化前后的 Git 控制状态不变，再初始化 native index，核对三个 index 的路径/身份分离和两个 Workspace 的固定 commit/tree、初始 clean。
4. 在 A 修改并 stage、随后 commit 两个检查点，分别证明 B 的工作树和 Git 控制/index 状态、Base 单元/receipt/tree、Base 临时 index、固定主引用与树外 sentinel 保持既定事实；A 的新 commit 需以固定 commit 为 parent，最终 clean。只允许 A 的明确修改范围及对应 Git objects/refs 变化。
5. 所有操作与现场均限本次独占合成根，失败保留，不自动清理。若真实组合暴露现有实现缺陷，先保存准确失败证据并报告主 Agent，不在测试内修补输出、不扩大为通用框架。此项是已有组件组合验收，不人为制造未实现 stub 或功能 RED。

它证明 Base、APFS 物化和原生 linked worktree/index 协同，不声称同一调用链重新验证来源导入，也不替代跨卷/回滚、损坏隔离重建或完整产品兼容矩阵。执行精确组合 debug/release、完整 B 与通用门禁后交规定模型审核；当前只有准入，尚无组合结果。

测试依赖已由主 Agent 接入：`thinws-p0-materialize` 仅为 `dev-dependency`，本次 root lockfile 相对上述冻结版本只增加这一依赖边，没有新增第三方包或版本；offline metadata 仍为 36 packages。fuzz workspace 的 `--offline --locked` metadata 为 29 packages，未包含这个测试依赖。根 `cargo deny --offline --locked check` 与 `cargo audit --deny warnings --file Cargo.lock`（session `52871`）均退出 0，audit 实际加载 1243 条 advisory 并扫描 36 个依赖；deny 的既有未命中许可例外/allowance 警告保留。上述检查不代表组合测试已经实现或通过。

### Base/APFS/双 Workspace 组合结果（2026-09-12）

Sol / `xhigh` 新增 `published_base_apfs_clone_supports_two_isolated_native_workspaces`，只改 B 的 `#[cfg(test)]` 区域。两个 Workspace 分别取得 APFS `Succeeded`、`CoW Confirmed` 和 6 次真实文件 clone；源/目标 manifest 与独立固定预期一致，受保护 `.git` 未进入创建清单。三个 index 的路径和文件身份两两不同；A 暂存、提交两个检查点中，B 的工作树/Git 状态、Base unit/receipt/完整后代、Base index、固定 main 和既有 sentinel 均保持检查值，提交后仅 A 的 ref 改变且 parent 为固定 commit。

实施时曾出现三类测试观察错误：`.gitattributes` 被错误的 `.git` 前缀判断排除、将 refresh 前的 index 快照当成最终身份、原生 status 的 optional-lock 刷新干扰 B index 比较。分别改为精确组件边界、读取实际最终 index、使用 `--no-optional-locks`；这些失败不算底层组件缺陷或功能 RED。主 Agent 通读完整增量后又补强 Base 快照：从 unit 扫描而不是只扫描 tree 的后代，避免漏掉 tree 目录自身身份和模式。

最终 B SHA-256 为 `3b336ea4faa1d718f175b7dff400639e4e77447c0fd90887d5faede3f84eba9f`；生产前缀仍为 `ad08eca188f19ad1d6678ab349a633cb0eb49b63f559a3f52d23bc074759c28c`。Sol 在该版本精确 debug/release 各 **1 passed、75 filtered**，现场为 `base-checkout-82911-1789242247952065000-0`、`base-checkout-84866-1789242259546401000-0`，位于既有 `target/p0-git-base-tests/`，均保留。

主 Agent 冻结后独立执行根/fuzz fmt、根 Clippy、带 `--cfg fuzzing` 的 fuzz Clippy、完整 workspace debug/release 和 `git diff --check`（session `79897`），最终退出 **0**，文件哈希前后一致。两种 profile 各 **200 passed、2 ignored**，其中 Git 为 76 unit＋4 attribute＋11 topology，包含完整 B 44 项及组合测试；子进程中的 FD-zero 测试不重复计数。两项跨卷测试本次未挂载专用第二卷而未执行，不改写先前双卷证据。规定模型的本切片限定审核进行中；此结果不表示 P0-03、P0.b 或阶段放行，也不覆盖损坏隔离重建、来源导入与物化同链重验或剩余兼容矩阵。

随后 Astra / `xhigh` 指出一项观察证据缺口：`write-tree` 可能更新 native index 的 cache-tree，而 helper 在它之后才拍 index 快照，`status --no-optional-locks` 不约束该命令。主 Agent 采纳最小补强，仅在 A stage/commit 后、任何 B Git 快照前各增加一次原始 B index 设备/inode/模式/完整字节比较。新 B SHA-256 为 `a00ef6e3a02c40498067696bd9646a079e5411f877f04c70a6c6fe2edd821011`，生产前缀未变；Sol 精确 debug/release 各 1 passed，fmt/diff 通过。主 Agent 通读八行增量并独立精确复验（session `8836`），同样各 **1 passed、75 filtered、退出 0**，前后哈希一致，保留现场为 `base-checkout-35391-1789242562310000000-0` 与 `base-checkout-36547-1789242569063245000-0`。

Astra 最终限定结论：原始 index 观察缺口已关闭，组合增量无新增阻断；Reviewer 为只读审核，未自行运行测试。这不是已证实的产品故障，也不是全 P0-03 Approve。上面的全 workspace 200 项仍准确归属 `3b336…`；`a00ef…` 仅新增八行测试断言，本轮完成精确增量复验，后续提交/收口仍须运行适用完整门禁。

### 已登记 Base 的显式隔离实验准入

此项补齐前述磁盘发布准入第 5 条，不改变正常发布/复用失败时的安全停止政策。主 Agent 采用 Astra / `xhigh` 的只读边界意见：复用既有 `PreparedFixedBase` 持有的目录 FD、登记身份和父路径报告；不新增 Recovery/Published 状态类、产品 Port、任意既存 Base 接管入口或自动隔离 fallback。

1. 新增明确命名的 P0 实验入口 `run_fixed_base_quarantine_experiment`，只接受现有 `FixedCheckoutRequest`。它仅对本次新建并成功发布/复用的固定 Base，在持续持有 Prepared 的前提下，构造一次可追溯损坏、取得实际复用拒绝，然后调用真实的私有隔离操作。可将已核对身份的 receipt 以 `NOREPLACE` 移到同父根固定保留名，构造 receipt 缺失；原文件也保留，不采用递归删除或未知路径。普通 `run_fixed_base_experiment` 不增加故障或隔离行为。
2. 入口成功返回既有 `FixedBaseEvidence`、实际隔离路径和原始拒绝 cause；前者明确是损坏前发布/复用的历史证据，不称隔离后的 Base 仍健康。失败使用既有结构化 error，保留 Git 命令及准确的系统调用 operation/path/errno；移动后的核验错误必须标明所处操作，不声称尚未移动或自动回滚。此窄实验结果不成为 P1 API 或跨进程恢复格式。
3. 隔离操作必须自行重验 parent/report/held FD；用同一父 FD、no-follow 查询当前 Base 名称，确认目录类型及其身份仍匹配登记单元和持续持有的原 FD，再整体 `NOREPLACE` 移到同根受控单组件隔离名并核对移动后身份。损坏 receipt、名称或内容吻合不能授权归属；不重验发布前已过期的 staging/tree 路径报告。
4. 重建由测试在隔离成功后显式调用已有正常 Base 入口，使用同一受控根中的新空 content/witness、新缺失 index 和原可信 commit/tree；不增加重建调度器。验收包含：成功重建保持 BaseId、取得新 unit 身份和有效 receipt；未知替换单元拒绝；空/非空隔离目标冲突均不覆盖；隔离成功而重建失败时，旧隔离物、新失败现场和无关对象保留。负例调用与显式入口相同的真实隔离 helper，不在测试里自行实现安全判断加 rename。
5. 由原 Sol / `xhigh` 编写 B，主 Agent 集成必要 re-export。先取得新操作的可解释 RED，再实现并执行精确 debug/release、完整 B、通用门禁和相关变异；由 Astra / `xhigh` 审核。它只证明独占合成根中的确定性步骤，身份检查加 rename 不是按 inode 条件执行的原子移动，不宣称任意同 UID 并发替换安全、强杀恢复、断电持久性或完整 P1 隔离政策。

本节为编码准入，不是已实现或已通过；P0-03 保持 In Progress。

### 基线短名与完整 OID 验证准入

主 Agent 核对用户手册 §4.4 与技术栈 Git 章节后确认：当前 `ManagedTopologyRequest.base_ref` 虽为字符串，实际预检只允许完整来源/tag ref；已有测试不能证明短名歧义或完整 OID 的入口行为。本切片在现有入口内部完成有限选择，不以测试内另写选择器或单独调用 Git 原语代替验收；不新增公开请求类型、通用解析框架、产品错误码或 CLI。

- 输入为允许的完整 ref、短名或恰好 40 位十六进制 OID。完整 OID 接受十六进制大小写并规范化为小写，符合手册的输入/输出表述；只把这类完整输入作为 OID 交给 Git。其他输入不得借用 Git DWIM、缩写匹配或 revision expression。39/41 位等看似 OID 的文本不能按 OID 解析，但若它确为允许命名空间中的完整短名，不额外发明“禁止十六进制分支名”的产品限制。
- 短名只精确枚举 `refs/remotes/thinws-source/<name>` 与 `refs/tags/<name>`，按 ref 数量作零候选、唯一候选、多候选判断；同 OID 的 branch/tag 也仍是两个候选。私有、origin 和 Workspace ref 不加入枚举。完整 ref 仍须通过现有允许命名空间和 Git ref-format 校验；最终仅将已唯一确定的完整 ref 或完整 OID 交给既有 commit/tree 解析。
- 用现有合成 Fixture 取得真实 RED：唯一来源短名和已导入完整 commit OID 在旧入口被预检拒绝。实现后补真实成功/失败矩阵：唯一来源分支、唯一 tag、同名 branch/tag（相同及不同 OID）、不存在短名、完整大小写 OID、不可用的缩写/不存在完整 OID及已有 tree/blob OID；保留原 revision-expression 拒绝测试。正例 OID 来自明确导入的可达 commit，不依赖 fetch 偶然带入的对象；这不新增 OID 可达性产品政策。
- 失败使用既有结构化 cause/stage/command evidence，区分不合法输入、候选不存在、候选歧义和非 commit；不把实验 detail 当成已经完成手册的 CLI 错误码映射。验证来源、已发布或保留 staging、refs、index 与 worktree 的实际边界，失败不产生部分可用 Workspace。

原拓扑 Sol / `xhigh` 负责 `managed_topology.rs` 及同名集成测试；主 Agent 管理共享文件。先冻结最小 RED，经主 Agent 核验后再 GREEN；之后执行专项、通用门禁和变异，由 Astra / `xhigh` 审核。此项未实现前不作短名/OID 契约已覆盖的声明；纯校验若需拆出供实际 fuzz 复用，必须报告真实双调用落点，不先建平行 parser。

### 显式隔离与基线选择的真实 RED（2026-09-12）

两位 Sol / `xhigh` 分别完成上述切片的最小测试，主 Agent 阅读代码后独立复验；以下都是预期失败，不是门禁通过。

| 切片与冻结输入 | 主 Agent 实际结果 | 失败前已证明的范围 |
|---|---|---|
| 唯一来源短名、完整 commit OID；topology 测试 `58e3470d295dd9bb1e61b7a387b5511d181cb17c03fefde8f75a1cb55dda9299`，生产仍 `f80ee84e…` | 精确 debug sessions `85876`、`2838` 均编译后退出 **101**，各 0 passed / 1 failed / 12 filtered | fixture 完整，main 无同名 tag，完整 OID 实际为 commit；旧入口精确在 Preflight 以 namespace 限制拒绝，commands 空，来源快照不变且四个目标仍缺失 |
| 已登记 Base 隔离/显式重建；B `5ba7065896c103e024d0e63cd8d4fe93f0f8e93bdfff6288493011f90bd2bd01` | 精确 debug session `47004` 编译后退出 **101**，0 passed / 1 failed / 76 filtered；停于 `fixed Base quarantine experiment is not implemented` | 已真实 checkout、发布、复用并保留 20 条成功 Git 命令；用独立预期 receipt/manifest 验证遗留 Base 健康，保护对象与 oracle 不变，尚无保留 receipt 或隔离路径 |

拓扑 RED 的主 Agent 现场为 `managed-topology-86157-1789242905833183000-0`、`managed-topology-86158-1789242905833496000-0`；隔离 RED 的 oracle/被测现场为 `base-checkout-93308-1789242952307254000-0`、`base-checkout-93308-1789242955578056000-1`，均在本工作区 `target/p0-git-base-tests/` 并保留。各自被测文件前后哈希一致。拓扑编译时 B 新入口尚未 re-export 的 dead-code warning 不构成行为 RED；主 Agent 随后仅接入已准入实验入口的 re-export，`lib.rs` 为 `c84666ba5d18bad9f6c26f8901237118db38accec37cb7f64cdc1fbb9febbe4f`，没有修改 CLI 或其他类型的可见性。

主 Agent 已分别批准在既定边界内进入 GREEN；隔离与基线选择此时尚未实现完成，全任务门禁和放行仍未完成，不以之前冻结版本的绿灯覆盖当前预期红状态。

### 显式隔离与基线选择 GREEN、剩余放行条件（2026-09-12）

两位 Sol / `xhigh` 完成已准入操作。首个组合冻结为 B `bf803eea2b67cd2f846e1dbed6b8267655bef285d7949397209ec7ede6b263ea`、topology `fd63540a283aa588dd294636457c22210d4bbe429cd27219452420f808746c51`、topology IT `a42d8dd33eca77d76a8ca5bdfc30a9e7fd6244eef76710adc7669f7b11838a82`，`lib.rs` 仍为 `c84666ba…`。主 Agent 独立执行根/fuzz fmt、根 Clippy、带 `--cfg fuzzing` 的 fuzz Clippy、完整 workspace debug/release 和 `git diff --check`（session `4949`）：全部退出 **0**，源码前后哈希一致；两种 profile 各 **219 passed、2 ignored**。两项跨卷测试因本轮未挂载专用第二卷未执行；不覆盖后续修改或代替阶段门禁。

显式隔离的五例真实验收包括成功重建、未知替换单元拒绝、空/非空隔离目标冲突、隔离后重建发布失败；均检查原 receipt、旧 index/witness、隔离物、冲突目标和适用失败 staging 的保留。Astra / `xhigh` 只读限定审核确认这些边界，指出两处移动后父目录检查会丢失已完成移动的诊断上下文。Sol 仅在这两处按类型补充上下文：Validation 标明 receipt/unit 移动已完成，FileSystem 保留原 source/errno 并补受控父路径及操作名，Probe/其他原因原样保留。修后 B 冻结为 `d7035a864f0fed8ea66b863e7ea4ce70949d943a2e1d3ec459e34908f9effa6c`；作者精确 debug/release 各 5 项、完整 B 各 49 项及 Clippy/fmt 通过。连续移动后检查的失败分支未取得确定性真实故障回归；未以新 hook、竞态或伪 RED 补数，不将成功路径测试当作该失败分支的执行证据。Astra 最终窄复核确认代码诊断缺口关闭，同时保留上述未执行边界。

基线选择的真实 IT 扩至 25 项。完整 ref 必须精确存在，短名按两个允许命名空间的 ref 数量判歧义（同 OID 仍歧义），完整 OID 规范化后必须等于实际解析的 commit。canary 区分三种实际结果：缺失完整 ref 可被 Git 通过嵌套 tag 名回退解析，精确存在检查阻止了此行为；不存在的 40 位 OID 加同名 tag 在本机 Apple Git 上仍为 GitExit，不声称它证明了 OID 等值防线；真实 annotated-tag 对象 OID 可被 Git peel 成 commit，但入口以 OID 不相等拒绝，后者才是该防线的直接反例。

随后批准等价提取真实词法分类器到现有 `topology_validation.rs`，生产路径与既有 fuzz target 共用；Git ref-format、导入候选计数和对象解析仍留在真实 I/O 路径，不创建平行 parser 或新 target。冻结为 topology `4faa682b99b8af8b14690374ec120ee259be3532bdd0867f671e46c59bf9ae0b`、validation `6b4dddc8881a49b25c170108d58dbcbb5568aadc9925a9ec956307b10cab422c`、harness `9d1dcde2e9fd14b9ad4ecde62108ac9c2c59e10c6770ea08604f71ceb2f7c2b6`，IT 仍为 `a42d8dd…`。作者重构前决策表通过，修后定向 debug/release 各 2 项、完整 IT 各 25 项及 Clippy/fmt 通过；未制造行为不变重构的伪 RED。主 Agent 通读增量，Astra 对生产接线、全部新增 IT 和独立 fuzz oracle 完成限定审核，无剩余局部代码阻断；不作全任务 Approve。

主 Agent 对上述修后五份冻结文件取得以下实际终态，前后哈希一致：

| 验证 | 本轮结果与边界 |
|---|---|
| 根/fuzz fmt、根/fuzz Clippy、完整 workspace debug/release、diff-check；session `9712` | 全部退出 **0**；debug/release 各 **221 passed、2 ignored**（Git 83 unit＋4 attribute＋25 topology，materialize 60，probe 49；FD-zero 子进程不重复计数）。两项跨卷测试仍因专用卷未挂载而未执行 |
| `cargo +nightly-2026-08-14 fuzz run thinws_git_topology_validation /tmp/thinws-p0-03-selector-corpus.GSeNfi -- -max_total_time=60 -timeout=5 -max_len=4096 -print_final_stats=1`；session `15445` | 退出 **0**，**2,184,693 runs / 61 秒**，峰值 RSS 531 MB，未发现 crash/timeout，artifact 为 0；报告 corpus 201 项，磁盘保留 200 项。启动出现 atos symbolizer 警告，运行继续至正常终态；不称诊断环境完全无警告，不代替长预算 |
| `cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/topology_validation.rs -F 'classify_base_ref' --leak-dirs --timeout 60 --jobs 1 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-selector-mutants.RVW9vh -C=--locked -- --lib topology_validation::tests::base_ref_classification_is_lexical_and_normalizes_full_object_ids -- --exact`；session `29645` | 退出 **0**，4 tested＝**3 caught＋1 unviable**，0 missed/timeout；baseline 通过。三个逻辑变异编译后在分类断言失败，`Default` 替代因缺少 trait 为 unviable，不计 caught。保留的 `cargo-mutants-thinworkspace-p0-03-tw4yLP.tmp` 恢复源码哈希匹配；仅是分类器精确专项，不是 IO/隔离模块或全 crate 变异 |

剩余退出条件不是新增产品范围：原验收表中的保留项冲突、submodule、sparse 与不兼容转换仍缺少实际拒绝操作的充分实验。现有固定 checkout、属性类型查询及 APFS witness 不能替代这些拒绝证据。Astra 另确认《技术栈》Git 章节及详细设计的 `ident` 表述与用户手册 §4.5 的“`ident` 展开”存在边界差异；本记录开头“声明存在但当前不发生转换”的处理仍待维护者决定。未决定前不静默改变公开兼容政策，不把固定 `.gitattributes` 字节不匹配或 checkout 后 manifest 不匹配当作准确的不兼容预检。全任务变异/fuzz、剩余真实矩阵、正式 ADR、候选 CI 和最终审核均未完成；P0-03 保持 In Progress，P0-04/P1 不因此获准提前实施，条件放行尚未成立。

### 新隔离逻辑专项与兼容边界复查（2026-09-12）

继续前重新核对工作区 HEAD `5eca407…` 和上述 d703/4faa/6b4/a42 冻结文件，未变化。当前全 Git crate 的变异枚举为 450 项；Astra / `xhigh` 对实际 `aarch64-apple-darwin`、rustix 1.1.4/libc 0.2.189/bitflags 2.13.2 和 macOS 26.1 SDK 重核 17 个 OR→XOR 节点，主 Agent 独立核对 SDK 数值及表达式。各表达式非零 flags 两两无重叠，OR 与 XOR 位值严格相同；共享创建的 NONBLOCK 必须用已核 B b03 的五节点证明，不沿用旧 A 四节点证明。主 Agent 在 `.cargo/mutants.toml` 按当前精确位置迁移并补入这 17 项，责任人为主 Agent；位置、表达式/操作数、平台/ABI、依赖映射或位运算实现变化时失效。再枚举为 **433 项，XOR 0 项、OR→AND 17 项仍全部保留**。这是经审核的等价范围核对，不是 433 项实际执行通过，也不排除身份或错误处理变异。

新隔离四个函数的实际专项命令为 `cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/base_materialization.rs -F 'in (run_fixed_base_quarantine_experiment|require_registered_fixed_base|retain_registered_receipt_for_quarantine|quarantine_registered_fixed_base)$' --leak-dirs --timeout 60 --jobs 2 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-quarantine-mutants.9S8ysA -C=--locked -- --lib base_materialization::tests:: -- --nocapture`。session `22389` 终态 **退出 2，20 tested＝15 caught＋5 missed，0 unviable/timeout**，结束于 `2026-09-12T20:49:32.750902Z`，baseline 通过；两份保留副本 `6dv3vz.tmp`、`YNn3Sh.tmp` 的恢复 B 哈希均匹配 d703。存活项为登记单元/移动后 receipt/移动后 unit 三处 OR→AND（710、834、925 行），以及两处移后缺失查询的 NOENT guard→true（800、898 行）。后两者可能吞掉其他 errno，不得按等价处理；三处逻辑项须检查与后续完整 stat 核验的关系，不能仅因难测而宣布等价。主 Agent 已交 Astra 作限定根因核对，尚未修正或获准放行。

Sol / `xhigh` 只读复查还确认 sparse 条款存在独立的产品语义缺口：用户手册 §4.5/详细设计 §11 只写拒绝 sparse checkout，未说明拒绝的是本机输入来源工作树，还是平台生成的 Workspace。详细设计 §6.1 又将同一 common directory 的不同 linked worktree 归为同一来源，因此“检查当前输入工作树”与“检查同仓库任一工作树”会产生不同接入结果。Git 的 sparse 配置、模式与 index 状态属于工作树，不能从所选 commit/tree 判断；远程/bare 来源也不具有本机来源工作树状态。主 Agent 对照[Git sparse-checkout 文档](https://git-scm.com/docs/git-sparse-checkout)的 worktree-specific config 说明核验了这一事实，但未运行真实 sparse fixture，不冒称实验完成。现有 source snapshot 未包含 config.worktree、sparse pattern 与 skip-worktree 证据。此语义须由维护者明确后再同步权威文档和实施相应验收；暂不擅自实现任一来源拒绝政策，也不把完整对象导入当作已满足 sparse 拒绝条件。

Astra 对上述五项终态完成只读根因核验后，主 Agent 准入以下局部去重与测试补强：只删三段已由后续完整 stat 比较包含的 if，保留 fstat、当前名称的登记类型/身份核验、全部 `require_same_stat` 链与移动后上下文；部分错误返回位置/detail 会变化，不称位级不变或将原逻辑变异排除。两处重复的 no-follow statat/ENOENT 判别提取为一个私有 `statat_if_present(parent, name) -> io::Result<Option<Stat>>`，两个原操作真实调用它：成功 Some、仅 NOENT 为 None、其他原 Errno 保留，由原调用点维护各自 Validation/FS operation/path。用缺失项、既存项、dangling symlink 和实际不可 search 的受控目录补测试；权限错误先以真实独立调用确认，再核对共享函数。该项是既有操作的去重和覆盖补强，不制造行为重构的伪 RED，不加新状态/政策模型、测试 hook 或 unsafe 关 FD。验证只证明真实共享查询边界和接线，不声称 I/O 错误已在完整 rename 后确定发生。Sol / `xhigh` 编码，Astra / `xhigh` 审核；修后须重跑相关变异及通用门禁，当前准入不是执行完成。

同一冻结 4faa 的新增 selector I/O 判断另执行 `cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/managed_topology.rs -F '^experiments/p0/git-base/src/managed_topology\.rs:(400|402|416|419|463):' --leak-dirs --timeout 90 --jobs 2 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-selector-io-mutants.UPhLVn -C=--locked -- --test managed_topology -- --nocapture`。session `48686` **退出 0，6 tested＝5 caught＋1 unviable，0 missed/timeout**，结束于 `2026-09-12T20:54:29.966282Z`，正常 baseline 25 项通过。五项存在性/名称相等/OID 相等判断变异由实际 IT 检出；let-chain 的 AND→OR 因 Rust 不接受该语法而 unviable，不计 caught。保留副本 `kmWqSD.tmp`、`dNXYEE.tmp` 的恢复 topology 哈希均匹配。此范围与前述分类器四项专项互补，仍不是完整 crate 收口结果。

### 不依赖转换政策的拒绝验收准备

Sol / `xhigh` 的只读方案把三类问题分开，主 Agent 不将它们混成一个通用兼容框架：根级 `.git` 保留项冲突可尝试由既有 `read-tree` 明确拒绝；大小写与 NFC/NFD 各一组冲突可尝试由无 force 的 `checkout-index` 明确拒绝，但必须以同一实际 APFS witness 为 Aliased 为前提；gitlink 则需在检出前识别真实 tree 中的 160000 项，不能靠后验固定 manifest 不匹配冒充精确拒绝。此时仅有代码/文档依据，尚无这三组实际执行证据。

主 Agent 准入前两类的测试补充，实施须在当前隔离修正冻结复验结束后串行进入同一 B 文件：使用既有私有 fixture，保留合法固定 attributes，独立核对所选 commit/tree 的 raw path、mode、OID；调用真实 Base 入口，只接受所期待的 GitExit 操作（read-tree 或 checkout-index），检查 index/lock、partial content、witness、无发布 Base/receipt 及保护对象的真实状态。冲突两项使用不同字节和 OID，验证留下的内容而非仅目录存在；不得因 Distinct 文件系统能表示这对名字而制造拒绝政策。既有行为若首次测试即满足条件，可以直接通过；若未满足则记录真实缺口并停止扩项，不为补测制造 RED，也不自行加入 Unicode 归一化模型、预检器或自动回滚。gitlink 的单用途字节解析与既有 fuzz 接线须另行完成具体准入，当前未授权据此新增 parser；sparse 与关闭转换属性的产品政策仍暂停待决定。

### 隔离查询去重后的复验结果（2026-09-12）

Sol / `xhigh` 完成准入修正，B 冻结为 `61d2ddf10e0f8536d2385e81bbcc43c7d6bcc12a4d1a964dc704a33b93b7143b`，生产前缀为 `f8232c58deb46da6013048648b54671daaae9a5a77b932591c4217a94804e8a2`。作者两个查询测试精确 debug/release 各 2 项、完整 B 各 51 项、Clippy/fmt 均通过。Astra 通读相对 d703 的完整增量，确认只做已准入去重、真实双调用查询和两项测试，无局部阻断；不声称完整 rename 后曾注入故障。flags 的操作数/列未变，主 Agent 将 14 个 B XOR 精确行锚迁移至 1301、1347、1363、1975–1979；topology 三项仍在 815。新候选枚举 **423 项，XOR 0、OR→AND 17**，没有新增安全判断排除。

主 Agent 对该冻结独立完成根/fuzz fmt、根/fuzz Clippy（fuzz 带 `--cfg fuzzing`）、完整 workspace debug/release 和 diff-check，session `89111` **退出 0**，哈希前后一致。两种 profile 各 **223 passed、2 ignored**；Git 为 85 unit＋4 attribute＋25 topology，materialize 60、probe 49，不重复计算 FD-zero 子进程。两项跨卷因专用卷未挂载仍未执行。

相关变异分两次互补执行，均在独立保留副本上，不以过滤后的运行冒充整个 crate 门禁：

| 范围、实际命令差异 | 终态 |
|---|---|
| 沿前述四函数命令，在 `-F` 名单加入 `statat_if_present`，输出改为 `target/p0-03-quarantine-shared-mutants.nrxoux`；该 `in …$` 筛选只覆盖函数内 operator/guard，不包含函数体替代 | session `44628` **退出 0，8 tested、8 caught**，0 missed/unviable/timeout；结束 `2026-09-12T21:04:28.658687Z`。原 NOENT guard→true 在真实权限例中返回 None，恰由 `expect_err` 检出，其余 50 项仍通过，不是 fixture 提前失败 |
| 同包/文件/测试参数，`-F 'replace (run_fixed_base_quarantine_experiment\|require_registered_fixed_base\|statat_if_present\|retain_registered_receipt_for_quarantine\|quarantine_registered_fixed_base) ->'`，输出 `target/p0-03-quarantine-body-mutants.l6T2Wy` | session `6389` **退出 0，6 tested＝3 caught＋3 unviable**，0 missed/timeout；覆盖前一筛选遗漏的全部函数体替代。三项 Default 构造无法编译，不计 caught |

两次合起来覆盖这五个函数当前枚举的全部 **14 项**，其中 **11 caught、3 unviable**；原 d703 的 20 项筛选同样只是 operator/guard 专项，其历史失败结果不改写。四份保留恢复副本 `mSOYJw.tmp`、`JtgCVH.tmp`、`CQDGsk.tmp`、`9WeS5I.tmp` 的 B 哈希均匹配 61d。根门禁完成后才由原 Sol 串行领取上一节的路径冲突测试补充，限定只改测试区、保持生产前缀；该新增测试工作不自动继承本节验证结论。P0-03、P0.b 及整个 P0 仍未放行。

### 跨卷环境缺口补验（2026-09-12）

主 Agent 复用 P0-02 保留的独占 128 MiB APFS 镜像 `/private/tmp/thinws-p0-materialize.OC4JPT/cross-volume.dmg`，挂载前核对父目录为当前用户所有的 0700 目录、镜像为普通文件；挂载后核对镜像关联、APFS、可写、owners enabled 和实际卷 UUID `E4C7D91A-EF7B-476B-9BEB-946B53FEE500`。没有操作同时存在的 Docker 镜像。

session `87377` 实际执行以下两个精确测试，每个分别以 debug 与 `--release` 运行，命令统一带 `--locked`，并设置 `THINWS_P0_CROSS_VOLUME_ROOT=/private/tmp/thinws-p0-materialize.OC4JPT/mounted`：

- `cargo test --locked -p thinws-p0-probe --test probe configured_real_cross_volume_distinguishes_clone_from_full_copy -- --exact --ignored --nocapture`
- `cargo test --locked -p thinws-p0-materialize --test same_volume actual_cross_volume_clone_returns_exdev_without_target_then_explicit_copy_succeeds -- --exact --ignored --nocapture`

全部退出 **0**，debug/release 各 **2 passed、0 ignored**。探测例证明真实不同卷身份以及克隆与 Full Copy 能力区分；物化例取得非注入的真实 `EXDEV`、目标回到基线，再显式 Full Copy 成功、manifest 正确且 CoW 为 NotUsed。此为前述全 workspace 运行中两项 ignored 的单独补验，不改写历史计数，也不代表新 Git 路径测试或整个 P0 已通过。测试按既有身份核验清理了自行创建的可重建合成夹具；主 Agent 正常卸载该精确挂载点，随后确认镜像不再挂载、原镜像文件仍保留，未删除其他历史现场。

### 三组实际路径拒绝补测（2026-09-12）

Sol / `gpt-5.6-sol` / `xhigh` 在准入范围内仅增加测试与 fixture，B 整文件冻结为 `da82bf2ef0f4bcfcb267b3566edce10da0fd8407b50b4a710a18f8d96dea44c7`；生产前缀保持 `f8232c58…`，没有新增冲突检测器、产品限制或公开错误码。真实入口的既有行为首跑即满足以下断言，没有产品 RED：

| 合成 tree | 实际拒绝与证据边界 |
|---|---|
| 根级 `.git` 普通文件，保留合法固定 attributes | GitExit `populate temporary index` / 128，stderr 指向 invalid path；此前命令成功，index/lock 缺失、content 空、无 Base/receipt，保护对象和拒绝后现场保持 |
| `CaseCollision` / `casecollision`，不同原始 bytes/OID | 同目标 APFS 卷的独立 oracle 实测 Aliased；GitExit `checkout fixed tree` / 1，第二项 already exists；保留非空 index、lock 缺失，第一项内容未被第二项覆盖，固定 payload 与真实 witness 可核对，无 Base/receipt，保护对象不变 |
| `a\u{0308}` / `ä`，不同原始 bytes/OID | 独立 oracle 同样实测 Aliased，实际拒绝与保留断言同上；只证明这一组 NFC/NFD 实例，不泛化为完整 Unicode 模型 |

三个 fixture 在调用入口前均用独立 `ls-tree` 清单核对 raw path、mode、OID，并以 `cat-file` 对照固定 blob 字节。别名测试依赖实测 Aliased；若目标报告 Distinct，测试明确以“不适用”的环境错误停止，不把能表示该名称对的文件系统判为产品应拒绝，也不返回虚假的通过。当前没有执行 Distinct 环境测试。

作者精确 debug/release 各 **3 passed**、完整 B 各 **54 passed**，fmt/Clippy/diff-check 通过。主 Agent 通读相对 61d 的全部 diff，独立执行根/fuzz fmt、根/fuzz Clippy（fuzz 带 `--cfg fuzzing`）、完整 workspace debug/release 与 diff-check；session `20613` **退出 0**，da82/4faa/6b4/a42 前后哈希一致。两种 profile 各 **226 passed、2 ignored**（Git 88 unit＋4 attribute＋25 topology，materialize 60、probe 49，不重复计算 FD-zero 子进程）；这两项 ignored 已由上一节同轮独立跨卷补验取得各 profile 两项通过，不改写本次普通测试输出。此测试-only 增量没有再次运行全 crate 变异或长预算 fuzz，不继承此前局部变异为全任务门禁。

Astra / `gpt-6-astra` / `xhigh` 核对完整 diff、冻结哈希和实际接线后给出限定结论：本增量无阻断。Reviewer 未自行运行测试或变异；后续快照相等只证明拒绝后观测不改写现场，不能扩大为 checkout 前后整个根目录不变。Distinct 分支属于此次 Aliased 环境实验的不适用提示，不成为产品拒绝 Distinct APFS 的要求，也不是该环境已执行的证据。本轮公开契约未改，未提交、push、合并或打 tag。

当前条件放行仍未成立：submodule、sparse、转换兼容的真实验收及未决政策、受影响 crate 全量变异/统一 fuzz 收口、正式 ADR、候选 CI 和全任务最终审核尚未完成。P0-03 保持 In Progress，P0-04 及 P1 仍按实施计划依赖等待；本轮限定审核与测试通过不代替整个 P0-03、P0.b 或 P0 的放行。

### Gitlink 拒绝切片准入（2026-09-12）

本项落实用户手册 §4.5 与详细设计 §11 已有的 submodule 拒绝要求，继续归属 P0-03/P0.b/R4，不处理未决的 sparse 或转换属性政策。主 Agent 核对当前 da82、checkout validation `f3f144…` 和既有 fuzz target 后准入以下最小行为，不新增产品 Port、兼容框架、CLI 错误码或依赖。

- 在实际 Base 入口已核对固定 commit/tree 后、读取固定 attributes 和创建临时 index 前，使用现有受控 Git 调用读取所选完整 tree 的递归 mode/raw path 列表；发现 160000 gitlink，返回既有结构化 `UnsupportedEntry` 并保留准确 raw path 和命令证据。不得初始化、fetch 或检出 submodule，也不以 `.gitmodules` 是否存在替代 tree 检测。[Git submodule 表示](https://git-scm.com/docs/gitsubmodules)、[ls-tree 的 NUL/format 语义](https://git-scm.com/docs/git-ls-tree)只作为命令依据，仍须取得本机真实结果。
- 首先只在 B 的测试区准备有合法固定 attributes、确切 gitlink 的私有 fixture，独立核对 commit/tree、mode/OID、保护对象；要求入口准确拒绝于 index/content 写入前，未产生 Base/receipt。旧入口若只在检出后报固定 manifest/未知内容错误，这是明确的行为缺口，不能把它称为精确预检通过。Sol 先冻结最小 RED，主 Agent 独立复验后才进入 GREEN。
- GREEN 使用 `ls-tree -rz --full-tree --format=%(objectmode)%x09%(path) <tree_oid>`；在现有 `checkout_validation.rs` 放置一个由生产与既有 fuzz target 共用的单用途纯解码函数。它只处理 NUL 记录、六字节 mode、固定 TAB 分隔与非空 raw path，不解析 `.gitattributes`、不新增路径归一化或文件系统政策；未知 mode/损坏记录应作为校验失败，不可当作无 gitlink 放行。普通文件、可执行文件与 symlink 不是 gitlink，空列表本身没有 gitlink，后续固定 checkout 约束仍有效。
- 真实测试至少覆盖根级和嵌套 gitlink，明确所指 commit 在父仓库存在/缺失的观察，验证缺失子模块对象不触发外部获取；纯测试与既有 fuzz 接线覆盖记录边界、非法 mode、空路径、原始非 UTF-8/TAB/LF 路径、多记录及无 gitlink 输入。检测只用于拒绝，不能把返回的 raw path 当成本机文件操作目标。
- Sol / `gpt-5.6-sol` / `xhigh` 编码，Astra / `gpt-6-astra` / `xhigh` 审核；主 Agent 管理记录、共享质量配置、独立通用门禁及专项变异/fuzz。失败现场继续保留，准确记录进入拒绝点前已经产生的 filesystem witness，不宣称整根未写入。该项准入不代表实现、验收或整个任务已经放行。

主 Agent 对 Sol 冻结的 RED `fd8d319473aeb47f55044fb3e86d52efffe19c33ff64e585dde7721f3698bfb9` 通读测试和 fixture 增量，并独立运行 `cargo test --locked -p thinws-p0-git-base --lib base_materialization::tests::root_gitlink_is_rejected_before_fixed_checkout_writes -- --exact --nocapture`（session `85352`）：编译成功后 **退出 101，0 passed、1 failed、88 filtered**，生产前缀仍 `f8232c58…`。gitlink 指向父仓库内实际存在的独立 commit，raw tree mode 为 160000，attributes 和 blob 均正确；旧入口成功完成 `read-tree` 与 `checkout-index`，随后返回 `UnknownContentEntry(linked-dependency)`。现场实际存在 index、固定内容与 gitlink 目录，lock 缺失、witness 已产生，无 Base/receipt，保护快照不变。这是拒绝时点与类型未落实的真实 RED，不是编译或 fixture 错误；作者现场 `base-checkout-95161-1789248790951871000-0` 与主 Agent 现场 `base-checkout-4890-1789248841402384000-0` 均继续保留于本工作区的 `target/p0-git-base-tests/`。

主 Agent 据此批准既定 GREEN：B 和 pure validation 由原 Sol 串行修改，既有 fuzz harness 的接线交另一位 Sol 独立负责，两者不交叉写文件。纯函数固定为 `first_gitlink_path(&[u8]) -> Result<Option<&[u8]>, &'static str>`，完整验证列表后报告首个 gitlink 的原始路径；损坏记录即使位于已发现 gitlink 之后也返回校验错误，不把截断或非法 mode 的输出视为可信证据。允许的四种 leaf mode 为 Git 实际递归列表中的 100644、100755、120000、160000，不额外建立通用 tree/path 模型。

### Gitlink 实现与专项复验（2026-09-12）

两位 Sol / `gpt-5.6-sol` / `xhigh` 分别完成 B＋纯函数和既有 fuzz harness，最终冻结 B 为 `c45d4df0f598de78ac1cc75b046dadb1fa5ecb105ead05ec16dd09cc9946748c`，纯函数文件为 `4a9fd7625d20d61cb17c7d86d58f272d936b4be7aa8a53a6b1b0014fad3d66bf`，harness 为 `b01fa7dae84d5f51aebda1cf38c28c0614ba236c2e550f979c6d678db5650b04`。B 生产前缀为 `14bd0fd2f49f066f122dec55033f643dc83aa88ecb48d9d79676ec1f615ef3b5`，纯函数生产前缀为 `3da9f176b744cd5f0943d6b59c0af713df5da46a5a8b3d7cd6cd14e07960738a`；前缀均指首个 `#[cfg(test)]` 之前的完整文本。

真实入口在 tree OID 核对后、attributes/index 操作前读取完整递归列表，使用原 Git wrapper 与原错误类型。根级 gitlink 指向实际存在的 commit，嵌套 `nested/linked-dependency` 指向经独立 `cat-file -e` 实测退出 1 且两流为空的缺失对象；两例均有完整 mode/OID map 和独立固定 raw listing，不以部分包含断言代替。准确拒绝发生在六条成功命令后：index/lock 缺失、content 空、无 Base/receipt，保护对象不变；witness 已存在，不能宣称入口前后整根未写入。没有 submodule 初始化、fetch 或 checkout。纯测试包含一字节合法路径、非 UTF-8/TAB/LF 路径、缺失 NUL、非法 mode、空记录/路径及首个 gitlink 后的坏尾部。harness 实际复用生产函数，以独立 split/首 TAB/range oracle 和 16 类结构化输入检查结果、返回字节及借用位置；旧三类校验仍保留，纯内存输入上限仍为 4096。

作者取得精确 gitlink debug/release 各 **3 passed**、纯模块各 **4 passed**、完整 B 各 **56 passed**，定向 Clippy、fmt、diff-check 退出 0。纯函数作者另有编译成功后的本地 RED：旧桩返回 `Ok(None)`，非 UTF-8 gitlink 预期为 `Some`，测试退出 101；此项是作者执行，不冒充主 Agent 独立执行。

冻结期间曾发生一次证据版本偏差：作者在最后 Clippy 前宣布纯文件 `15a40e0a66dc7f408de40b8b77b1eefe0382dd1b22e983d5f9163db389fca9cc` 已冻结，随后为 `clippy::octal-escapes` 仅将两处测试字面量的 `\0` 改为 `\x00`，形成最终 4a9。主 Agent 对最终文本仅逆变换这两处后取得精确 15a 哈希；Astra 独立 diff 确认字节值等价、生产前缀不变。以下原始变异运行明确对应 15a，不改写成实际使用 4a9；fuzz 不编译这两处 `cfg(test)`，生产前缀与 harness 未变，不能据此声称其整文件冻结版本未发生变化。

| 主 Agent 实际专项 | 终态与范围 |
|---|---|
| `cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/checkout_validation.rs --leak-dirs --timeout 60 --jobs 1 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-gitlink-validation-mutants.eWEQ1e -C=--locked -- --lib checkout_validation::tests:: -- --nocapture`；session `57512` | **退出 0，28 tested、28 caught，0 missed/unviable/timeout**，baseline 通过。恢复副本 `fK5mXJ.tmp` 的纯文件哈希为上述 15a；覆盖当前纯校验模块，不是 B I/O 或全 crate 门禁 |
| `cargo +nightly-2026-08-14 fuzz run thinws_checkout_validation /private/tmp/thinws-p0-03-gitlink-corpus.DnxQs2 -- -max_total_time=60 -timeout=5 -max_len=4096 -print_final_stats=1`；session `82708` | **退出 0，538,749 runs / 61 秒**，峰值 RSS 542 MB，无 crash/timeout，artifact 为 0；从空 corpus 开始，报告最终 300 项、磁盘保留 298 项。启动有 atos symbolizer/fd 3 警告，继续至正常终态，不称环境无警告；这是短预算而非阶段长预算 |

Astra / `gpt-6-astra` / `xhigh` 完成上述冻结代码、真实 fixture、纯函数、harness 与原始变异结果的只读审核，限定结论为 **Gitlink 增量无局部阻断**。Reviewer 未运行测试、变异或 fuzz；结论不包含全 crate 门禁、topology 超时、sparse/转换政策或整个 P0-03。主 Agent 仅把既有 14 个 B flags OR→XOR 的精确行锚向后迁移 17 行至 1318、1364、1380、1992–1996，topology 三项仍在 815；操作数、列与等价证明不变，没有新增排除。Astra 另行核对配置增量并实际枚举：无配置 456 项、当前配置 **439 项**，恰好排除原 17 个等价 XOR，**17 个 OR→AND 全部保留**。该枚举不是 439 项实际执行通过。

### 当前拓扑模块全量变异与超时复跑（2026-09-12）

主 Agent 在不变的 topology `4faa682…` 上执行 `cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/managed_topology.rs --leak-dirs --timeout 90 --jobs 2 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-topology-full-current.XxK4IG -C=--locked -- --lib --test managed_topology -- --skip base_materialization::tests:: --nocapture`。当时 B 正在 Gitlink RED/GREEN，因此只对本模块变异执行库中非 B 测试与完整 topology IT；不把此范围称为整个 crate 门禁。session `62264` **退出 3，93 tested＝80 caught＋12 unviable＋1 timeout，0 missed**；baseline 通过，恢复副本 `LDOtdw.tmp`、`TAiq8Q.tmp` 的 topology 哈希均匹配 4faa。

唯一 timeout 是 `managed_topology.rs:1021:23: replace != with == in create_workspace`。原日志已经包含 index tree 核对处的预期 Validation 失败和 IT **16 passed、9 failed / 85.10 秒**，但库测试加 IT 的总测试时间达到工具 90 秒上限，工具判定仍为 Timeout；不能仅凭其中已有失败就改记为 caught，也不将并发负载推测写成已证实原因。

其他重负载运行结束后，主 Agent 保持相同测试范围、baseline 和 **90 秒**限制，增加精确 `-F '^experiments/p0/git-base/src/managed_topology\.rs:1021:23: replace != with == in create_workspace$'`，仅把 jobs 改为 1、输出改为 `target/p0-03-topology-timeout-replay.eExJpC`。session `13503` **退出 0，1 tested、1 caught，0 missed/unviable/timeout**；baseline build 10.81 秒、test 41.71 秒，变异编译成功后测试 40.15 秒退出 101，其中 IT 仍准确为 16 passed、9 failed。恢复副本 `wvKNPf.tmp` 的 topology 哈希匹配 4faa。此复跑补足该项可被测试检出的证据，原全模块退出 3 的历史结果保持；不合写成“原 93 项运行退出 0”，也不代替受影响 crate 的全量收口。

### Gitlink 最终冻结的独立通用门禁与放行边界（2026-09-12）

主 Agent 在其他变异/fuzz 进程结束后，对 c45d/4a9/b01 的最终冻结独立执行根与 fuzz 的 `cargo fmt --all -- --check`、两 workspace 的 `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`（fuzz 使用 `--manifest-path fuzz/Cargo.toml`，且 rustc 参数另含 `--cfg fuzzing`），随后执行 `cargo test --locked --workspace --all-targets`、`cargo test --locked --release --workspace --all-targets` 和 `git diff --check`。session `50619` 全部 **退出 0**；两种 profile 各 **229 passed、0 failed、2 ignored**（Git 91 unit＋4 attribute＋25 topology，materialize 60、probe 49；FD-zero 子进程不重复计数）。B、纯校验、harness、topology、topology validation 和 topology IT 六份源码前后哈希一致。本次普通测试的两项跨卷 ignored 未执行，前文 session `87377` 在专用卷的独立 debug/release 各两项通过证据另行保留，不改写当前输出。

当前 Gitlink 切片的实际拒绝、专项测试和限定审核符合本节准入预期；公开 CLI 契约未改变，实验实现补齐已有 submodule 拒绝要求，不表示 P1 接入命令已经实现。最新源码的受影响 crate 全量变异、统一短预算 fuzz 收口、剩余 sparse/转换真实矩阵、正式 ADR、候选 CI 和全任务最终审核仍未完成；阶段长预算与 P0-04 更不在本轮通过范围。声明存在但关闭转换的兼容政策，以及 sparse 拒绝应针对哪个来源工作树的产品语义，仍须维护者决定；条件确认不代替这两个决定。

P0-03 保持 In Progress，P0-04/P1 依赖状态不变，P0.b/整个 P0 的条件放行尚未成立。本轮未提交、push、合并或打 tag，未删除保留实验现场，也未新增独立审核文档。

### 内建转换事实对照准入（2026-09-12）

在未决产品政策之外，主 Agent 准入 P0-03/P0.b/R4 的真实 Git 内建转换对照，以补足类型查询不能证明实际检出行为的证据缺口。仅允许 Sol / `gpt-5.6-sol` / `xhigh` 修改既有 `tests/attribute_types.rs`：保留原 filter fixture/四项测试及生产入口不变，不把硬编码 filter 查询泛化成新接口；Astra / `gpt-6-astra` / `xhigh` 审核。此实验既不接纳这些属性，也不实现不兼容预检，不能代替用户对公开边界的决定。

- 新例使用独立私有 fixture、空 global/system 配置、隔离 HOME、明确关闭 hooks/fsmonitor/external diff 和外部 attributes。不配置或运行 filter/canary，不联网、不修改用户仓库或全局配置。Git 所有参数、路径、属性和内容均来自固定合成数据；失败保留现场，不加清理/回滚机制。
- 复用已有受控 runner 的 spawn/poll/wait 和超时逻辑，必要时仅机械提取一个返回 `std::process::Output` 的私有下层；旧调用仍经 `successful_output`，原“非零或成功带 stderr 都失败”的断言不得削弱。新实验直接保存真实 status/stdout/stderr，不能把测试 helper 的 panic 当作 Git 原始拒绝证据。
- 每种声明使用独立空 worktree 与固定 index，以 raw blob/cacheinfo 建树，不使用 `git add` 或准备期 checkout。先以固定完整 mode/OID 清单、blob/tree OID、原始 bytes 和 Git attr pathspec 的独立非空集合自证 fixture，再执行真实 checkout。`ident` 覆盖 Set、字符串 `set`、Unset、Unspecified；编码覆盖 UTF-16LE、UTF-8、Set、Unset、字符串 `unset`/`unspecified` 和 Unspecified。预期检出字节使用独立字面量，含非 ASCII 与完整固定 blob OID，不从被测输出回算预期。
- 对不合法编码同时观察退出状态、stderr 和是否留下原始/转换/部分内容，不预设“有错误必然非零”。[Git v2.39.5 转换源码](https://github.com/git/git/blob/v2.39.5/convert.c#L437-L458)提示编码失败可能只写诊断而返回未转换；这只是待实测依据，不作为本机已通过的结果。真实结果与假设不符时先核验原因，不能直接改预期让测试变绿。
- 前后核对各 index 的字节/身份、lock、Git 配置与保护对象，分别记录各目标 worktree 的实际内容与根身份，不把有意发生的检出写入称为全根不变。新增例只测系统 Git 事实，不改变产品行为，因此不制造产品 RED；普通失败断言必须可解释，并由主 Agent 独立重跑精确 debug/release 与通用门禁。该准入不是实现完成或条件放行。

### 当前 Git/Base 统一短预算 fuzz（2026-09-12）

主 Agent 在转换实验尚未实施、生产源码冻结时串行执行四个既有 Git/Base 纯函数目标；session `55734` **退出 0**。统一命令为 `cargo +nightly-2026-08-14 fuzz run <target> /private/tmp/thinws-p0-03-unified-fuzz.hZn8FQ/<target> -- -max_total_time=60 -timeout=5 -max_len=4096 -print_final_stats=1`，工具版本遵循 `tools/quality-tools.toml`，每个 target 均从独立空 corpus 开始。下表是实际终态，不累计此前其他批次为本次成绩。

| Target | 实际执行次数 / 秒 | 峰值 RSS MB | 最终 corpus：工具报告 / 磁盘保留 |
|---|---|---|---|
| `thinws_git_path_output` | 257,263 / 61 | 718 | 318 / 316 |
| `thinws_base_key_candidate` | 5,536,119 / 61 | 487 | 79 / 78 |
| `thinws_git_topology_validation` | 2,565,819 / 61 | 629 | 216 / 215 |
| `thinws_checkout_validation` | 3,254,556 / 61 | 759 | 401 / 396 |

共 **11,613,757 runs**；四项目标均正常结束、未发现 crash/timeout、最慢单输入报告 0 秒，对应 artifact 目录均为 0 个文件，语料目录继续保留。主 Agent 核对各 harness 实际复用的源码：`path_output` b3e63a4、`base_key` c359cdc、`topology_validation` 6b4dddc、`checkout_validation` 4a9fd76；harness 分别 f4e2628、95d00ee、9d1dcde、b01fa7d。上述八份文件在本批开始和结束的完整 SHA-256 均一致；当前 checkout 目标此次确实使用最终 4a9 文件，不回写前一批 15a 的历史归属。

本批补齐当前 Git/Base 相关目标的统一短预算证据，不包含 Probe/Materialize 两个未受本轮改动的目标，不代表六个目标的阶段长预算、真实转换/文件系统行为或受影响 crate 全量变异已通过。后续若这些生产函数或 harness 改动，必须重新判断并执行受影响范围，不能沿用本批冻结结果。

### CI 预算适配待验证（2026-09-12）

Astra / `gpt-6-astra` / `xhigh` 只读核对固定 `cargo-mutants 27.1.0` 源码、当前 workflow 和实际 outcomes 后确认：现有 `--timeout 60` 限制一次完整 Cargo Test 阶段（包含正常 baseline），不是单个测试用例/二进制；baseline 未通过时不会执行 mutant。本机当前 Git 全套普通测试约 23 秒 unit＋7 秒 attributes＋40 秒 topology，60 秒预算与这组证据不匹配。此前跳过 B 和属性 IT 的 41.71 秒 baseline 不证明完整范围足够，也不能直接采用 90 秒专项限制作为新的全范围上限。

当前全 workspace 枚举为 **953 项**（Git 439、materialize 270、probe 244），原 topology 93 项窄范围实际已耗时约 904 秒。现有 CI job 的 45 分钟还包含工具安装、双 profile 测试和六项 fuzz，容量尚未证实；不线性外推为远端一定失败。本轮没有远端候选 CI 结果。`--workspace` 表示生成所有包的变异，默认每个 mutant 执行其所属包的测试，不是每项都重跑全 workspace。

主 Agent 接受“需校准但不缩减门禁”的限定结论：待新增转换 IT 冻结，先取得相同完整测试参数的 baseline 与可解释慢样本，再确定有限 Test 上限；job 上限仍须由完整运行总时和明确余量支持。可用带正常 baseline 的单个精确 mutant 诊断测试预算，但它不能代替 953 项全量门禁或证明整 job 足够。该准入时暂不改变两个 timeout，不引入新并行工作流、测试过滤或排除；后续实际对照与有限调整见“完整 Git 测试参数下的预算对照”，旧超时仍保留真实结果。

### 内建转换假设纠正（2026-09-12）

Sol 首跑编码对照时，七个场景先全部执行并报告原始 Output，随后在 `encoding-unset` 的错误预期（128）处失败，真实 Git 是退出 0、两流为空。作者依规则停下，没有自行改预期；保留根为 `target/p0-git-base-tests/attribute-types-11700-1789251760528420000-0`。主 Agent 对测试文件冻结 `40eb16032c69a3bcb4896c9490872c693b44ddf98e9471fed9a9ea16343852b7` 独立运行 `cargo test --locked -p thinws-p0-git-base --test attribute_types apple_git_working_tree_encoding_records_conversion_rejection_and_diagnostics -- --exact --nocapture`（session `84776`），编译成功后 **退出 101，0 passed、1 failed、5 filtered**，同样只在该错误预期处失败；前后哈希一致。这是实验假设被事实证伪，不是产品行为 RED。

主 Agent 的独立根 `target/p0-git-base-tests/attribute-types-23645-1789251839558354000-0` 中，Unset 目标为普通 0644 文件，逐字节读取为 `41 c3 a9 e6 9d b1 0a`，与固定 UTF-8 输入一致；七个场景在断言前均已有原始退出结果。Set 实际为 128 和固定 fatal 诊断；字符串 `unset`/`unspecified` 实际为 0，但 stderr 有编码失败诊断；其余 UTF-16LE、UTF-8、Unspecified 为 0 且两流为空。该失败运行未执行完后续全部断言，不能称完整编码验收通过。

原因已由主 Agent 对照上游固定版本核验：`git_attr__false` 的表示以 NUL 开头，因此 `git_path_check_encoding` 的 `!strlen(value)` 先返回 NULL，后面的 `ATTR_FALSE` 报错分支对此值不可达。不是已证实的 Apple Git 私有差异。主 Agent 据此纠正本记录前文的源码误读，并准许把 Unset 测试预期改为成功、两流为空及精确原始字节；产品是否接纳这类声明仍待用户决定。另去掉新鲜空工作树 checkout 的多余 force 参数，并复用一次保护快照断言以免重复维护清单；无新增政策、框架或生产代码，修后须重新冻结与复验。

### 内建转换最终观察与独立复验（2026-09-12）

最终 `tests/attribute_types.rs` SHA-256 为 `d99ba0ec6bc3b806bb28d8165e2d9f010e691057a958237472384b9e2d3611c1`，新增两项测试承载 11 个独立场景；原四项测试及失败契约保留。下表仅描述本机受控 Git 的检出事实，不批准产品支持。

| 声明场景 | 实际退出与诊断 | 精确内容结果 |
|---|---|---|
| `ident`（布尔 Set） | 0，两流为空 | `$Id$` 展开为固定 blob OID |
| `ident=set`、`-ident`、Unspecified | 均为 0，两流为空 | 原始字节不变 |
| `working-tree-encoding=UTF-16LE` | 0，两流为空 | 固定非 ASCII 输入转换为独立预期 UTF-16LE 字节 |
| `working-tree-encoding=UTF-8`、`-working-tree-encoding`、Unspecified | 均为 0，两流为空 | 原始 UTF-8 字节不变 |
| `working-tree-encoding`（布尔 Set） | 128，固定 fatal 诊断 | 已检出 `.gitattributes`，目标文件不存在 |
| `working-tree-encoding=unset`、`working-tree-encoding=unspecified` | 均为 **0，但 stderr 有编码错误** | 目标保留原始 UTF-8 字节，不能称为转换成功 |

其中 Unspecified 使用无对应声明的空属性文件，未覆盖显式 `!attribute` 拼写。所有场景的完整 index/tree、属性来源、目标内容和受保护对象均按准入断言核对。主 Agent 用 Ruby `Digest::SHA1` 从固定输入独立构造原始 blob/tree 帧，Astra 另用 Perl `Digest::SHA` 独立重算，11 组固定 OID 全部一致；不是从 checkout 输出推导预期。

主 Agent session `71714` 对最终冻结依次执行根与 fuzz 格式/Clippy、两项新测试的精确 debug/release、全 workspace 的 debug/release（`--all-targets -- --include-ignored`）及 `git diff --check`，全部 **退出 0**。两种 profile 各 **233 passed、0 failed、0 ignored**：Git 91 unit＋6 attributes＋25 topology，materialize 61，probe 50；FD-zero 子进程不重复计数。专用 APFS 卷上的两项跨卷测试均实际执行通过。新例精确复验各 profile 两项通过；四个保留根依次为 `attribute-types-62725-1789252098632920000-0`、`attribute-types-63306-1789252102295429000-0`、`attribute-types-64233-1789252108038810000-0`、`attribute-types-64802-1789252111368588000-0`，均位于 `target/p0-git-base-tests/`。测试前后 d99、B c45d、lib c846 哈希一致。

Astra / `gpt-6-astra` / `xhigh` 对最终增量的限定结论为**未发现局部阻断**，已核对上述原始帧、隔离、保护断言、runner 机械提取及旧错误契约。Reviewer 未运行测试或变异。此结论不批准转换产品接纳、完整兼容拒绝矩阵或 P0-03 放行；用户对关闭声明及 sparse 来源范围的决定仍未收到。公开 CLI 契约未变，全任务门禁、正式 ADR、候选 CI、P0-04 和阶段放行仍未完成。

### 完整 Git 测试参数下的预算对照（2026-09-12）

本诊断使用同一精确 mutant `managed_topology.rs:1021:23: replace != with == in create_workspace`，保留正常 baseline、完整 Git 包测试范围与 `--include-ignored --nocapture`，不再采用前文跳过 B/属性 IT 的窄范围。命令为 `cargo mutants -p thinws-p0-git-base -F '^experiments/p0/git-base/src/managed_topology\.rs:1021:23: replace != with == in create_workspace$' --leak-dirs --timeout <seconds> --jobs 2 --output <directory> -C=--locked -- -- --include-ignored --nocapture`；两次均设置 `THINWS_P0_CROSS_VOLUME_ROOT=/private/tmp/thinws-p0-materialize.OC4JPT/mounted`。只有一个 mutant，实际最多一个 worker，不能据此证明双 worker 争用下的容量。

60 秒对照 session `5636` **退出 4**：`outcomes.json` 记录 baseline Build 成功 9.49 秒、Test **60.03 秒 Timeout**，没有执行任何 mutant（`total_mutants=0`）。日志中 unit 91 项和 attributes 6 项已通过，topology 尚未完成即被超时终止，不能称为产品 RED 或 mutant caught。结果目录 `target/p0-03-ci-budget-60.MWHRKh/mutants.out/` 和 scratch `cargo-mutants-thinworkspace-p0-03-nZA21s.tmp` 均保留；scratch 的 d99 测试文件与 4faa topology 哈希匹配当前冻结。这是本机当前完整 Git 参数下预算不足的实际证据，不是远端 CI 已失败的证明。

180 秒对照 session `89574` **退出 0，1 tested、1 caught，0 missed/unviable/timeout**。baseline Build 9.00 秒、Test **64.29 秒成功**；mutant Build 0.92 秒成功、Test **60.95 秒退出 101**，unit 91 项和 attributes 6 项通过，topology 为 16 passed、9 failed，失败定位于已预期的固定 tree/index 核对。结果目录 `target/p0-03-ci-budget-180.IR9E4s/mutants.out/` 和恢复 scratch `cargo-mutants-thinworkspace-p0-03-kXihzh.tmp` 保留；恢复后 d99、4faa 匹配。两次实际 Cargo argv 相同，除预算与独立输出目录外未缩减测试范围；原 60 秒失败不被改写。

主 Agent 据此将现有 CI 变异命令的 Test 上限由 60 调为有限的 **180 秒**，保留 workspace 全量、jobs 2、正常 baseline、全部测试参数和排除清单不变。该值为当前完整 Git 测试提供余量，并仍能在有限时间识别无法结束的运行；不是 439/953 项或双 worker 负载验证。job 的 45 分钟上限本轮未改，先前提出的 240 分钟只是候选，尚无完整运行总时支持；整 job 容量仍是待验证项，不能宣称 CI 已修复并通过。此一行配置调整不改变产品契约或放行状态。

最终 workflow SHA-256 为 `10f3475a9729d6c7d3b42cc126e6315e770f429376b249ee59c4b64107941b8b`。主 Agent 将其数值在内存中逆替换后，SHA-256 精确回到调整前的 864749，确认本轮仅此一行变化；YAML 解析及命令/保留 job 上限断言、`git diff --check` 均退出 0。Ruby 启动提示已有 ffi 扩展未构建，但上述检查正常完成，未修改环境。`actionlint` 未执行：本机无该命令。Astra / `gpt-6-astra` / `xhigh` 独立核对两个 outcomes、原始日志、恢复哈希和这一行 diff，限定结论为无局部阻断、记录未夸大；未运行测试，不批准全 CI 或 P0-03。

所有本轮测试终止后，主 Agent 重验专用卷 UUID `E4C7D91A-EF7B-476B-9BEB-946B53FEE500`，于 `2026-09-12 22:42:17 UTC` 正常 detach 精确挂载点，退出 0；`hdiutil info` 确认该镜像不再挂载。原 128 MiB 镜像、fixture、corpus 与变异 scratch 均保留，未触碰其他挂载、删除证据、提交、push、合并或打 tag。P0-03 仍为 In Progress，条件放行未成立。

### 当前冻结 Git crate 全量变异准入（2026-09-12）

在兼容政策仍未决时，主 Agent 继续执行不改变政策的现有候选质量验证：冻结 Git 七份源码、两份 IT 与当前排除配置，运行完整受影响 crate 的变异门禁。实际枚举 **439 项**：lib 35、base key 13、Base materialization 233、checkout validation 28、managed topology 93、path output 14、topology validation 23。生产与 IT 冻结沿用上述 c846/c359/c45d/4a9/4faa/b3e63/6b4d、d99/a42d，配置 b8a4；本批运行中不修改这些输入，也不追加筛选或排除。

准入命令为 `cargo mutants -p thinws-p0-git-base --leak-dirs --timeout 180 --jobs 2 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-git-crate-full-current.TCOVID -C=--locked -- -- --include-ignored --nocapture`，使用固定 27.1.0、正常 baseline、完整 Git 包测试及两 worker。Git 包没有 ignored 或跨卷环境依赖，本批不重新挂载已卸载的测试镜像；此前跨卷普通门禁证据保留。本批旨在补全量质量证据，不批准缺失的兼容行为或替代整个 workspace/阶段验收；后因下述未变异 runner 缺陷有序中止，实际未完成全量，不能认定通过。

主 Agent session `76415` 于 `2026-09-12T22:46:24Z` 启动；正常 baseline Build **9.57 秒**、Test **61.08 秒**，均成功。实际 argv 包含 `--package=thinws-p0-git-base@0.0.0 --locked -- --include-ignored --nocapture`，没有库/测试名过滤。两个保留 scratch 为 `cargo-mutants-thinworkspace-p0-03-p5zpF0.tmp` 和 `cargo-mutants-thinworkspace-p0-03-J2o3yi.tmp`。主 Agent 核实 runner 缺陷后，对已核验归属的父进程 `48067` 发出 SIGINT；session 在 **2026-09-13 00:08:37 UTC 退出 1（interrupted）**，不是工具自然完成，也不是因一次观察等待超时而重启。最终保留的部分 outcomes 为 **258/439 项已有分类结果：221 caught、34 unviable、3 timeout、0 missed**，`end_time` 仍为 null；其余 181 项没有完成结果（其中两项在 Test 阶段被中断），不能算通过或存活。主目录与两份恢复 scratch 各 10 个冻结文件的完整 SHA-256 均匹配，随后进程核对已无这两份 scratch 的测试进程。所有原始日志、fixture 与 scratch 保留，不把已有专项成绩拼接为本次 439 项通过。

运行途中完成计数曾停在 66。主 Agent 通过该 session、父子进程和原始日志核验：两项正在 Build，不是 Test；两个 Cargo 各自的三个 rustc 正等待 `xcrun --sdk macosx --show-sdk-path`，进程均存活、低 CPU，磁盘仍有余量。对所属 xcrun 的 1 秒只读采样保存在 `target/p0-03-xcrun-diagnostic.Wl6ddo/xcrun-44709.sample.txt`，但调用栈无法符号化，未证明具体等待原因。`xcode-select --print-path` 返回既有 Xcode 路径，相关目录存在；未修改选中路径、清理缓存、杀进程或重启实验。

原运行随后自行推进。`base_materialization.rs:577:25` 和 `621:25` 两项 `!= → ==` 的 outcomes 分别记录 Build **318.80/308.38 秒成功**、Test **29.33/30.99 秒退出 101**，均为 CaughtMutant。不能将这段 Build 等待记作 180 秒 Test timeout，或宣称已定位/修复 Xcode、缓存或项目问题；其实际总耗时应保留在本批容量证据中。

计数 128 时再次出现同样的可观察进程链：两组 Cargo/rustc 等待六个 `xcrun --sdk macosx --show-sdk-path`。主 Agent 只读核对后继续等待原 session，未做干预，运行再次自行恢复。此次 `base_materialization.rs:1332:36`（`require_missing` 的 `== → !=`）与 `1341:5`（删除 `require_info_attributes_missing` 检查）分别 Build **310.04/309.93 秒成功**、Test **22.82/34.67 秒退出 101**，均为 CaughtMutant；根因仍未确定，不把编译等待归因于测试漏测。

本批首次 Test Timeout 为 `base_materialization.rs:1770:12: delete ! in require_fixed_name_set`：Build **2.514816291 秒成功**、Test **180.03103475 秒 Timeout**。原始日志 `mutants.out/log/experiments__p0__git-base__src__base_materialization.rs_line_1770_col_12.log` 第 151–153 行表明，不依赖 Git fixture 的固定名称集合单测在 `complete root name set succeeds` 处已因合法 `.gitattributes` 被误判为 `UnknownContentEntry` 而失败；同时，后续多个真实 Git fixture 命令超时，整次测试未及时收束。Astra / `gpt-6-astra` / `xhigh` 只读核验上述断言、函数与 outcomes，主 Agent 独立核对同一证据：此变异非等价且已有直接断言覆盖，但工具终态必须保留 Timeout，不能改为 caught，也不能据此确定 xcrun 是 Git 超时根因。

该项先保留待处置；全量运行期间不修改源码、测试、超时或排除配置。全量结束后可对同一冻结输入、精确 mutation、正常 baseline、180 秒预算和完整 Git 测试参数做独立复验，核验目标断言以及整个测试进程是否正常以非零状态收束；复验结果另记，不能覆盖本批原始超时。当前没有新增排除或为这项超时修改 guard。

主 Agent 对已完成 217 项时的原始日志做超时关键词筛查，13 个 CaughtMutant 日志包含 `Timeout {`；这只是待核验线索，不是全部异常的判据。Astra / `gpt-6-astra` / `xhigh` 逐项只读核验后，11 项已有直接目标断言失败（其中 runner 截止时间变异造成的受控超时本身是预期检出）；另两项 `base_materialization.rs:351:26` 与 `568:23` 的 `!= → ==` 暂无目标断言证据。主 Agent 独立解析这两份原始日志，分别确认 **30/40 个 panic 全含前置 Git Timeout**，实际 Test **39.332027417/40.676292875 秒退出 101**；它们的原始 CaughtMutant 分类保留，但不能作为有效目标检出的充分证据。当时将这两项与上述 `1770:12` 列为待精确复验，后续顺序以下文确认 runner 缺陷后的处置为准；不因工具给出 caught 就忽略夹具失败，也不把其他 11 项仅因含超时词而重复运行。该扫描仅覆盖当时已完成的日志，不代替后续结果检查。

随后 `base_materialization.rs:2147:23` 的 `+ → -`、`+ → *` 两项分别 Test **180.0149305/180.023336709 秒 Timeout**。两者纯 Cursor 断言已分别观察到少读一个字节和错误超额读取，但工具终态仍为 Timeout。在前一项测试进程 `20929` 存活时，主 Agent 取得 1 秒采样 `target/p0-03-test-wait.x9Eclc/test-20929.sample.txt`；其中 879/879 次采样在 `Fixture → collect_bounded_output → terminate_child → wait_with_output → read_output → poll`，主线程等测试结束，先前进程快照中直接子进程为 Z。Astra / `gpt-6-astra` / `xhigh` 独立核对源码、采样与两项日志，确认冻结且未变异的 `lib.rs` 只对 `try_wait` 设 deadline，正常退出与终止后的 `wait_with_output` 没有输出收集期限，构成 P0-03 收口前须修复的有界性缺口。主 Agent 核对同一调用链后决定停止本批候选验收；不把增加 Test 预算作为修复。证据不证明永久挂死、具体管道写端持有者或 xcrun 根因，成功 kill 也不证明已经 reap。

中断后，主 Agent 复扫全部 258 个已分类结果：含超时词的 caught 仍为上述 13 项，没有新增候选；34 个 unviable 均有编译错误日志，不把它们算成测试检出。这仍不是全量门禁通过。

下一步只修复现有 P0 实验 helper 的有界返回与诚实的收尾证据：先用明确同步就绪、受控保留管道写端的真实场景取得 RED，覆盖直接子正常退出及被终止后仍无 EOF，再局部实现和验证；保留正常输出、非零退出、signal 与错误来源契约，不增加 P1 Supervisor 或通用执行框架。上述精确复验与完整受影响 crate 门禁必须在修复后重新安排，旧 258 项只保留为缺陷发现及历史证据，不当作新候选通过。编码由 Sol / `gpt-5.6-sol` / `xhigh`，审核由 Astra / `gpt-6-astra` / `xhigh`。

### P0 runner 局部修复准入（2026-09-13 UTC）

主 Agent 核验 Sol 的只读方案后准入本切片，仍属 P0-03/P0.b/R4；唯一代码写入区域为 `experiments/p0/git-base/src/lib.rs` 及其同文件单测。复用已锁定的 `rustix 1.1.4`/`fs` 与固定标准库，保持现有调用签名和 `forbid(unsafe_code)`，不新增依赖、feature、模块、trait、进程组或产品公开政策。源码未通过回归与指定审核前，不恢复完整候选门禁。

验收要求是运行等待与双流收集共享原 timeout；若需要终止，至多再使用同值的有限收尾预算，即本 P0 helper 的总算法预算最多为两段 timeout，而非承诺硬实时调度。每轮输出读取和中断重试也必须受限，持续输出不能饿死另一流或期限检查。只有直接子状态及两流 EOF 均完整取得时返回原 `Output`；其他情况结构化保留已收集字节、原始错误及已回收/未确认的事实，不把成功 kill 或部分输出当成完整完成。部分输出只保留一个清晰归属，不堆叠平行结果模型。

RED 使用真实 `std::io::pipe()`、由父端明确持有的写端，以及已确认正常退出的直接子进程或因受控 stdin 未关闭而保持运行的直接子进程；无需未知后代或 sleep 猜测同步。测试局部 watchdog 仅用于释放所有保留写端后安全 join，使旧代码以“未在自身预算返回”断言失败；它不作为 GREEN 成功证据，也不允许留下后台线程/子进程。先保留可解释 RED，再实现并执行精确单测、Git 包普通测试的 debug/release、fmt 与定向 Clippy；主 Agent 独立复验并交 Astra 审核后，才安排剩余门禁。本段仅定义准入条件，实际 RED、GREEN 与审核结果须分别记录，准入本身不代表完成或放行。

Sol 随后报告真实 RED：`cargo test --locked -p thinws-p0-git-base --lib retained_pipe_writer_returns_within -- --nocapture` 退出 101，0 passed、2 failed、91 filtered。主 Agent 在 `lib.rs` SHA-256 为 `f825e615c31cf42853ae8456ef1ea4cc2ee3b403b0b4488142a24263c6a1d713` 时，独立与原 c846 scratch 比较，确认 `#[cfg(test)]` 前生产逻辑完全一致，差异仅为两项回归及测试辅助代码。正常例实际用 `/usr/bin/printf normal-output` 并先确认 exit 0；超时例用持有 stdin 写端的 `/bin/cat`。两者均以父端持有 stdout 写端保证无 EOF，collector 参数为 20ms，旧实现直到 1 秒外层 watchdog 释放写端后才分别返回正常 Output 或 Timeout/KilledAndReaped。RED 证明旧实现超过预算，但该 watchdog 不是 2T 的精确阈值；主 Agent 要求 GREEN 另检查 collector 内实际耗时与结构化不完整证据，不能只断言未触发外层 watchdog。

修复冻结为 `e8100fcd18fa659d9d1052b45dca125c4c3f2a3996b1d32503dc2d1cd2552f85`：非阻塞双流读取及一个私有 `OutputPipes` 聚合统一有限收尾，没有 `wait_with_output`、unsafe、新增依赖或 lint 豁免。P0 实验错误新增输出不完整、有限回收未确认等证据；P1 CLI/Git 公开契约不变。主 Agent 独立执行上述两项回归与 `tests::complete_output_collects_both_streams` 的 debug/release，分别 2/2 与 1/1 通过；`cargo fmt --all -- --check` 及 `cargo clippy --locked -p thinws-p0-git-base --all-targets --all-features -- -D warnings` 均退出 0。两项回归内部核对结构化结果与 elapsed≤250ms，测试组约 0.04 秒完成；250ms 为调度容差，不能宣称严格 40ms 实测保证。主 Agent 复验前后 lib e810、Cargo.toml `7ffe7df…`、Cargo.lock `caef7cd…` 哈希一致。

作者 Git 包普通测试的可引用终态为 debug session `26219`、release session `25516`，均退出 0，各 **94 unit＋6 attributes IT＋25 topology IT＝125 passed**。此前两次并行调用因作者只保留输出文本、丢失返回 session ID，结束后无法恢复退出码，明确记为 **终态未知、不算通过**；工具的 30 秒观察截止不是失败或成功依据。本轮没有继续启动第三次运行。Astra / `gpt-6-astra` / `xhigh` 只读核对 e810 与 c846 的限定差异，结论无阻断正确性/边界问题；该结论不替代非阻塞配置失败、读取错误、kill/reap 失败及持续输出的动态验证，更不是完整 P0-03 批准。

先验证改动模块的任务级测试强度：主 Agent 枚举 `lib.rs` **75 项**变异，并用 `cargo test --locked -p thinws-p0-git-base --lib -- tests:: --skip '::tests::' --list` 核对范围为 11 个根模块直接单测。当前命令为 `cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/lib.rs --leak-dirs --timeout 180 --jobs 2 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-runner-module-e810.PShfKM -C=--locked -C=--lib -- -- tests:: --skip '::tests::' --nocapture`，session `99530`；保持正常 baseline、现有配置和排除清单，e810 源码冻结。此处明确只验证改动模块，不是完整 Git crate/小阶段门禁；该 session 的结果须另行核验后再安排稳定候选全量门禁。

### 当前锁定依赖的供应链复验（2026-09-12）

全量变异执行期间，主 Agent 使用固定 `cargo-deny 0.20.2`、`cargo-audit 0.22.2` 执行根 `cargo deny --locked check`、fuzz `cargo deny --locked --manifest-path fuzz/Cargo.toml --config deny.toml check`，以及两个 lockfile 各自的 `cargo audit --deny warnings --file <lockfile>`；session `94552` **退出 0**，四项全部完成。deny 的 advisories/bans/licenses/sources 均通过，保留既有的未匹配自有包许可例外及 NCSA allowance 警告，不称零警告；audit 加载 1,243 项公告，分别扫描根 36、fuzz 29 项依赖，未报告漏洞。检查后本机 RustSec 公告库 HEAD 为 `b50980aad8b8f14f77e25a97b32dd94bf008b0af`。

检查前后根 lockfile `caef7cd…`、fuzz lockfile `cdab36a…`、`deny.toml` `dd9140c…`、工具清单 `ad23127…` 的完整 SHA-256 一致，`git diff --check` 通过；本轮未修改依赖、例外或工具版本。这是当前锁定依赖和所加载公告库的检查结果，不保证不存在尚未公开的漏洞，也不代替仍在运行的变异或阶段门禁。

### 文档基线合入与中断任务恢复

维护者在文件克隆/锁调研完成后明确要求提交文档并继续原任务。主 Agent 将 `0a261a7`（取消用户命令包装）与 `b3af363`（锁语义调研与 P0-05 排期）合入本工作区，提交 `f57d15d` 并推送原任务分支；13 个合入路径均为文档。计划中相邻插入的唯一冲突保留 P0-03 实施入口和 P0-05 整节，P0-03 仍 In Progress、P0-05 仍 Backlog、P1-11 仍 Cancelled，并写明恢复状态；不覆盖或提交原有候选代码。

合入前后原 19 个未提交文件的 SHA-256 全部相同；lib 仍为 `e8100fc…`。主 Agent 独立执行 fmt、全 workspace Clippy 与普通测试（session `31765`），均终态退出 0；2 项额外 APFS 卷测试保留既有 ignored，未重新挂载跨卷实验环境。20 份 Markdown、90 个本地链接和 2 个 JSON 示例检查通过。Astra / `gpt-6-astra` / `xhigh` 对文档合入给出限定 Approve，不代表候选源码或 P0-03 放行。

恢复时只读核验旧 `target/p0-03-runner-module-e810.PShfKM/mutants.out/outcomes.json`：75 项均有分类、`end_time=2026-09-13T00:48:43.610271Z`，46 caught、14 unviable、15 missed、0 timeout。14 个 unviable 均有编译错误日志；这不是全通过。旧命令只运行 11 个根 lib 单测，7 个 matching_paths/run_git 存活项需用已有属性 IT 复验；其他存活涉及非阻塞重复设置、双流进展、关闭读端、错误传播、成功输出判定和两个休眠门控。主 Agent 与规定 Reviewer 均未批准将这些项目直接等价排除。

本轮先补已有 runner 私有 helper 的直接测试，仍只修改 lib 的测试模块；编码由 Sol / `gpt-5.6-sol` / `xhigh`，主 Agent 负责冻结、独立复验和记录。测试断言为真实 pipe 非阻塞设置幂等且保留 flags、各流进展/EOF 与内容归属、关闭读端后的实际 BrokenPipe、可重试与非瞬时读取错误的区别，以及 Git 成功/失败输出保真。既有行为补测可以首次直接 GREEN，不伪造行为 RED；随后用对应真实 mutation 验证断言强度。没有准入新增生产抽象、依赖、排除项或用户命令包装；两个休眠门控和其余任务门禁继续保留待验证。

### 恢复后的 runner 首次完整分区结果（2026-09-13 UTC）

该次 lib SHA-256 为 `32d2b445adb87f9266301068e542f2189914d88e23420cb55d8c7775daf68ef3`，相对 e810 仅在测试模块净增 388 行、9 项测试。首个 `#[cfg(test)]` 前的生产区 SHA-256 始终为 `2ff4fedfc01108367dfdb836948e44385f3bd8400a3b7fce26e9c164f255075a`；本次没有修改生产逻辑、依赖或变异排除配置。此输入随后在审核中发现测试的非目标失败，不能作为补测最终验收，纠正见末节。

两个休眠门控使用有限输出的完成语义验收：collect 在 2 秒预算内完整取得 8 MiB，terminate 在 1 秒收尾预算内完整取得 16 MiB；由真实 pipe/显式就绪信号同步，不以 CPU 采样或紧贴正常耗时的阈值判定。Astra 发现测试就绪超时分支可能在 rendezvous send/join 上互等，Sol 仅补 `drop(ready_receiver)` 后由主 Agent 和 Astra 复核；失败日志只报告状态、长度和固定字节谓词，不展开大缓冲。该修正是测试清理，不是生产 runner 缺陷修复。

补测中间候选 bbb481 的非查询 68 项批次 `target/p0-03-runner-resume.9lKfji` 已中断：删除非阻塞设置后，其他依赖非阻塞读的测试也随之阻塞，`set_nonblocking → Ok(())` 实际记录 180 秒 Timeout，主 Agent 随后停止该批。此原始分类保留，不算通过；中间查询批 `target/p0-03-query-resume.QIbZEL` 的 7 caught 也不与最终候选混算。

该轮固定 `cargo-mutants 27.1.0`，正常 baseline、`--timeout 180 --jobs 1 --leak-dirs -C=--locked`，将 lib 原有 75 个变异按实际断言完整分区；每组均终态退出 0：

| 变异范围 | 实际测试入口 | 结果 | 本工作区 `target/` 下原始结果目录 |
|---|---|---|---|
| 64 项其余 runner 变异 | `--lib -- tests:: --skip '::tests::' --nocapture`，20 个根 lib 测试 | 50 caught、14 unviable | `p0-03-runner-final.BLi4zL/mutants.out/` |
| 3 项自由函数 set_nonblocking 变异 | `--lib -- tests::set_nonblocking_is_idempotent_and_preserves_existing_pipe_flags --exact --nocapture` | 3 caught | `p0-03-flags-final.AIYvQp/mutants.out/` |
| 1 项 OutputPipes::set_nonblocking 变异 | `--lib -- tests::bounded_timeout_preserves_killed_child_status_and_output --exact --nocapture` | 1 caught | `p0-03-pipes-final.uS7Bd8/mutants.out/` |
| 7 项 matching_paths/run_git 变异 | `--test attribute_types -- apple_git_attr_pathspec_preserves_filter_types_and_controlled_sources --exact --nocapture` | 7 caught | `p0-03-query-final.0akJOZ/mutants.out/` |

主 Agent 逐名核对四组并集与旧 e810 的 75 项完全相等、没有重复或遗漏；该候选为 **61 caught、14 unviable、0 missed、0 timeout**，14 个 unviable 均有编译错误日志。临时选择测试不是新增全局排除，也不是全 crate 门禁。3 个 flags 变异由实际标志断言检出；管道初始化删除由自有 sleep 子进程退出后的 signal 断言检出，约 5.02 秒收束；两个反转休眠门控分别在约 2.002 秒/1.000 秒取得结构化超时或不完整结果，有限输出断言失败，测试进程正常退出 101；查询组由实际非空/来源集合或结构化输出断言失败。没有用无关 Git fixture 超时替代这些目标证据。

主 Agent 在 32d 冻结输入执行《任务流程》三条通用命令及 `cargo test --locked --release -p thinws-p0-git-base --lib -- tests:: --skip '::tests::'`，session `23460` 终态退出 0：全 workspace **243 passed、0 failed、2 ignored**，release 根 lib **20 passed**。两项额外 APFS 卷测试仍因本工作区未配置第二卷而未执行；保留全部实验现场。该模块补测不证明空闲 CPU、硬实时调度或任意输出容量，不新增公开 CLI 行为；整项 P0-03 的兼容政策、正式 ADR、全 crate 门禁及远端 CI 仍未完成。

### 五项历史 Base 疑点的独立复验

保持当时的 lib `32d2b445…`、Base 模块 `c45d4df…` 及其余候选不变，主 Agent 执行 `cargo mutants -p thinws-p0-git-base -f experiments/p0/git-base/src/base_materialization.rs -F 'base_materialization.rs:(351:26|568:23|1770:12|2147:23):' --leak-dirs --timeout 180 --jobs 2 --output /Volumes/data/code/thinworkspace-p0-03/target/p0-03-base-recheck.qyzv4j -C=--locked -- -- --nocapture`。session `18996` 终态退出 0，正常 baseline 通过，5 项均为 caught、0 missed、0 timeout；`end_time=2026-09-13T04:05:43.389838Z`。

原两项 `!= → ==` 现在分别由显式隔离/重建和独立发布/复用正例的 `ManifestMismatch` 检出；固定名称集合的 `delete !` 由合法根名称被误拒绝的直接断言检出；两项溢出字节读取变异由纯 Cursor 的精确字节断言检出。主 Agent 核对五份原始日志，均无 `Timeout {` 标记，Test 分别在约 24.35、34.98、32.96、34.25、37.76 秒以 101 收束；命令保留完整 Git crate 测试参数，变异导致库测试失败时 Cargo 正常提前停止，并不声称每项变异后仍执行了所有 IT。这是新运行的有效检出证据，不覆盖旧批两项夹具超时 caught 与三项 Timeout 的原始分类，也不将本次 5 项和 runner 75 项拼成全 crate 通过。

本轮未启动完整候选的全 crate 变异、重新执行全部 fuzz/阶段长预算或远端代码 CI；兼容政策尚待维护者确认，先保留当前实验候选，不扩大实现范围。代码及现场仍保留在原任务工作区，文档提交不代表源码已提交、PR 合并或阶段放行。

### 测试句柄边界纠正与最终补测候选

Astra 逐日志核验 32d 的 75 项时发现，`stop_reading` 的即时 BrokenPipe 断言在四个未修改该方法的变异中收到 `Ok(())`；例如 `p0-03-runner-final.BLi4zL/mutants.out/log/experiments__p0__git-base__src__lib.rs_line_586_col_16.log`。这些日志仍有对应目标断言失败，因此没有把原存活变异误算 caught，但额外失败证明新测试不稳定，Reviewer 据此保留 Changes requested。并发 spawn 暂时继承句柄只是可能解释，未证实根因。

主 Agent 核验后收回不成立的测试推论：释放本实例的 owned reader 不证明全系统已无 reader；同样，关闭本实例 writer 不证明立即全局 EOF。Sol 只调整三个测试：保留 owned reader 清空、幂等、状态/字节保真；真实 pipe 在 writer 存活时验证 idle/data/双流进展；确定的 Read EOF 映射改用标准库 Cursor，真实双流最终 EOF 仍由已有有期限 collector 测试覆盖。没有新增进程隔离、等待重试框架、依赖、生产逻辑或排除项。Astra 对此限定差异给出 Approve；中间 `3c78084…` 不作最终验收输入。

最终 lib SHA-256 为 `3e800f36cfaef6e296c6c4e51ee92460a0afafe07b8f5cc4e98d870657b2c243`，生产区仍为上文 `2ff4fedf…`，其余 17 个原候选代码/配置/lockfile/harness 文件哈希全部保持。主 Agent 在此输入重跑相同的四组互斥分区，原始目录按上表顺序为 `target/p0-03-runner-verified.9o8x7X/mutants.out/`、`target/p0-03-flags-verified.r30Qle/mutants.out/`、`target/p0-03-pipes-verified.wWxckV/mutants.out/`、`target/p0-03-query-verified.XJbd9z/mutants.out/`。session `35770`、`47130` 均终态退出 0，全部 75 个唯一名称再次与原集合严格相等：**61 caught、14 unviable、0 missed、0 timeout**；14 个编译失败均已检查，未用排除或阈值降低消除存活项。最终日志中 owned reader 清空测试仅在删除 `stop_reading` 本身时失败。

最终输入的原样通用门禁和相同 release 根测试入口由主 Agent 执行，session `98611` 终态退出 0：全 workspace **243 passed、2 ignored**，release 根 lib **20 passed**。四份入口/过程 Markdown 的 42 个本地链接目标存在，`git diff --check` 通过；链接目标检查不代表锚点校验。未执行项、兼容政策待确认、代码暂未提交及不作完整 P0-03 放行的边界继续适用。

Astra / `gpt-6-astra` / `xhigh` 完成最终限定审核并给出 **Approve**：独立核对 75 个名称、四组终态、14 个编译失败及全部 caught 的目标失败范围，未发现非目标失败；同时认可本轮恢复状态与证据文档可单独提交。该批准仅覆盖 runner 补测及所核验的进度记录，不表示整个拓扑/Base 候选源码、任务或阶段已获批准。

### 当前候选短预算 fuzz、完整 Git Release 与远端前置审核复核（2026-09-13 UTC）

上一节证据已单独提交并推送为 `80782a5`；候选代码仍未提交。本轮 lib 保持 `3e800f36…`，根与 fuzz lockfile 分别保持 `caef7cd…`、`cdab36a…`，不改变兼容政策。主 Agent 使用固定 `cargo-fuzz 0.12.0`、`nightly-2026-08-14`，逐一执行四个 Git 相关目标的 `fuzz build` 和 `fuzz run`，均取得工具终态退出 0。

每个目标使用独立、初始为空的 corpus 和 artifacts 目录，预算参数统一为 `-max_total_time=60 -timeout=5 -max_len=4096 -verbosity=0 -print_final_stats=1`。以下目录均相对于本工作区的 `target/p0-03-fuzz-current.sgAOrV/`；各自 `run.log` 保存工具返回输出及其终态，`corpus/` 和 `artifacts/` 原样保留。

| 目标 | 子目录 | 执行输入数 | 工具报告 new_units_added | 最终 corpus 文件数 | artifacts 文件数 |
|---|---|---:|---:|---:|---:|
| `thinws_git_path_output` | `git-path` | 252,784 | 561 | 330 | 0 |
| `thinws_base_key_candidate` | `base-key` | 5,213,740 | 192 | 78 | 0 |
| `thinws_git_topology_validation` | `topology` | 1,584,645 | 308 | 187 | 0 |
| `thinws_checkout_validation` | `checkout` | 2,027,982 | 1,133 | 401 | 0 |

四次均未报告 crash 或单输入 timeout；`new_units_added` 与最终 corpus 数量含义不同，不能互换。topology 和 checkout 的 NEW_FUNC 输出各有 `Can't read from symbolizer at fd 3` / `atos failed to symbolize address` 警告，已保留，不称零警告或完整符号化。未调整 sanitizer、工具版本或警告设置。这些目标只覆盖其已声明的解析/纯校验边界，不证明真实 Git/文件系统行为、兼容政策或阶段长预算通过。

主 Agent 同时执行 `cargo test --locked --release -p thinws-p0-git-base --all-targets --quiet`，session `85733` 终态退出 0：**103 unit＋6 attributes IT＋25 topology IT＝134 passed、0 failed、0 ignored**。本次是完整 Git 包 Release 测试，不再仅为 runner 的 20 项；也不替代完整变异门禁。

按照维护者“不遗漏已推送 PR 审核”的要求，主 Agent 只读核对远端所有 PR、CI、审核评论和 reviewThreads：当前仅 #1、#2、#3，均已合并且对应 CI 成功，三者 reviewThreads 均为空，无开放 PR。规定模型审核证据分别在 [PR #1](https://github.com/chinayangxiaowei/thinworkspace/pull/1#issuecomment-5645043915)、[PR #2](https://github.com/chinayangxiaowei/thinworkspace/pull/2#issuecomment-5645915953)、[PR #3](https://github.com/chinayangxiaowei/thinworkspace/pull/3#issuecomment-5646263248) 的主 Agent 评论中；三者 GitHub `reviews` 数组为空，不能称已有独立账号的 GitHub Approval。详细前置任务结果仍以其原实施记录和 PR 为准，不在本文复制。P0-03 当前只有已推送分支，尚未创建 PR，不将旧 PR 的审核或 CI 外推到当前候选。

当前只读枚举为 Git crate **479** 项、workspace **993** 项变异。原全包入口会使非阻塞设置相关四项进入依赖非阻塞读取的其他测试；上文已记录取消设置后的实际 Timeout，不能外推为四项都已实测 Timeout。因此本轮准入仅调整 CI 的互斥分组：自由 helper 三项使用已验收的 flags 精确测试，OutputPipes 一项使用已验收的 bounded-timeout 精确测试，其余项目保留所属包完整测试。每轮须从实际工具枚举验证完整覆盖、无重复、无遗漏，不新增全局排除，不以 Timeout 作为检出。

Sol / `gpt-5.6-sol` / `xhigh` 完成唯一 CI 文件修改：相对准入 workflow `10f3475…` 为 +142/-2，保留原有未提交增量；冻结 SHA-256 为 `541d109a982d01cd9d6176b189640261746da076910244c9f01c8430b2b5acf6`。主 Agent 从实际 YAML 抽取预检执行，session `75167` 退出 0，工具实际枚举并核验 **993＝989＋3＋1**，不硬编码总数；同一 validator 对缺失、组内重复、额外 selected 和跨组重叠四种受控篡改均拒绝，session `47586` 退出 0。预检输出分别保留于 `target/p0-03-partition-main-20260912-50652-ucxdc1/` 和 `target/p0-03-partition-negatives.cVey7I/`；这些只枚举，不执行变异。

主 Agent 再从冻结 YAML 抽取 flags/pipes 两个真实命令，只把输出目录替换为本轮自有的 `target/p0-03-ci-four-20260912-64816-rkwfd1/{flags,pipes}/`，其余 workspace、正常 baseline、180 秒、jobs 2、leak-dirs、locked、lib 和精确测试参数均保留。session `52955` 终态退出 0：两组 baseline 通过，分别 **3 caught / 1 caught，0 missed、0 timeout、0 unviable**；两个 `mutants.out/outcomes.json` 的 end_time 分别为 `2026-09-13T05:05:13.812905Z`、`2026-09-13T05:05:30.896485Z`。三个 flags 变异由 NONBLOCK/重复设置断言失败，pipes 变异由 signal 断言失败；Test 均正常退出 101，后者工具记录约 5.37 秒、测试内部约 5.02 秒，不靠 180 秒 Timeout 检出。仅此四项已按新入口实跑，989 项主分区和远端 workflow 尚未执行。

本轮原样通用门禁由主 Agent 执行，session `62779` 终态退出 0：fmt、全 workspace Clippy 与普通测试 **243 passed、2 ignored**；两项额外 APFS 卷用例因未配置第二卷仍未执行。fuzz 格式检查、YAML 语法解析、本文 6 个本地链接目标存在性和 `git diff --check` 通过；YAML 工具保留本机 Ruby ffi 扩展提示，链接检查不包含锚点。现有 CI 的 45 分钟总容量尚无当前全量实测证明，本轮不自行增大预算、不发布未审核源码，所有实验现场保留。

Astra / `gpt-6-astra` / `xhigh` 对冻结 workflow 的本次分区增量及本节进度记录给出限定 **Approve**：独立核验动态分区、四项真实 caught 的 baseline/终态/目标日志和四份 fuzz 证据，未发现非目标失败；允许进度文档单独提交。批准不覆盖未执行的主分区、CI 总容量或全候选。CI 修改依赖尚未提交的实验候选，本轮与这些源码一同保留在原工作区，不夹带进文档提交。

P0-03 保持 In Progress。仍须维护者确定无实际转换属性声明及 sparse 来源的产品边界，随后完成对应兼容矩阵、正式 ADR、完整候选门禁、全候选规定审核、源码提交和远端候选 CI；旧 439 项中断、75 项 runner 和五项 Base 专项不能拼成 479 项通过。

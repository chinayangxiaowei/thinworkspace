# P0-03 Git 托管拓扑与 Base 实施记录

## 职责与非职责

本文管理 P0-03 的准入、实验验收、候选 ADR 证据和未解决边界。不重新定义产品 Git 拓扑、BaseKey 字段、公开兼容范围或任务门禁；实验结论改变长期设计时，必须修改对应权威来源，不能只留在本文。

## 任务准入

| 项目 | 当前值 |
|---|---|
| 类型、阶段、小阶段、风险 | Spike/ADR；P0；P0.b；R4（Git refs、发布与精确回收） |
| 状态 | In Progress；首个属性类型判别切片经主 Agent 准入，基线门禁通过后领取；全任务 Git/Base 验收尚未完成 |
| 负责人 | 主 Agent |
| 模型分工 | 编码拟使用 GPT-5.6 Sol / `gpt-5.6-sol` / `xhigh`；审核必须使用 GPT-6 Astra / `gpt-6-astra` / `xhigh` |
| 准备日期、环境 | 2026-09-12；实测 macOS 15.7.2（24G325）、arm64、Apple Git 2.39.5（Apple Git-154） |
| 初始源码基线 | 已合并并通过 CI 的 `main`：`2e29f88a1aae2ee53ce52dbc38a7c0765b58cb15` |
| 当前集成基线 | `2a1250d50a9073c7ca8f59b35e65469307e76732`；已合入 P0-02 的 `main` 合并提交 `df989a8730057f26622363399ea4b046245d5365`，原 P0-03 准备记录及本次未提交补充保留 |
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
- 尚无本任务代码、合成 Git 仓库、Base、实验分支或物化测试结果；工作区中的项目任务分支仅用于开发，不是被测产品的托管分支。
- GPT-6 Astra / `xhigh` 完成任务卡与两处导航的只读复核，无阻断发现，认可这三个准备文档可提交；此结论不表示 Ready、实现批准或 Go。
- 以上为准入准备时的证据，不作 Go 或阶段放行声明。2026-09-12 主 Agent 根据规定 Reviewer 的限定意见补齐首个切片验收，完成 Ready 检查并领取为 In Progress；不是把只读审核直接视为状态批准。
- 当前集成基线的 fmt、Clippy 和原样普通 workspace 测试均退出 0：109 passed、0 failed、2 ignored。两个跨卷用例因未配置本工作区专用第二卷而未执行，不替代 P0-02 的真实双卷证据；测试产生的保留现场不删除。尚未取得本切片 RED/GREEN 或实际 Git 属性判别结果。

# P0-02 APFS clone 与跨卷实施记录

## 职责与非职责

本文保存 P0-02 的任务准入、实验覆盖、验收断言、执行证据和 Go/No-Go 结论。本文不定义产品 Port、完整 Plan/Receipt schema、Git 拓扑或阶段放行规则，也不把实验代码直接认定为 Phase 1 实现。

## 任务准入

| 项目 | 当前值 |
|---|---|
| 类型、阶段、小阶段、风险 | Spike；P0；P0.b；R4（FFI、创建与回滚删除） |
| 状态 | Backlog；任务卡准备中，尚未开始物化编码 |
| 负责人 | 主 Agent |
| 编码模型 | 计划使用 GPT-5.6 Sol / `gpt-5.6-sol` / `xhigh` |
| 审核模型 | GPT-6 Astra / `gpt-6-astra` / `xhigh`；已完成只读准入审查，非实现批准 |
| 准备日期、基线 | 2026-09-12；`7d350a91df5c1b3612cf9fc3bc3d545e8a53f690` |
| 工作区、分支 | `/Volumes/data/code/thinworkspace-p0-02`；`task/p0-02-apfs-materialization`；标准 Git linked worktree，不声明 CoW |
| 前置依赖 | P0-01 输出稳定；当前等待其完整远端 CI 与正式审核，编码前重新核对 |
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
- 本任务卡、索引和计划链接已由同一 Reviewer 复核通过。新工作区基线 fmt、Clippy、普通测试通过（49 passed、1 个需专用双卷环境的测试 ignored），22 个相对链接检查通过；这些结果只验证文档与既有 P0-01 基线，不证明本任务物化能力。P0-01 远端质量步骤均通过但最终卷卸载失败，当前继续等待其清理修复与整轮 CI。
- 尚未编写本任务代码，未执行真实 clone、Copy、回滚、变异或新增 fuzz；未创建本任务 APFS 镜像。待 P0-01 完整 CI/最终审核后核验基线并启动编码。

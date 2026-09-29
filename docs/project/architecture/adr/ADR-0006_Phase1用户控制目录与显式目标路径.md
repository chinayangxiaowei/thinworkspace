# ADR-0006：Phase 1 用户控制目录与显式目标路径

**状态：Accepted｜日期：2026-09-28｜适用范围：Phase 1 新布局**

## 职责与非职责

本文决定实例控制数据、用户指定目标、持久归属和删除边界，取代 ADR-0004 中的单一 data root 布局及其持久化和锁协议。ADR-0004 保留为旧候选的历史记录，不再指导新布局实现。

本文不重新定义文件内容保真、Git 检查或用户可见 JSON 字段；这些分别由物化设计、单机 CLI 详细设计和用户操作手册维护。本文冻结新持久化字段与不变量；DDL 和编码必须实现这些约束，不得借实现细节恢复旧 data root。

## 背景

旧版将配置、SQLite、锁、日志、暂存、隔离和所有工作区绑定到一个 APFS data root。源目录必须与该 root 同卷，因此本机其他 APFS 卷上的项目无法获得低空间克隆，即使用户能指定另一个输出位置也无效。控制数据所在卷与本次克隆所需的源/目标同卷关系本应是两个独立问题。

## 决策

1. macOS 用户的固定控制目录为 `~/.thinws/`。实例配置、SQLite、生命周期锁、日志和 Workspace 持久归属证据均位于其中；目录、文件及路径组件必须按既有 no-follow、属主和私有权限边界验证。首版控制目录本身也须位于 APFS 卷，但它与某次 source/target 可以是不同的 APFS 卷。`init` 不接受 data root 参数；控制目录所在卷不决定 Workspace 可创建的卷。
2. `workspace create` 必须接收用户指定的**最终 target 绝对目录路径**。目标末级在创建前必须不存在，不能覆盖已有空目录；来源、目标及控制目录不得相等或相互包含。新 target 不得与任一活跃 Workspace 的 target 相等或相互包含，也不得进入该 Workspace 可清理的受控暂存、回滚或隔离目录；APFS 大小写/Unicode 别名按实际目录身份识别。否则清理旧 Workspace 可能误删新 Workspace。创建预览与正式创建均须拒绝冲突，正式创建在 lifecycle lock 内重验；已登记目录的别名归为 target 冲突，普通未登记既有目录才归为目标已存在。创建记录以 WorkspaceId、名称和完整 target 路径关联；名称及 target 均唯一，不从统一 data root 推导目标。
3. macOS 首版仍要求本次 source 与 target parent 位于同一 APFS Volume，且实际 clone 前重验卷与路径身份。`--allow-copy` 只允许该路径组合中明确的同卷 Full Copy 降级，不是跨卷开关。控制目录可以在另一个卷。APFS Volume UUID 而非磁盘名、路径前缀或 container ID 是同卷判断依据。
4. 目标卷在成功完成后不保留平台配置、数据库、锁、日志或归属文件；最终 target 是可直接使用的普通克隆目录。为保证同卷 clone、原子发布或隔离删除，操作期间可在 target parent 下使用仅属于本次操作的私有暂存/隔离项。它们须有 WorkspaceId 及持久身份关联；中断残留不自动扫描或回收，不得按名称猜测归属。
5. 使用单一 `~/.thinws` SQLite 状态与一把本机 lifecycle lock 协调所有卷上的创建、删除和名称/target 唯一性。数据库先记录非 Ready 过渡状态；target 创建后的卷、父目录和 target 自身历史身份须在外部物化前持久化为归属证据。创建与删除都不依赖文件系统和 SQLite 之间不存在的原子事务，也不宣称中断后自动续做。
6. 清理前必须从登记记录定位 target，并以 no-follow 打开和核对其创建时卷与目录身份；若先前同一删除尝试已将该目录隔离，只有可证明是同一 target 的隔离项才算目标仍存在。target 在原位置及受控隔离位置均缺失、磁盘离线、路径被替换或身份不符时，普通清理和 `--force` 都不得删除其他路径、写删除 tombstone 或释放名称；登记和诊断信息保留。`--force` 只影响内容检查，不越过目标存在与归属证明。
7. 物理内容删除完成而 SQLite 收口前若中断，可能留下“target 不存在、登记仍在”的状态。首版宁可保留关联，也不因路径缺失自动解除登记；不提供隐式 `forget`、GC 或修复。未来若需要只解除登记，必须作为独立显式命令重新决策，不能混入 `remove --force`。
8. 新布局与旧版**不迁移、不兼容**。新程序不读取、接管或自动删除旧 `~/Library/Application Support/ThinWorkspace/` 配置及旧 data root；旧数据是否手工处置由用户另行决定。不能把旧候选的测试、变异或人工放行证据直接用于新布局。

## 新布局的持久化与锁契约

- `~/.thinws/config.toml` 和 `~/.thinws/.thinws-control.toml` 使用新的文档版本 `2`，都记录 `instance_id`、`control_root_hex`、`control_volume_id`；控制标记另含 `initializing|ready`。路径字节用规范小写偶数位十六进制。文档拒绝未知字段、旧 `data_root_hex` 和旧版本。控制目录自身以及 `metadata/`、`logs/` 为私有目录，不再创建 `workspaces/`、`staging/`、`trash/` 子目录。
- 数据库固定在 `~/.thinws/metadata/state.db`，新库 `user_version=2`；只初始化空库，不对 v1 执行迁移，也不读取旧库。`installation` 单例包含 `instance_id`、`control_root`（BLOB）、`control_volume_id`、创建时间。`workspaces` 保留状态、名称、来源、创建策略、时间和回执关系，目标字段为用户指定 `target_path`（BLOB、活跃唯一）、`source_volume_id` 与 `target_volume_id`；插入约束要求两者相等且只校验所属 `instance_id`，**不得**要求它们等于 `installation.control_volume_id`。旧 `data_volume_id` 字段不进入新表。状态与 receipt/tombstone 的插入顺序和不可变约束维持旧版强度；缺失目标时不插入 tombstone。
- macOS 每个 Workspace 在 `~/.thinws/metadata/` 有独立的版本 `2` 归属文档，包含实例与 Workspace ID、规范 target 路径字节、目标卷 UUID、创建时目标父目录和 target 自身的历史目录身份（至少 inode 与 birthtime）；隔离删除开始前，还须持久记录隔离项的精确路径与同一目标历史身份。重验要求同一 APFS Volume UUID、同 inode，且当前 birthtime **不晚于**登记的原始 birthtime 上界；真实 APFS 上调早目录 mtime 可能同时调早 birthtime，故不能要求严格相等。此证明拒绝普通路径替换，但不宣称抵御同 UID 主动回填身份或刻意制造的 inode 复用。归属文档没有有效匹配、登记路径已换对象或两个候选位置冲突时，不以 `--force` 猜测。数据库行与归属文档必须逐字段核对，不能只信其一。Linux 归属版本及身份差异由 [ADR-0007](ADR-0007_Phase1_Linux_Btrfs_CLI适配.md) 决定。
- 成功删除 target 并提交删除 tombstone 后，首版仍保留对应归属文档作为本机历史身份与清理诊断资料；它不表示 Workspace 仍活跃，也不参与按已删除 ID 重试时的路径删除授权。`workspace remove` 不清除此文档，Phase 1 也不提供 GC 或按目录名自动回收它。后续若要设置留存期限或删除历史归属，须另行定义精确 ID、数据库状态、文件身份与中断失败边界，不得凭 `ownership-*.toml` 名称批量删除。
- 唯一 OS advisory lock 文件为 `~/.thinws/lifecycle.lock`，首次 init、创建和删除均持有它；只读 doctor/list/path/status 不以该锁作为前置。该文件 PID 仅是诊断，锁正确性来自持有的文件描述符及路径身份重验。SQLite busy timeout 不替代 OS 锁。旧 `init.lock`、`metadata/lifecycle.lock` 不属于新布局。
- 首次 init 可创建空的固定控制目录及 lifecycle lock；除该已验证锁文件外，非空而无本次创建证明的控制目录不得被接管。控制标记先以 `initializing` 写入，完成私有布局与数据库后发布 `ready`，最后发布 config；进程重启不能把旧 `initializing` 自动提升。文件模式 `0600`，私有目录 `0700`；使用 no-follow、create-new 和同步发布，与 SQLite 不假设跨系统原子性。

## 代价与备选方案

与“每卷一个 data root”相比，单一控制目录和显式 target 不需要 root 注册表、跨 root 查询及名称消歧，但任意 target 的路径安全、持久目录归属和中断残留更难验证。与旧版固定私有根相比，target parent 是用户指定位置；创建与删除必须在真实 APFS 上验证路径替换、同卷临时项、卷离线和重复清理，不得以普通字符串路径授权递归删除。

拒绝把 `~/.thinws` 当作所有暂存/隔离项的物理位置：当 target 在另一卷时，APFS clone 与原子 rename 都不能跨卷完成。拒绝在缺失 target 时自动删除数据库行：外置卷卸载会把仍存在的克隆误判为已删除。

## 验证与实施边界

- 新 `init` 仅建立和校验固定控制目录；旧配置、旧 data root 字节保持不变。
- 两个不同 APFS 卷各自的 source/target 同卷组合均能创建，并且控制目录可位于第三个卷；source/target 异卷明确拒绝，`--allow-copy` 不绕过。
- 任意目标已有、源/目标或活跃 Workspace 可清理范围与新 target 包含、符号链接、父目录或卷被替换均在写入或删除前安全失败。
- `remove` 在 target 缺失、外置卷离线、身份不符时保留名称、登记和旧数据，即使有 `--force`；仅在目标归属可证且实际清理完成时收口。
- 创建/删除各故障点、真实 APFS、CLI/JSON、SQLite 约束、变异与模糊测试按受影响范围重证；旧候选证据不能替代。

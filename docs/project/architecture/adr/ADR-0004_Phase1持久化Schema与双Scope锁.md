# ADR-0004：Phase 1 持久化 Schema 与双 Scope 锁

**状态：Accepted｜日期：2026-09-23｜适用范围：Phase 1 / P1-02 起**

## 一、职责

本文是以下持久化事实的唯一精确来源：

- bootstrap `config.toml` 与 data root `.thinws-root.toml` 的版本化字段、发布顺序和冲突规则；
- SQLite schema v1、迁移规则、数据库约束和并发唯一性；
- bootstrap 与 data-root lifecycle 两个 advisory lock scope 的路径、互斥和超时语义。

## 二、非职责

本文不定义 CLI 参数、输出或退出码，不复制 Workspace 创建/删除步骤，不定义 APFS 物化 Receipt 的业务字段，也不引入中断恢复。公开行为以用户手册为准，生命周期顺序和状态机以 Phase 1 详细设计为准，最终 Receipt 的类型语义以跨平台物化设计为准，系统库选择以《技术栈》为准。

## 三、背景

Phase 1 需要在没有 daemon 的多个 CLI 进程之间保持三类事实一致：固定 bootstrap 配置、data root 归属和 SQLite Workspace 状态。文件系统与 SQLite 没有跨系统事务，因此必须先持久化非 Ready 状态，最后才发布可用结果；进程退出后不得根据目录外观自动补写 Ready。

P1-02 还需要解决两个独立竞争面：首次初始化竞争发生在 data root 建立之前，只能由 bootstrap lock 协调；Workspace 创建、删除和 GC 发生在已验证 data root 内，由另一个 lifecycle lock 串行。SQLite busy timeout 只处理数据库页锁，不能替代这两个 OS 锁。

## 四、决策

### 4.1 版本与身份表示

首版持久化 schema version 均为 `1`。`InstanceId`、`WorkspaceId` 和 Workspace 名称继续使用 Core 值对象；不得在 Adapter 内建立第二套解析规则。

Phase 1 首发平台的稳定 `VolumeId` 表示为规范小写、带连字符的 UUID 文本。它是从已打开目录取得的 APFS Volume UUID，不是卷名、设备显示名或 APFS container ID。路径以 macOS 文件系统原始字节保存：TOML 使用小写偶数长度十六进制，SQLite 使用 `BLOB`。解码后必须仍是无 NUL、无 `.`/`..` 组件的规范绝对路径；展示字符串不参与身份比较。

时间字段使用非负 Unix 毫秒整数，语义为 UTC。P1-02 不持久化源码、patch、凭据、完整环境、构建日志或可重放 Operation。

### 4.2 Bootstrap 文件契约

固定 bootstrap 目录仍由详细设计决定。两个 TOML 文档都拒绝未知字段，最大编码长度为 64 KiB；字段如下：

```text
config.toml
  schema_version = 1
  instance_id
  data_root_hex
  volume_id

.thinws-root.toml
  schema_version = 1
  instance_id
  data_root_hex
  volume_id
  state = "initializing" | "ready"
```

同一记录中的 ID、路径和 Volume ID 必须通过类型校验。读取到未知 schema、无效编码、符号链接、非普通文件或不安全权限时返回结构化失败，不能当成缺失。

发布遵循详细设计的初始化顺序，并使用以下文件级规则：

1. `initializing` root marker 只能以 create-new 语义写入尚无 marker 的已验证 data root；任何已有 `initializing` marker 都作为未完成初始化报告，不能以字段相同为由自动接管。同一次进程内流程只保留自己成功创建 marker 的内存事实，不重新发布它。
2. 创建 `initializing` marker 成功时，BootstrapStore 返回不可复制的本次初始化证明，内部持有该 marker 的打开文件身份；`ready` 发布必须消费这份证明、仍持有 bootstrap lock，并重新核对父目录、当前 marker 的文件身份与完整字段。进程重启后没有该证明，不能仅凭已有 `initializing` marker 提升为 `ready`。已有完全相同的 `ready` marker 只可由重复 init 读取为幂等结果，不能退回 `initializing`。
3. bootstrap config 最后发布。已有完全相同的 config 幂等；任一字段冲突都拒绝覆盖。
4. 每次发布先在同一目录 create-new 私有临时普通文件并写完、`fsync`。首次 `initializing` marker 与 bootstrap config 必须使用原子 no-replace 发布；目标在发布前出现时操作失败并读取/分类现有项，绝不使用普通覆盖 rename。
5. `ready` 是唯一获准替换。实现须使用原子 exchange/等价的 compare-and-replace 协议，把交换出的旧 marker 与本次初始化证明再次比对；只有身份和字段完全一致才删除旧 marker并确认发布。比对失败时不得发布 bootstrap config；只有在能证明目标仍是本次新文件、交换出的项仍是刚才旧项时才可交换回去，否则保留可解释残留并返回未完成，不能覆盖第三方新对象。
6. 成功发布后 `fsync` 父目录。文件模式为 `0600`，管理目录和 data root 模式为 `0700`。所有目标项均以 no-follow 方式打开和校验；进程崩溃可能留下本次未发布的临时文件或没有 bootstrap config 的未完成 data root，Phase 1 不跨进程续做或自动接管。

bootstrap config、ready root marker 和 SQLite `installation` 单例必须逐字段一致；任何缺失、schema 不支持或身份冲突均安全失败。

### 4.3 SQLite 连接与迁移

数据库位于 `metadata/state.db`，SQLite `application_id` 固定为十进制 `1414027091`（ASCII `THWS`），`PRAGMA user_version` 保存 schema version。每个读写连接必须实际设置并验证：

```text
journal_mode = WAL
synchronous = FULL
foreign_keys = ON
busy_timeout = 调用方提供的有界时限
```

迁移规则：

1. `application_id=0` 只在数据库没有用户表时可初始化；其他未知 application ID 拒绝接管。
2. v0 到 v1 的 DDL、`application_id`、`user_version` 和 `installation` 单例写入在一个 SQLite 事务中完成。
3. 打开 v1 必须验证 `installation` 与 bootstrap/root identity 完全一致。
4. 比当前二进制新的版本直接失败；未来迁移只能逐版本、单向、事务化，且必须测试空库和每个仍受支持旧版本。
5. 不在 SQLite 事务中等待文件物化、Git、外部进程或 lifecycle lock。

### 4.4 SQLite schema v1

以下 DDL 是 v1 的权威模型。实现可以拆分语句，但不得改变约束语义。

```sql
CREATE TABLE installation (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    instance_id TEXT NOT NULL UNIQUE
        CHECK (
            length(instance_id) = 36
            AND substr(instance_id, 9, 1) = '-'
            AND substr(instance_id, 14, 1) = '-'
            AND substr(instance_id, 15, 1) = '7'
            AND substr(instance_id, 19, 1) = '-'
            AND substr(instance_id, 20, 1) IN ('8', '9', 'a', 'b')
            AND substr(instance_id, 24, 1) = '-'
            AND instr(instance_id, char(0)) = 0
            AND length(replace(instance_id, '-', '')) = 32
            AND replace(instance_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    data_root BLOB NOT NULL UNIQUE CHECK (length(data_root) > 0),
    volume_id TEXT NOT NULL
        CHECK (
            length(volume_id) = 36
            AND substr(volume_id, 9, 1) = '-'
            AND substr(volume_id, 14, 1) = '-'
            AND substr(volume_id, 19, 1) = '-'
            AND substr(volume_id, 24, 1) = '-'
            AND instr(volume_id, char(0)) = 0
            AND length(replace(volume_id, '-', '')) = 32
            AND replace(volume_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0)
) STRICT, WITHOUT ROWID;

CREATE TABLE workspaces (
    workspace_id TEXT PRIMARY KEY
        CHECK (
            length(workspace_id) = 39
            AND substr(workspace_id, 1, 3) = 'ws_'
            AND substr(workspace_id, 12, 1) = '-'
            AND substr(workspace_id, 17, 1) = '-'
            AND substr(workspace_id, 18, 1) = '7'
            AND substr(workspace_id, 22, 1) = '-'
            AND substr(workspace_id, 23, 1) IN ('8', '9', 'a', 'b')
            AND substr(workspace_id, 27, 1) = '-'
            AND instr(workspace_id, char(0)) = 0
            AND length(replace(substr(workspace_id, 4), '-', '')) = 32
            AND replace(substr(workspace_id, 4), '-', '')
                NOT GLOB '*[^0-9a-f]*'
        ),
    instance_id TEXT NOT NULL,
    name TEXT NOT NULL COLLATE BINARY UNIQUE
        CHECK (
            length(name) BETWEEN 1 AND 63
            AND instr(name, char(0)) = 0
            AND name NOT GLOB '*[^a-z0-9._-]*'
            AND substr(name, 1, 1) GLOB '[a-z0-9]'
            AND substr(name, -1, 1) GLOB '[a-z0-9]'
            AND instr(name, '..') = 0
        ),
    source_path BLOB NOT NULL CHECK (length(source_path) > 0),
    target_path BLOB NOT NULL UNIQUE CHECK (length(target_path) > 0),
    source_volume_id TEXT NOT NULL,
    data_volume_id TEXT NOT NULL,
    allow_full_copy INTEGER NOT NULL CHECK (allow_full_copy IN (0, 1)),
    state TEXT NOT NULL
        CHECK (state IN ('creating', 'ready', 'deleting', 'error')),
    last_error_code TEXT,
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL
        CHECK (updated_at_unix_ms >= created_at_unix_ms),
    FOREIGN KEY (instance_id) REFERENCES installation(instance_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    CHECK (
        (
            state = 'error'
            AND last_error_code IS NOT NULL
            AND last_error_code GLOB 'E_*'
        )
        OR (state <> 'error' AND last_error_code IS NULL)
    )
) STRICT, WITHOUT ROWID;

CREATE INDEX workspaces_state_name ON workspaces(state, name);

CREATE TABLE materialization_receipts (
    workspace_id TEXT PRIMARY KEY,
    receipt_schema_version INTEGER NOT NULL CHECK (receipt_schema_version > 0),
    receipt_json TEXT NOT NULL CHECK (json_valid(receipt_json)),
    recorded_at_unix_ms INTEGER NOT NULL CHECK (recorded_at_unix_ms >= 0),
    FOREIGN KEY (workspace_id) REFERENCES workspaces(workspace_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

CREATE TABLE deletion_tombstones (
    workspace_id TEXT PRIMARY KEY,
    instance_id TEXT NOT NULL,
    deleted_at_unix_ms INTEGER NOT NULL CHECK (deleted_at_unix_ms >= 0),
    FOREIGN KEY (instance_id) REFERENCES installation(instance_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID;
```

四张表均为 `WITHOUT ROWID`，业务主键是唯一的行身份，不保留可被 `INSERT OR REPLACE` 单独命中的隐藏 rowid。v1 同时建立以下触发器；触发器错误是数据库不变量失败，不替代 Core/Application 的业务判断：

- `installation` 单例只能首次插入，禁止更新、删除或用 `INSERT OR REPLACE` 替换；切换 data root 不以数据库更新实现。
- Workspace 只能以 `creating` 插入；其 ID、名称、目标路径的任何唯一冲突均拒绝 `INSERT OR REPLACE`，其余实例、路径、卷、copy 许可和创建时间随后不可变。
- 插入 Workspace 时，`source_volume_id` 和 `data_volume_id` 必须都等于 `installation.volume_id`，且 ID 不能已存在于 tombstone。
- 状态变化只允许 `creating→ready|error|deleting`、`ready→deleting|error`、`deleting→error`、`error→deleting`；同值写入不构成迁移。`updated_at_unix_ms` 不得回退。
- `ready` 更新前必须已有同 Workspace 的 final receipt；P1-02 不提供写 Ready 的公共存储方法，直到 P1-06/P1-07 冻结类型化 Receipt、P1-09 在同一短事务写 receipt 并转 Ready。
- final receipt 只能在 Workspace 为 `creating` 时首次插入，之后不可更新，也不可用 `INSERT OR REPLACE` 改写。只要父 Workspace 行仍存在，直接删除 receipt 必须由 trigger 拒绝；仅允许在满足 tombstone 前置条件并删除活跃 Workspace 时，由 `ON DELETE CASCADE` 删除 receipt。
- tombstone 只能为 `deleting` Workspace 首次插入，不可用 `INSERT OR REPLACE` 改写。删除活跃记录前必须已有同 ID tombstone；两步必须位于同一事务。tombstone 不保存名称、路径、源码或日志，也不可更新或删除。

Core 状态事件仍负责区分普通删除与显式 force。数据库只接受详细设计允许的状态边，不把直接 SQL 能通过解释为用户已授权。

### 4.5 P1-02 存储 API 边界

P1-02 实现以下最小可调用能力：

- 初始化/验证 schema 与 installation 单例；
- 以单条受约束 insert 预留 `creating` Workspace，不使用“先查再写”保证唯一性；
- 读取单个或稳定排序的全部活跃记录；
- 按 expected-state 条件更新记录失败或进入获准的 `deleting`，并把并发变化作为冲突返回；
- 在一个事务中写最小 tombstone 并删除已处于 `deleting` 的活跃记录。

P1-02 不实现 Workspace 物化、不写 final receipt、不转 Ready、不删除目录，也不实现 init/doctor CLI。后续任务不得绕过 Port 直接从 Application/CLI 执行 SQL。

### 4.6 双 Scope lifecycle lock

锁文件位置：

```text
bootstrap scope: ~/Library/Application Support/ThinWorkspace/init.lock
data-root scope: <data-root>/metadata/lifecycle.lock
```

两者使用各自文件描述符上的排他 advisory lock，文件以 no-follow、create-if-missing、`0600` 打开，并验证为当前用户拥有的普通文件。成功取得锁后、写 PID 或修改任何目标状态前，必须重新读取锁文件目录项并确认它仍指向持锁文件描述符的同一身份；被替换或链接数异常时释放并失败。获取过程使用非阻塞尝试和单调时钟等待；到达调用方时限仍竞争则返回 timeout，且等待期间不得修改目标状态。

锁文件内 PID 仅在成功持锁后写入，供诊断使用；不能通过读取 PID 判定锁所有权，也不终止该进程。guard 持有文件描述符，drop/close 释放锁。两个 scope 相互独立：持有 bootstrap lock 不阻止已初始化 data root 的生命周期操作，反之亦然。当前 Phase 1 没有同时持有两把锁的用例；初始化全过程只持 bootstrap lock，创建/删除/GC 只持对应 data-root lock。

查询命令不取得 lifecycle lock，也不写 SQLite。测试可注入 bootstrap 目录、data root 和较短时限；生产位置不能由隐藏环境变量覆盖。

### 4.7 P1-03 初始化调用边界

P1-03 不增加第八个概念 Port。`BootstrapStore` 在 P1-02 文档发布能力之外增加四个同职责调用：`prepare_bootstrap` 准备固定 bootstrap 目录；`prepare_data_root` 逐段准备并返回不可伪造的 `PreparedDataRoot`，其中持有最终目录 FD、目录身份、规范路径和实际 Volume ID；`initialize_layout` 只接受本次 `InitializingProof` 并建立受控布局；`validate_layout` 只读验证既有布局与登记身份。

证明链不得退化为裸路径：`create_initializing` 必须消费 `PreparedDataRoot`，核对 Application 提供的 identity 后使用其中的 FD 写 marker，不得按 identity 路径重开 data root；返回的 `InitializingProof` 继续持有同一目录证据。`initialize_layout` 只用该证明做 dirfd-relative 创建，并通过 no-follow、create-new 预建 `0600` 的空 `state.db`，然后返回不可由 Application 自行构造的 `DataRootLayout`；它绑定 data root、metadata 目录、`state.db` 目录项身份、Volume ID 和版本化数据库规范路径。SQLite factory 只接受该布局能力而非 `Path`，以 no-create 方式打开数据库，并在打开前、打开后、事务提交后分别触发布局身份/卷重验；任何失败都禁止发布 Ready/config。重复 init 与 doctor 的 `validate_layout` 也返回同类只读能力。P1-03 不用自定义 SQLite VFS 宣称无法提供的绝对防竞态保证，但把每个外部写边界限制到同一证明链，并在紧邻写入处检测目录替换。

`MetadataStoreFactory` 是 MetadataStore Port 的构造边界，不是新的存储模型。它只允许两类显式操作：`initialize` 接受 `DataRootLayout` 与期望 installation，在读写事务中初始化或核对并返回实际 installation；`inspect` 以 SQLite read-only 模式接受同类布局与期望 installation，验证现有 `application_id`、`user_version`、schema 和 installation，并返回 installation 及按 WorkspaceId 稳定排序的活动 Workspace 快照。`inspect` 不创建主数据库、不执行 migration、不修改主数据库或 journal mode；两者都不向 Application 泄漏 SQLite 连接。

SQLite 的“read-only”是产品状态只读，不等于目录字节零变化。WAL 模式读取可能创建、更新或删除 `state.db-wal`/`state.db-shm` 协调文件；P1 允许这一 SQLite 引擎行为，但不把它记录为产品成功或修复。如果辅助文件不可访问则结构化失败，不得回退 `immutable=1`。专项测试至少覆盖：初始无 sidecar、已有未 checkpoint WAL，以及并发写连接存在时读取到一致的已提交状态。

CLI crate 是 composition root：业务调用和 renderer 只面向 Application，但生产装配可以直接依赖 macOS/SQLite Adapter 以构造 Port 实现。该依赖只做 wiring，不能在 CLI 复制初始化顺序、身份判断或错误策略。

## 五、备选方案

### 5.1 只使用 SQLite 锁

拒绝。初始化时 SQLite 尚不存在，且数据库页锁无法覆盖目录创建、marker 发布、物化或清理。

### 5.2 一把全局锁覆盖所有 data root 操作

拒绝。它会让已完成初始化的 Workspace 生命周期无谓依赖 bootstrap 目录，也混淆首次初始化与 data root 归属。Phase 1 虽只登记一个 data root，仍保留两个职责明确的 scope。

### 5.3 检查后插入保证名称唯一

拒绝。多个 CLI 进程可同时通过检查；唯一约束和单条 insert 才是并发正确性依据。

### 5.4 目录存在时自动补写 Ready 或恢复初始化

拒绝。目录存在不能证明 marker、SQLite、Receipt 或卷身份一致，也违反 ADR-0003。

### 5.5 在 v1 展开尚未冻结的 Receipt 全部字段

拒绝。P1-02 只预留版本化、受外键保护的 final receipt envelope，并禁止提前转 Ready；类型化 payload 由物化任务冻结。若后续需要新的可查询约束列，必须以单向 migration 增加，不能静默改变 v1。

## 六、代价与影响

- 原子文件发布、目录同步和 no-follow 检查比直接覆盖 TOML 复杂，但避免半写文件和链接替换被误认成合法归属。
- SQLite trigger 与 Core 状态事件形成纵深校验；二者必须使用同一状态边，测试需防止漂移。
- WAL 会产生 `state.db-wal`/`state.db-shm`，它们属于 metadata，不是 GC 或 Workspace 清理对象。
- advisory lock 不能阻止用户进程直接改写源或副本；它只协调 ThinWorkspace 生命周期命令。
- crash 后可能留下 `initializing` marker、`creating/deleting/error` 记录或未发布临时文件；这些是可报告残留，不触发自动恢复。

## 七、迁移与实施

P1-02 激活已有规划目录中的 `thinws-ports`、`thinws-adapter-macos` 和 `thinws-metadata-sqlite`，仅在存在实际 trait、实现和测试时加入 workspace，不创建空 crate。依赖保持 `adapter/metadata → ports → core`；Core 不依赖 SQLite、TOML、rustix 或具体路径布局。

P1-03 依据详细设计编排初始化和 doctor，使用本 ADR 的原子发布与锁能力。P1-06/P1-07 定义类型化 Receipt，P1-09 才获得写 receipt 并转 Ready 的用例。任何改变表关系、状态边、root/config 字段或 lock scope 的方案都必须更新本 ADR并重新审核。

## 八、验证计划

P1-02 至少验证：

1. TOML 新建、幂等、冲突、未知 schema、损坏/超限输入、符号链接、首次发布目标竞态和 `initializing→ready` 单向发布；重启后或使用另一份 marker 证明不能提升 Ready；
2. 空库 v0→v1、v1 重开、未知 application ID、未来版本、失败迁移原子性及四项 PRAGMA；
3. 两连接同时争用名称和目标时只有一个 insert 成功，数据库重开后 `creating/error/deleting` 不自动变 Ready；
4. InstanceId、VolumeId、WorkspaceId 与名称的嵌入 NUL/尾随垃圾均被拒绝；Error 缺少错误码和非 Error 携带错误码均被拒绝；
5. 非法状态边、无 Receipt 的 Ready、直接删除 Receipt、无 tombstone 的删除以及错误实例/卷均被拒绝；父 Workspace 合法删除时 Receipt 级联成功；
6. tombstone 与活跃记录删除同事务完成，注入失败时二者不出现半提交；
7. 两个 lock scope 独立，同 scope 跨进程竞争有界超时，释放后可重新获得；锁文件符号链接、非普通文件和取得锁后的目录项替换均失败；
8. 状态序列与 TOML 解析分别有 fuzz target；状态与约束判断执行受影响范围的变异测试；
9. 所有验证只使用受控临时根，不触碰真实 bootstrap 配置或用户 data root。

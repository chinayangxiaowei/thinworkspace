# ThinWorkspace Phase 1 用户操作手册

**版本：v1.0｜对应架构阶段：Phase 1 单机 CLI MVP｜文档性质：目标态使用体验**

> 本手册描述 Phase 1 完成后的预期产品行为，用于评审最终用户体验；其中命令、输出、JSON 和错误码同时作为后续 CLI 设计基线。

本手册只维护用户可见契约。内部 Port、SQLite 字段、系统调用、Git 实现序列和故障协调算法分别由 Phase 1 详细设计、跨平台物化设计和技术栈文档管理，不在此重复。

---

## 一、Phase 1 能解决什么

Phase 1 用一个命令行工具在当前 macOS 机器上管理多个独立 Git 工作区。

用户只需要理解两个对象：

| 对象 | 用户理解 |
|---|---|
| Repository | 要并行开发的 Git 仓库 |
| Workspace | 某个 Agent 独立使用的开发目录 |

典型使用方式：

```text
同一个仓库
    ├── Workspace A → Agent A 修改登录功能
    ├── Workspace B → Agent B 修改测试
    └── Workspace C → Agent C 重构数据库代码
```

每个 Workspace：

- 有独立源码视图；
- 有独立 Git HEAD、index 和托管分支；
- 对工作区内独立普通文件的修改、删除和重命名不会改变其他 Workspace 的对应文件；
- 优先使用 APFS Clone，共享未修改的文件数据块；
- 提供普通绝对路径，现有命令、编辑器和 Agent 可直接使用；
- 删除前检查未提交内容，并尽力探测外部进程占用。

Phase 1 不提供实时冲突预警、远程执行、Agent 编排或安全 Sandbox。这些边界在本手册末尾集中说明。

---

## 二、五分钟快速体验

假设 `thinws` 已经安装，可以按以下流程完成一次开发：

```bash
thinws init --data-root /Volumes/data/thinws-data
thinws doctor

thinws repo add /Volumes/data/code/my-app
thinws workspace create --repo my-app --base main --name auth-refresh

cd "$(thinws workspace path auth-refresh)"
# 在这里启动你现有的 Coding Agent，或者直接编辑代码

# 直接运行项目原有命令；此处以语言无关的 Git 检查为例
git diff --check
thinws workspace status auth-refresh
thinws workspace diff auth-refresh

git add .
git commit -m "Implement token refresh"
git push -u origin HEAD  # 仅当 repo add 显示 Push remote: origin

# 停止使用该工作区的工具，并离开该目录后再删除
cd /Volumes/data/code
thinws workspace remove auth-refresh
thinws gc --dry-run
```

日常使用最核心的路径只有：

```text
create → path → 直接开发/测试 → status/diff → remove
```

---

## 三、首次初始化

### 3.1 初始化数据根目录

当前机器的 `/Volumes/data` 是 APFS 卷，推荐把平台数据统一放在：

```bash
thinws init --data-root /Volumes/data/thinws-data
```

本手册对应已验证组合为 arm64 macOS 15.7.2、`/Volumes/data` APFS 和 Apple Git 2.39.5。表格中 macOS 10.13/APFS 的历史能力起点不代表 Phase 1 已验证全部旧系统。

目标输出示例：

```text
Initialized thinws

Data root:       /Volumes/data/thinws-data
Filesystem:      APFS
Volume:          data
Default policy:  require-cow
Config:          created
Config path:     /Users/yangxiaowei/Library/Application Support/ThinWorkspace/config.toml
Metadata:        created

Next: thinws doctor
```

初始化只创建配置和平台管理的目录，不会接入仓库或创建 Workspace。配置文件使用输出中的固定路径；后续命令会核验已登记的数据根和卷身份，生产命令不支持用环境变量切换配置位置。

重复运行同一命令是幂等的：

```text
thinws is already initialized at /Volumes/data/thinws-data
No changes made.
```

Phase 1 不允许通过再次执行 `init` 静默切换到另一个数据根目录，也不提供 data root reset/migrate 命令。用户必须继续使用已登记的数据根；迁移或整体删除需要后续专门流程，不能把手工删除目录写成受支持操作。

向已经初始化的实例传入不同路径时，命令返回 `E_DATA_ROOT_CHANGE_UNSUPPORTED`，且不创建新目录、不改配置：

```text
Data root change is not supported in Phase 1

Code:       E_DATA_ROOT_CHANGE_UNSUPPORTED
Configured: /Volumes/data/thinws-data
Requested:  /other/thinws-data
```

首次初始化只接管全新目录、空目录，或本平台上一次中断初始化留下且能够完整验证的目录。无法证明由本平台创建的非空目录会返回 `E_DATA_ROOT_NOT_EMPTY`，不会把其中内容当成平台数据：

```text
Data root is not empty and is not owned by thinws

Code:      E_DATA_ROOT_NOT_EMPTY
Requested: /Volumes/data/existing-files

No files were changed.
```

初始化中断后，对同一路径重复执行 `init` 只会在能够证明它属于同一次初始化时幂等续跑；验证失败则返回 `E_RECOVERY_REQUIRED` 并保留所有内容。

### 3.2 检查当前机器能力

```bash
thinws doctor
```

目标输出示例：

```text
ThinWorkspace Doctor

[OK] Platform
     macOS 15.7.2

[OK] Data root
     /Volumes/data/thinws-data
     APFS volume: data

[OK] Git
     git 2.39.5 (Apple Git-154)

[OK] Workspace materialization
     Preferred: apfs-file-clone
     Clone support: supported
     Full-copy fallback: explicit only

[INFO] File changes
     Phase 1 uses on-demand Git status/diff
     Real-time observation is not enabled

Result: HEALTHY
```

`doctor` 只检测，不会修改工作区。

如果上一次创建或删除操作被中断，可以执行：

```bash
thinws doctor --repair
```

它只进行能够证明安全的幂等恢复，例如：

- 完成可以验证的 `Creating` 状态；
- 撤销不完整的 linked worktree 注册；
- 清理确定无引用的临时文件；
- 继续已经进入 `Deleting` 的删除操作；
- 协调可验证的仓库接入、更新、基线生成或 GC 中断。

它不会自动删除 dirty 内容、托管分支或状态无法判定的对象。

---

## 四、接入仓库

### 4.1 接入本机仓库

```bash
thinws repo add /Volumes/data/code/my-app
```

目标输出示例：

```text
Repository added

Name:             my-app
Repository ID:    repo_019...
Source:           /Volumes/data/code/my-app
Push remote:      origin
Default branch:   main
Git topology:     linked-worktree
Compatibility:   supported
```

Repository 默认名从规范来源推导：普通本机仓库取 common `.git` 的父目录名，bare 仓库取自身目录名；远程取 locator path 最后一个非空 ASCII 段；然后只移除一个精确小写 `.git` 后缀。平台不会自动转小写或 percent-decode。名称长度 1–63，必须匹配 `^[a-z0-9](?:[a-z0-9._-]{0,61}[a-z0-9])?$` 且不含连续 `..`，并在活跃 Repository 中全局唯一；推导结果为空、不合法或已经存在时，必须显式传入 `--name`：

```bash
thinws repo add /Volumes/data/code/my-app --name my-app-2
```

后续命令既可以使用名称，也可以使用完整 Repository ID。

重复接入同一来源且名称相同是幂等操作，返回已有 Repository。同一 repository 的不同 linked worktree 会识别成同一来源，两个独立 clone 仍是不同来源；远程 locator 不会被自动猜测为其他等价写法。相同来源换名返回 `E_REPO_ALREADY_ADDED`；同名却指向不同来源返回 `E_NAME_CONFLICT`。

### 4.2 接入远程仓库

```bash
thinws repo add git@github.com:example/my-app.git
```

平台调用系统 Git，并沿用系统已有的 SSH、HTTPS 和 credential helper 行为。Phase 1 的远程来源只接受 `https://`、`ssh://` 和 Git scp-like（如上例）：全部形式拒绝控制字符；HTTPS 拒绝 userinfo、query、fragment；`ssh://` 允许用户名但拒绝 password、query、fragment。用户必须通过 credential helper、SSH agent 或交互式认证提供秘密。平台不会把任意 path 片段猜测为 token，因此不要把秘密放入 locator 的其他位置。

无论输入本机路径还是 Git URL，平台都建立独立托管的 Git repository。Workspace 和托管分支属于该平台仓库，不会向用户原始仓库登记 linked worktree；输入路径或 URL 只是导入来源。

从本机路径接入时只导入已提交对象、`refs/heads/*` 和 `refs/tags/*`；来源自己的 remote-tracking refs、reflog、stash、其他私有 refs，以及原始工作树中的未提交/未跟踪内容都不会进入 Repository 或 Base。

`Push remote` 是给用户直接执行 `git push` 的远端，与平台自己的来源更新通道分离：

- 远程 URL 接入时，校验通过的来源会配置为 `origin`；
- 本机路径接入时，仅在来源的 `remote.origin.url` 也通过同样的 locator 安全校验时复制为 `origin`；
- 其他情况输出 `Push remote: none`，快速示例中的 push 命令不可直接使用。用户可以在任一 Workspace 中执行 `git remote add origin <url>`，但该设置属于整个托管 Repository，会同时对它的所有 Workspace 可见。

`thinws repo fetch` 始终从接入时登记的平台来源更新，不受用户后来新增或修改的 `origin` 影响。

### 4.3 查看仓库

```bash
thinws repo list
```

目标输出示例：

```text
NAME      DEFAULT BRANCH  WORKSPACES  SOURCE
my-app    main            0           /Volumes/data/code/my-app
sdk       main            3           git@github.com:example/sdk.git
```

### 4.4 更新来源引用

```bash
thinws repo fetch my-app
```

该命令更新平台保存的来源分支和 tags，不导入来源的 remote-tracking refs、reflog、stash 或其他私有 refs。整次更新是全有或全无且不 prune：来源分支允许回退，已存在 tag 若被来源同名改写则整次命令以 `E_GIT` 失败，所有 refs 保持原状。Phase 1 不提供 force-tag/prune。已经固定的 Base 和现有 Workspace 不受成功 fetch 影响。`workspace create` 和 `--dry-run` 从不隐式联网；需要最新来源分支时先显式执行 `repo fetch`。

来源分支、tags 和可写托管分支使用不同命名空间：

```text
refs/remotes/thinws-source/*  平台来源分支
refs/tags/*                 tags
refs/heads/thinws/*        Workspace 托管分支
```

`--base main` 只有在短名无歧义时才成功；存在同名 branch/tag 等多个候选时，必须使用完整 ref。也可以传入完整 commit OID。Phase 1 不接受 reflog 表达式、`@{-1}` 或解析为 tree/blob 的 revision expression。

Phase 1 只支持 Git SHA-1 repository；完整 commit OID 输入必须是 40 位十六进制。CLI 和 JSON 中的 commit/tree OID 始终输出完整小写 40 位，不使用短 OID。SHA-256 或未知 object format 会以 `E_REPO_UNSUPPORTED` 拒绝接入。

### 4.5 不兼容仓库

Phase 1 遇到以下情况会明确拒绝，而不是创建内容不完整的 Workspace：

- submodule；
- Git LFS；
- sparse checkout；
- 任何自定义 filter、`working-tree-encoding` 或 `ident` 展开；
- 当前 APFS 卷无法表示的大小写冲突路径。

示例：

```text
Repository is not supported by Phase 1

Code:     E_REPO_UNSUPPORTED
Feature:  git-lfs
Reason:   checkout content cannot be reproduced by the Phase 1 base builder

No repository state was created.
```

---

## 五、创建 Workspace

### 5.1 先查看执行计划

```bash
thinws workspace create \
  --repo my-app \
  --base main \
  --name auth-refresh \
  --dry-run
```

目标输出示例：

```text
Workspace creation plan

Repository:       my-app
Requested base:   main
Resolved commit:  8f42b7d0c5a1e9f8342e7d01ab56cd7890ef1234
Workspace name:   auth-refresh
Workspace ID:     not allocated (--dry-run)
Target root:      /Volumes/data/thinws-data/workspaces/
Target pattern:   <workspace-id>/root

Source filesystem:  APFS / volume data
Target filesystem:  APFS / volume data
Materializer:        apfs-file-clone
Planned mode:        cow-clone
Fallback allowed:    no

No changes made (--dry-run).
```

`--base main` 在创建时会解析并固定为具体 commit。之后远程 `main` 前进，不会悄悄改变已经创建的 Workspace。

`--dry-run` 不分配 Workspace ID，也不保留名称、路径或计划。它展示的是当前时刻解析出的 commit、目标数据根、路径模式和后端选择；正式创建仍会重新探测并分配新的 UUIDv7。脚本不能把 dry-run 输出中的计划当作可提交的 plan token。

### 5.2 创建 Workspace

```bash
thinws workspace create \
  --repo my-app \
  --base main \
  --name auth-refresh
```

Workspace 名称长度 1–63，必须匹配 `^[a-z0-9](?:[a-z0-9._-]{0,61}[a-z0-9])?$` 且不含连续 `..`，并在全部活跃 Workspace 中全局唯一；名称不做大小写或 Unicode 归一化。名称只用于查询和展示，实际目录始终由 Workspace ID 推导；所有接受 Workspace 名称的命令也接受完整 Workspace ID。公开输出中的 Repository、Workspace 和 Operation ID 分别使用 `repo_`、`ws_`、`op_` 前缀的标准小写 UUIDv7 文本。手册中的 `ws_019...` 和 `repo_019...` 是缩写展示。

重复执行完全相同的 create，在已有 Workspace 已经 Ready 且 Repository、解析后的 commit 和创建策略一致时，返回已有 Workspace 而不新建。相同名称但参数不同返回 `E_NAME_CONFLICT`；已有记录处于 Creating、Deleting 或 Error 时返回 `E_RECOVERY_REQUIRED`，要求先处理原记录。

目标输出示例：

```text
Workspace ready

Name:             auth-refresh
Workspace ID:     ws_019...
Path:             /Volumes/data/thinws-data/workspaces/ws_019.../root
Base commit:      8f42b7d0c5a1e9f8342e7d01ab56cd7890ef1234
Branch:           thinws/ws_019...a17c
Git status:       clean

Requested mode:   cow-clone
Effective plan:  cow-clone
Actual mode:      cow-clone
Adapter:          apfs-file-clone
Outcome:          succeeded
CoW:              confirmed
Fallback used:   no
Created in:       1.8s
```

默认策略是 `require-cow`。如果 APFS Clone 没有实际成功，命令会失败，不会偷偷完整复制仓库。

### 5.3 显式允许完整复制

如果同一受控 APFS 数据根在预检或真实调用时报告 file clone 不可用，可以显式允许降级：

```bash
thinws workspace create \
  --repo my-app \
  --base main \
  --name auth-refresh \
  --allow-copy
```

降级输出必须醒目标注：

```text
Workspace ready with fallback

Requested mode:   cow-clone
Effective plan:  full-copy
Actual mode:      full-copy
Adapter:          full-copy
Outcome:          succeeded
CoW:              not-used
Fallback used:   yes
Reason:           managed data root does not support block sharing
Estimated added:  3.2 GiB
```

Phase 1 的全部平台管理目录都由 data root 派生并位于初始化时记录的同一卷；不能用子挂载或符号链接把其中一部分重定向到其他卷。发现布局跨卷时返回数据根布局错误，`--allow-copy` 不能绕过该不变量。用户工具自行配置的工作区外输出或缓存不属于平台管理目录，也不由平台回收。

Phase 1 init 只接受 APFS data root；非 APFS 返回 `E_CAPABILITY_UNAVAILABLE`。Full Copy Adapter 的非 APFS/跨文件系统能力属于 Phase 0 技术验证，不是本手册承诺的产品路径。

如果 APFS Clone 执行中途失败，平台不会把 Clone/Copy 混合或来源不明的目录报告为可用 Workspace。允许复制时，也只有在本次不完整结果安全回滚并重新检查后才会改用 Full Copy。

### 5.4 Phase 1 不支持任意 Workspace 路径

Phase 1 的 Workspace 始终由平台创建在配置的数据根目录下。这样可以保证：

- 路径安全；
- 全部平台管理目录的卷关系可验证；
- linked worktree 的路径生命周期受控；
- 删除和故障恢复行为一致。

用户通过 `workspace path` 获取位置，不需要自己指定目标目录。

---

## 六、进入 Workspace 并启动 Agent

### 6.1 获取工作目录

```bash
thinws workspace path auth-refresh
```

该命令成功时只向 stdout 输出绝对路径，方便 shell 使用：

```text
/Volumes/data/thinws-data/workspaces/ws_019.../root
```

进入目录：

```bash
cd "$(thinws workspace path auth-refresh)"
```

### 6.2 启动现有 Coding Agent

Phase 1 不负责启动或管理 Agent 会话。用户在该目录运行已有工具即可：

```bash
your-agent-command
```

也可以直接把绝对路径交给编辑器、终端或脚本。工作区是普通目录，工具不需要 ThinWorkspace 插件或命令包装；读写权限和 Git 工作树规则仍然适用。不要手工移动整个工作区或改写 `.git`、平台管理目录；工作区生命周期继续通过平台命令管理。

如果要并行开发，可以继续创建其他 Workspace：

```bash
thinws workspace create --repo my-app --base main --name api-change
thinws workspace create --repo my-app --base main --name tests-update
```

然后在不同终端分别进入相应路径。

---

## 七、查看 Workspace 状态

### 7.1 查看全部 Workspace

```bash
thinws workspace list
```

目标输出示例：

```text
NAME          STATE  REPOSITORY  BRANCH                       DIRTY  STORAGE
auth-refresh  Ready  my-app      thinws/ws_019...a17c         yes    APFS Clone
api-change    Ready  my-app      thinws/ws_019...91e2         no     APFS Clone
tests-update  Ready  my-app      thinws/ws_019...2c44         no     APFS Clone
old-create    Error  my-app      thinws/ws_019...8d10         -      Incomplete
```

默认列表展示全部活跃 Workspace，包括 `Creating`、`Ready`、`Deleting` 和 `Error`。删除审计记录不属于活跃 Workspace，不出现在列表中。过渡或错误状态的 `DIRTY` 等不可安全计算字段显示 `-`，不能猜测。

### 7.2 查看详细状态

```bash
thinws workspace status auth-refresh
```

目标输出示例：

```text
Workspace: auth-refresh

State:              Ready
Repository:         my-app
Path:               /Volumes/data/thinws-data/workspaces/ws_019.../root
Base commit:        8f42b7d0c5a1e9f8342e7d01ab56cd7890ef1234
Branch:             thinws/ws_019...a17c

Git:
  Modified:         3
  Added:            1
  Deleted:          0
  Untracked:        2

Storage:
  Materializer:     apfs-file-clone
  CoW:              confirmed
  Logical size:     3.2 GiB
  Physical delta:   best available estimate
```

`workspace status` 可用于查看任何活跃状态。只有 `workspace path` 和 `workspace diff` 要求 Workspace 为 `Ready`；其他状态调用这些命令返回 `E_WORKSPACE_NOT_READY`。这是平台命令的准入条件，不代表平台能阻止外部程序访问已知目录。

Phase 1 的命令/状态边界固定如下：

| 命令 | Creating | Ready | Deleting | Error |
|---|---|---|---|---|
| `workspace list` | 展示 | 展示 | 展示 | 展示 |
| `workspace status` | 只读诊断 | 只读状态 | 只读诊断 | 只读诊断 |
| `workspace path/diff` | 拒绝 | 允许 | 拒绝 | 拒绝 |
| `workspace remove` | 要求先 repair | 开始删除 | 相同 flags 时幂等继续；不同则拒绝 | 仅在删除保护和对象归属可证明时允许，否则要求 repair/人工处理 |
| `doctor` | 只读诊断 | 只读检查 | 只读诊断 | 只读诊断 |
| `doctor --repair` | 安全继续或转 Error | 只修复已证明的不一致 | 幂等继续 | 修复或只报告，不猜测删除 |

表中标为只读的命令不会修改平台状态、Git 或文件。

`Error → Ready` 只允许 `doctor --repair` 在全部相关事实重新验证成功后执行；`Error → Deleting` 只允许用户显式 remove 且删除保护可以证明。不会把失败对象直接改回旧状态来隐藏故障。

### 7.3 查看改动

```bash
thinws workspace diff auth-refresh
```

该命令展示此 Workspace 自身的 Git diff。Phase 1 不会比较其他 Workspace，也不会生成冲突预警。

---

## 八、直接运行现有工具

### 8.1 在普通目录内开发和测试

```bash
cd "$(thinws workspace path auth-refresh)"
git diff --check
# 接着运行项目原有的构建、测试或开发服务器命令
```

例如，已有相应工具和项目配置时，可以直接使用 `cargo test`、`npm test`、`python -m unittest` 或项目自己的脚本；这些只是示例，不是语言支持白名单。

Phase 1 不提供 `thinws workspace exec`，也不提供等价的执行包装入口。用户命令的参数、环境、输出、退出码、超时和 Ctrl-C 全部遵循当前终端、Agent Runtime 与工具自身的行为；ThinWorkspace 不转发命令、不注入构建变量、不保存运行清单或构建日志。

工作区不是 Sandbox。程序仍拥有当前用户授予的宿主权限，可能读取环境凭据或访问工作区外路径；运行不可信代码应使用外部安全隔离工具。

### 8.2 构建输出与缓存

编译输出、增量缓存和依赖缓存的位置与复用方式由项目和工具原有配置决定。ThinWorkspace 不按语言检测、重定向、强制共享或隔离这些目录，也不承诺缓存命中。

创建时的输入范围仍遵循本手册 §4.2 的 Git 导入契约：来源中未纳入该基线的本地构建输出和缓存不会自动进入新工作区。取消命令包装不等于已支持克隆现有编译缓存。

工作区内的生成文件仍占用空间，并随工作目录按 §10 的保护规则处理；工作区外的缓存和输出不由 ThinWorkspace 删除或 GC。

---

## 九、提交和交付代码

Phase 1 不重做 Git。进入 Workspace 后直接使用标准 Git：

```bash
cd "$(thinws workspace path auth-refresh)"
git status
git add .
git commit -m "Implement token refresh"
git push -u origin HEAD  # 仅当 repo add 显示 Push remote: origin
```

Workspace 默认分支类似：

```text
thinws/ws_0192f3ab-7c6d-7a21-8e31-123456789abc
```

分支固定为 `refs/heads/thinws/<workspace-id>`，使用完整 Workspace ID；Workspace 显示名称不参与 Git ref 构造。这样名称语法和 Git ref 语法不会互相泄漏，重试也能从持久化 WorkspaceId 得到完全相同的分支。

Phase 1 不提供合并队列或自动创建 Pull Request。提交、push、PR 和合并沿用团队现有 Git 工作流。

平台自己的状态和生命周期操作使用固定的非交互 Git 语义，不会运行用户 hook 或受 pager、颜色和外部 diff 影响；联网接入/更新仍可使用系统 credential helper 和 SSH agent。用户进入 Workspace 后直接运行 `git` 仍按用户自己的 Git 配置工作。

接入时平台不会复制用户 global checkout 偏好：平台默认使用 LF，保留文件模式和真实 symlink，并固定 data root 的大小写/Unicode 语义。以后修改用户 global Git config 不会改变平台状态或新 Workspace 的 checkout 语义；受支持的标准 `.gitattributes` 规则仍可覆盖默认行尾规则。

---

## 十、删除 Workspace

### 10.1 删除干净 Workspace

```bash
thinws workspace remove auth-refresh
```

目标输出示例：

```text
Removal check

Git working tree:       clean
External process check: no-evidence (best-effort)
Managed branch:         thinws/ws_019...a17c

Workspace removed
Branch preserved: thinws/ws_019...a17c
```

默认只删除：

- Workspace 工作目录；
- 平台为该 Workspace 保存的 Git worktree 登记；
- Workspace 活跃元数据。

平台保留最小删除审计信息，但该记录不会再被当作活跃 Workspace，也不会单独授权删除归属不明的残留。托管 Git 分支默认保留，因此已经提交的代码不会因普通删除 Workspace 消失；普通删除不要求提交已经存在于远程引用。

工作目录内的编译输出、缓存和其他生成文件也会随目录删除，不因被 Git 忽略而永久保留。请先保存需要保留的内容，并停止相关工具、让终端离开该目录；工作区外的输出和缓存不会被删除。

### 10.2 删除 dirty Workspace

如果存在未提交内容，默认拒绝：

```text
Workspace removal refused

Code:       E_WORKSPACE_DIRTY
Modified:   3
Untracked:  2

Commit or preserve the changes first.
To discard them explicitly, rerun with --force.
```

确认不再需要未提交内容时：

```bash
thinws workspace remove auth-refresh --force
```

`--force` 会丢弃未提交内容，但默认仍保留托管分支。

### 10.3 删除分支是独立动作

只有明确请求时才同时删除分支：

```bash
thinws workspace remove auth-refresh --delete-branch
```

如果分支含有删除该分支后无法由其他保护引用到达的提交，操作仍会以 `E_UNPROTECTED_COMMITS` 拒绝。保护引用包括删除后仍保留的其他本地分支、remote-tracking refs、tags 和 `refs/thinws/archive/*`；reflog、`ORIG_HEAD` 和待删除分支本身不算保护引用。

删除开始后，如果中断，重复 remove 必须再次传入完全相同的 `--force`/`--delete-branch` 组合；缺失或新增任一 flag 都返回 `E_RECOVERY_REQUIRED` 并显示所需重试命令。`doctor --repair` 可以按原意图续跑。真正删除分支前平台会再次验证目标分支和保护引用；验证失败时保留可恢复的 Error 状态，不会写成删除完成。

Phase 1 不提供绕过未保护提交检查的参数。`--force` 只丢弃未提交工作树内容，不能授权删除仅由目标分支保存的提交。要删除该分支，必须先 push、创建 tag/其他分支，或建立归档引用。

### 10.4 外部进程正在使用工作区

```text
Workspace removal refused

Code:                   E_WORKSPACE_BUSY
External process check: confirmed-in-use

Wait for the command to finish or stop it from its owning terminal.
```

平台只尽力检查当前用户可见的外部进程，结果分为 `confirmed-in-use`、`no-evidence`、`scan-incomplete`。confirmed 返回 `E_WORKSPACE_BUSY`，用户必须先停止占用再重试；no-evidence 只表示未取得占用证据；权限不足或扫描不完整时显示 scan-incomplete 警告，不会伪装成“未发现”。Phase 1 不提供绕过参数，`--force` 也不改变判定；后两种结果都不构成绝对无人使用的保证。

ThinWorkspace 不替用户终止进程，`doctor --repair` 也不管理用户命令。占用扫描不能阻止其他工具在扫描后重新访问目录，因此删除前仍须由用户停止相关工具。

---

## 十一、空间查看和垃圾回收

### 11.1 先查看计划

```bash
thinws gc --dry-run
```

目标输出示例：

```text
Garbage collection plan

Unreferenced bases:  2    1.4 GiB logical
Expired trash:       3    620 MiB
Failed remnants:     0    0 B
Active workspaces:   never removed by gc
Protected branches:  never removed by gc

No changes made (--dry-run).
```

### 11.2 执行 GC

```bash
thinws gc
```

交互式终端会展示相同计划并要求确认。自动化脚本必须使用明确的非交互确认参数，不能依赖默认回答：

```bash
thinws gc --yes
```

GC 只回收：

- 无 Workspace 引用的 Base；
- 已到期的 trash；
- 归属可证明且明确标为可回收的失败残留。

GC 不删除任何状态的活跃 Workspace、未完成操作、托管分支、用户工具的外部缓存或无法确认引用关系的对象。删除审计记录本身不会让已无引用的数据永久占用空间。

Phase 1 的 `thinws gc` 也不运行 Git object GC，不删除 Git refs/reflog，并且不执行无范围的 `git worktree prune`。托管 Repository 的自动 Git maintenance 默认关闭；本阶段宁可多占用 Git object 空间，也不引入未经单独验收的提交回收语义。

Phase 1 也不提供 `repo remove`。接入后的托管 Repository 和其中保留的分支不会被 GC 删除；Repository 删除和 Git object 回收必须作为后续独立能力设计删除保护、保留期和恢复流程。

---

## 十二、故障与恢复体验

### 12.1 创建中断

如果创建过程中 CLI 被终止，普通查询会只读识别并报告该状态，不会因为一次 `list` 或 `status` 自动清理目录。执行以下命令进行显式恢复：

```bash
thinws doctor --repair
```

目标输出示例：

```text
Recovering interrupted operation

Workspace: auth-refresh
Previous state: Creating
Action: incomplete Git registration removed
Action: partial materialization removed
Result: workspace marked Error
```

不完整的 Workspace 会以 `Creating` 或 `Error` 出现在默认 `workspace list` 中，但不会被 `workspace path` 或 `diff` 当作可用 Workspace。

### 12.2 删除中断

`Deleting` 操作是幂等的。再次执行同一 `workspace remove` 时必须带与原操作相同的 `--force`/`--delete-branch` 组合；或者显式执行 `doctor --repair`。平台从持久化的原删除意图续跑，并在每个尚未完成的破坏性步骤前重新验证适用的 Volume、对象归属、进程和引用保护；不满足时转为或保持 Error，而不是沿用过期检查。普通查询只展示状态。

### 12.3 数据卷不在线

如果 `/Volumes/data` 未挂载：

```text
Data root unavailable

Code:          E_DATA_ROOT_UNAVAILABLE
Configured:    /Volumes/data/thinws-data
Expected volume: data / <volume-id>

No fallback path was selected.
```

平台不会因为同名目录出现，就在系统盘自动创建新的空数据根目录。

配置记录与实际数据根身份不一致时，`doctor` 只报告 `E_RECOVERY_REQUIRED`；`doctor --repair` 也不会猜测应覆盖哪一边。Phase 1 没有 reset/migrate，用户需要先恢复原卷或从可信备份恢复匹配的数据和配置。

### 12.4 状态无法判定

遇到无法安全自动修复的对象时：

```text
Manual attention required

Workspace: ws_019...
State:     Error
Reason:    Git administrative state and workspace path disagree

No files were deleted.
Run: thinws doctor --repair
```

如果 `--repair` 仍不能证明安全，只输出诊断信息，不强制清理。

---

## 十三、给 Agent 和脚本使用 JSON

Phase 1 的 JSON 支持按命令冻结如下：

| 命令 | `--json` | 说明 |
|---|---|---|
| `init` | 支持 | 返回配置的数据根和 Volume ID |
| `doctor`、`doctor --repair` | 支持 | repair 结果逐项报告实际动作 |
| `repo add`、`repo fetch`、`repo list` | 支持 | add 返回 Repository ID；fetch 返回更新的 refs；list 返回稳定排序数组 |
| `workspace create`、`create --dry-run` | 支持 | dry-run 的 `workspace_id` 为 null，不预留计划 |
| `workspace list`、`workspace status` | 支持 | list 包含全部活跃状态 |
| `workspace remove` | 支持 | 返回删除结果、分支是否保留、Workspace ID 和删除 operation ID |
| `gc`、`gc --dry-run` | 支持 | 返回计划或实际回收结果 |
| `workspace path` | 不支持成功 JSON | stdout 专用于一行原始绝对路径；平台侧 `--json` 返回 JSON envelope `E_USAGE`，不查询路径 |
| `workspace diff` | 不支持成功 JSON | stdout 专用于原始 patch；平台侧 `--json` 返回 JSON envelope `E_USAGE`，不计算 diff |

示例：

```bash
thinws workspace status auth-refresh --json
```

目标输出示例：

```json
{
  "schema_version": 1,
  "ok": true,
  "data": {
    "workspace_id": "ws_019...",
    "name": "auth-refresh",
    "state": "ready",
    "repository": "my-app",
    "path": "/Volumes/data/thinws-data/workspaces/ws_019.../root",
    "base_commit": "8f42b7d0c5a1e9f8342e7d01ab56cd7890ef1234",
    "branch": "thinws/ws_019...a17c",
    "git": {
      "dirty": true,
      "modified": 3,
      "added": 1,
      "deleted": 0,
      "untracked": 2
    },
    "materialization": {
      "requested_mode": "cow-clone",
      "effective_planned_mode": "cow-clone",
      "actual_mode": "cow-clone",
      "adapter": "apfs-file-clone",
      "outcome": "succeeded",
      "cow": "confirmed",
      "fallback": {
        "used": false,
        "reason": null
      },
      "failed_attempts": []
    }
  }
}
```

失败使用统一 envelope，并保持非零进程退出码：

```json
{
  "schema_version": 1,
  "ok": false,
  "error": {
    "code": "E_WORKSPACE_NOT_READY",
    "message": "workspace is not ready",
    "context": {
      "workspace_id": "ws_019...",
      "state": "creating"
    },
    "remediation": "run thinws doctor --repair"
  }
}
```

JSON 模式要求：

- 使用 `--json` 后，无论成功或失败，stdout 只输出一个 JSON 文档；
- 平台预识别全局 `--json`，扫描在 `--` 处停止，之后的文本不作为平台选项解释；参数格式错误也使用已识别的 JSON 模式返回 `E_USAGE`。`--help/--version` 与 `--json` 互斥，单独 help/version 仍是人类输出；
- 诊断日志写 stderr；
- 字段含义由 `schema_version` 管理；
- 不使用本地化字符串作为状态值；
- 脚本只依赖 `ok`、`error.code` 和版本化 data/context 字段，不依赖自然语言 `message` 或 `remediation`。
- `gc --json` 属于非交互调用，实际回收必须同时传 `--yes`；缺少时返回 JSON 格式的 `E_USAGE`，不能在 JSON 流程中读取确认提示。

### 13.1 Phase 1 退出码

| 退出码 | 名称 | 含义 |
|---:|---|---|
| 0 | OK | 操作成功 |
| 2 | E_USAGE | 参数或命令格式错误 |
| 10 | E_NOT_INITIALIZED | 尚未初始化 |
| 11 | E_CAPABILITY_UNAVAILABLE | 所需平台能力不可用 |
| 12 | E_COW_UNAVAILABLE | 默认要求 CoW，但本次路径不支持 |
| 13 | E_REPO_UNSUPPORTED | 仓库特性超出 Phase 1 范围 |
| 14 | E_REPO_ALREADY_ADDED | 同一规范化来源已经以其他名称接入 |
| 15 | E_NAME_CONFLICT | 同类型活跃对象名称已被不同参数的 Repository 或 Workspace 占用 |
| 16 | E_DATA_ROOT_CHANGE_UNSUPPORTED | 已初始化实例请求切换 data root；原配置保持不变 |
| 17 | E_REPOSITORY_NOT_FOUND | 指定 Repository 名称或 ID 不存在 |
| 18 | E_GIT_REF_NOT_FOUND | `--base` 在允许的 ref/OID 范围内没有候选 |
| 19 | E_GIT_REF_AMBIGUOUS | `--base` 短名命中多个允许候选，必须改用完整 ref/OID |
| 20 | E_WORKSPACE_NOT_FOUND | Workspace 不存在 |
| 21 | E_WORKSPACE_NOT_READY | `path/diff` 等要求 Ready 的命令遇到 Creating、Deleting 或 Error |
| 22 | E_WORKSPACE_DIRTY | 删除被未提交内容阻止 |
| 23 | E_WORKSPACE_BUSY | 明确检测到外部进程正在使用 Workspace |
| 24 | E_UNPROTECTED_COMMITS | 删除分支会导致提交失去保护引用 |
| 30 | E_GIT | Git 操作失败 |
| 31 | E_FILESYSTEM | 文件系统操作失败 |
| 32 | E_DATA_ROOT_UNAVAILABLE | 数据根目录或预期卷不可用 |
| 33 | E_DATA_ROOT_LAYOUT | 受控 data root 子目录不满足路径归属或 Phase 1 同卷布局 |
| 35 | E_METADATA | SQLite/schema/元数据操作失败且不能映射为更具体的恢复错误 |
| 36 | E_DATA_ROOT_NOT_EMPTY | 首次 init 的目标非空且没有可验证的平台根标记 |
| 40 | E_RECOVERY_REQUIRED | 需要恢复或人工处理 |
| 41 | E_LOCK_TIMEOUT | 生命周期变更锁等待超过 5 秒；当前命令未开始修改目标 |

本表只定义 ThinWorkspace 管理命令的产品错误码，不定义用户直接运行程序的退出状态。首发前移除执行包装后，其余错误码不重排，34 保留不分配。

---

## 十四、一个完整的三 Agent 使用场景

### 14.1 创建三个并行工作区

```bash
thinws workspace create --repo my-app --base main --name login-api
thinws workspace create --repo my-app --base main --name login-tests
thinws workspace create --repo my-app --base main --name session-refactor
```

### 14.2 在三个终端启动 Agent

终端 A：

```bash
cd "$(thinws workspace path login-api)"
your-agent-command
```

终端 B：

```bash
cd "$(thinws workspace path login-tests)"
your-agent-command
```

终端 C：

```bash
cd "$(thinws workspace path session-refactor)"
your-agent-command
```

### 14.3 分别执行测试

假设示例项目已有 `scripts/test.sh`，在各自终端直接调用它；实际项目使用自己的命令：

```bash
# 终端 A
cd "$(thinws workspace path login-api)"
./scripts/test.sh

# 终端 B
cd "$(thinws workspace path login-tests)"
./scripts/test.sh

# 终端 C
cd "$(thinws workspace path session-refactor)"
./scripts/test.sh
```

这些命令可以在不同终端同时运行。各工作区源码文件独立；构建输出是否留在工作区内、是否使用外部缓存，遵循项目原有配置，平台不改写。

### 14.4 查看结果

```bash
thinws workspace list
thinws workspace status login-api
thinws workspace diff login-api
```

### 14.5 提交并交给现有 Git 流程

```bash
cd "$(thinws workspace path login-api)"
git add .
git commit -m "Implement login API"
git push -u origin HEAD  # 仅当 repo add 显示 Push remote: origin
```

其他两个 Workspace 使用同样流程。Phase 1 不自动决定合并顺序。

### 14.6 清理工作区

先停止三个工作区中的 Agent、开发服务器等工具，并让相关终端离开这些目录，再执行：

```bash
cd /Volumes/data/code
thinws workspace remove login-api
thinws workspace remove login-tests
thinws workspace remove session-refactor
thinws gc --dry-run
thinws gc
```

---

## 十五、Phase 1 明确边界

| 用户可能期待的能力 | Phase 1 实际行为 |
|---|---|
| 两个 Agent 修改同一区域时提前提醒 | 不支持；Phase 2 才提供受管写前预警 |
| 实时显示其他 Workspace 的变化 | 不支持；Phase 1 只按需读取当前 Workspace Git 状态 |
| 创建可恢复检查点 | 不支持；Phase 2 引入 WorkspaceCheckpoint |
| 远程构建测试 | 不支持；Phase 3 引入 Worker 和 Job |
| 把开发 Workspace 放到其他节点 | 不支持；Phase 4 引入 Placement |
| 自动启动和管理 Agent | 不支持；用户使用现有 Agent Runtime |
| 包装或托管用户命令 | 不支持；用户直接在普通工作区路径运行工具 |
| 自动配置构建目录或共享缓存 | 不支持；遵循项目和工具原有配置 |
| 安全执行不可信代码 | 不支持；普通目录不构成 Sandbox |
| 自动合并或创建 PR | 不支持；继续使用标准 Git 和现有平台 |
| 任意指定 Workspace 目录 | 不支持；Phase 1 使用受控 data root |
| 删除已接入 Repository 或回收不可达 Git objects | 不支持；Phase 1 只回收 Base、trash 和归属可证明的失败残留 |
| 所有 Git 仓库特性 | 不支持；LFS、submodule、sparse checkout 等明确拒绝 |

Phase 1 的产品承诺只有：

> 在一台受信任的 macOS 机器上，用简单 CLI 创建和管理多个低成本、Git 状态正确、相互隔离并可受保护回收的开发工作区。

---

## 十六、目标用户体验验收

Phase 1 完成时，以下体验必须成立：

1. 第一次初始化只需要指定一次数据根目录。
2. `doctor` 能解释当前机器为什么选择 APFS Clone 或为什么不能使用它。
3. 创建 Workspace 前可以通过 `--dry-run` 看到具体 commit、目标数据根、路径模式、文件系统和后端计划；dry-run 不分配或预留 ID。
4. 默认不允许 CoW 静默退化为完整复制。
5. `workspace path` 可以直接用于 `cd "$(...)"`。
6. 用户不需要理解 lowerdir、upperdir、Git administrative directory 或 SQLite。
7. 现有命令、编辑器和 Agent 可直接使用普通路径，不需要包装；平台不覆盖工具链配置，也不承诺安全 Sandbox。
8. dirty 内容、已确认的外部进程占用和未保护提交分别阻止适用的删除操作；扫描不完整如实警告，不宣称绝对无占用。
9. 创建或删除中断后，查询命令能只读识别问题，`doctor --repair` 或重复删除能安全恢复，无法证明安全时明确停止。
10. Agent 和脚本可以依赖已声明支持命令的 JSON、稳定错误码和 `workspace path` 的绝对路径完成管理自动化；`path` 和 `diff` 的原始输出是成功 JSON 的明确例外。

如果这十项中任何一项不能稳定成立，Phase 1 就不应标记为完成。

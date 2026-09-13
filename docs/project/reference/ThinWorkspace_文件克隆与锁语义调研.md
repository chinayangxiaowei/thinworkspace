# ThinWorkspace：文件克隆与锁语义调研

**调研日期：2026-09-12｜状态：文献核实完成；首轮本机实测及待完成项见 [P0-05 实施记录](../../development/implementation/P0-05_文件副本锁隔离实验.md)**

## 一、职责与非职责

本文保存文件身份、CoW、锁、时间戳和缓存一致性的跨平台证据，评估“克隆后扫描被锁文件，再重写以消除冲突”的提议，并给出可重复验证的场景。

本文不是新的物化接口设计，不定义 CLI、任务状态或缓存导入策略；不代表支持所有镜像、挂载、文件系统或语言工具。排期和任务状态只在[实施计划](../../development/planning/ThinWorkspace_Phase1实施计划_v1.0.md)的 P0-05 维护；实际实验结果在执行该任务时记录，不能用文献推论代替实测。

## 二、结论

建议验证“独立文件副本是否已经隔离锁”，不直接实施“扫描后原地重写解锁”。需要纠正三个前提：

1. CoW 的写时分离对象是数据块，不是每写一次就生成新的 inode。文件克隆与硬链接必须区分。
2. Windows 不能概括成按路径加锁；不同路径可能打开同一文件，且字节范围锁与打开时的共享模式是不同机制。
3. 可以按权限和 API 能力保留部分时间属性，但不能据此保证缓存命中、缓存一致性或锁隔离。

以下区分“官方接口事实”“由事实推导的预期”和“尚待实测的产品行为”。当前没有取得用户所述锁冲突的具体工具、错误或复现，因此不能把该提议标记为已证实的缺陷修复。

## 三、先识别副本是什么

这里的“文件克隆”指创建独立目标文件并共享数据块，不泛指磁盘镜像、快照或挂载同一路径。

| 方式 | 文件身份与数据关系 | 锁隔离判断 |
|---|---|---|
| APFS file clone / Linux reflink 到独立目标文件 | 两个逻辑文件共享部分或全部数据块；写入对各自文件生效 | 预期不继承源文件的内核锁；仍需双进程验证 |
| Full Copy 到新建文件 | 独立文件和复制的数据 | 预期不继承源文件内核锁；复制源文件本身仍可能被拒绝 |
| 硬链接 | 多个目录项引用同一文件 | 路径不同不产生锁隔离；不能作为独立可写副本 |
| 符号链接或同一文件的路径别名 | 解引用后可能仍访问原文件 | 必须看实际文件身份，不看路径字符串 |
| 卷镜像、文件系统快照、OverlayFS | 取决于具体挂载、文件身份和 copy-up 机制 | 不从文件克隆结论外推；不纳入本轮 macOS 文件克隆验收 |

Apple 明确说明克隆文件共享数据块但具有独立属性，后续写入只影响被写文件；Linux `FICLONE` 同样在源、目标文件之间共享存储。这是上表克隆预期的依据，不是已完成的 ThinWorkspace 锁测试。[Apple clonefile](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/man/man2/clonefile.2)、[Linux FICLONE](https://man7.org/linux/man-pages/man2/FICLONE.2const.html)

Unix 排查应把文件系统/设备身份与 inode 一起比较，不能仅用 `find -inum` 命中的数字判断共享。inode 编号只在所属文件系统内唯一；记录还必须对应同一次观察，防止对象删除后编号被复用。平台受控路径仍遵循[物化设计](../design/ThinWorkspace_跨平台工作区物化设计_v1.0.md)的身份重验，不另建平行身份模型。[Linux inode](https://man7.org/linux/man-pages/man7/inode.7.html)

## 四、锁的语义与扫描工具边界

### 4.1 Linux / macOS

“锁的是 inode”适合提醒人们不要只看路径，但不足以定义锁契约：需要同时区分被锁文件、字节范围、锁所有者和释放条件。Linux 的 `flock` 锁关联 open file description；传统 POSIX `fcntl` 记录锁按进程管理，Linux OFD 锁则关联 open file description。Apple `flock` 也不是独立的路径名锁。应分别运行各锁 API 的测试，不把不同 API 混用后能否互相阻塞当成统一结论。[Linux flock](https://man7.org/linux/man-pages/man2/flock.2.html)、[Linux fcntl locking](https://man7.org/linux/man-pages/man2/F_GETLK.2const.html)、[Apple flock](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/man/man2/flock.2)

这些常见本地 Unix 文件锁是协作式锁：不参与锁协议的程序可能仍能写文件。因此“写成功”不能证明“锁已解除”，还可能破坏正在使用该文件的程序。文件映射、网络文件系统及其他锁机制不能用这个简化模型承诺兼容。

| 工具 | 可用于什么 | 不能据此承诺什么 |
|---|---|---|
| Linux `/proc/locks` | 查看可见的 POSIX、FLOCK、OFD 锁和 leases，取得设备、inode、范围等信息 | 不是所有应用互斥机制清单；受 PID namespace 影响；OFD 的 PID 可能为 `-1` |
| Linux `lslocks` | 整理上述锁及可解析的持有者、路径 | 路径可能缺失或受权限限制，持有者不一定唯一；默认输出不宜作为稳定解析契约 |
| Linux `/proc/<pid>/fd/`、`find -inum` | 辅助关联打开对象与文件路径 | 打开不等于持锁；不能忽略权限、文件系统范围、别名和观察竞态 |
| macOS `lsof` | 查看可见打开对象及可报告的锁状态 | 报告能力依平台而异；可能只报告一个字节范围锁；无结果不证明无锁 |
| macOS `ls -lO` | 查看文件 flags，例如不可变标志 | 不是 `flock` / POSIX 锁查询工具；不能把清除保护标志当作解锁 |

工具边界依据：[proc_locks](https://man7.org/linux/man-pages/man5/proc_locks.5.html)、[lslocks](https://man7.org/linux/man-pages/man8/lslocks.8.html)、[proc_pid_fd 权限说明](https://man7.org/linux/man-pages/man5/proc_pid_fd.5.html)、[lsof LOCKS 章节](https://lsof.readthedocs.io/en/stable/manpage/)、[Apple ls 的 `-O`](https://github.com/apple-oss-distributions/file_cmds/blob/main/ls/ls.1)。

扫描只是某一时刻的诊断：扫描后可以新建锁、释放锁或替换路径。实验必须主动制造已知冲突作为阳性对照，再尝试锁定副本，不能以扫描为空作为成功断言。这些工具在本调研中是实验辅助，不是新增产品依赖；现有只读 ProcessProbe 的删除占用职责不变。

### 4.2 Windows

`LockFileEx` 通过文件 handle 锁定字节范围；`CreateFile` 的 `dwShareMode` 则控制其他打开请求是否兼容。前者不能等同于后者，也不能笼统理解成“锁路径”。NTFS 硬链接允许不同路径引用同一个文件，足以否定“只要路径不同就无冲突”的判断。[Microsoft LockFileEx](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex)、[CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)、[Hard links](https://learn.microsoft.com/en-us/windows/win32/fileio/hard-links-and-junctions)

独立文件副本预期有独立锁目标，但源文件可能因共享模式不兼容或锁定范围而无法读取/复制；必须分别验证“副本成功建立”和“副本独立使用”。`LockFileEx` 也不阻止所有映射视图访问，不能宣称是普遍的读写隔离。Windows 不安排无条件“免检测”分支，未来按实际文件系统和 API 实测。

## 五、“重写即可形成副本”为什么不能直接采用

### 5.1 原地写入

对已打开文件执行写入，通常继续使用同一文件身份。若它是硬链接别名，写入仍修改同一逻辑文件；若它已经是独立克隆，写入只是让被修改的共享数据块私有化。由上述文件身份、克隆和锁契约可推导：**原地写入不是切换锁对象的操作**，不能以发生写调用证明获得新的 inode 或全部数据块已独占。

全文件重写还可能增加 I/O、物理占用和 ENOSPC 风险，抵消 ThinWorkspace 的节省空间目标；只写一个字节又不能证明整文件已取消块共享。这些都不是解决“独立副本锁冲突”的必要前提。

### 5.2 新文件加 rename 替换

新建文件后替换目录项，可以使后续路径访问指向新文件，但旧 FD 及其锁不会因此转移到新文件。旧使用者继续操作旧对象、新使用者操作新对象，可能形成两份互不协调的状态；这不是安全解除旧锁。[Linux rename](https://man7.org/linux/man-pages/man2/rename.2.html)

该方式仅在受控、未发布的实验副本里用于验证旧 FD 行为，不能替换用户正在使用的缓存、数据库或锁文件。若以后确需切断别名关系，应先证明原因并评审既有物化边界，不能自动新增“解锁服务”。

## 六、时间戳和缓存必须分开验收

Unix 写入会影响 mtime/ctime。时间设置 API 可以按权限设置 atime/mtime，但正常设置操作也会更新 ctime；ctime 是状态变更时间，不是创建时间，不能通过恢复 mtime 把所有时间属性还原。时间精度也必须按实际 API/文件系统核对，不能只比较 `ls` 显示到秒的值。[Linux inode 时间字段](https://man7.org/linux/man-pages/man7/inode.7.html)、[Linux utimensat](https://man7.org/linux/man-pages/man2/utimensat.2.html)、[Apple utimes](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/man/man2/utimes.2)

缓存命中取决于工具自身规则，不只是缓存文件 mtime。例如 ccache 的键还涉及编译器、选项、输入内容及配置下的工作目录；保留时间无法抵消所有路径或环境变化。这是不能做通用命中承诺的反例，不是要求产品接管 C/C++ 或任何其他语言。[ccache 工作原理](https://ccache.dev/manual/latest.html#_how_ccache_works)

还需要区分三种完全不同的东西：

- **内核锁**：运行时状态；独立文件复制不等于复制这个状态。
- **应用锁标记**：磁盘上的普通文件也可能表达互斥。Git 的 lockfile 协议通过 `O_CREAT | O_EXCL` 检测已有文件；这类标记若被复制，重写其内容仍不会使该路径消失。不能按 `.lock` 后缀批量删除。[Git lockfile API](https://git-scm.com/docs/api-lockfile)
- **多文件一致性**：数据库、journal、WAL 等可能构成整体。单文件克隆成功不证明整目录是同一事务时点；随意复制或移除恢复文件可能损坏副本。[SQLite 复制与恢复风险](https://www.sqlite.org/howtocorrupt.html)

锁隔离、内容一致、元数据保留、缓存命中是四个验收维度，任何一个通过都不替代其余三个。未知缓存格式交由使用它的工具判断；不通过伪造时间、清除标记或绕过工具校验提高“命中率”。

后续产品边界已由 [ADR-0002](../architecture/adr/ADR-0002_Phase1原始目录镜像与流程交付.md) 改为原始目录镜像，已有构建产物随普通文件复制；输入一致性和保真范围以[物化设计](../design/ThinWorkspace_跨平台工作区物化设计_v1.0.md)为准。该决策不把本调研的锁、时间戳或缓存预期变成实测结果，也不批准扫描后重写功能。

## 七、可重复验证矩阵

所有场景只能使用实验新建的临时文件和自有测试进程，不能扫描后修改真实用户数据。先固定持锁进程已经成功加锁，再启动独立竞争进程，以有界等待和显式退出结果判定；不靠固定 sleep 猜测时序。保存 OS/API 版本、文件身份、调用结果、内容摘要和相关元数据，扫描结果只作辅助。

首轮使用独占锁，字节范围锁使用重叠范围；竞争者独立打开文件，不继承持锁者 FD。持锁进程在整段观察期间保持锁和 FD，物化与摘要读取由其他进程完成，避免持锁进程关闭该文件的其他 FD 意外释放传统 POSIX 记录锁。[fcntl 记录锁释放规则](https://man7.org/linux/man-pages/man2/F_GETLK.2const.html)

| 编号 | 场景 | 必须观察的结果/证据 |
|---|---|---|
| L1 | 同一路径、硬链接、符号链接别名，分别测试同一种锁 API | 第二进程应产生已知冲突；否则测试装置无效，不能继续宣称隔离 |
| L2 | 源文件持锁时创建 APFS clone 和新建 Full Copy 目标，再分别加锁 | 记录物化结果；成功副本身份独立，副本锁可获取且原锁仍有效；双方写入互不改变对方内容 |
| L3 | 对同一已锁文件原地写入；与独立克隆上的写入对照 | 写成功不作为解锁证据；比较 inode、内容和另一进程的锁尝试，区分文件身份与 CoW |
| L4 | 实验副本新建文件加 rename；保留旧 FD | 新路径与旧 FD 指向的身份、内容和锁行为分别记录，证明没有把旧使用者迁移过来 |
| L5 | 已知锁与扫描结果对比；拒绝访问/扫描失败/锁释放后再获取 | 区分确证据、无证据和不完整观察；不得把快照当成持续无锁保证 |
| L6 | 克隆、复制、原地写入、实验性时间恢复 | 比较内容摘要、atime/mtime/ctime、精度和可读取的其他元数据；不承诺 ctime 复原 |
| L7 | 普通 `.lock` 标记与自有进程内核锁对照；模拟两文件更新中的复制 | 分别观察持久标记和运行时锁；展示逐文件成功不足以证明应用一致性；不自动删除标记 |
| L8 | 如果提供了原始冲突样例，在隔离的样例副本复现 | 固定工具/版本/命令/实际缓存位置；单独报告锁冲突、缓存命中/失效和输出正确性；样例缺失写“未执行” |

首轮在 macOS/APFS 上分别覆盖 `flock` 和 POSIX `fcntl`。Linux 复用上述断言时追加 Btrfs/XFS `FICLONE` 与 OFD 锁；Windows 则分别覆盖共享模式、字节范围锁及可用的独立复制/块克隆方式，NTFS 硬链接仅作别名对照。后两者需各自真实环境，不能用 macOS 结果或 mock 代替；网络文件系统、OverlayFS 和卷镜像需独立验证，不在本矩阵的首轮支持声明中。

## 八、如何决定后续处理

- 若标准独立克隆已隔离锁：保留回归证据，**不做扫描后重写功能**。
- 若冲突来自别名、外部共享路径或应用标记：按真实原因报告边界；不通过改写原文件、杀进程、强制解锁或语言专用重定向掩盖问题。
- 若正确物化后的独立文件仍有可重复的系统级冲突：先由主 Agent 核验 syscall、文件身份和测试时序；确需修改现行接口/安全边界时提出 ADR 和独立实现任务，获准后再排实施。
- 若仅缓存命中失败：按工具的一致性规则诊断，不能把它归为锁隔离失败。原样复制已有文件不等于接管缓存策略；新增策略仍须单独的产品范围决策。

本轮 Go/No-Go 指“是否有证据支持继续实施锁冲突处理”，不是 Phase 0 或 Phase 1 放行。

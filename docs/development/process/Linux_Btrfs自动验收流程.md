# Linux/Btrfs 自动验收流程

## 职责

本文规定如何用同一份静态 musl Release 候选，在 Debian 12/aarch64 虚拟机的真实 ext4＋Btrfs 上自动执行用户可见的 CLI 操作验收。它代替维护者逐条敲命令、肉眼判断输出的功能测试，并定义可留存的通过/失败证据。

## 非职责

CLI 参数、JSON 和错误码仍只由[用户操作手册](../../project/reference/ThinWorkspace_Phase1用户操作手册_v1.0.md)定义；平台能力由[跨平台物化设计](../../project/design/ThinWorkspace_跨平台工作区物化设计_v1.0.md)定义；Cargo、变异、fuzz 与阶段门禁仍由[任务流程](任务流程.md)决定。此流程不在用户真实目录运行，不替代其他质量门禁或人类对最终发布的授权决定，不因自动测试通过而自行修改支持矩阵或打 tag。

## 输入与资格预检

验收输入是产品源码已冻结、相互匹配的两个 ARM64 musl Release 可执行文件：`thinws` 和 `e2e_linux`。构建与复制方法见[Docker Linux/Btrfs 开发测试方法](Docker_LinuxBtrfs开发测试方法.md)的静态 musl 部分；不得混入 GNU Debug、旧测试程序或产品语义不同的二进制。若只更新测试入口而复用产品字节，实施记录须分别绑定产品源码候选、测试源码候选、两个产物 SHA-256 和运行结果。

在目标 VM 上，以普通用户提供六个绝对路径：

1. `thinws` 可执行文件；
2. `e2e_linux` 可执行文件；
3. 专用、可写的 ext4 测试根，用于隔离 `HOME/.thinws`；
4. 专用、可写的真实 Btrfs 测试根，用于 source 和 target；
5. Parallels `prl_fs` 共享盘上专用极小夹具内已存在的普通文件，路径及其各级父目录不得含符号链接，仅供测试取其父目录作为只读对照；
6. 尚不存在、父目录已存在的报告目录。

两个测试根和共享对照源都不得是用户实际项目目录。脚本只让测试程序在两个可写根内创建随机临时子目录，不删除提供的根；共享对照源仅用于应拒绝的来源路径，不应包含其他数据；报告目录只新建一次，不覆盖既有报告。普通用户权限、Debian 12/aarch64、资格内核 `5.10.0-24-arm64`、ext4 与 Btrfs 不同挂载、对照文件及其父目录确在 `prl_fs`、其路径无符号链接组件、Git 和必要检查工具、两个产物的静态 ARM64 ELF 与无 GLIBC 依赖均由脚本在测试前检查。不满足时退出非零，不能把 Docker 或其他内核、文件系统的结果当成这台 VM 的合格结果。

## 一条命令执行

在 Debian VM 上调用仓库内的 `tools/linux-btrfs-acceptance.sh`；以下路径是格式示例，先按实际挂载创建**专用测试根**并把两个产物复制到 VM 本地可执行目录：

```bash
bash /path/to/worktree/tools/linux-btrfs-acceptance.sh \
  /home/user/thinws-acceptance/bin/thinws \
  /home/user/thinws-acceptance/bin/e2e_linux \
  /home/user/thinws-acceptance/ext4 \
  /media/user/thinws/thinws-acceptance-btrfs \
  /media/psf/data/thinws-acceptance-fixture/source/marker.txt \
  /home/user/thinws-acceptance/report-001
```

示例中的共享盘文件必须替换为 VM 中实际存在的专用小夹具文件；不得直接指向项目源码。测试只把其父目录作为应拒绝的 `prl_fs` 来源，不复制或改写该文件。脚本先用哨兵程序替换 CLI 做一次**必须失败**的负控制，并要求日志确实看到哨兵标记；这可防止测试悄悄走库入口却宣称黑盒通过。随后同一 `e2e_linux` 套件以 `THINWS_LINUX_E2E_BINARY` 指向指定 Release CLI，逐项启动真实进程，要求全部列出的测试执行、零失败、零忽略。这个变量仅供测试程序选择被验收的二进制，不是 ThinWorkspace 的产品配置项。

验收场景按既有测试维护，不另复制命令、字段和错误码常量：

| 场景组 | 自动判别的关键结果 |
|---|---|
| 普通使用 | init、doctor、dry-run 无副作用、真实 reflink 创建、普通路径直接使用、list/path/status、空间统计及删除；普通目录和 Btrfs 子卷来源均覆盖 |
| 能力与布局拒绝 | ext4 或共享盘来源不被偷偷完整复制；NOCOW 的真实失败及非 Ready 清理；已存在 target 符号链接、活动 target 内嵌路径被拒绝且不跟随、不覆盖 |
| Git 与清理 | 未跟踪内容不阻塞；已跟踪修改普通清理拒绝且保留目录；显式 force 后删除并保留拒绝、开始和完成日志 |
| 归属与占用 | 登记 target 缺失时即使 force 仍保留关联；已确认外部进程占用不被 force 绕过；活动删除隔离目录与新 target 冲突时拒绝 |

测试程序内的库入口模式仍用于开发时的快速集成测试；**只有脚本负控制通过、指定 Release CLI 的整组进程模式通过，才可记为 Debian VM 黑盒自动验收通过**。Docker 真实 Btrfs 测试是前置补充证据，不能代替 VM 结果。

## 判定与证据

报告目录产生 `environment.txt`（含实际测试根）、`artifacts.sha256`（两个产物及执行脚本）、`elf.txt`、`negative-control.log`、`tests.list`、`tests.log` 和 `summary.txt`。退出码 0 且 `summary.txt` 为 `status=PASS` 才算本次自动功能验收通过；任一预检、负控制或场景失败都不得登记通过。实施记录保留候选 commit、执行命令、报告位置、产物哈希、VM 内核与文件系统、通过/失败及未执行项，不把报告正文复制进计划或手册。

本流程只替代人工**操作和目测功能测试**。阶段放行仍须完成《任务流程》规定的其余证据和人类最终授权；若维护者希望取消“人工签字放行”规则，应先单独修改该规则和阶段计划，不能由测试脚本暗中替代。

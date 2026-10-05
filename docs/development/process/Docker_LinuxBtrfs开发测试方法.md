# Docker Linux/Btrfs 开发测试方法

## 职责

本文只规定在 macOS 上使用 Docker Desktop 为 Linux 开发测试提供可重复的真实 Btrfs、ext4 夹具，以及如何执行仓库现有 Cargo 门禁。

## 非职责

测试种类、范围和完成门槛仍由《任务流程》决定；Linux 产品资格、平台语义和公开契约仍由对应设计、ADR-0007 与用户手册决定。本文不把 Docker Desktop 宣称为目标 Debian 12 VM，也不改变发布支持矩阵。

## 前提与边界

- 从仓库根目录执行命令；Docker Desktop 使用 Linux 容器，并允许本机可信镜像以 `--privileged` 创建 loop 挂载。`--privileged` 权限较大，仅用于本机隔离测试，不挂载 Docker socket、用户 HOME、凭据或其他宿主目录。
- 常规开发命令把仓库以只读 `/work` 挂载；两个同名 target 挂载指向同一个 Docker 命名卷，让 Cargo 的 `/target` 和测试程序内嵌的 `/work/target` 路径均指向同一产物，避免测试写入宿主仓库。Cargo registry 另用命名卷缓存；宿主 bind mount 失效时采用下文的一次性容器源码快照。Debian 11 补充测试也须把同一 musl 产物卷挂到这两个路径；runner 会对 `/work/target` 执行 `chown`，因此该挂载必须可写，不使用匿名空卷或只读挂载。
- `tools/docker-linux-btrfs/run.sh` 仅在一次性容器内部创建稀疏 Btrfs/ext4 镜像并挂载，预检 Btrfs reflink，随后以普通用户运行传入的命令。退出时卸载；`docker run --rm` 删除容器及其临时镜像文件，不删除挂载的命名缓存卷。
- 镜像内含 `btrfs-progs`。需要真实子卷的测试以普通用户在临时 Btrfs 夹具内创建空子卷，并用 `rmdir` 清理；该挂载布局下普通用户执行 `btrfs subvolume delete` 可能返回 `EPERM`，不能把夹具清理失败当作产品删除失败。
- 默认开发镜像是 Debian 12 用户态加 Docker Desktop 的 LinuxKit 内核；下文的运行镜像换成 Debian 11 用户态，但仍共用 LinuxKit 内核。两者都不是当前资格基线的 Debian 12/5.10 VM，也不提供 Parallels `prl_fs`。容器能验证真实 Btrfs/ext4、Linux 编译及生命周期行为；目标内核、`prl_fs` 与 VM 挂载拓扑须由目标 VM 的自动验收核对。
- 本文的 Linux Release 目标是 ARM64 musl；示例按本机 Docker 的 `linux/arm64` 镜像运行。其他架构不能直接沿用 `musl-gcc` 与 `aarch64-unknown-linux-musl` 的组合，必须另行验证交叉链接器和目标运行环境。

## 构建与运行

首次、Dockerfile 或 `tools/quality-tools.toml` 改动后构建本地镜像。镜像包含 Btrfs 工具、musl C 链接器、Rust 的 `aarch64-unknown-linux-musl` 标准库，以及按质量工具清单固定版本安装的 `cargo-mutants`。额外构建上下文只传入 `tools/`，不把仓库的 `target/` 送入 Docker：

```bash
docker build --build-context quality-tools=tools \
  -t thinws-linux-btrfs-dev:rust-1.97.1-musl tools/docker-linux-btrfs
docker run --rm thinws-linux-btrfs-dev:rust-1.97.1-musl cargo mutants --version
```

从仓库根目录运行全工作区普通测试：

```bash
docker run --rm --privileged \
  -v "$PWD":/work:ro \
  -v thinws-linux-cargo:/usr/local/cargo/registry \
  -v thinws-linux-target:/target \
  -v thinws-linux-target:/work/target \
  -e CARGO_TARGET_DIR=/target \
  thinws-linux-btrfs-dev:rust-1.97.1-musl \
  sh /work/tools/docker-linux-btrfs/run.sh \
  cargo test --locked --workspace --all-targets
```

GNU 开发测试的严格 Clippy 或定向测试只替换命令末尾，例如分别使用 `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`、`cargo test --locked -p thinws-adapter-linux --lib process::tests`。Linux Release 候选须使用下文的 musl 命令，不把此默认 GNU 目标构建当成发布产物。不要在同一时间对同一命名 target 卷并行启动 Cargo。首次下载依赖需要网络；缓存齐全后可增加 `--network none` 并让 Cargo 使用 `--offline`，但离线失败不能冒充测试失败。

若 Docker Desktop 临时无法 bind mount 外置卷（例如报 `mkdir /host_mnt/Volumes/data: file exists`），先确认宿主仓库仍可读取、无宿主目录挂载的容器能运行；不要把 Docker 环境错误记作项目测试失败。可用一次性容器接收 Git 快照，再逐个覆盖本次尚未提交的源码文件：

```bash
container_id=$(docker create --rm --privileged \
  -v thinws-linux-cargo:/usr/local/cargo/registry \
  -v thinws-linux-target:/target \
  -v thinws-linux-target:/work/target \
  -e CARGO_TARGET_DIR=/target \
  thinws-linux-btrfs-dev:rust-1.97.1-musl \
  sh /work/tools/docker-linux-btrfs/run.sh \
  cargo test --locked -p thinws-adapter-linux --lib materializer::tests)
docker cp tools/docker-linux-btrfs "$container_id":/work
git archive HEAD | docker cp - "$container_id":/work
docker cp crates/thinws-adapter-linux/src/materializer.rs \
  "$container_id":/work/crates/thinws-adapter-linux/src/materializer.rs
docker start -a "$container_id"
```

上例中 `docker cp` 的文件只是示范：运行前核对 `git status`，把当前候选中所有未提交且影响构建/测试的文件逐个复制进容器；否则测到的是旧 `HEAD`。新建的未跟踪文件若父目录不在归档内，也须先复制其父目录。一次性容器退出后自动删除；命名 Cargo 缓存卷仍保留。记录所测源码的哈希、容器命令和退出码；复制路径仅用于开发验证，不能替代原 Debian VM 的资格验收。

脚本在容器内设置 `THINWS_LINUX_BTRFS_TEST_ROOT=/mnt/thinws-btrfs/fixtures`、`THINWS_LINUX_EXT4_TEST_ROOT=/mnt/thinws-ext4/fixtures`、`THINWS_LINUX_OTHER_TEST_FILE=/mnt/thinws-ext4/fixtures/other.txt`，还为专项测试设置同设备不同挂载的 `THINWS_LINUX_BIND_MOUNT_SOURCE`、`THINWS_LINUX_BIND_MOUNT_CHILD` 与指向后者的 `THINWS_LINUX_SECOND_BTRFS_MOUNT_ROOT`。这些路径仅是容器内测试夹具，不能用作产品 CLI 的持久配置。四个跨挂载专项测试默认被忽略；要实际运行，将上述命令末尾分别替换为 `cargo test --locked -p thinws-adapter-linux --lib destroy::tests::same_device_child_on_different_mount_is_not_in_deletion_scope -- --ignored`、`cargo test --locked -p thinws-adapter-linux --lib tree::tests::source_snapshot_rejects_a_child_on_another_mount -- --ignored`、`cargo test --locked -p thinws-adapter-linux --test btrfs_probe same_btrfs_filesystem_on_a_different_mount_is_rejected -- --ignored` 或 `cargo test --locked -p thinws-adapter-linux --test linux_workspace bind_alias_inside_control_root_is_rejected_before_target_creation -- --ignored --exact`。预检必须打印 `Btrfs reflink confirmed`，且命令以退出码 0 结束，才能记录为通过。遇到测试失败应保留原始失败原因；不要通过改用 root、跳过测试或改设非 Btrfs 路径来制造绿色结果。

使用 `git archive HEAD` 快照与 `cargo-mutants --in-place` 时，跨源码批次复用编译卷可能让归档文件的旧 mtime 命中上一批变异产物，使“未变异基线”误用旧代码。换用新的独立变异编译卷，或先明确清除该批受影响的 Cargo 产物；只有基线按本批确定源码重新构建并通过，变异结果才有效。执行器测试参数中的额外 `--` 须按实际生成的 `cargo test` argv 核对，例如要运行 ignored 测试，应传 `cargo mutants … -- -- --include-ignored`，不能把 `--include-ignored` 误传给 Cargo 本身。

一次性快照由 `docker cp` 建立时源码通常归 root；在该**容器私有快照**上运行 `cargo mutants --in-place`，须在切换到 `thinws` 测试用户前，仅将待变异 crate 的源码目录交给该用户，否则未变异基线通过后会因无法覆盖源码而失败。不得对宿主 bind mount 做这项归属修改。`--in-place` 不可同时指定 `-j/--jobs`，应省略并发参数；参数或快照权限错误属于测试布置失败，不计作变异结果。

Linux Release 候选使用静态 musl 目标。用与普通测试分开的编译卷，避免 GNU 与 musl 产物或变异测试缓存混用：

```bash
docker run --rm --privileged \
  -v "$PWD":/work:ro \
  -v thinws-linux-cargo:/usr/local/cargo/registry \
  -v thinws-linux-musl-target:/target \
  -v thinws-linux-musl-target:/work/target \
  -e CARGO_TARGET_DIR=/target \
  -e CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc \
  -e CC_aarch64_unknown_linux_musl=musl-gcc \
  thinws-linux-btrfs-dev:rust-1.97.1-musl \
  sh /work/tools/docker-linux-btrfs/run.sh \
  cargo build --locked --release --target aarch64-unknown-linux-musl -p thinws-cli
```

如宿主 bind mount 故障，按上面的 `docker create`＋`git archive`＋`docker cp` 方法输入相同源码快照，把此命令的 musl 编译卷、两个 `-e` 链接器变量和 `cargo build` 参数原样用于一次性容器。验证真实 Btrfs CLI 链路时，只把末尾改成 `cargo test --locked --release --target aarch64-unknown-linux-musl -p thinws-cli --test e2e_linux`；更大范围的测试仍按《任务流程》确定。

核验最终产物，而不是仅看 Cargo 目标名称；`file` 应显示 `statically linked`，`readelf -l` 不得有 `INTERP`，`readelf -d` 不得列出 `NEEDED`，版本信息不得包含 `GLIBC_`：

```bash
docker run --rm -v thinws-linux-musl-target:/target:ro \
  thinws-linux-btrfs-dev:rust-1.97.1-musl \
  sh -c 'file /target/aarch64-unknown-linux-musl/release/thinws; readelf -l /target/aarch64-unknown-linux-musl/release/thinws; readelf -d /target/aarch64-unknown-linux-musl/release/thinws; readelf --version-info /target/aarch64-unknown-linux-musl/release/thinws'
docker run --rm -v thinws-linux-musl-target:/target:ro \
  debian:11 /target/aarch64-unknown-linux-musl/release/thinws --version
```

要在 Debian 11 用户态运行同一份静态 musl CLI 的真实 Btrfs 黑盒测试，构建仅供本地验收的运行镜像；它不用 Rust 工具链。Dockerfile 使用基础镜像标注的固定 Debian 包快照，避免滚动安全仓库索引与已移走包版本不一致；该快照镜像不是产品运行镜像，也不代表最新安全补丁：

```bash
docker build -f tools/docker-linux-btrfs/Dockerfile.debian11-runtime \
  -t thinws-linux-btrfs-runtime:debian11 tools/docker-linux-btrfs
docker run --rm --privileged \
  -v "$PWD":/work:ro \
  -v thinws-linux-musl-target:/target:ro \
  -v thinws-linux-musl-target:/work/target \
  thinws-linux-btrfs-runtime:debian11 \
  sh /work/tools/docker-linux-btrfs/run.sh \
  sh -c '
    binary=
    for candidate in /target/aarch64-unknown-linux-musl/release/deps/e2e_linux-*; do
      [ -f "$candidate" ] && [ -x "$candidate" ] || continue
      [ -z "$binary" ] || exit 1
      binary=$candidate
    done
    [ -n "$binary" ] && exec "$binary" --nocapture
  '
```

该命令运行的是 musl Release 模式编出的 CLI 测试程序，不在 Debian 11 容器内重新编译；它实际启动编译时内嵌的 `/work/target/aarch64-unknown-linux-musl/release/thinws`。预检必须打印 `Btrfs reflink confirmed`，`e2e_linux` 套件全部通过且容器退出码为 0。若宿主 bind mount 故障，沿用上文的 `docker create`＋`git archive HEAD`＋`docker cp` 快照法，保留这里的运行镜像、同一产物卷的双路径挂载与测试命令；不要把失败的挂载当成产品测试结果。

需要在 macOS 宿主取得 Linux 产物时，可从停止的一次性容器复制到仓库忽略的 `target/linux-musl/thinws`；不要覆盖本机 macOS 的 `target/release/thinws`，也不要把 Linux ELF 安装到 macOS 用户 bin：

```bash
container_id=$(docker create -v thinws-linux-musl-target:/target:ro \
  thinws-linux-btrfs-dev:rust-1.97.1-musl true)
mkdir -p target/linux-musl
docker cp "$container_id":/target/aarch64-unknown-linux-musl/release/thinws \
  target/linux-musl/thinws
docker rm "$container_id"
shasum -a 256 target/linux-musl/thinws
```

Debian 11 容器冒烟及黑盒测试只证明该用户态与 Docker LinuxKit 内核下的行为，不证明目标 Debian 12/5.10 VM、`prl_fs` 或实际挂载拓扑资格；静态 musl 也不使不支持的文件系统自动具备 reflink。目标 VM 的 Release CLI 自动黑盒验收由[Linux/Btrfs 自动验收流程](Linux_Btrfs自动验收流程.md)单独管理，剩余阶段质量门禁与最终放行授权仍须补齐。

## 证据使用

记录 Docker 镜像、内核、文件系统预检、执行命令、退出码和候选提交或 diff。Docker 结果可补充 Linux 开发证据，但不能替代《Phase 1 实施计划》P1-L05 所要求的目标 Debian VM 全命令黑盒验收。变异测试仍按《任务流程》使用与普通验证隔离的编译目标，并处理存活变异；此处的普通测试命名卷不得直接复用为变异编译缓存。

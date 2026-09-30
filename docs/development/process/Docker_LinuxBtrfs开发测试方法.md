# Docker Linux/Btrfs 开发测试方法

## 职责

本文只规定在 macOS 上使用 Docker Desktop 为 Linux 开发测试提供可重复的真实 Btrfs、ext4 夹具，以及如何执行仓库现有 Cargo 门禁。

## 非职责

测试种类、范围和完成门槛仍由《任务流程》决定；Linux 产品资格、平台语义和公开契约仍由对应设计、ADR-0007 与用户手册决定。本文不把 Docker Desktop 宣称为 Debian 11.7 VM，也不改变发布支持矩阵。

## 前提与边界

- 从仓库根目录执行命令；Docker Desktop 使用 Linux 容器，并允许本机可信镜像以 `--privileged` 创建 loop 挂载。`--privileged` 权限较大，仅用于本机隔离测试，不挂载 Docker socket、用户 HOME、凭据或其他宿主目录。
- 仓库以只读 `/work` 挂载；两个同名 target 挂载指向同一个 Docker 命名卷，让 Cargo 的 `/target` 和少数旧测试使用的 `/work/target` 均可写，避免测试写入宿主仓库。Cargo registry 另用命名卷缓存。
- `tools/docker-linux-btrfs/run.sh` 仅在一次性容器内部创建稀疏 Btrfs/ext4 镜像并挂载，预检 Btrfs reflink，随后以普通用户运行传入的命令。退出时卸载；`docker run --rm` 删除容器及其临时镜像文件，不删除两个命名缓存卷。
- 镜像内含 `btrfs-progs`。需要真实子卷的测试以普通用户在临时 Btrfs 夹具内创建空子卷，并用 `rmdir` 清理；该挂载布局下普通用户执行 `btrfs subvolume delete` 可能返回 `EPERM`，不能把夹具清理失败当作产品删除失败。
- 该环境是 Debian 12 用户态加 Docker Desktop 的 LinuxKit 内核，既不是原定 Debian 11.7/5.10 VM，也不提供 Parallels `prl_fs`。它能验证真实 Btrfs/ext4、Linux 编译和大部分生命周期行为；内核版本、`prl_fs`、VM 挂载拓扑及维护者人工验收仍需在原定环境完成。

## 构建与运行

首次或 Dockerfile 改动后构建本地镜像：

```bash
docker build -t thinws-linux-btrfs-dev:rust-1.97.1 tools/docker-linux-btrfs
```

从仓库根目录运行全工作区普通测试：

```bash
docker run --rm --privileged \
  -v "$PWD":/work:ro \
  -v thinws-linux-cargo:/usr/local/cargo/registry \
  -v thinws-linux-target:/target \
  -v thinws-linux-target:/work/target \
  -e CARGO_TARGET_DIR=/target \
  thinws-linux-btrfs-dev:rust-1.97.1 \
  sh /work/tools/docker-linux-btrfs/run.sh \
  cargo test --locked --workspace --all-targets
```

严格 Clippy、定向测试或 Release 构建只替换命令末尾，例如分别使用 `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`、`cargo test --locked -p thinws-adapter-linux --lib process::tests`、`cargo build --locked --release -p thinws-cli`。不要在同一时间对同一命名 target 卷并行启动 Cargo。首次下载依赖需要网络；缓存齐全后可增加 `--network none` 并让 Cargo 使用 `--offline`，但离线失败不能冒充测试失败。

若 Docker Desktop 临时无法 bind mount 外置卷（例如报 `mkdir /host_mnt/Volumes/data: file exists`），先确认宿主仓库仍可读取、无宿主目录挂载的容器能运行；不要把 Docker 环境错误记作项目测试失败。可用一次性容器接收 Git 快照，再逐个覆盖本次尚未提交的源码文件：

```bash
container_id=$(docker create --rm --privileged \
  -v thinws-linux-cargo:/usr/local/cargo/registry \
  -v thinws-linux-target:/target \
  -v thinws-linux-target:/work/target \
  -e CARGO_TARGET_DIR=/target \
  thinws-linux-btrfs-dev:rust-1.97.1 \
  sh /work/tools/docker-linux-btrfs/run.sh \
  cargo test --locked -p thinws-adapter-linux --lib materializer::tests)
docker cp tools/docker-linux-btrfs "$container_id":/work
git archive HEAD | docker cp - "$container_id":/work
docker cp crates/thinws-adapter-linux/src/materializer.rs \
  "$container_id":/work/crates/thinws-adapter-linux/src/materializer.rs
docker start -a "$container_id"
```

上例中 `docker cp` 的文件只是示范：运行前核对 `git status`，把当前候选中所有未提交且影响构建/测试的文件逐个复制进容器；否则测到的是旧 `HEAD`。新建的未跟踪文件若父目录不在归档内，也须先复制其父目录。一次性容器退出后自动删除；命名 Cargo 缓存卷仍保留。记录所测源码的哈希、容器命令和退出码；复制路径仅用于开发验证，不能替代原 Debian VM 的资格验收。

脚本在容器内设置 `THINWS_LINUX_BTRFS_TEST_ROOT=/mnt/thinws-btrfs/fixtures`、`THINWS_LINUX_EXT4_TEST_ROOT=/mnt/thinws-ext4/fixtures`、`THINWS_LINUX_OTHER_TEST_FILE=/mnt/thinws-ext4/fixtures/other.txt`，还为专项测试设置同设备不同挂载的 `THINWS_LINUX_BIND_MOUNT_CHILD` 与指向该挂载的 `THINWS_LINUX_SECOND_BTRFS_MOUNT_ROOT`。这些路径仅是容器内测试夹具，不能用作产品 CLI 的持久配置。三个跨挂载专项测试默认被忽略；要实际运行，分别将上述命令末尾替换为 `cargo test --locked -p thinws-adapter-linux --lib destroy::tests::same_device_child_on_different_mount_is_not_in_deletion_scope -- --ignored`、`cargo test --locked -p thinws-adapter-linux --lib tree::tests::source_snapshot_rejects_a_child_on_another_mount -- --ignored` 和 `cargo test --locked -p thinws-adapter-linux --test btrfs_probe same_btrfs_filesystem_on_a_different_mount_is_rejected -- --ignored`。预检必须打印 `Btrfs reflink confirmed`，且命令以退出码 0 结束，才能记录为通过。遇到测试失败应保留原始失败原因；不要通过改用 root、跳过测试或改设非 Btrfs 路径来制造绿色结果。

检查 GNU/Linux Release 二进制对 glibc 的要求时，可对已构建的容器内产物执行：

```bash
docker run --rm -v thinws-linux-target:/target:ro \
  thinws-linux-btrfs-dev:rust-1.97.1 \
  sh -c 'readelf --version-info /target/release/thinws | grep -o "GLIBC_[0-9.]*" | sort -Vu | tail -1'
```

该值必须与**拟发布目标系统**的 libc 版本核对；符号版本检查不能代替在目标系统运行。当前镜像基于 Debian 12，不能仅因在此容器中构建、测试通过，就认为其二进制兼容 Debian 11。Debian 11 候选产物需要兼容的构建用户态及原定 VM 实测；本方法不把 Docker LinuxKit 内核、Debian 12 用户态或容器生成的 Release 文件冒充该验收。

## 证据使用

记录 Docker 镜像、内核、文件系统预检、执行命令、退出码和候选提交或 diff。Docker 结果可补充 Linux 开发证据，但不能替代《Phase 1 实施计划》P1-L05 所要求的 Debian VM 全命令黑盒验收。变异测试仍按《任务流程》使用与普通验证隔离的编译目标，并处理存活变异；此处的普通测试命名卷不得直接复用为变异编译缓存。

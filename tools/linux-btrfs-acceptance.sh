#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 6 ]]; then
    echo "usage: $0 <thinws-binary> <e2e-linux-binary> <ext4-test-root> <btrfs-test-root> <other-file> <new-report-dir>" >&2
    exit 2
fi

cli=$1
suite=$2
ext4_root=$3
btrfs_root=$4
other_file=$5
report_dir=$6

for path in "$cli" "$suite" "$ext4_root" "$btrfs_root" "$other_file" "$report_dir"; do
    if [[ "$path" != /* ]]; then
        echo "all paths must be absolute: $path" >&2
        exit 2
    fi
done
if [[ $(id -u) -eq 0 ]]; then
    echo "run acceptance as an ordinary user, not root" >&2
    exit 2
fi
if [[ ! -f "$cli" || ! -x "$cli" || ! -f "$suite" || ! -x "$suite" ]]; then
    echo "the CLI and E2E test binaries must be executable regular files" >&2
    exit 2
fi
if [[ ! -d "$ext4_root" || ! -w "$ext4_root" || ! -d "$btrfs_root" || ! -w "$btrfs_root" || ! -f "$other_file" || -L "$other_file" ]]; then
    echo "test roots must be writable directories and the comparison file must be a regular non-symlink file" >&2
    exit 2
fi
if [[ -e "$report_dir" || -L "$report_dir" || ! -d $(dirname -- "$report_dir") ]]; then
    echo "report directory must be new and its parent must exist" >&2
    exit 2
fi

if [[ $(uname -m) != aarch64 ]]; then
    echo "this acceptance run requires aarch64" >&2
    exit 2
fi
if [[ $(uname -r) != 5.10.0-24-arm64 ]]; then
    echo "this acceptance run requires the qualified 5.10.0-24-arm64 kernel" >&2
    exit 2
fi
# The target qualification is Debian 12; the VM's display name is not evidence.
if ! grep -qx 'ID=debian' /etc/os-release || ! grep -qx 'VERSION_ID="12"' /etc/os-release; then
    echo "this acceptance run requires Debian 12" >&2
    exit 2
fi
if [[ $(findmnt -n -o FSTYPE -T "$ext4_root") != ext4 ||
      $(findmnt -n -o FSTYPE -T "$other_file") != prl_fs ||
      $(findmnt -n -o FSTYPE -T "$(dirname -- "$other_file")") != prl_fs ||
      $(findmnt -n -o FSTYPE -T "$btrfs_root") != btrfs ]]; then
    echo "expected ext4 control root, prl_fs comparison file and real Btrfs workspace root" >&2
    exit 2
fi
if [[ $(findmnt -n -o TARGET -T "$ext4_root") == $(findmnt -n -o TARGET -T "$btrfs_root") ]]; then
    echo "control and workspace test roots must be on different mounts" >&2
    exit 2
fi
for tool in file readelf sha256sum git tee; do
    command -v "$tool" >/dev/null || { echo "missing test tool: $tool" >&2; exit 2; }
done

mkdir -- "$report_dir"
chmod 700 -- "$report_dir"
result=FAIL
trap 'printf "status=%s\n" "$result" > "$report_dir/summary.txt"' EXIT

{
    date -u '+time_utc=%Y-%m-%dT%H:%M:%SZ'
    uname -srm
    grep -E '^(ID|VERSION_ID|PRETTY_NAME)=' /etc/os-release
    printf 'ext4_test_root=%s\nbtrfs_test_root=%s\ncomparison_file=%s\n' \
        "$ext4_root" "$btrfs_root" "$other_file"
    findmnt -n -o TARGET,FSTYPE,SOURCE -T "$ext4_root"
    findmnt -n -o TARGET,FSTYPE,SOURCE -T "$btrfs_root"
    findmnt -n -o TARGET,FSTYPE,SOURCE -T "$other_file"
    "$cli" --version
} > "$report_dir/environment.txt"
sha256sum "$cli" "$suite" "$0" > "$report_dir/artifacts.sha256"

for binary in "$cli" "$suite"; do
    description=$(file -b "$binary")
    printf '%s: %s\n' "$binary" "$description" >> "$report_dir/elf.txt"
    if [[ "$description" != *"ARM aarch64"* || "$description" != *"statically linked"* ]]; then
        echo "not a static aarch64 ELF: $binary" >&2
        exit 1
    fi
    readelf -l "$binary" >> "$report_dir/elf.txt"
    readelf -d "$binary" >> "$report_dir/elf.txt"
    readelf --version-info "$binary" >> "$report_dir/elf.txt"
    if readelf -l "$binary" | grep -q INTERP ||
       readelf -d "$binary" | grep -q NEEDED ||
       readelf --version-info "$binary" | grep -q GLIBC_; then
        echo "dynamic loader, library or GLIBC dependency detected: $binary" >&2
        exit 1
    fi
done

export THINWS_LINUX_EXT4_TEST_ROOT="$ext4_root"
export THINWS_LINUX_BTRFS_TEST_ROOT="$btrfs_root"
export THINWS_LINUX_OTHER_TEST_FILE="$other_file"

# If the suite silently ignores the binary override, a library-level suite
# could pass while the release CLI is broken. The sentinel must be observed.
sentinel="$report_dir/sentinel-cli"
printf '#!/bin/sh\necho THINWS_ACCEPTANCE_SENTINEL >&2\nexit 77\n' > "$sentinel"
chmod 700 -- "$sentinel"
if THINWS_LINUX_E2E_BINARY="$sentinel" "$suite" \
    occupied_target_symlink_returns_target_exists_without_following_it \
    --exact --nocapture > "$report_dir/negative-control.log" 2>&1; then
    echo "negative control passed unexpectedly: suite did not use the selected CLI" >&2
    exit 1
fi
if ! grep -q THINWS_ACCEPTANCE_SENTINEL "$report_dir/negative-control.log"; then
    echo "negative control failed before reaching the selected CLI" >&2
    exit 1
fi

THINWS_LINUX_E2E_BINARY="$cli" "$suite" --list > "$report_dir/tests.list"
test_count=$(grep -c ': test$' "$report_dir/tests.list" || true)
if [[ "$test_count" -lt 11 ]]; then
    echo "expected at least 11 Linux E2E cases; found $test_count" >&2
    exit 1
fi

THINWS_LINUX_E2E_BINARY="$cli" "$suite" --test-threads=1 --nocapture 2>&1 |
    tee "$report_dir/tests.log"
if ! grep -Fq "test result: ok. $test_count passed; 0 failed; 0 ignored; 0 measured; 0 filtered out" "$report_dir/tests.log"; then
    echo "E2E suite did not execute every listed case" >&2
    exit 1
fi
result=PASS

#!/bin/sh
set -eu

if [ "$#" -eq 0 ] || [ ! -f /work/Cargo.toml ] || ! mountpoint -q /work/target; then
    echo 'usage: mount the repository read-only at /work, a writable volume at /work/target, and pass a test command' >&2
    exit 2
fi

btrfs_image=$(mktemp /tmp/thinws-btrfs.XXXXXX.img)
ext4_image=$(mktemp /tmp/thinws-ext4.XXXXXX.img)
btrfs_mount=/mnt/thinws-btrfs
ext4_mount=/mnt/thinws-ext4

cleanup() {
    if mountpoint -q "$btrfs_mount/fixtures/bind-child"; then
        umount "$btrfs_mount/fixtures/bind-child"
    fi
    if mountpoint -q "$btrfs_mount"; then
        umount "$btrfs_mount"
    fi
    if mountpoint -q "$ext4_mount"; then
        umount "$ext4_mount"
    fi
}
trap cleanup EXIT

truncate -s 2G "$btrfs_image"
truncate -s 512M "$ext4_image"
mkfs.btrfs -f -q "$btrfs_image" >/dev/null
mkfs.ext4 -F -q "$ext4_image"
mkdir -p "$btrfs_mount" "$ext4_mount"
mount -t btrfs -o loop "$btrfs_image" "$btrfs_mount"
mount -t ext4 -o loop "$ext4_image" "$ext4_mount"

export THINWS_LINUX_BTRFS_TEST_ROOT="$btrfs_mount/fixtures"
export THINWS_LINUX_EXT4_TEST_ROOT="$ext4_mount/fixtures"
export THINWS_LINUX_OTHER_TEST_FILE="$ext4_mount/fixtures/other.txt"
mkdir -p "$THINWS_LINUX_BTRFS_TEST_ROOT" "$THINWS_LINUX_EXT4_TEST_ROOT"
mkdir -p "$THINWS_LINUX_BTRFS_TEST_ROOT/bind-source" "$THINWS_LINUX_BTRFS_TEST_ROOT/bind-child"
mount --bind "$THINWS_LINUX_BTRFS_TEST_ROOT/bind-source" "$THINWS_LINUX_BTRFS_TEST_ROOT/bind-child"
export THINWS_LINUX_BIND_MOUNT_SOURCE="$THINWS_LINUX_BTRFS_TEST_ROOT/bind-source"
export THINWS_LINUX_BIND_MOUNT_CHILD="$THINWS_LINUX_BTRFS_TEST_ROOT/bind-child"
export THINWS_LINUX_SECOND_BTRFS_MOUNT_ROOT="$THINWS_LINUX_BIND_MOUNT_CHILD"
cp -- /work/README.md "$THINWS_LINUX_OTHER_TEST_FILE"
chown -R thinws:thinws "$THINWS_LINUX_BTRFS_TEST_ROOT" "$THINWS_LINUX_EXT4_TEST_ROOT"
mkdir -p /usr/local/cargo/registry
chown thinws:thinws /usr/local/cargo
chown -R thinws:thinws /usr/local/cargo/registry /work/target
if mountpoint -q /mutants-target; then
    chown -R thinws:thinws /mutants-target
fi

runuser -u thinws -- python3 -B /work/tools/linux_btrfs_preflight.py
if [ "$(findmnt -n -o FSTYPE -T "$THINWS_LINUX_EXT4_TEST_ROOT")" != ext4 ]; then
    echo 'expected an ext4 control test root' >&2
    exit 1
fi

cd /work
runuser -u thinws -- "$@"

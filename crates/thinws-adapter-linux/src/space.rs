//! Bounded, descriptor-relative current-space estimates for a Ready Btrfs copy.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::time::{Duration, Instant};

use rustix::fs::{AtFlags, Mode, OFlags, StatxFlags};
use thinws_ports::WorkspaceSpace;

const MAX_SCAN_DEPTH: usize = 256;
const MAX_SCAN_ENTRIES: usize = 200_000;
const MAX_SCAN_DURATION: Duration = Duration::from_secs(5);
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

struct Totals {
    logical: u64,
    allocated: u64,
    entries: usize,
    seen_inodes: HashSet<(u64, u64)>,
    started: Instant,
}

pub(crate) fn measure_root(root: &OwnedFd, expected_mount: u64) -> WorkspaceSpace {
    let Some(stat) = rustix::fs::fstat(root).ok() else {
        return WorkspaceSpace::Unknown;
    };
    if stat.st_mode & libc::S_IFMT != libc::S_IFDIR {
        return WorkspaceSpace::Unknown;
    }
    let mut totals = Totals {
        logical: 0,
        allocated: 0,
        entries: 0,
        seen_inodes: HashSet::new(),
        started: Instant::now(),
    };
    if scan_directory(root, &stat, expected_mount, 0, &mut totals).is_none() {
        return WorkspaceSpace::Unknown;
    }
    WorkspaceSpace::Complete {
        logical_bytes: totals.logical,
        allocated_bytes_estimate: totals.allocated,
    }
}

fn scan_directory(
    directory: &OwnedFd,
    expected: &rustix::fs::Stat,
    expected_mount: u64,
    depth: usize,
    totals: &mut Totals,
) -> Option<()> {
    if limits_exceeded(depth, totals.entries, totals.started.elapsed())
        || mount_id(directory, OsStr::new(""), AtFlags::EMPTY_PATH)? != expected_mount
    {
        return None;
    }
    let before = rustix::fs::fstat(directory).ok()?;
    if !same_identity(&before, expected) || before.st_mode & libc::S_IFMT != libc::S_IFDIR {
        return None;
    }
    add_allocated(expected, totals)?;
    let mut entries = rustix::fs::Dir::read_from(directory).ok()?;
    for entry in &mut entries {
        let entry = entry.ok()?;
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        totals.entries = totals.entries.checked_add(1)?;
        if limits_exceeded(depth, totals.entries, totals.started.elapsed()) {
            return None;
        }
        let before = rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW).ok()?;
        if before.st_dev != expected.st_dev
            || mount_id(
                directory,
                OsStr::from_bytes(name.to_bytes()),
                AtFlags::SYMLINK_NOFOLLOW,
            )? != expected_mount
        {
            return None;
        }
        match before.st_mode & libc::S_IFMT {
            libc::S_IFDIR => {
                let child =
                    rustix::fs::openat(directory, name, DIRECTORY_FLAGS, Mode::empty()).ok()?;
                scan_directory(&child, &before, expected_mount, depth + 1, totals)?;
            }
            libc::S_IFREG | libc::S_IFLNK => {
                totals.logical = totals
                    .logical
                    .checked_add(u64::try_from(before.st_size).ok()?)?;
                add_allocated(&before, totals)?;
            }
            _ => return None,
        }
        let after = rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW).ok()?;
        if !identity_matches_mount(&after, &before, expected_mount, || {
            mount_id(
                directory,
                OsStr::from_bytes(name.to_bytes()),
                AtFlags::SYMLINK_NOFOLLOW,
            )
        }) {
            return None;
        }
    }
    let after = rustix::fs::fstat(directory).ok()?;
    identity_matches_mount(&after, expected, expected_mount, || {
        mount_id(directory, OsStr::new(""), AtFlags::EMPTY_PATH)
    })
    .then_some(())
}

fn mount_id(directory: &OwnedFd, name: &OsStr, flags: AtFlags) -> Option<u64> {
    let observed = rustix::fs::statx(directory, name, flags, StatxFlags::MNT_ID).ok()?;
    reported_mount_id(observed.stx_mask, observed.stx_mnt_id)
}

fn reported_mount_id(mask: u32, id: u64) -> Option<u64> {
    (mask & StatxFlags::MNT_ID.bits() != 0).then_some(id)
}

fn identity_matches_mount(
    current: &rustix::fs::Stat,
    expected: &rustix::fs::Stat,
    expected_mount: u64,
    observed_mount: impl FnOnce() -> Option<u64>,
) -> bool {
    same_identity(current, expected) && observed_mount() == Some(expected_mount)
}

fn limits_exceeded(depth: usize, entries: usize, elapsed: Duration) -> bool {
    depth > MAX_SCAN_DEPTH || entries > MAX_SCAN_ENTRIES || elapsed > MAX_SCAN_DURATION
}

fn same_identity(left: &rustix::fs::Stat, right: &rustix::fs::Stat) -> bool {
    (left.st_dev, left.st_ino, left.st_mode & libc::S_IFMT)
        == (right.st_dev, right.st_ino, right.st_mode & libc::S_IFMT)
}

fn add_allocated(stat: &rustix::fs::Stat, totals: &mut Totals) -> Option<()> {
    let blocks = u64::try_from(stat.st_blocks).ok()?.checked_mul(512)?;
    if totals.seen_inodes.insert((stat.st_dev, stat.st_ino)) {
        totals.allocated = totals.allocated.checked_add(blocks)?;
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::process::Command;

    use tempfile::Builder;

    use super::*;

    #[test]
    fn child_subvolume_on_the_same_mount_makes_space_unknown() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test root");
        let fixture = Builder::new()
            .prefix("thinws-linux-space-subvolume-")
            .tempdir_in(root)
            .unwrap();
        let directory = rustix::fs::open(fixture.path(), DIRECTORY_FLAGS, Mode::empty()).unwrap();
        let expected_mount = mount_id(&directory, OsStr::new(""), AtFlags::EMPTY_PATH).unwrap();
        let subvolume = fixture.path().join("subvolume");
        let created = Command::new("btrfs")
            .args(["subvolume", "create"])
            .arg(&subvolume)
            .output()
            .unwrap();
        assert!(created.status.success(), "{created:?}");
        let root_stat = rustix::fs::fstat(&directory).unwrap();
        let child_stat = rustix::fs::statat(
            &directory,
            OsStr::new("subvolume"),
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .unwrap();
        assert_ne!(child_stat.st_dev, root_stat.st_dev);
        assert_eq!(
            mount_id(
                &directory,
                OsStr::new("subvolume"),
                AtFlags::SYMLINK_NOFOLLOW,
            ),
            Some(expected_mount)
        );
        let result = measure_root(&directory, expected_mount);
        fs::remove_dir(&subvolume).unwrap();
        assert_eq!(result, WorkspaceSpace::Unknown);
    }

    #[test]
    fn root_on_a_different_mount_is_unknown_before_any_scan() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test root");
        let fixture = Builder::new()
            .prefix("thinws-linux-space-mount-")
            .tempdir_in(root)
            .unwrap();
        let directory = rustix::fs::open(fixture.path(), DIRECTORY_FLAGS, Mode::empty()).unwrap();
        let actual_mount = mount_id(&directory, OsStr::new(""), AtFlags::EMPTY_PATH).unwrap();
        let different_mount = actual_mount.wrapping_add(1);
        assert_ne!(different_mount, actual_mount);
        assert_eq!(
            measure_root(&directory, different_mount),
            WorkspaceSpace::Unknown
        );
    }

    #[test]
    fn mismatched_root_identity_is_rejected_without_traversing_entries() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test root");
        let fixture = Builder::new()
            .prefix("thinws-linux-space-identity-")
            .tempdir_in(root)
            .unwrap();
        fs::write(fixture.path().join("content"), b"x").unwrap();
        let directory = rustix::fs::open(fixture.path(), DIRECTORY_FLAGS, Mode::empty()).unwrap();
        let expected_mount = mount_id(&directory, OsStr::new(""), AtFlags::EMPTY_PATH).unwrap();
        let mut incorrect = rustix::fs::fstat(&directory).unwrap();
        incorrect.st_ino = incorrect.st_ino.wrapping_add(1);
        let mut totals = Totals {
            logical: 0,
            allocated: 0,
            entries: 0,
            seen_inodes: HashSet::new(),
            started: Instant::now(),
        };
        assert!(scan_directory(&directory, &incorrect, expected_mount, 0, &mut totals).is_none());
        assert_eq!(totals.entries, 0);
        assert!(totals.seen_inodes.is_empty());
    }

    #[test]
    fn directory_tree_beyond_the_depth_limit_has_no_complete_estimate() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test root");
        let fixture = Builder::new()
            .prefix("thinws-linux-space-depth-")
            .tempdir_in(root)
            .unwrap();
        let directory = rustix::fs::open(fixture.path(), DIRECTORY_FLAGS, Mode::empty()).unwrap();
        let expected_mount = mount_id(&directory, OsStr::new(""), AtFlags::EMPTY_PATH).unwrap();
        let mut child = fixture.path().to_path_buf();
        for _ in 0..=MAX_SCAN_DEPTH {
            child.push("d");
            fs::create_dir(&child).unwrap();
        }
        assert_eq!(
            measure_root(&directory, expected_mount),
            WorkspaceSpace::Unknown
        );
    }

    #[test]
    fn final_identity_and_mount_must_each_match() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test root");
        let fixture = Builder::new()
            .prefix("thinws-linux-space-final-")
            .tempdir_in(root)
            .unwrap();
        let directory = rustix::fs::open(fixture.path(), DIRECTORY_FLAGS, Mode::empty()).unwrap();
        let expected = rustix::fs::fstat(&directory).unwrap();
        let expected_mount = mount_id(&directory, OsStr::new(""), AtFlags::EMPTY_PATH).unwrap();
        let mut changed = rustix::fs::fstat(&directory).unwrap();
        changed.st_ino = changed.st_ino.wrapping_add(1);

        assert!(!identity_matches_mount(
            &changed,
            &expected,
            expected_mount,
            || panic!("mount check must not inspect a changed directory"),
        ));
        assert!(!identity_matches_mount(
            &expected,
            &expected,
            expected_mount,
            || Some(expected_mount.wrapping_add(1)),
        ));
        assert!(!identity_matches_mount(
            &expected,
            &expected,
            expected_mount,
            || None,
        ));
        assert!(identity_matches_mount(
            &expected,
            &expected,
            expected_mount,
            || Some(expected_mount),
        ));
    }

    #[test]
    fn mount_id_requires_the_reported_statx_mask_bit() {
        assert_eq!(reported_mount_id(0, 41), None);
        assert_eq!(reported_mount_id(StatxFlags::SIZE.bits(), 41), None);
        assert_eq!(reported_mount_id(StatxFlags::MNT_ID.bits(), 41), Some(41));
    }

    #[test]
    fn each_limit_has_an_independent_inclusive_boundary() {
        assert!(!limits_exceeded(
            MAX_SCAN_DEPTH,
            MAX_SCAN_ENTRIES,
            MAX_SCAN_DURATION,
        ));
        assert!(limits_exceeded(MAX_SCAN_DEPTH + 1, 0, Duration::ZERO));
        assert!(limits_exceeded(0, MAX_SCAN_ENTRIES + 1, Duration::ZERO));
        assert!(limits_exceeded(
            0,
            0,
            MAX_SCAN_DURATION + Duration::from_nanos(1),
        ));
    }
}

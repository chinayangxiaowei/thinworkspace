//! Read-only, descriptor-relative estimates for a verified Workspace root.

use std::collections::HashSet;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::time::{Duration, Instant};

use thinws_ports::WorkspaceSpace;

use crate::ffi::{
    RawFileKind, RawNodeMetadata, c_string, node_metadata, node_metadata_at, open_directory_at,
    read_directory_bounded,
};

const MAX_SCAN_DEPTH: usize = 256;
const MAX_SCAN_ENTRIES: usize = 200_000;
const MAX_SCAN_DURATION: Duration = Duration::from_secs(5);

struct Totals {
    logical: u64,
    allocated: u64,
    entries: usize,
    seen_inodes: HashSet<(u64, u64)>,
    started: Instant,
}

pub(crate) fn measure_root(root: &OwnedFd) -> WorkspaceSpace {
    let Ok(metadata) = node_metadata(root) else {
        return WorkspaceSpace::Unknown;
    };
    if metadata.kind != RawFileKind::Directory {
        return WorkspaceSpace::Unknown;
    }
    let mut totals = Totals {
        logical: 0,
        allocated: 0,
        entries: 0,
        seen_inodes: HashSet::new(),
        started: Instant::now(),
    };
    if scan_directory(root, metadata, 0, &mut totals).is_err() {
        return WorkspaceSpace::Unknown;
    }
    WorkspaceSpace::Complete {
        logical_bytes: totals.logical,
        allocated_bytes_estimate: totals.allocated,
    }
}

fn scan_directory(
    directory: &OwnedFd,
    expected: RawNodeMetadata,
    depth: usize,
    totals: &mut Totals,
) -> io::Result<()> {
    if scan_limit_exceeded(depth, totals.entries, totals.started.elapsed()) {
        return Err(io::Error::from_raw_os_error(libc::EOVERFLOW));
    }
    let before = node_metadata(directory)?;
    if before.kind != RawFileKind::Directory || !same_identity(before, expected) {
        return Err(io::Error::from_raw_os_error(libc::ESTALE));
    }
    add_allocated(expected, totals)?;
    let names = read_directory_bounded(directory, MAX_SCAN_ENTRIES)?;
    for name in &names {
        totals.entries = totals
            .entries
            .checked_add(1)
            .ok_or_else(|| io::Error::from_raw_os_error(libc::EOVERFLOW))?;
        if scan_limit_exceeded(depth, totals.entries, totals.started.elapsed()) {
            return Err(io::Error::from_raw_os_error(libc::EOVERFLOW));
        }
        let component = c_string(name.as_bytes())?;
        let entry = node_metadata_at(directory, &component)?;
        if entry.device != expected.device {
            return Err(io::Error::from_raw_os_error(libc::EXDEV));
        }
        match entry.kind {
            RawFileKind::Directory => {
                let child = open_directory_at(directory, &component)?;
                scan_directory(&child, entry, depth + 1, totals)?;
            }
            RawFileKind::RegularFile | RawFileKind::SymbolicLink => {
                if entry.size == u64::MAX {
                    return Err(io::Error::from_raw_os_error(libc::EOVERFLOW));
                }
                totals.logical = totals
                    .logical
                    .checked_add(entry.size)
                    .ok_or_else(|| io::Error::from_raw_os_error(libc::EOVERFLOW))?;
                add_allocated(entry, totals)?;
            }
            _ => return Err(io::Error::from_raw_os_error(libc::ENOTSUP)),
        }
        if !same_identity(node_metadata_at(directory, &component)?, entry) {
            return Err(io::Error::from_raw_os_error(libc::ESTALE));
        }
    }
    if !same_identity(node_metadata(directory)?, expected) {
        return Err(io::Error::from_raw_os_error(libc::ESTALE));
    }
    Ok(())
}

fn scan_limit_exceeded(depth: usize, entries: usize, elapsed: Duration) -> bool {
    depth > MAX_SCAN_DEPTH || entries > MAX_SCAN_ENTRIES || elapsed > MAX_SCAN_DURATION
}

fn same_identity(left: RawNodeMetadata, right: RawNodeMetadata) -> bool {
    left.device == right.device && left.inode == right.inode && left.kind == right.kind
}

fn add_allocated(entry: RawNodeMetadata, totals: &mut Totals) -> io::Result<()> {
    if entry.allocated_bytes == u64::MAX {
        return Err(io::Error::from_raw_os_error(libc::EOVERFLOW));
    }
    if totals.seen_inodes.insert((entry.device, entry.inode)) {
        totals.allocated = totals
            .allocated
            .checked_add(entry.allocated_bytes)
            .ok_or_else(|| io::Error::from_raw_os_error(libc::EOVERFLOW))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn totals() -> Totals {
        Totals {
            logical: 0,
            allocated: 0,
            entries: 0,
            seen_inodes: HashSet::new(),
            started: Instant::now(),
        }
    }

    #[test]
    fn each_scan_limit_has_an_independent_inclusive_boundary() {
        assert!(!scan_limit_exceeded(
            MAX_SCAN_DEPTH,
            MAX_SCAN_ENTRIES,
            MAX_SCAN_DURATION
        ));
        assert!(scan_limit_exceeded(MAX_SCAN_DEPTH + 1, 0, Duration::ZERO));
        assert!(scan_limit_exceeded(0, MAX_SCAN_ENTRIES + 1, Duration::ZERO));
        assert!(scan_limit_exceeded(
            0,
            0,
            MAX_SCAN_DURATION + Duration::from_nanos(1)
        ));
    }

    #[test]
    fn nested_content_is_counted_but_the_next_level_is_unknown() {
        let temp = tempfile::tempdir().unwrap();
        let root: OwnedFd = fs::File::open(temp.path()).unwrap().into();
        let mut deepest = temp.path().to_path_buf();
        for _ in 0..MAX_SCAN_DEPTH {
            deepest.push("d");
            fs::create_dir(&deepest).unwrap();
        }
        fs::write(deepest.join("leaf"), b"nested bytes").unwrap();
        assert!(matches!(
            measure_root(&root),
            WorkspaceSpace::Complete {
                logical_bytes: 12,
                ..
            }
        ));
        deepest.push("d");
        fs::create_dir(&deepest).unwrap();
        assert_eq!(measure_root(&root), WorkspaceSpace::Unknown);
    }

    #[test]
    fn directory_identity_mismatch_is_rejected_before_traversal() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("entry"), b"must not be traversed").unwrap();
        let root: OwnedFd = fs::File::open(temp.path()).unwrap().into();
        let actual = node_metadata(&root).unwrap();
        let replaced = RawNodeMetadata {
            inode: actual.inode.wrapping_add(1),
            ..actual
        };
        let mut usage = totals();
        usage.entries = MAX_SCAN_ENTRIES;
        assert_eq!(
            scan_directory(&root, replaced, 0, &mut usage)
                .unwrap_err()
                .raw_os_error(),
            Some(libc::ESTALE)
        );
    }

    #[test]
    fn identity_checks_all_fields_and_allocation_deduplicates_inodes() {
        let node = RawNodeMetadata {
            device: 1,
            inode: 2,
            kind: RawFileKind::RegularFile,
            mode: 0,
            size: 8,
            allocated_bytes: 512,
            modified_seconds: 0,
            modified_nanoseconds: 0,
        };
        assert!(same_identity(node, node));
        assert!(!same_identity(node, RawNodeMetadata { device: 3, ..node }));
        assert!(!same_identity(node, RawNodeMetadata { inode: 3, ..node }));
        assert!(!same_identity(
            node,
            RawNodeMetadata {
                kind: RawFileKind::Directory,
                ..node
            }
        ));
        let mut usage = totals();
        add_allocated(node, &mut usage).unwrap();
        add_allocated(node, &mut usage).unwrap();
        assert_eq!(usage.allocated, 512);
        assert_eq!(usage.seen_inodes.len(), 1);
        assert_eq!(
            add_allocated(
                RawNodeMetadata {
                    allocated_bytes: u64::MAX,
                    ..node
                },
                &mut usage
            )
            .unwrap_err()
            .raw_os_error(),
            Some(libc::EOVERFLOW)
        );
    }
}

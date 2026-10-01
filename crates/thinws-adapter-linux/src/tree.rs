//! Descriptor-relative Linux tree snapshots used by Btrfs materialization.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};

use rustix::fs::{AtFlags, Mode, OFlags, StatxFlags};
use thinws_core::{FileIdentity, MaterializationFailureKind, TreeDigest};
use thinws_ports::{PortError, PortErrorKind};

pub(super) const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);
pub(super) const FILE_READ_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NodeKind {
    Directory,
    File,
    Symlink,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Node {
    pub device: u64,
    pub inode: u64,
    pub kind: NodeKind,
    pub mode: u32,
    pub size: u64,
    pub blocks: u64,
    pub mtime_seconds: i64,
    pub mtime_nanoseconds: i64,
}

impl Node {
    pub const fn identity(self) -> FileIdentity {
        FileIdentity::new(self.device, self.inode)
    }

    pub const fn mtime(self) -> (i64, i64) {
        (self.mtime_seconds, self.mtime_nanoseconds)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SnapshotEntry {
    pub node: Node,
    pub link_text: Option<Vec<u8>>,
    pub content_digest: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TreeSnapshot {
    pub root: Node,
    pub entries: BTreeMap<Vec<u8>, SnapshotEntry>,
    pub manifest: TreeManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TreeManifest {
    root_mode: u32,
    root_mtime: (i64, i64),
    entries: Vec<ManifestEntry>,
    pub regular_files: u64,
    pub logical_bytes: u64,
    pub physical_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManifestEntry {
    path: Vec<u8>,
    kind: NodeKind,
    mode: Option<u32>,
    mtime: Option<(i64, i64)>,
    length: u64,
    digest: Option<[u8; 32]>,
}

impl TreeManifest {
    pub fn matches_promised(&self, source: &Self) -> bool {
        self.root_mode == source.root_mode
            && self.root_mtime == source.root_mtime
            && self.entries == source.entries
            && self.regular_files == source.regular_files
            && self.logical_bytes == source.logical_bytes
    }

    pub fn digest(&self) -> TreeDigest {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"thinws-tree-manifest-v1\0");
        hasher.update(&self.root_mode.to_le_bytes());
        update_time(&mut hasher, self.root_mtime);
        for entry in &self.entries {
            update_bytes(&mut hasher, &entry.path);
            hasher.update(&[match entry.kind {
                NodeKind::Directory => 1,
                NodeKind::File => 2,
                NodeKind::Symlink => 3,
                NodeKind::Unsupported => 255,
            }]);
            match entry.mode {
                Some(mode) => {
                    hasher.update(&[1]);
                    hasher.update(&mode.to_le_bytes());
                }
                None => {
                    hasher.update(&[0]);
                }
            }
            match entry.mtime {
                Some(time) => {
                    hasher.update(&[1]);
                    update_time(&mut hasher, time);
                }
                None => {
                    hasher.update(&[0]);
                }
            }
            hasher.update(&entry.length.to_le_bytes());
            match entry.digest {
                Some(digest) => {
                    hasher.update(&[1]);
                    hasher.update(&digest);
                }
                None => {
                    hasher.update(&[0]);
                }
            }
        }
        TreeDigest::new(*hasher.finalize().as_bytes())
    }
}

pub(super) struct TreeFailure {
    pub kind: MaterializationFailureKind,
    pub port_kind: PortErrorKind,
    pub operation: &'static str,
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl TreeFailure {
    pub const fn new(
        kind: MaterializationFailureKind,
        port_kind: PortErrorKind,
        operation: &'static str,
    ) -> Self {
        Self {
            kind,
            port_kind,
            operation,
            source: None,
        }
    }

    pub fn io(operation: &'static str, error: impl Into<std::io::Error>) -> Self {
        let error = error.into();
        let kind = if error.raw_os_error() == Some(libc::ENOSPC) {
            MaterializationFailureKind::NoSpace
        } else {
            MaterializationFailureKind::Filesystem
        };
        Self {
            kind,
            port_kind: PortErrorKind::Io,
            operation,
            source: Some(Box::new(error)),
        }
    }

    pub fn with_source(
        kind: MaterializationFailureKind,
        port_kind: PortErrorKind,
        operation: &'static str,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind,
            port_kind,
            operation,
            source: Some(Box::new(source)),
        }
    }

    pub fn source_changed() -> Self {
        Self::new(
            MaterializationFailureKind::SourceChanged,
            PortErrorKind::InvalidData,
            "source tree changed during materialization",
        )
    }

    pub fn target_changed() -> Self {
        Self::new(
            MaterializationFailureKind::TargetChanged,
            PortErrorKind::InvalidLayout,
            "target tree changed during materialization",
        )
    }

    pub fn into_port_error(self) -> PortError {
        let error = PortError::new(self.port_kind, self.operation);
        match self.source {
            Some(source) => error.with_boxed_source(source),
            None => error,
        }
    }
}

pub(super) fn node_from_stat(stat: rustix::fs::Stat) -> Node {
    let kind = match stat.st_mode & libc::S_IFMT {
        libc::S_IFDIR => NodeKind::Directory,
        libc::S_IFREG => NodeKind::File,
        libc::S_IFLNK => NodeKind::Symlink,
        _ => NodeKind::Unsupported,
    };
    Node {
        device: stat.st_dev,
        inode: stat.st_ino,
        kind,
        mode: stat.st_mode & 0o7777,
        size: stat.st_size.max(0) as u64,
        blocks: stat.st_blocks.max(0) as u64,
        mtime_seconds: stat.st_mtime,
        mtime_nanoseconds: stat.st_mtime_nsec as i64,
    }
}

pub(super) fn node(fd: impl AsFd) -> Result<Node, TreeFailure> {
    rustix::fs::fstat(fd)
        .map(node_from_stat)
        .map_err(|error| TreeFailure::io("inspect held tree node", error))
}

pub(super) fn node_at(parent: &OwnedFd, name: &OsStr) -> Result<Node, TreeFailure> {
    rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .map(node_from_stat)
        .map_err(|error| TreeFailure::io("inspect named tree node", error))
}

pub(super) fn mount_id(fd: &OwnedFd) -> Result<u64, TreeFailure> {
    let observed = rustix::fs::statx(fd, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)
        .map_err(|error| TreeFailure::io("inspect tree mount ID", error))?;
    mount_id_available(observed.stx_mask)
        .then_some(observed.stx_mnt_id)
        .ok_or_else(|| {
            TreeFailure::new(
                MaterializationFailureKind::InvalidLayout,
                PortErrorKind::InvalidLayout,
                "tree mount ID is unavailable",
            )
        })
}

pub(super) fn mount_id_at(parent: &OwnedFd, name: &OsStr) -> Result<u64, TreeFailure> {
    let observed = rustix::fs::statx(parent, name, AtFlags::SYMLINK_NOFOLLOW, StatxFlags::MNT_ID)
        .map_err(|error| TreeFailure::io("inspect tree entry mount ID", error))?;
    mount_id_available(observed.stx_mask)
        .then_some(observed.stx_mnt_id)
        .ok_or_else(|| {
            TreeFailure::new(
                MaterializationFailureKind::InvalidLayout,
                PortErrorKind::InvalidLayout,
                "tree entry mount ID is unavailable",
            )
        })
}

fn mount_id_available(mask: u32) -> bool {
    mask & StatxFlags::MNT_ID.bits() != 0
}

pub(super) fn directory_names(directory: &OwnedFd) -> Result<Vec<OsString>, TreeFailure> {
    let mut entries = rustix::fs::Dir::read_from(directory)
        .map_err(|error| TreeFailure::io("enumerate tree directory", error))?;
    let mut names = Vec::new();
    for entry in &mut entries {
        let entry = entry.map_err(|error| TreeFailure::io("read tree directory entry", error))?;
        let bytes = entry.file_name().to_bytes();
        if bytes != b"." && bytes != b".." {
            names.push(OsString::from_vec(bytes.to_vec()));
        }
    }
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    Ok(names)
}

pub(super) fn open_directory(parent: &OwnedFd, name: &OsStr) -> Result<OwnedFd, TreeFailure> {
    rustix::fs::openat(parent, name, DIRECTORY_FLAGS, Mode::empty())
        .map_err(|error| TreeFailure::io("open tree directory without following links", error))
}

pub(super) fn open_file(parent: &OwnedFd, name: &OsStr) -> Result<OwnedFd, TreeFailure> {
    rustix::fs::openat(parent, name, FILE_READ_FLAGS, Mode::empty())
        .map_err(|error| TreeFailure::io("open tree file without following links", error))
}

pub(super) fn snapshot(root: &OwnedFd) -> Result<TreeSnapshot, TreeFailure> {
    snapshot_with_hook(root, &mut |_| {})
}

fn snapshot_with_hook(
    root: &OwnedFd,
    before_directory_open: &mut impl FnMut(&[u8]),
) -> Result<TreeSnapshot, TreeFailure> {
    let before = node(root)?;
    if before.kind != NodeKind::Directory {
        return Err(TreeFailure::new(
            MaterializationFailureKind::InvalidLayout,
            PortErrorKind::InvalidLayout,
            "require tree root directory",
        ));
    }
    let root_mount = mount_id(root)?;
    let mut snapshot = TreeSnapshot {
        root: before,
        entries: BTreeMap::new(),
        manifest: TreeManifest {
            root_mode: before.mode,
            root_mtime: before.mtime(),
            entries: Vec::new(),
            regular_files: 0,
            logical_bytes: 0,
            physical_bytes: 0,
        },
    };
    snapshot_directory_with_hook(root, &[], root_mount, &mut snapshot, before_directory_open)?;
    if node(root)? != before {
        return Err(TreeFailure::source_changed());
    }
    Ok(snapshot)
}

fn snapshot_directory_with_hook(
    directory: &OwnedFd,
    prefix: &[u8],
    root_mount: u64,
    snapshot: &mut TreeSnapshot,
    before_directory_open: &mut impl FnMut(&[u8]),
) -> Result<(), TreeFailure> {
    for name in directory_names(directory)? {
        let name_bytes = name.as_bytes();
        let mut path = prefix.to_vec();
        if !path.is_empty() {
            path.push(b'/');
        }
        path.extend_from_slice(name_bytes);
        let before = node_at(directory, &name)?;
        if mount_id_at(directory, &name)? != root_mount {
            return Err(TreeFailure::new(
                MaterializationFailureKind::UnsupportedSourceEntry,
                PortErrorKind::InvalidLayout,
                "reject tree submount",
            ));
        }
        let mut entry = SnapshotEntry {
            node: before,
            link_text: None,
            content_digest: None,
        };
        let (mode, mtime, length, digest) = match before.kind {
            NodeKind::Directory => {
                before_directory_open(&path);
                let child = open_directory(directory, &name)?;
                if node(&child)? != before || mount_id(&child)? != root_mount {
                    return Err(TreeFailure::source_changed());
                }
                snapshot_directory_with_hook(
                    &child,
                    &path,
                    root_mount,
                    snapshot,
                    before_directory_open,
                )?;
                if node(&child)? != before {
                    return Err(TreeFailure::source_changed());
                }
                (Some(before.mode), Some(before.mtime()), 0, None)
            }
            NodeKind::File => {
                let file = open_file(directory, &name)?;
                let (length, digest) = digest_file(file, before)?;
                entry.content_digest = Some(digest);
                snapshot.manifest.regular_files = snapshot.manifest.regular_files.saturating_add(1);
                snapshot.manifest.logical_bytes =
                    snapshot.manifest.logical_bytes.saturating_add(length);
                snapshot.manifest.physical_bytes = snapshot
                    .manifest
                    .physical_bytes
                    .saturating_add(before.blocks.saturating_mul(512));
                (
                    Some(before.mode),
                    Some(before.mtime()),
                    length,
                    Some(digest),
                )
            }
            NodeKind::Symlink => {
                let text = rustix::fs::readlinkat(directory, &name, Vec::new())
                    .map_err(|error| TreeFailure::io("read tree symbolic link", error))?
                    .into_bytes();
                if node_at(directory, &name)? != before {
                    return Err(TreeFailure::source_changed());
                }
                let digest = *blake3::hash(&text).as_bytes();
                entry.link_text = Some(text.clone());
                entry.content_digest = Some(digest);
                (None, None, text.len() as u64, Some(digest))
            }
            NodeKind::Unsupported => {
                return Err(TreeFailure::new(
                    MaterializationFailureKind::UnsupportedSourceEntry,
                    PortErrorKind::InvalidData,
                    "reject unsupported tree entry",
                ));
            }
        };
        snapshot.entries.insert(path.clone(), entry);
        snapshot.manifest.entries.push(ManifestEntry {
            path,
            kind: before.kind,
            mode,
            mtime,
            length,
            digest,
        });
    }
    Ok(())
}

fn digest_file(file: OwnedFd, expected: Node) -> Result<(u64, [u8; 32]), TreeFailure> {
    if node(&file)? != expected {
        return Err(TreeFailure::source_changed());
    }
    let mut file = File::from(file);
    let mut hasher = blake3::Hasher::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| TreeFailure::io("read tree file", error))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        length = length.saturating_add(count as u64);
    }
    if !file_snapshot_matches(node(&file)?, expected, length) {
        return Err(TreeFailure::source_changed());
    }
    Ok((length, *hasher.finalize().as_bytes()))
}

fn file_snapshot_matches(current: Node, expected: Node, length: u64) -> bool {
    current == expected && length == expected.size
}

fn update_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn update_time(hasher: &mut blake3::Hasher, (seconds, nanoseconds): (i64, i64)) {
    hasher.update(&seconds.to_le_bytes());
    hasher.update(&nanoseconds.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::os::unix::net::UnixListener;

    use super::*;

    fn manifest() -> TreeManifest {
        TreeManifest {
            root_mode: 0o755,
            root_mtime: (1_700_000_000, 123_456_789),
            entries: vec![ManifestEntry {
                path: b"file".to_vec(),
                kind: NodeKind::File,
                mode: Some(0o644),
                mtime: Some((1_700_000_001, 234_567_890)),
                length: 3,
                digest: Some(*blake3::hash(b"abc").as_bytes()),
            }],
            regular_files: 1,
            logical_bytes: 3,
            physical_bytes: 4096,
        }
    }

    #[test]
    fn node_mtime_preserves_seconds_and_nanoseconds() {
        let node = Node {
            device: 1,
            inode: 2,
            kind: NodeKind::File,
            mode: 0o644,
            size: 3,
            blocks: 8,
            mtime_seconds: 1_700_000_001,
            mtime_nanoseconds: 234_567_890,
        };
        assert_eq!(node.mtime(), (1_700_000_001, 234_567_890));
    }

    #[test]
    fn promised_manifest_requires_each_guaranteed_field() {
        let source = manifest();
        assert!(source.matches_promised(&source));

        let mut changed = source.clone();
        changed.root_mode ^= 0o100;
        assert!(!changed.matches_promised(&source));

        let mut changed = source.clone();
        changed.root_mtime.1 += 1;
        assert!(!changed.matches_promised(&source));

        let mut changed = source.clone();
        changed.entries[0].path = b"else".to_vec();
        assert!(!changed.matches_promised(&source));

        let mut changed = source.clone();
        changed.regular_files += 1;
        assert!(!changed.matches_promised(&source));

        let mut changed = source.clone();
        changed.logical_bytes += 1;
        assert!(!changed.matches_promised(&source));

        let mut changed = source.clone();
        changed.physical_bytes += 4096;
        assert!(changed.matches_promised(&source));
    }

    #[test]
    fn manifest_digest_binds_path_and_mtime() {
        let source = manifest();
        let mut changed = source.clone();
        changed.entries[0].path = b"else".to_vec();
        assert_ne!(changed.digest(), source.digest());

        let mut changed = source.clone();
        changed.root_mtime.1 += 1;
        assert_ne!(changed.digest(), source.digest());

        let mut changed = source.clone();
        changed.entries[0].mtime = Some((1_700_000_001, 234_567_891));
        assert_ne!(changed.digest(), source.digest());
    }

    #[test]
    fn file_digest_records_actual_bytes_and_length_across_read_chunks() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-tree-digest-")
            .tempdir_in(root)
            .unwrap();
        let bytes = (0..65_553).map(|index| index as u8).collect::<Vec<_>>();
        fs::write(fixture.path().join("file"), &bytes).unwrap();
        let directory = rustix::fs::open(fixture.path(), DIRECTORY_FLAGS, Mode::empty()).unwrap();
        let file = open_file(&directory, OsStr::new("file"))
            .map_err(TreeFailure::into_port_error)
            .unwrap();
        let expected = node(&file).map_err(TreeFailure::into_port_error).unwrap();
        let (length, digest) = digest_file(file, expected)
            .map_err(TreeFailure::into_port_error)
            .unwrap();
        assert_eq!(length, bytes.len() as u64);
        assert_eq!(digest, *blake3::hash(&bytes).as_bytes());
    }

    #[test]
    fn filesystem_error_preserves_no_space_classification() {
        let no_space = TreeFailure::io("test", std::io::Error::from_raw_os_error(libc::ENOSPC));
        assert_eq!(no_space.kind, MaterializationFailureKind::NoSpace);
        let ordinary = TreeFailure::io("test", std::io::Error::from_raw_os_error(libc::EIO));
        assert_eq!(ordinary.kind, MaterializationFailureKind::Filesystem);
    }

    #[test]
    fn completed_file_requires_both_unchanged_identity_and_read_length() {
        let expected = Node {
            device: 1,
            inode: 2,
            kind: NodeKind::File,
            mode: 0o644,
            size: 3,
            blocks: 8,
            mtime_seconds: 1_700_000_001,
            mtime_nanoseconds: 234_567_890,
        };
        assert!(file_snapshot_matches(expected, expected, 3));
        assert!(!file_snapshot_matches(
            Node {
                mtime_nanoseconds: expected.mtime_nanoseconds + 1,
                ..expected
            },
            expected,
            3
        ));
        assert!(!file_snapshot_matches(expected, expected, 2));
    }

    #[test]
    fn mount_id_requires_its_own_statx_mask_bit() {
        let mount_bit = StatxFlags::MNT_ID.bits();
        let other_bit = StatxFlags::SIZE.bits();
        assert_eq!(mount_bit & other_bit, 0);
        assert!(!mount_id_available(0));
        assert!(!mount_id_available(other_bit));
        assert!(mount_id_available(mount_bit));
        assert!(mount_id_available(mount_bit | other_bit));
    }

    #[test]
    fn source_snapshot_refuses_replaced_child_before_scanning_it() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-tree-child-race-")
            .tempdir_in(root)
            .unwrap();
        let child = fixture.path().join("child");
        let displaced = fixture.path().join("displaced");
        fs::create_dir(&child).unwrap();
        let source = rustix::fs::open(fixture.path(), DIRECTORY_FLAGS, Mode::empty()).unwrap();
        let mut socket = None;

        let failure = snapshot_with_hook(&source, &mut |path| {
            if path == b"child" {
                fs::rename(&child, &displaced).unwrap();
                fs::create_dir(&child).unwrap();
                socket = Some(UnixListener::bind(child.join("socket")).unwrap());
            }
        })
        .unwrap_err();
        assert_eq!(failure.kind, MaterializationFailureKind::SourceChanged);
        assert!(socket.is_some());
        assert!(displaced.is_dir());
    }

    #[test]
    #[ignore = "requires THINWS_LINUX_BIND_MOUNT_CHILD on a distinct bind mount of the same Btrfs filesystem"]
    fn source_snapshot_rejects_a_child_on_another_mount() {
        let child = env::var_os("THINWS_LINUX_BIND_MOUNT_CHILD")
            .expect("set THINWS_LINUX_BIND_MOUNT_CHILD to a Btrfs bind-mounted child");
        let source = std::path::Path::new(&child).parent().unwrap();
        let source = rustix::fs::open(source, DIRECTORY_FLAGS, Mode::empty()).unwrap();
        let failure =
            snapshot(&source).expect_err("source snapshot must reject a child on another mount");
        assert_eq!(
            failure.kind,
            MaterializationFailureKind::UnsupportedSourceEntry
        );
    }
}

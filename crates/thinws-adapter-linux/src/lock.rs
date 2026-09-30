use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use rustix::fs::{AtFlags, FlockOperation, Mode, OFlags, StatxFlags};
use thinws_core::{
    AbsolutePath, FileIdentity, HostCapabilityReport, MaterializationPathReport,
    PathCapabilityReport,
};
use thinws_ports::{
    LifecycleLock, LifecycleLockGuard, LifecycleScope, MaterializationPathProbeRequest,
    PlatformProbe, PortError, PortErrorKind,
};

use crate::LinuxPlatformProbe;

const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);
const LOCK_FLAGS: OFlags = OFlags::CREATE
    .union(OFlags::RDWR)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

/// Linux adapter bound to one caller-provided control directory.
#[derive(Clone)]
pub struct LinuxHostAdapter {
    control_root: PathBuf,
}

impl LinuxHostAdapter {
    /// Validates the canonical control-root path without creating it.
    pub fn new(control_root: impl Into<PathBuf>) -> Result<Self, PortError> {
        let control_root = control_root.into();
        AbsolutePath::try_from_bytes(control_root.as_os_str().as_bytes().to_vec()).map_err(
            |error| {
                PortError::new(PortErrorKind::InvalidData, "validate control root")
                    .with_source(error)
            },
        )?;
        Ok(Self { control_root })
    }

    /// Returns the configured control directory path.
    #[must_use]
    pub fn control_root(&self) -> &Path {
        &self.control_root
    }

    /// Creates or validates the fixed private bootstrap directory.
    pub fn prepare_bootstrap(&self) -> Result<(), PortError> {
        prepare_private_directory(&self.control_root).map(|_| ())
    }
}

impl PlatformProbe for LinuxHostAdapter {
    fn inspect_host(&self) -> Result<HostCapabilityReport, PortError> {
        LinuxPlatformProbe.inspect_host()
    }

    fn inspect_path(&self, path: &AbsolutePath) -> Result<PathCapabilityReport, PortError> {
        LinuxPlatformProbe.inspect_path(path)
    }

    fn inspect_materialization_paths(
        &self,
        request: &MaterializationPathProbeRequest,
    ) -> Result<MaterializationPathReport, PortError> {
        LinuxPlatformProbe.inspect_materialization_paths(request)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Identity {
    device: u64,
    inode: u64,
    birth_seconds: i64,
    birth_nanoseconds: u32,
}

#[derive(Debug)]
pub(crate) struct PrivateDirectory {
    pub(crate) fd: OwnedFd,
    path: PathBuf,
    identity: Identity,
}

impl PrivateDirectory {
    pub(crate) const fn file_identity(&self) -> FileIdentity {
        FileIdentity::new(self.identity.device, self.identity.inode)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn same_identity(&self, other: &Self) -> bool {
        self.identity == other.identity
    }

    pub(crate) fn relabel(&mut self, path: PathBuf) {
        self.path = path;
    }
}

/// Held Linux advisory lock; dropping it releases the kernel lock.
#[derive(Debug)]
pub struct LinuxLockGuard {
    scope: LifecycleScope,
    parent: PrivateDirectory,
    file: File,
    identity: (u64, u64),
}

impl LifecycleLockGuard for LinuxLockGuard {
    fn scope(&self) -> LifecycleScope {
        self.scope
    }

    fn revalidate(&self) -> Result<(), PortError> {
        revalidate_private_directory(&self.parent)?;
        validate_lock_file(&self.parent.fd, &self.file, self.identity)
    }
}

impl LinuxLockGuard {
    pub(crate) fn protects_directory(&self, directory: &PrivateDirectory) -> bool {
        self.parent.path == directory.path && self.parent.identity == directory.identity
    }
}

impl LifecycleLock for LinuxHostAdapter {
    type Guard = LinuxLockGuard;

    fn acquire_bootstrap(&self, timeout: Duration) -> Result<Self::Guard, PortError> {
        acquire(self, LifecycleScope::Bootstrap, timeout)
    }

    fn acquire_data_root(
        &self,
        data_root: &AbsolutePath,
        timeout: Duration,
    ) -> Result<Self::Guard, PortError> {
        if self.control_root.as_os_str().as_bytes() != data_root.as_bytes() {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "lock fixed control root",
            ));
        }
        acquire(self, LifecycleScope::DataRoot, timeout)
    }
}

fn acquire(
    adapter: &LinuxHostAdapter,
    scope: LifecycleScope,
    timeout: Duration,
) -> Result<LinuxLockGuard, PortError> {
    let parent = open_private_directory(&adapter.control_root)?;
    let fd = rustix::fs::openat(
        &parent.fd,
        "lifecycle.lock",
        LOCK_FLAGS,
        Mode::from_bits_retain(0o600),
    )
    .map_err(|error| io_error("open lifecycle lock", error))?;
    let mut file = File::from(fd);
    let identity = validate_lock_file_initial(&parent.fd, &file)?;
    let started = Instant::now();
    loop {
        match rustix::fs::flock(file.as_fd(), FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => break,
            Err(rustix::io::Errno::AGAIN) => {
                let elapsed = started.elapsed();
                if elapsed >= timeout {
                    return Err(PortError::new(
                        PortErrorKind::Timeout,
                        "acquire lifecycle lock",
                    ));
                }
                thread::sleep(
                    timeout
                        .saturating_sub(elapsed)
                        .min(Duration::from_millis(5)),
                );
            }
            Err(error) => return Err(io_error("acquire lifecycle lock", error)),
        }
    }
    revalidate_private_directory(&parent)?;
    validate_lock_file(&parent.fd, &file, identity)?;
    file.set_len(0)
        .map_err(|error| io_error("truncate lifecycle lock diagnostic", error))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| io_error("seek lifecycle lock diagnostic", error))?;
    writeln!(file, "{}", std::process::id())
        .map_err(|error| io_error("write lifecycle lock diagnostic", error))?;
    file.sync_data()
        .map_err(|error| io_error("sync lifecycle lock diagnostic", error))?;
    Ok(LinuxLockGuard {
        scope,
        parent,
        file,
        identity,
    })
}

pub(crate) fn open_private_directory(path: &Path) -> Result<PrivateDirectory, PortError> {
    open_private_directory_optional(path)?
        .ok_or_else(|| PortError::new(PortErrorKind::NotFound, "open private directory"))
}

pub(crate) fn open_private_directory_optional(
    path: &Path,
) -> Result<Option<PrivateDirectory>, PortError> {
    let absolute = AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec())
        .map_err(|error| io_error("validate private directory path", error))?;
    let mut fd = rustix::fs::open("/", DIRECTORY_FLAGS, Mode::empty())
        .map_err(|error| io_error("open private directory root", error))?;
    for component in absolute.as_bytes()[1..].split(|byte| *byte == b'/') {
        if component.is_empty() {
            continue;
        }
        fd = match rustix::fs::openat(
            &fd,
            std::ffi::OsStr::from_bytes(component),
            DIRECTORY_FLAGS,
            Mode::empty(),
        ) {
            Ok(next) => next,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(error) => return Err(secure_directory_open_error(error)),
        };
    }
    let stat =
        rustix::fs::fstat(&fd).map_err(|error| io_error("inspect private directory", error))?;
    if stat.st_uid != rustix::process::geteuid().as_raw() || stat.st_mode & 0o7777 != 0o700 {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "private directory owner or mode is invalid",
        ));
    }
    let identity = identity(&fd, stat.st_dev, stat.st_ino)?;
    Ok(Some(PrivateDirectory {
        fd,
        path: path.to_path_buf(),
        identity,
    }))
}

pub(crate) fn prepare_private_directory(path: &Path) -> Result<PrivateDirectory, PortError> {
    let absolute =
        AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).map_err(|error| {
            PortError::new(
                PortErrorKind::InvalidData,
                "validate private directory path",
            )
            .with_source(error)
        })?;
    if absolute.as_bytes() == b"/" {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "filesystem root is not a private control directory",
        ));
    }
    let mut parent = rustix::fs::open("/", DIRECTORY_FLAGS, Mode::empty())
        .map_err(|error| io_error("open private directory root", error))?;
    for component in absolute.as_bytes()[1..].split(|byte| *byte == b'/') {
        let name = std::ffi::OsStr::from_bytes(component);
        let child = match rustix::fs::openat(&parent, name, DIRECTORY_FLAGS, Mode::empty()) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => {
                match rustix::fs::mkdirat(&parent, name, Mode::from_bits_retain(0o700)) {
                    Ok(()) => {
                        rustix::fs::fsync(&parent)
                            .map_err(|error| io_error("sync created directory parent", error))?;
                    }
                    // Another initializer may have won the first-creation race.
                    Err(rustix::io::Errno::EXIST) => {}
                    Err(error) => return Err(io_error("create private directory", error)),
                }
                rustix::fs::openat(&parent, name, DIRECTORY_FLAGS, Mode::empty())
                    .map_err(secure_directory_open_error)?
            }
            Err(error) => return Err(secure_directory_open_error(error)),
        };
        parent = child;
    }
    // Reopening the full no-follow path must still name the directory we held
    // while traversing; a concurrent parent rename cannot authorize a peer.
    let held_stat = rustix::fs::fstat(&parent)
        .map_err(|error| io_error("inspect prepared private directory", error))?;
    let held_identity = identity(&parent, held_stat.st_dev, held_stat.st_ino)?;
    let current = open_private_directory(path)?;
    if current.identity != held_identity {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "prepared private directory path changed",
        ));
    }
    Ok(current)
}

fn secure_directory_open_error(error: rustix::io::Errno) -> PortError {
    match error {
        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => PortError::new(
            PortErrorKind::InvalidLayout,
            "private directory contains a link or non-directory",
        ),
        _ => io_error("open private directory component", error),
    }
}

pub(crate) fn revalidate_private_directory(directory: &PrivateDirectory) -> Result<(), PortError> {
    let current = open_private_directory(&directory.path)?;
    if current.identity != directory.identity {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "private directory identity changed",
        ));
    }
    Ok(())
}

fn identity(fd: &OwnedFd, device: u64, inode: u64) -> Result<Identity, PortError> {
    let statx = rustix::fs::statx(fd, "", AtFlags::EMPTY_PATH, StatxFlags::BTIME)
        .map_err(|error| io_error("inspect directory birthtime", error))?;
    let (birth_seconds, birth_nanoseconds) = checked_birthtime(
        statx.stx_mask,
        statx.stx_btime.tv_sec,
        statx.stx_btime.tv_nsec,
    )?;
    Ok(Identity {
        device,
        inode,
        birth_seconds,
        birth_nanoseconds,
    })
}

fn checked_birthtime(mask: u32, seconds: i64, nanoseconds: u32) -> Result<(i64, u32), PortError> {
    if mask & StatxFlags::BTIME.bits() == 0 || seconds <= 0 || nanoseconds >= 1_000_000_000 {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "directory birthtime is unavailable",
        ));
    }
    Ok((seconds, nanoseconds))
}

fn validate_lock_file_initial(parent: &OwnedFd, file: &File) -> Result<(u64, u64), PortError> {
    let stat =
        rustix::fs::fstat(file).map_err(|error| io_error("inspect lifecycle lock", error))?;
    let identity = (stat.st_dev, stat.st_ino);
    validate_lock_file(parent, file, identity)?;
    Ok(identity)
}

fn validate_lock_file(
    parent: &OwnedFd,
    file: &File,
    identity: (u64, u64),
) -> Result<(), PortError> {
    let held =
        rustix::fs::fstat(file).map_err(|error| io_error("inspect held lifecycle lock", error))?;
    let named = rustix::fs::statat(parent, "lifecycle.lock", AtFlags::SYMLINK_NOFOLLOW).map_err(
        |error| match error {
            rustix::io::Errno::NOENT => PortError::new(
                PortErrorKind::InvalidLayout,
                "named lifecycle lock is missing",
            ),
            _ => io_error("inspect named lifecycle lock", error),
        },
    )?;
    if held.st_mode & libc::S_IFMT != libc::S_IFREG
        || held.st_mode & 0o777 != 0o600
        || held.st_uid != rustix::process::geteuid().as_raw()
        || held.st_nlink != 1
        || (held.st_dev, held.st_ino) != identity
        || (named.st_dev, named.st_ino) != identity
        || named.st_mode & libc::S_IFMT != libc::S_IFREG
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "lifecycle lock identity or mode is invalid",
        ));
    }
    Ok(())
}

fn io_error(
    operation: &'static str,
    error: impl std::error::Error + Send + Sync + 'static,
) -> PortError {
    PortError::new(PortErrorKind::Io, operation).with_source(error)
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::time::Duration;

    use super::*;

    #[test]
    fn prepares_a_private_control_root_without_following_links() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs test mount");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-linux-control-")
            .tempdir_in(root)
            .unwrap();
        let control = fixture.path().join("nested/control");
        let prepared = prepare_private_directory(&control).unwrap();
        assert_eq!(
            std::fs::metadata(&control).unwrap().permissions().mode() & 0o777,
            0o700
        );
        revalidate_private_directory(&prepared).unwrap();
        assert_eq!(
            prepare_private_directory(&control).unwrap().identity,
            prepared.identity
        );

        let alias = fixture.path().join("alias");
        symlink(fixture.path().join("nested"), &alias).unwrap();
        assert_eq!(
            prepare_private_directory(&alias.join("control"))
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            prepare_private_directory(&control).unwrap_err().kind(),
            PortErrorKind::InvalidLayout
        );
    }

    #[test]
    fn lock_scope_requires_both_the_recorded_path_and_directory_identity() {
        let root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
            .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-linux-lock-scope-")
            .tempdir_in(root)
            .unwrap();
        let control = fixture.path().join("control");
        let prepared = prepare_private_directory(&control).unwrap();
        let reopened = open_private_directory(&control).unwrap();
        assert!(prepared.same_identity(&reopened));

        let other = prepare_private_directory(&fixture.path().join("other")).unwrap();
        assert!(!prepared.same_identity(&other));

        let adapter = LinuxHostAdapter::new(&control).unwrap();
        let guard = adapter
            .acquire_bootstrap(Duration::from_millis(100))
            .unwrap();
        assert!(guard.protects_directory(&reopened));

        let mut path_changed = reopened;
        path_changed.relabel(fixture.path().join("alias"));
        assert!(!guard.protects_directory(&path_changed));

        let mut identity_changed = other;
        identity_changed.relabel(control);
        assert!(!guard.protects_directory(&identity_changed));
    }

    #[test]
    fn held_lock_rejects_mode_and_link_count_changes_independently() {
        let root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
            .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-linux-lock-leaf-mode-")
            .tempdir_in(root)
            .unwrap();
        let control = fixture.path().join("control");
        prepare_private_directory(&control).unwrap();
        let adapter = LinuxHostAdapter::new(&control).unwrap();
        let guard = adapter
            .acquire_bootstrap(Duration::from_millis(100))
            .unwrap();
        let leaf = control.join("lifecycle.lock");

        std::fs::set_permissions(&leaf, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            guard.revalidate().unwrap_err().kind(),
            PortErrorKind::InvalidLayout
        );
        std::fs::set_permissions(&leaf, std::fs::Permissions::from_mode(0o600)).unwrap();
        guard.revalidate().unwrap();

        let extra_link = fixture.path().join("extra-link");
        std::fs::hard_link(&leaf, &extra_link).unwrap();
        assert_eq!(
            guard.revalidate().unwrap_err().kind(),
            PortErrorKind::InvalidLayout
        );
        std::fs::remove_file(&extra_link).unwrap();
        guard.revalidate().unwrap();
    }

    #[test]
    fn birthtime_evidence_requires_each_independent_field() {
        let mask = StatxFlags::BTIME.bits();
        assert_eq!(checked_birthtime(mask, 123, 456).unwrap(), (123, 456));
        for (observed_mask, seconds, nanoseconds) in [
            (0, 123, 456),
            (mask, 0, 456),
            (mask, -1, 456),
            (mask, 123, 1_000_000_000),
        ] {
            assert_eq!(
                checked_birthtime(observed_mask, seconds, nanoseconds)
                    .unwrap_err()
                    .kind(),
                PortErrorKind::InvalidLayout
            );
        }
    }
}

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use rustix::fs::{AtFlags, FlockOperation, Mode, OFlags, StatxFlags};
use thinws_core::{
    AbsolutePath, HostCapabilityReport, MaterializationPathReport, PathCapabilityReport,
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
    let absolute = AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec())
        .map_err(|error| io_error("validate private directory path", error))?;
    let mut fd = rustix::fs::open("/", DIRECTORY_FLAGS, Mode::empty())
        .map_err(|error| io_error("open private directory root", error))?;
    for component in absolute.as_bytes()[1..].split(|byte| *byte == b'/') {
        if component.is_empty() {
            continue;
        }
        fd = rustix::fs::openat(
            &fd,
            std::ffi::OsStr::from_bytes(component),
            DIRECTORY_FLAGS,
            Mode::empty(),
        )
        .map_err(|error| match error {
            rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => PortError::new(
                PortErrorKind::InvalidLayout,
                "private directory contains a link or non-directory",
            ),
            _ => io_error("open private directory component", error),
        })?;
    }
    let stat =
        rustix::fs::fstat(&fd).map_err(|error| io_error("inspect private directory", error))?;
    if stat.st_uid != rustix::process::geteuid().as_raw() || stat.st_mode & 0o077 != 0 {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "private directory owner or mode is invalid",
        ));
    }
    let identity = identity(&fd, stat.st_dev, stat.st_ino)?;
    Ok(PrivateDirectory {
        fd,
        path: path.to_path_buf(),
        identity,
    })
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
    if statx.stx_mask & StatxFlags::BTIME.bits() == 0
        || statx.stx_btime.tv_sec <= 0
        || statx.stx_btime.tv_nsec >= 1_000_000_000
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "directory birthtime is unavailable",
        ));
    }
    Ok(Identity {
        device,
        inode,
        birth_seconds: statx.stx_btime.tv_sec,
        birth_nanoseconds: statx.stx_btime.tv_nsec,
    })
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

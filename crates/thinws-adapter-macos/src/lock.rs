use std::ffi::OsString;
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use rustix::fs::{FlockOperation, Mode, OFlags};
use thinws_core::AbsolutePath;
use thinws_ports::{LifecycleLock, LifecycleLockGuard, LifecycleScope, PortError, PortErrorKind};

use crate::MacOsHostAdapter;
use crate::filesystem::{
    FileIdentity, ValidatedDirectory, io_error, open_private_directory, path_from_absolute,
    revalidate_directory, validate_file_entry, validate_private_file,
};

const LOCK_OPEN_FLAGS: OFlags = OFlags::CREATE
    .union(OFlags::RDWR)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK);

/// Held macOS advisory lock; dropping it closes the locked file descriptor.
pub struct MacOsLockGuard {
    scope: LifecycleScope,
    parent: ValidatedDirectory,
    name: OsString,
    file: File,
    identity: FileIdentity,
    token: Arc<()>,
}

impl MacOsLockGuard {
    pub(crate) fn belongs_to(&self, adapter: &MacOsHostAdapter) -> bool {
        Arc::ptr_eq(&self.token, &adapter.token)
    }

    pub(crate) fn protects_directory(&self, directory: &ValidatedDirectory) -> bool {
        self.parent.path == directory.path && self.parent.identity == directory.identity
    }
}

impl LifecycleLockGuard for MacOsLockGuard {
    fn scope(&self) -> LifecycleScope {
        self.scope
    }

    fn revalidate(&self) -> Result<(), PortError> {
        revalidate_directory(&self.parent)?;
        validate_file_entry(&self.parent.fd, &self.name, &self.file, self.identity)
    }
}

impl LifecycleLock for MacOsHostAdapter {
    type Guard = MacOsLockGuard;

    fn acquire_bootstrap(&self, timeout: Duration) -> Result<Self::Guard, PortError> {
        acquire(
            self,
            self.bootstrap_dir.clone(),
            OsString::from("lifecycle.lock"),
            LifecycleScope::Bootstrap,
            timeout,
        )
    }

    fn acquire_data_root(
        &self,
        data_root: &AbsolutePath,
        timeout: Duration,
    ) -> Result<Self::Guard, PortError> {
        if path_from_absolute(data_root) != self.bootstrap_dir {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "lock fixed control root",
            ));
        }
        acquire(
            self,
            path_from_absolute(data_root),
            OsString::from("lifecycle.lock"),
            LifecycleScope::DataRoot,
            timeout,
        )
    }
}

fn acquire(
    adapter: &MacOsHostAdapter,
    parent_path: std::path::PathBuf,
    name: OsString,
    scope: LifecycleScope,
    timeout: Duration,
) -> Result<MacOsLockGuard, PortError> {
    let parent = open_private_directory(&parent_path)?;
    let started = Instant::now();
    let fd = loop {
        match rustix::fs::openat(
            &parent.fd,
            &name,
            LOCK_OPEN_FLAGS,
            Mode::from_bits_retain(0o600),
        ) {
            Ok(fd) => break fd,
            // Concurrent first-use create-if-missing can transiently report
            // ENOENT on APFS. Retry only while the held parent still names
            // the validated directory; never reinterpret another errno.
            Err(error @ rustix::io::Errno::NOENT) => {
                if retry_open_before_deadline(started.elapsed(), timeout) {
                    revalidate_directory(&parent)?;
                    thread::sleep(retry_delay(timeout, started.elapsed()));
                    continue;
                }
                return Err(io_error("open lifecycle lock", error));
            }
            Err(error) => return Err(io_error("open lifecycle lock", error)),
        }
    };
    let mut file = File::from(fd);
    let identity = validate_private_file(&file, "validate lifecycle lock")?;
    loop {
        match classify_lock_attempt(rustix::fs::flock(
            file.as_fd(),
            FlockOperation::NonBlockingLockExclusive,
        )) {
            Ok(LockAttempt::Acquired) => break,
            Ok(LockAttempt::Contended) => {
                let elapsed = started.elapsed();
                if elapsed >= timeout {
                    return Err(PortError::new(
                        PortErrorKind::Timeout,
                        "acquire lifecycle lock",
                    ));
                }
                thread::sleep(retry_delay(timeout, elapsed));
            }
            Err(error) => return Err(io_error("acquire lifecycle lock", error)),
        }
    }

    // The directory entry is checked after flock and before PID diagnostics so
    // a replaced or multiply-linked inode never authorizes target mutations.
    revalidate_directory(&parent)?;
    validate_file_entry(&parent.fd, &name, &file, identity)?;
    file.set_len(0)
        .map_err(|error| io_error("truncate lifecycle lock diagnostic", error))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| io_error("seek lifecycle lock diagnostic", error))?;
    writeln!(file, "{}", std::process::id())
        .map_err(|error| io_error("write lifecycle lock diagnostic", error))?;
    file.sync_data()
        .map_err(|error| io_error("sync lifecycle lock diagnostic", error))?;

    Ok(MacOsLockGuard {
        scope,
        parent,
        name,
        file,
        identity,
        token: Arc::clone(&adapter.token),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LockAttempt {
    Acquired,
    Contended,
}

fn classify_lock_attempt(
    result: Result<(), rustix::io::Errno>,
) -> Result<LockAttempt, rustix::io::Errno> {
    match result {
        Ok(()) => Ok(LockAttempt::Acquired),
        // On macOS EAGAIN and EWOULDBLOCK are the same errno value.
        Err(rustix::io::Errno::AGAIN) => Ok(LockAttempt::Contended),
        Err(error) => Err(error),
    }
}

fn retry_delay(timeout: Duration, elapsed: Duration) -> Duration {
    timeout
        .saturating_sub(elapsed)
        .min(Duration::from_millis(5))
}

fn retry_open_before_deadline(elapsed: Duration, timeout: Duration) -> bool {
    elapsed < timeout
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use tempfile::Builder;

    use super::*;

    #[test]
    fn contention_classification_and_retry_delay_are_exact() {
        assert_eq!(classify_lock_attempt(Ok(())), Ok(LockAttempt::Acquired));
        assert_eq!(
            classify_lock_attempt(Err(rustix::io::Errno::AGAIN)),
            Ok(LockAttempt::Contended)
        );
        assert_eq!(
            classify_lock_attempt(Err(rustix::io::Errno::INVAL)),
            Err(rustix::io::Errno::INVAL)
        );
        assert_eq!(
            retry_delay(Duration::from_millis(10), Duration::from_millis(2)),
            Duration::from_millis(5)
        );
        assert_eq!(
            retry_delay(Duration::from_millis(10), Duration::from_millis(8)),
            Duration::from_millis(2)
        );
    }

    #[test]
    fn first_lock_open_retry_stops_at_the_exact_deadline() {
        assert!(retry_open_before_deadline(
            Duration::from_millis(9),
            Duration::from_millis(10)
        ));
        assert!(!retry_open_before_deadline(
            Duration::from_millis(10),
            Duration::from_millis(10)
        ));
        assert!(!retry_open_before_deadline(
            Duration::from_millis(11),
            Duration::from_millis(10)
        ));
        assert!(!retry_open_before_deadline(Duration::ZERO, Duration::ZERO));
    }

    #[test]
    fn lock_flags_and_parent_identity_are_both_enforced() {
        assert_eq!(
            LOCK_OPEN_FLAGS,
            OFlags::CREATE
                .union(OFlags::RDWR)
                .union(OFlags::CLOEXEC)
                .union(OFlags::NOFOLLOW)
                .union(OFlags::NONBLOCK)
        );

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-02-macos-tests");
        fs::create_dir_all(&root).unwrap();
        let temp = Builder::new()
            .prefix("lock-parent-")
            .tempdir_in(fs::canonicalize(root).unwrap())
            .unwrap();
        let bootstrap = temp.path().join("bootstrap");
        fs::create_dir(&bootstrap).unwrap();
        fs::set_permissions(&bootstrap, fs::Permissions::from_mode(0o700)).unwrap();
        let adapter = MacOsHostAdapter::new(&bootstrap).unwrap();
        assert_eq!(adapter.bootstrap_dir(), bootstrap.as_path());
        let guard = adapter
            .acquire_bootstrap(Duration::from_millis(100))
            .unwrap();

        let displaced = temp.path().join("displaced-bootstrap");
        fs::rename(&bootstrap, &displaced).unwrap();
        fs::create_dir(&bootstrap).unwrap();
        fs::set_permissions(&bootstrap, fs::Permissions::from_mode(0o700)).unwrap();
        let replacement = open_private_directory(&bootstrap).unwrap();
        assert!(!guard.protects_directory(&replacement));
    }
}

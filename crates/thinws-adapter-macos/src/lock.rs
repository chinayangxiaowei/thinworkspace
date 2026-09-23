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
            OsString::from("init.lock"),
            LifecycleScope::Bootstrap,
            timeout,
        )
    }

    fn acquire_data_root(
        &self,
        data_root: &AbsolutePath,
        timeout: Duration,
    ) -> Result<Self::Guard, PortError> {
        acquire(
            self,
            path_from_absolute(data_root).join("metadata"),
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
    let fd = rustix::fs::openat(
        &parent.fd,
        &name,
        OFlags::CREATE | OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::from_bits_retain(0o600),
    )
    .map_err(|error| io_error("open lifecycle lock", error))?;
    let mut file = File::from(fd);
    let identity = validate_private_file(&file, "validate lifecycle lock")?;
    let started = Instant::now();
    loop {
        match rustix::fs::flock(file.as_fd(), FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => break,
            Err(error)
                if error == rustix::io::Errno::AGAIN || error == rustix::io::Errno::WOULDBLOCK =>
            {
                let elapsed = started.elapsed();
                if elapsed >= timeout {
                    return Err(PortError::new(
                        PortErrorKind::Timeout,
                        "acquire lifecycle lock",
                    ));
                }
                thread::sleep((timeout - elapsed).min(Duration::from_millis(5)));
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

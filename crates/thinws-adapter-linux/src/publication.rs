use std::fs::File;
use std::io::Write;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;

use rustix::fs::{AtFlags, Mode, OFlags, RenameFlags};
use thinws_core::{InstallationIdentity, RootMarker, RootMarkerState};
use thinws_ports::{
    DataRootLayoutEvidence, LifecycleLockGuard, LifecycleScope, PortConflict, PortError,
    PortErrorKind, PreparedDataRootEvidence, PublishResult,
};

use crate::control::{document_error, read_private_document, require_unclaimed_control_root};
use crate::document::{decode_marker, encode_config, encode_marker};
use crate::lock::{PrivateDirectory, open_private_directory, revalidate_private_directory};
use crate::{LinuxDataRootLayout, LinuxHostAdapter, LinuxLockGuard, LinuxPreparedDataRoot};

/// Same-process proof of one newly published initializing marker.
pub struct LinuxInitializingProof {
    directory: PrivateDirectory,
    marker_file: File,
    marker_identity: (u64, u64),
    marker: RootMarker,
}

impl LinuxInitializingProof {
    fn revalidate(&self) -> Result<(), PortError> {
        revalidate_private_directory(&self.directory)?;
        validate_private_file(
            &self.directory,
            &self.marker_file,
            ".thinws-control.toml",
            self.marker_identity,
        )?;
        let bytes =
            read_private_document(&self.directory, ".thinws-control.toml")?.ok_or_else(|| {
                PortError::new(
                    PortErrorKind::InvalidLayout,
                    "initializing marker is missing",
                )
            })?;
        if decode_marker(&bytes).map_err(document_error)? != self.marker {
            return Err(PortError::conflict(
                "initializing marker content changed",
                PortConflict::InstallationIdentity,
            ));
        }
        Ok(())
    }
}

impl LinuxHostAdapter {
    /// Publishes an initializing marker only for the held, unclaimed root.
    pub fn create_initializing(
        &self,
        lock: &LinuxLockGuard,
        prepared: LinuxPreparedDataRoot,
        identity: &InstallationIdentity,
    ) -> Result<LinuxInitializingProof, PortError> {
        self.validate_bootstrap_lock(lock, &prepared.directory)?;
        prepared.revalidate()?;
        if prepared.data_root() != identity.data_root()
            || prepared.volume_id() != identity.volume_id()
        {
            return Err(PortError::conflict(
                "validate prepared Linux control root",
                PortConflict::InstallationIdentity,
            ));
        }
        let directory = prepared.directory;
        require_unclaimed_control_root(&directory)?;
        let marker = RootMarker::new(identity.clone(), RootMarkerState::Initializing);
        let bytes = encode_marker(&marker).map_err(document_error)?;
        let temporary = PrivateTemp::create(&directory, "root-marker", &bytes)?;
        let (marker_file, marker_identity) = temporary.publish_noreplace(".thinws-control.toml")?;
        let proof = LinuxInitializingProof {
            directory,
            marker_file,
            marker_identity,
            marker,
        };
        proof.revalidate()?;
        Ok(proof)
    }

    /// Creates the private metadata/log layout using the held marker proof.
    pub fn initialize_layout(
        &self,
        lock: &LinuxLockGuard,
        proof: &LinuxInitializingProof,
    ) -> Result<LinuxDataRootLayout, PortError> {
        self.validate_bootstrap_lock(lock, &proof.directory)?;
        proof.revalidate()?;
        let metadata = create_private_child(&proof.directory, "metadata")?;
        let logs = create_private_child(&proof.directory, "logs")?;
        let database = create_database_file(&metadata)?;
        let layout = self.validate_layout(proof.marker.identity())?;
        if !layout.matches_newly_created(&metadata, &logs, &database)? {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "new Linux control layout identity changed",
            ));
        }
        layout.revalidate()?;
        proof.revalidate()?;
        Ok(layout)
    }

    /// Advances the exact held initializing marker to Ready.
    pub fn publish_ready(
        &self,
        lock: &LinuxLockGuard,
        proof: LinuxInitializingProof,
    ) -> Result<RootMarker, PortError> {
        self.validate_bootstrap_lock(lock, &proof.directory)?;
        proof.revalidate()?;
        if proof.marker.state() != RootMarkerState::Initializing {
            return Err(PortError::conflict(
                "publish non-initializing Linux marker",
                PortConflict::InstallationIdentity,
            ));
        }
        let ready = RootMarker::new(proof.marker.identity().clone(), RootMarkerState::Ready);
        let bytes = encode_marker(&ready).map_err(document_error)?;
        let mut temporary = PrivateTemp::create(&proof.directory, "root-marker-ready", &bytes)?;
        temporary.exchange_with(".thinws-control.toml")?;

        // The atomic exchange must have moved precisely the two held files.
        // If validation fails, leave the incomplete installation for explicit
        // diagnosis; never guess which marker is safe to delete.
        validate_private_file(
            &proof.directory,
            &proof.marker_file,
            &temporary.name,
            proof.marker_identity,
        )?;
        validate_private_file(
            &proof.directory,
            temporary.file.as_ref().expect("new marker remains held"),
            ".thinws-control.toml",
            temporary.identity,
        )?;
        let published = read_private_document(&proof.directory, ".thinws-control.toml")?
            .ok_or_else(|| {
                PortError::new(PortErrorKind::InvalidLayout, "ready marker is missing")
            })?;
        let displaced = read_private_document(&proof.directory, &temporary.name)?
            .ok_or_else(|| PortError::new(PortErrorKind::InvalidLayout, "old marker is missing"))?;
        if decode_marker(&published).map_err(document_error)? != ready
            || decode_marker(&displaced).map_err(document_error)? != proof.marker
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "exchanged Linux marker content changed",
            ));
        }
        self.validate_bootstrap_lock(lock, &proof.directory)?;
        rustix::fs::unlinkat(
            &proof.directory.fd,
            temporary.name.as_str(),
            AtFlags::empty(),
        )
        .map_err(|error| {
            PortError::new(PortErrorKind::Io, "remove old Linux control marker").with_source(error)
        })?;
        sync_directory(&proof.directory.fd)?;
        Ok(ready)
    }

    /// Publishes the fixed bootstrap config only after the Ready marker.
    pub fn publish_config(
        &self,
        lock: &LinuxLockGuard,
        identity: &InstallationIdentity,
    ) -> Result<PublishResult, PortError> {
        let directory = open_private_directory(self.control_root())?;
        self.validate_bootstrap_lock(lock, &directory)?;
        if directory.path().as_os_str().as_bytes() != identity.data_root().as_bytes() {
            return Err(PortError::conflict(
                "publish config for different Linux control root",
                PortConflict::InstallationIdentity,
            ));
        }
        let expected_marker = RootMarker::new(identity.clone(), RootMarkerState::Ready);
        if self.read_root_marker(identity.data_root())?.as_ref() != Some(&expected_marker) {
            return Err(PortError::conflict(
                "publish Linux config before Ready marker",
                PortConflict::InstallationIdentity,
            ));
        }
        if let Some(current) = self.read_config()? {
            return if current == *identity {
                Ok(PublishResult::AlreadyCurrent)
            } else {
                Err(PortError::conflict(
                    "publish different Linux bootstrap config",
                    PortConflict::InstallationIdentity,
                ))
            };
        }
        let bytes = encode_config(identity).map_err(document_error)?;
        let temporary = PrivateTemp::create(&directory, "bootstrap-config", &bytes)?;
        match temporary.publish_noreplace("config.toml") {
            Ok((file, file_identity)) => {
                self.validate_bootstrap_lock(lock, &directory)?;
                validate_private_file(&directory, &file, "config.toml", file_identity)?;
                if self.read_config()?.as_ref() != Some(identity) {
                    return Err(PortError::new(
                        PortErrorKind::InvalidLayout,
                        "published Linux config content changed",
                    ));
                }
                Ok(PublishResult::Published)
            }
            Err(error) if error.kind() == PortErrorKind::Conflict => {
                if self.read_config()?.as_ref() == Some(identity) {
                    Ok(PublishResult::AlreadyCurrent)
                } else {
                    Err(error)
                }
            }
            Err(error) => Err(error),
        }
    }

    fn validate_bootstrap_lock(
        &self,
        lock: &LinuxLockGuard,
        directory: &PrivateDirectory,
    ) -> Result<(), PortError> {
        if lock.scope() != LifecycleScope::Bootstrap
            || directory.path() != self.control_root()
            || !lock.protects_directory(directory)
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "require matching Linux bootstrap lock",
            ));
        }
        lock.revalidate()
    }
}

fn create_private_child(
    parent: &PrivateDirectory,
    name: &str,
) -> Result<PrivateDirectory, PortError> {
    revalidate_private_directory(parent)?;
    for _ in 0..32 {
        let temporary_name = format!(".thinws-dir.tmp-{}", uuid::Uuid::now_v7());
        match rustix::fs::mkdirat(
            &parent.fd,
            temporary_name.as_str(),
            Mode::from_bits_retain(0o700),
        ) {
            Err(rustix::io::Errno::EXIST) => continue,
            Err(error) => {
                return Err(
                    PortError::new(PortErrorKind::Io, "stage Linux control directory")
                        .with_source(error),
                );
            }
            Ok(()) => {}
        }
        sync_directory(&parent.fd)?;
        let mut staged = open_private_directory(&parent.path().join(&temporary_name))?;
        let staged_identity = staged.file_identity();
        revalidate_private_directory(parent)?;
        let result = rustix::fs::renameat_with(
            &parent.fd,
            temporary_name.as_str(),
            &parent.fd,
            name,
            RenameFlags::NOREPLACE,
        );
        if let Err(error) = result {
            cleanup_staged_child(parent, &temporary_name, staged_identity);
            return Err(if error == rustix::io::Errno::EXIST {
                PortError::new(
                    PortErrorKind::NotEmpty,
                    "Linux control child already exists",
                )
            } else {
                PortError::new(PortErrorKind::Io, "publish Linux control directory")
                    .with_source(error)
            });
        }
        staged.relabel(parent.path().join(name));
        sync_directory(&parent.fd)?;
        revalidate_private_directory(&staged)?;
        revalidate_private_directory(parent)?;
        return Ok(staged);
    }
    Err(PortError::new(
        PortErrorKind::Io,
        "allocate Linux control directory staging name",
    ))
}

fn cleanup_staged_child(
    parent: &PrivateDirectory,
    name: &str,
    expected: thinws_core::FileIdentity,
) {
    if let Ok(named) = rustix::fs::statat(&parent.fd, name, AtFlags::SYMLINK_NOFOLLOW)
        && (named.st_dev, named.st_ino) == (expected.device(), expected.inode())
        && named.st_mode & libc::S_IFMT == libc::S_IFDIR
    {
        let _ = rustix::fs::unlinkat(&parent.fd, name, AtFlags::REMOVEDIR);
        let _ = sync_directory(&parent.fd);
    }
}

fn create_database_file(metadata: &PrivateDirectory) -> Result<File, PortError> {
    revalidate_private_directory(metadata)?;
    let fd = rustix::fs::openat(
        &metadata.fd,
        "state.db",
        OFlags::CREATE
            | OFlags::EXCL
            | OFlags::RDWR
            | OFlags::CLOEXEC
            | OFlags::NOFOLLOW
            | OFlags::NONBLOCK,
        Mode::from_bits_retain(0o600),
    )
    .map_err(|error| {
        PortError::new(PortErrorKind::Io, "create Linux control database").with_source(error)
    })?;
    let file = File::from(fd);
    let stat = rustix::fs::fstat(file.as_fd()).map_err(|error| {
        PortError::new(PortErrorKind::Io, "inspect created Linux control database")
            .with_source(error)
    })?;
    validate_private_file(metadata, &file, "state.db", (stat.st_dev, stat.st_ino))?;
    file.sync_all().map_err(|error| {
        PortError::new(PortErrorKind::Io, "sync created Linux control database").with_source(error)
    })?;
    sync_directory(&metadata.fd)?;
    Ok(file)
}

pub(crate) fn validate_private_file(
    parent: &PrivateDirectory,
    file: &File,
    name: &str,
    expected: (u64, u64),
) -> Result<(), PortError> {
    revalidate_private_directory(parent)?;
    let held = rustix::fs::fstat(file.as_fd()).map_err(|error| {
        PortError::new(PortErrorKind::Io, "inspect held Linux private file").with_source(error)
    })?;
    let named =
        rustix::fs::statat(&parent.fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| {
            PortError::new(
                PortErrorKind::InvalidLayout,
                "inspect named Linux private file",
            )
            .with_source(error)
        })?;
    if (held.st_dev, held.st_ino) != expected
        || (named.st_dev, named.st_ino) != expected
        || held.st_mode & libc::S_IFMT != libc::S_IFREG
        || named.st_mode & libc::S_IFMT != libc::S_IFREG
        || held.st_mode & 0o7777 != 0o600
        || named.st_mode & 0o7777 != 0o600
        || held.st_uid != rustix::process::geteuid().as_raw()
        || named.st_uid != held.st_uid
        || held.st_nlink != 1
        || named.st_nlink != 1
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Linux private file identity or mode changed",
        ));
    }
    revalidate_private_directory(parent)
}

pub(crate) fn sync_directory(directory: &OwnedFd) -> Result<(), PortError> {
    rustix::fs::fsync(directory).map_err(|error| {
        PortError::new(PortErrorKind::Io, "sync Linux control directory").with_source(error)
    })
}

pub(crate) struct PrivateTemp {
    parent: OwnedFd,
    name: String,
    file: Option<File>,
    identity: (u64, u64),
    active: bool,
}

impl PrivateTemp {
    pub(crate) fn create(
        parent: &PrivateDirectory,
        prefix: &str,
        bytes: &[u8],
    ) -> Result<Self, PortError> {
        revalidate_private_directory(parent)?;
        let parent_fd = rustix::io::dup(&parent.fd).map_err(|error| {
            PortError::new(PortErrorKind::Io, "duplicate Linux control directory")
                .with_source(error)
        })?;
        for _ in 0..32 {
            let name = format!(".{prefix}.tmp-{}", uuid::Uuid::now_v7());
            let fd = match rustix::fs::openat(
                &parent_fd,
                name.as_str(),
                OFlags::CREATE
                    | OFlags::EXCL
                    | OFlags::RDWR
                    | OFlags::CLOEXEC
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK,
                Mode::from_bits_retain(0o600),
            ) {
                Ok(fd) => fd,
                Err(rustix::io::Errno::EXIST) => continue,
                Err(error) => {
                    return Err(
                        PortError::new(PortErrorKind::Io, "stage Linux control document")
                            .with_source(error),
                    );
                }
            };
            let file = File::from(fd);
            let stat = rustix::fs::fstat(file.as_fd()).map_err(|error| {
                PortError::new(PortErrorKind::Io, "inspect staged Linux control document")
                    .with_source(error)
            })?;
            let mut temporary = Self {
                parent: parent_fd,
                name,
                file: Some(file),
                identity: (stat.st_dev, stat.st_ino),
                active: true,
            };
            let file = temporary.file.as_mut().expect("staged file remains held");
            file.write_all(bytes).map_err(|error| {
                PortError::new(PortErrorKind::Io, "write staged Linux control document")
                    .with_source(error)
            })?;
            file.sync_all().map_err(|error| {
                PortError::new(PortErrorKind::Io, "sync staged Linux control document")
                    .with_source(error)
            })?;
            sync_directory(&temporary.parent)?;
            validate_private_file(parent, file, &temporary.name, temporary.identity)?;
            return Ok(temporary);
        }
        Err(PortError::new(
            PortErrorKind::Io,
            "allocate Linux control document staging name",
        ))
    }

    pub(crate) fn publish_noreplace(
        mut self,
        target: &str,
    ) -> Result<(File, (u64, u64)), PortError> {
        rustix::fs::renameat_with(
            &self.parent,
            self.name.as_str(),
            &self.parent,
            target,
            RenameFlags::NOREPLACE,
        )
        .map_err(|error| {
            if error == rustix::io::Errno::EXIST {
                PortError::conflict(
                    "publish Linux control document without replacement",
                    PortConflict::InstallationIdentity,
                )
            } else {
                PortError::new(PortErrorKind::Io, "publish Linux control document")
                    .with_source(error)
            }
        })?;
        self.active = false;
        sync_directory(&self.parent)?;
        Ok((
            self.file.take().expect("staged file remains held"),
            self.identity,
        ))
    }

    pub(crate) fn exchange_with(&mut self, target: &str) -> Result<(), PortError> {
        rustix::fs::renameat_with(
            &self.parent,
            self.name.as_str(),
            &self.parent,
            target,
            RenameFlags::EXCHANGE,
        )
        .map_err(|error| {
            PortError::new(PortErrorKind::Io, "exchange Linux control marker").with_source(error)
        })?;
        // After the exchange, our temporary name refers to the old marker.
        self.active = false;
        sync_directory(&self.parent)
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for PrivateTemp {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Ok(named) =
            rustix::fs::statat(&self.parent, self.name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
            && (named.st_dev, named.st_ino) == self.identity
        {
            let _ = rustix::fs::unlinkat(&self.parent, self.name.as_str(), AtFlags::empty());
            let _ = sync_directory(&self.parent);
        }
    }
}

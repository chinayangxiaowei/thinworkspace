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

const DATABASE_CREATE_FLAGS: OFlags = OFlags::CREATE
    .union(OFlags::EXCL)
    .union(OFlags::RDWR)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK);

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
        if !exchanged_marker_contents_match(&published, &displaced, &ready, &proof.marker)? {
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
            Err(error) => self.reconcile_config_publication_error(error, identity),
        }
    }

    fn reconcile_config_publication_error(
        &self,
        error: PortError,
        identity: &InstallationIdentity,
    ) -> Result<PublishResult, PortError> {
        if error.kind() != PortErrorKind::Conflict {
            return Err(error);
        }
        if self.read_config()?.as_ref() == Some(identity) {
            Ok(PublishResult::AlreadyCurrent)
        } else {
            Err(error)
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

fn exchanged_marker_contents_match(
    published: &[u8],
    displaced: &[u8],
    ready: &RootMarker,
    initializing: &RootMarker,
) -> Result<bool, PortError> {
    Ok(
        !(decode_marker(published).map_err(document_error)? != *ready
            || decode_marker(displaced).map_err(document_error)? != *initializing),
    )
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
        DATABASE_CREATE_FLAGS,
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
    if !private_file_facts_match(&held, &named, expected) {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Linux private file identity or mode changed",
        ));
    }
    revalidate_private_directory(parent)
}

fn private_file_facts_match(
    held: &rustix::fs::Stat,
    named: &rustix::fs::Stat,
    expected: (u64, u64),
) -> bool {
    !((held.st_dev, held.st_ino) != expected
        || (named.st_dev, named.st_ino) != expected
        || held.st_mode & libc::S_IFMT != libc::S_IFREG
        || named.st_mode & libc::S_IFMT != libc::S_IFREG
        || held.st_mode & 0o7777 != 0o600
        || named.st_mode & 0o7777 != 0o600
        || held.st_uid != rustix::process::geteuid().as_raw()
        || named.st_uid != held.st_uid
        || held.st_nlink != 1
        || named.st_nlink != 1)
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

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::Path;
    use std::str::FromStr;
    use std::time::Duration;

    use tempfile::TempDir;
    use thinws_core::{AbsolutePath, InstanceId, VolumeId};
    use thinws_ports::LifecycleLock;

    use super::*;

    fn private_fixture(prefix: &str) -> (TempDir, PrivateDirectory) {
        let fixture = if let Some(root) = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT") {
            tempfile::Builder::new()
                .prefix(prefix)
                .tempdir_in(root)
                .unwrap()
        } else {
            tempfile::Builder::new().prefix(prefix).tempdir().unwrap()
        };
        fs::set_permissions(fixture.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let directory = open_private_directory(fixture.path()).unwrap();
        (fixture, directory)
    }

    fn identity(path: &Path) -> (u64, u64) {
        let metadata = fs::metadata(path).unwrap();
        (metadata.dev(), metadata.ino())
    }

    #[test]
    fn config_publication_conflict_keeps_the_original_error_unless_config_matches() {
        let (fixture, _directory) = private_fixture("thinws-linux-config-conflict-");
        let adapter = LinuxHostAdapter::new(fixture.path()).unwrap();
        let root =
            AbsolutePath::try_from_bytes(fixture.path().as_os_str().as_bytes().to_vec()).unwrap();
        let volume = VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let expected = InstallationIdentity::new(
            InstanceId::from_str("01890a5d-ac96-774b-bd5b-55c7b8d09f33").unwrap(),
            root.clone(),
            volume,
        );
        let other = InstallationIdentity::new(
            InstanceId::from_str("01890a5d-ac96-774b-bd5b-55c7b8d09f34").unwrap(),
            root,
            volume,
        );
        let conflict = || {
            PortError::conflict(
                "original publication conflict",
                PortConflict::InstallationIdentity,
            )
        };
        assert_eq!(
            adapter
                .reconcile_config_publication_error(conflict(), &expected)
                .err()
                .unwrap()
                .operation(),
            "original publication conflict"
        );

        let config = fixture.path().join("config.toml");
        fs::write(&config, encode_config(&other).unwrap()).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            adapter
                .reconcile_config_publication_error(conflict(), &expected)
                .err()
                .unwrap()
                .operation(),
            "original publication conflict"
        );

        fs::write(&config, encode_config(&expected).unwrap()).unwrap();
        assert_eq!(
            adapter
                .reconcile_config_publication_error(conflict(), &expected)
                .unwrap(),
            PublishResult::AlreadyCurrent
        );
        assert_eq!(
            adapter
                .reconcile_config_publication_error(
                    PortError::new(PortErrorKind::Io, "original publication I/O"),
                    &expected,
                )
                .err()
                .unwrap()
                .operation(),
            "original publication I/O"
        );
    }

    #[test]
    fn bootstrap_publication_requires_the_exact_scope_root_and_directory_identity() {
        let (fixture, directory) = private_fixture("thinws-linux-publication-lock-");
        let adapter = LinuxHostAdapter::new(fixture.path()).unwrap();
        let root =
            AbsolutePath::try_from_bytes(fixture.path().as_os_str().as_bytes().to_vec()).unwrap();
        let data_root_guard = adapter
            .acquire_data_root(&root, Duration::from_millis(100))
            .unwrap();
        assert_eq!(
            adapter
                .validate_bootstrap_lock(&data_root_guard, &directory)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        drop(data_root_guard);

        let bootstrap_guard = adapter
            .acquire_bootstrap(Duration::from_millis(100))
            .unwrap();
        adapter
            .validate_bootstrap_lock(&bootstrap_guard, &directory)
            .unwrap();
        let other_adapter = LinuxHostAdapter::new(fixture.path().join("another-control")).unwrap();
        assert_eq!(
            other_adapter
                .validate_bootstrap_lock(&bootstrap_guard, &directory)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );

        let other_path = fixture.path().join("other");
        fs::create_dir(&other_path).unwrap();
        fs::set_permissions(&other_path, fs::Permissions::from_mode(0o700)).unwrap();
        let mut wrong_identity = open_private_directory(&other_path).unwrap();
        wrong_identity.relabel(fixture.path().to_path_buf());
        assert_eq!(
            adapter
                .validate_bootstrap_lock(&bootstrap_guard, &wrong_identity)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
    }

    #[test]
    fn directory_sync_reports_a_real_fd_error() {
        let (fixture, _directory) = private_fixture("thinws-linux-sync-failure-");
        let path_only = rustix::fs::open(
            fixture.path(),
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        let error = sync_directory(&path_only).unwrap_err();
        assert_eq!(error.kind(), PortErrorKind::Io);
        assert_eq!(error.operation(), "sync Linux control directory");
    }

    #[test]
    fn exchanged_marker_requires_both_independent_contents() {
        let identity = InstallationIdentity::new(
            InstanceId::from_str("01890a5d-ac96-774b-bd5b-55c7b8d09f33").unwrap(),
            AbsolutePath::try_from_bytes(b"/home/test/.thinws".to_vec()).unwrap(),
            VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        );
        let initializing = RootMarker::new(identity.clone(), RootMarkerState::Initializing);
        let ready = RootMarker::new(identity, RootMarkerState::Ready);
        let initializing_bytes = encode_marker(&initializing).unwrap();
        let ready_bytes = encode_marker(&ready).unwrap();
        assert!(
            exchanged_marker_contents_match(
                &ready_bytes,
                &initializing_bytes,
                &ready,
                &initializing,
            )
            .unwrap()
        );
        assert!(
            !exchanged_marker_contents_match(
                &initializing_bytes,
                &initializing_bytes,
                &ready,
                &initializing,
            )
            .unwrap()
        );
        assert!(
            !exchanged_marker_contents_match(&ready_bytes, &ready_bytes, &ready, &initializing,)
                .unwrap()
        );
    }

    #[test]
    fn existing_control_child_is_not_replaced_or_misclassified() {
        let (fixture, directory) = private_fixture("thinws-linux-child-conflict-");
        let existing = fixture.path().join("metadata");
        fs::create_dir(&existing).unwrap();
        fs::write(existing.join("foreign"), b"keep").unwrap();
        assert_eq!(
            create_private_child(&directory, "metadata")
                .err()
                .unwrap()
                .kind(),
            PortErrorKind::NotEmpty
        );
        assert_eq!(fs::read(existing.join("foreign")).unwrap(), b"keep");
        assert!(fs::read_dir(fixture.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".thinws-dir.tmp-")
        }));
    }

    #[test]
    fn database_creation_requires_private_create_new_descriptor_flags() {
        for flag in [
            OFlags::CREATE,
            OFlags::EXCL,
            OFlags::RDWR,
            OFlags::CLOEXEC,
            OFlags::NOFOLLOW,
            OFlags::NONBLOCK,
        ] {
            assert!(DATABASE_CREATE_FLAGS.contains(flag), "missing {flag:?}");
        }
        let (fixture, directory) = private_fixture("thinws-linux-db-create-");
        let file = create_database_file(&directory).unwrap();
        let path = fixture.path().join("state.db");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o600
        );
        assert!(
            rustix::io::fcntl_getfd(&file)
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
        let open_flags = rustix::fs::fcntl_getfl(&file).unwrap();
        assert!(open_flags.contains(OFlags::RDWR));
        assert!(open_flags.contains(OFlags::NONBLOCK));
    }

    #[test]
    fn database_creation_does_not_open_existing_file_or_symlink() {
        let (fixture, directory) = private_fixture("thinws-linux-db-existing-");
        let path = fixture.path().join("state.db");
        fs::write(&path, b"foreign").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(create_database_file(&directory).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"foreign");

        fs::remove_file(&path).unwrap();
        let destination = fixture.path().join("destination");
        fs::write(&destination, b"outside").unwrap();
        std::os::unix::fs::symlink(&destination, &path).unwrap();
        assert!(create_database_file(&directory).is_err());
        assert_eq!(fs::read(destination).unwrap(), b"outside");
    }

    #[test]
    fn private_file_snapshot_rejects_each_independent_held_and_named_fact() {
        let (fixture, _directory) = private_fixture("thinws-linux-private-facts-");
        let path = fixture.path().join("document");
        fs::write(&path, b"owned").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let file = File::open(&path).unwrap();
        let snapshot = || rustix::fs::fstat(file.as_fd()).unwrap();
        let expected = identity(&path);
        assert!(private_file_facts_match(&snapshot(), &snapshot(), expected));

        let mut changed = snapshot();
        changed.st_dev = expected.0 + 1;
        assert!(!private_file_facts_match(&changed, &snapshot(), expected));
        assert!(!private_file_facts_match(&snapshot(), &changed, expected));

        let mut changed = snapshot();
        changed.st_ino = expected.1 + 1;
        assert!(!private_file_facts_match(&changed, &snapshot(), expected));
        assert!(!private_file_facts_match(&snapshot(), &changed, expected));

        let mut changed = snapshot();
        changed.st_mode = (changed.st_mode & !libc::S_IFMT) | libc::S_IFDIR;
        assert!(!private_file_facts_match(&changed, &snapshot(), expected));
        assert!(!private_file_facts_match(&snapshot(), &changed, expected));

        let mut changed = snapshot();
        changed.st_mode |= 0o040;
        assert!(!private_file_facts_match(&changed, &snapshot(), expected));
        assert!(!private_file_facts_match(&snapshot(), &changed, expected));

        let mut changed = snapshot();
        changed.st_uid += 1;
        assert!(!private_file_facts_match(&changed, &snapshot(), expected));
        assert!(!private_file_facts_match(&snapshot(), &changed, expected));

        let mut changed = snapshot();
        changed.st_nlink = 2;
        assert!(!private_file_facts_match(&changed, &snapshot(), expected));
        assert!(!private_file_facts_match(&snapshot(), &changed, expected));
    }

    #[test]
    fn private_file_validation_rejects_identity_mode_link_and_name_changes() {
        let (fixture, directory) = private_fixture("thinws-linux-private-file-");
        let path = fixture.path().join("document");
        fs::write(&path, b"owned").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let file = File::open(&path).unwrap();
        let expected = identity(&path);
        validate_private_file(&directory, &file, "document", expected).unwrap();

        let wrong = (expected.0, expected.1 + 1);
        assert_eq!(
            validate_private_file(&directory, &file, "document", wrong)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            validate_private_file(&directory, &file, "document", expected)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        let alias = fixture.path().join("alias");
        fs::hard_link(&path, &alias).unwrap();
        assert_eq!(
            validate_private_file(&directory, &file, "document", expected)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        fs::remove_file(alias).unwrap();
        validate_private_file(&directory, &file, "document", expected).unwrap();

        fs::rename(&path, fixture.path().join("held-document")).unwrap();
        fs::write(&path, b"foreign").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            validate_private_file(&directory, &file, "document", expected)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        assert_eq!(fs::read(&path).unwrap(), b"foreign");
    }

    #[test]
    fn staged_document_cleanup_removes_only_the_original_named_file() {
        let (fixture, directory) = private_fixture("thinws-linux-temp-cleanup-");
        let temporary = PrivateTemp::create(&directory, "document", b"owned").unwrap();
        let original = fixture.path().join(temporary.name());
        assert_eq!(fs::read(&original).unwrap(), b"owned");
        drop(temporary);
        assert!(!original.exists());

        let temporary = PrivateTemp::create(&directory, "document", b"owned").unwrap();
        let named = fixture.path().join(temporary.name());
        let held = fixture.path().join("held-temp");
        fs::rename(&named, &held).unwrap();
        fs::write(&named, b"foreign").unwrap();
        drop(temporary);
        assert_eq!(fs::read(&named).unwrap(), b"foreign");
        assert_eq!(fs::read(&held).unwrap(), b"owned");
    }

    #[test]
    fn staged_document_descriptor_keeps_required_runtime_flags() {
        let (fixture, directory) = private_fixture("thinws-linux-temp-flags-");
        let temporary = PrivateTemp::create(&directory, "document", b"owned").unwrap();
        let descriptor = temporary.file.as_ref().unwrap();

        assert!(
            rustix::io::fcntl_getfd(descriptor)
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
        let flags = rustix::fs::fcntl_getfl(descriptor).unwrap();
        assert!(flags.contains(OFlags::RDWR));
        assert!(flags.contains(OFlags::NONBLOCK));
        let staged = fixture.path().join(temporary.name());
        assert_eq!(
            fs::metadata(&staged).unwrap().permissions().mode() & 0o7777,
            0o600
        );
    }

    #[test]
    fn no_replace_publication_preserves_an_existing_document_and_cleans_staging() {
        let (fixture, directory) = private_fixture("thinws-linux-temp-conflict-");
        let existing = fixture.path().join("document");
        fs::write(&existing, b"foreign").unwrap();
        let temporary = PrivateTemp::create(&directory, "document", b"owned").unwrap();
        let staged = fixture.path().join(temporary.name());
        assert_eq!(
            temporary.publish_noreplace("document").unwrap_err().kind(),
            PortErrorKind::Conflict
        );
        assert_eq!(fs::read(&existing).unwrap(), b"foreign");
        assert!(!staged.exists());
    }
}

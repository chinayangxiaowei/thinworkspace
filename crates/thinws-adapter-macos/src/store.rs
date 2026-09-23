use std::ffi::OsStr;
use std::fs::File;
use std::str::FromStr;

use thinws_core::{AbsolutePath, InstallationIdentity, RootMarker, RootMarkerState, VolumeId};
use thinws_ports::{
    BootstrapStore, DataRootLayoutEvidence, LifecycleLockGuard, LifecycleScope, PortConflict,
    PortError, PortErrorKind, PreparedDataRootEvidence, PublishResult,
};

use crate::document::{
    DocumentError, decode_bootstrap_config, decode_root_marker, encode_config, encode_marker,
};
use crate::filesystem::{
    FileIdentity, NoReplaceError, PrivateTemp, ValidatedDirectory, create_private_child_directory,
    create_private_file, duplicate_validated_directory, entry_identity, io_error,
    open_private_child_directory, open_private_directory, open_private_directory_optional,
    open_private_file, path_from_absolute, prepare_private_directory, read_private_file,
    require_empty_directory, revalidate_directory, sync_directory, unlink_entry,
    validate_file_entry,
};
use crate::{MacOsHostAdapter, MacOsLockGuard};

const CONFIG_NAME: &str = "config.toml";
const MARKER_NAME: &str = ".thinws-root.toml";
const STATE_DATABASE_NAME: &str = "state.db";
const CONTROLLED_DIRECTORIES: [&str; 5] = ["metadata", "logs", "workspaces", "staging", "trash"];

/// Non-copyable proof that this process published one exact initializing marker.
pub struct MacOsInitializingProof {
    data_root: ValidatedDirectory,
    marker_file: File,
    marker_identity: FileIdentity,
    marker: RootMarker,
}

/// Descriptor-backed evidence for one prepared APFS data root.
pub struct MacOsPreparedDataRoot {
    data_root: AbsolutePath,
    volume_id: VolumeId,
    directory: ValidatedDirectory,
}

impl PreparedDataRootEvidence for MacOsPreparedDataRoot {
    fn data_root(&self) -> &AbsolutePath {
        &self.data_root
    }

    fn volume_id(&self) -> VolumeId {
        self.volume_id
    }
}

/// Descriptor-backed evidence for one controlled data-root layout.
pub struct MacOsDataRootLayout {
    data_root: ValidatedDirectory,
    controlled_directories: Vec<ValidatedDirectory>,
    database_file: File,
    database_identity: FileIdentity,
    database_path: AbsolutePath,
    volume_id: VolumeId,
}

impl DataRootLayoutEvidence for MacOsDataRootLayout {
    fn database_path(&self) -> &AbsolutePath {
        &self.database_path
    }

    fn revalidate(&self) -> Result<(), PortError> {
        self.revalidate_inner()
            .map_err(classify_layout_revalidation)
    }
}

impl MacOsDataRootLayout {
    fn revalidate_inner(&self) -> Result<(), PortError> {
        revalidate_directory(&self.data_root)?;
        if volume_id_for_directory(&self.data_root)? != self.volume_id {
            return Err(PortError::new(
                PortErrorKind::InvalidData,
                "revalidate data-root volume identity",
            ));
        }
        for directory in &self.controlled_directories {
            revalidate_directory(directory)?;
            if volume_id_for_directory(directory)? != self.volume_id {
                return Err(PortError::new(
                    PortErrorKind::InvalidData,
                    "revalidate controlled-directory volume identity",
                ));
            }
        }
        let metadata = self
            .controlled_directories
            .first()
            .expect("layout always contains metadata first");
        validate_file_entry(
            &metadata.fd,
            OsStr::new(STATE_DATABASE_NAME),
            &self.database_file,
            self.database_identity,
        )
    }
}

impl BootstrapStore for MacOsHostAdapter {
    type LockGuard = MacOsLockGuard;
    type PreparedDataRoot = MacOsPreparedDataRoot;
    type InitializingProof = MacOsInitializingProof;
    type DataRootLayout = MacOsDataRootLayout;

    fn prepare_bootstrap(&self) -> Result<(), PortError> {
        prepare_private_directory(&self.bootstrap_dir, false).map(|_| ())
    }

    fn prepare_data_root(
        &self,
        data_root: &AbsolutePath,
    ) -> Result<Self::PreparedDataRoot, PortError> {
        let directory = prepare_private_directory(&path_from_absolute(data_root), true)?;
        let volume_id = volume_id_for_directory(&directory)?;
        Ok(MacOsPreparedDataRoot {
            data_root: data_root.clone(),
            volume_id,
            directory,
        })
    }

    fn read_config(&self) -> Result<Option<InstallationIdentity>, PortError> {
        let Some(directory) = open_private_directory_optional(&self.bootstrap_dir)? else {
            return Ok(None);
        };
        read_config_at(&directory)
    }

    fn read_root_marker(
        &self,
        data_root: &thinws_core::AbsolutePath,
    ) -> Result<Option<RootMarker>, PortError> {
        let Some(directory) = open_private_directory_optional(&path_from_absolute(data_root))?
        else {
            return Ok(None);
        };
        read_marker_at(&directory, OsStr::new(MARKER_NAME))
    }

    fn create_initializing(
        &self,
        lock: &Self::LockGuard,
        prepared: Self::PreparedDataRoot,
        identity: &InstallationIdentity,
    ) -> Result<Self::InitializingProof, PortError> {
        self.validate_bootstrap_lock(lock)?;
        if prepared.data_root() != identity.data_root()
            || prepared.volume_id() != identity.volume_id()
        {
            return Err(PortError::conflict(
                "validate prepared data root",
                PortConflict::InstallationIdentity,
            ));
        }
        let directory = prepared.directory;
        revalidate_directory(&directory)?;
        if volume_id_for_directory(&directory)? != identity.volume_id() {
            return Err(PortError::conflict(
                "revalidate prepared data-root volume",
                PortConflict::InstallationIdentity,
            ));
        }
        require_empty_directory(&directory)?;
        let marker = RootMarker::new(identity.clone(), RootMarkerState::Initializing);
        let bytes = encode_marker(&marker).map_err(document_error)?;
        let temporary = PrivateTemp::create(&directory.fd, "root-marker", &bytes)?;
        match temporary.publish_noreplace(OsStr::new(MARKER_NAME)) {
            Ok((marker_file, marker_identity)) => {
                let verification = revalidate_directory(&directory).and_then(|()| {
                    validate_file_entry(
                        &directory.fd,
                        OsStr::new(MARKER_NAME),
                        &marker_file,
                        marker_identity,
                    )
                });
                if let Err(error) = verification {
                    remove_exact_entry(&directory, OsStr::new(MARKER_NAME), marker_identity);
                    return Err(error);
                }
                Ok(MacOsInitializingProof {
                    data_root: directory,
                    marker_file,
                    marker_identity,
                    marker,
                })
            }
            Err(NoReplaceError::Exists) => {
                // Existing objects are parsed only for a structured diagnostic;
                // even an identical initializing marker is never auto-adopted.
                let _ = self.read_root_marker(identity.data_root())?;
                Err(PortError::conflict(
                    "publish initializing root marker",
                    PortConflict::InstallationIdentity,
                ))
            }
            Err(NoReplaceError::Other(error)) => Err(error),
        }
    }

    fn initialize_layout(
        &self,
        lock: &Self::LockGuard,
        proof: &Self::InitializingProof,
    ) -> Result<Self::DataRootLayout, PortError> {
        self.validate_bootstrap_lock(lock)?;
        revalidate_directory(&proof.data_root)?;
        if volume_id_for_directory(&proof.data_root)? != proof.marker.identity().volume_id() {
            return Err(PortError::conflict(
                "revalidate initializing data-root volume",
                PortConflict::InstallationIdentity,
            ));
        }

        let mut controlled_directories = Vec::with_capacity(CONTROLLED_DIRECTORIES.len());
        for name in CONTROLLED_DIRECTORIES {
            controlled_directories.push(create_private_child_directory(
                &proof.data_root,
                OsStr::new(name),
            )?);
        }
        let metadata = controlled_directories
            .first()
            .expect("controlled directory list always begins with metadata");
        let (database_file, database_identity) =
            create_private_file(metadata, OsStr::new(STATE_DATABASE_NAME))?;
        let database_path =
            crate::filesystem::absolute_from_path(&metadata.path.join(STATE_DATABASE_NAME))
                .map_err(|error| {
                    PortError::new(PortErrorKind::InvalidData, "derive metadata database path")
                        .with_source(error)
                })?;
        let layout = MacOsDataRootLayout {
            data_root: duplicate_validated_directory(&proof.data_root)?,
            controlled_directories,
            database_file,
            database_identity,
            database_path,
            volume_id: proof.marker.identity().volume_id(),
        };
        layout.revalidate()?;
        Ok(layout)
    }

    fn validate_layout(
        &self,
        identity: &InstallationIdentity,
    ) -> Result<Self::DataRootLayout, PortError> {
        let Some(data_root) =
            open_private_directory_optional(&path_from_absolute(identity.data_root()))?
        else {
            return Err(PortError::new(
                PortErrorKind::Unavailable,
                "open registered data root",
            ));
        };
        if volume_id_for_directory(&data_root)? != identity.volume_id() {
            return Err(PortError::new(
                PortErrorKind::InvalidData,
                "validate registered data-root volume",
            ));
        }
        let mut controlled_directories = Vec::with_capacity(CONTROLLED_DIRECTORIES.len());
        for name in CONTROLLED_DIRECTORIES {
            controlled_directories
                .push(open_private_child_directory(&data_root, OsStr::new(name))?);
        }
        let metadata = controlled_directories
            .first()
            .expect("controlled directory list always begins with metadata");
        let (database_file, database_identity) =
            open_private_file(metadata, OsStr::new(STATE_DATABASE_NAME))?;
        let database_path =
            crate::filesystem::absolute_from_path(&metadata.path.join(STATE_DATABASE_NAME))
                .map_err(|error| {
                    PortError::new(PortErrorKind::InvalidData, "derive metadata database path")
                        .with_source(error)
                })?;
        let layout = MacOsDataRootLayout {
            data_root,
            controlled_directories,
            database_file,
            database_identity,
            database_path,
            volume_id: identity.volume_id(),
        };
        layout.revalidate()?;
        Ok(layout)
    }

    fn publish_ready(
        &self,
        lock: &Self::LockGuard,
        proof: Self::InitializingProof,
    ) -> Result<RootMarker, PortError> {
        self.validate_bootstrap_lock(lock)?;
        let directory = &proof.data_root;
        revalidate_directory(directory)?;
        validate_file_entry(
            &directory.fd,
            OsStr::new(MARKER_NAME),
            &proof.marker_file,
            proof.marker_identity,
        )?;
        let current = read_marker_at(directory, OsStr::new(MARKER_NAME))?;
        if (current.as_ref(), proof.marker.state())
            != (Some(&proof.marker), RootMarkerState::Initializing)
        {
            return Err(PortError::conflict(
                "validate initializing root marker",
                PortConflict::InstallationIdentity,
            ));
        }

        let ready = RootMarker::new(proof.marker.identity().clone(), RootMarkerState::Ready);
        let bytes = encode_marker(&ready).map_err(document_error)?;
        let mut temporary = PrivateTemp::create(&directory.fd, "root-marker-ready", &bytes)?;
        temporary.exchange_with(OsStr::new(MARKER_NAME))?;

        let verification = (|| {
            let observed_identities = (
                entry_identity(&directory.fd, OsStr::new(MARKER_NAME))?,
                entry_identity(&directory.fd, temporary.name())?,
            );
            if observed_identities != (temporary.identity(), proof.marker_identity) {
                return Err(PortError::new(
                    PortErrorKind::InvalidData,
                    "verify exchanged root marker identities",
                ));
            }
            validate_file_entry(
                &directory.fd,
                temporary.name(),
                &proof.marker_file,
                proof.marker_identity,
            )?;
            let observed_markers = (
                read_marker_at(directory, OsStr::new(MARKER_NAME))?,
                read_marker_at(directory, temporary.name())?,
            );
            if observed_markers != (Some(ready.clone()), Some(proof.marker.clone())) {
                return Err(PortError::new(
                    PortErrorKind::InvalidData,
                    "verify exchanged root marker content",
                ));
            }
            revalidate_directory(directory)?;
            Ok(())
        })();

        if let Err(error) = verification {
            let can_restore = exchanged_pair_matches(
                (
                    entry_identity(&directory.fd, OsStr::new(MARKER_NAME)).ok(),
                    entry_identity(&directory.fd, temporary.name()).ok(),
                ),
                (temporary.identity(), proof.marker_identity),
            );
            if can_restore {
                // Both names still identify the exact exchanged pair, so a
                // second atomic swap restores the previous marker safely.
                temporary.exchange_with(OsStr::new(MARKER_NAME))?;
                unlink_entry(&directory.fd, temporary.name())?;
                sync_directory(&directory.fd)?;
            }
            return Err(error);
        }

        unlink_entry(&directory.fd, temporary.name())?;
        sync_directory(&directory.fd)?;
        Ok(ready)
    }

    fn publish_config(
        &self,
        lock: &Self::LockGuard,
        identity: &InstallationIdentity,
    ) -> Result<PublishResult, PortError> {
        self.validate_bootstrap_lock(lock)?;
        let marker = self.read_root_marker(identity.data_root())?;
        if marker.as_ref() != Some(&RootMarker::new(identity.clone(), RootMarkerState::Ready)) {
            return Err(PortError::conflict(
                "publish bootstrap config before Ready",
                PortConflict::InstallationIdentity,
            ));
        }

        let directory = open_private_directory(&self.bootstrap_dir)?;
        if !lock.protects_directory(&directory) {
            return Err(PortError::new(
                PortErrorKind::InvalidData,
                "bootstrap lock directory changed",
            ));
        }
        if let Some(current) = read_config_at(&directory)? {
            return if config_is_current(Some(&current), identity) {
                Ok(PublishResult::AlreadyCurrent)
            } else {
                Err(PortError::conflict(
                    "publish bootstrap config",
                    PortConflict::InstallationIdentity,
                ))
            };
        }

        let bytes = encode_config(identity).map_err(document_error)?;
        let temporary = PrivateTemp::create(&directory.fd, "bootstrap-config", &bytes)?;
        match temporary.publish_noreplace(OsStr::new(CONFIG_NAME)) {
            Ok((config_file, config_identity)) => {
                let verification = revalidate_directory(&directory).and_then(|()| {
                    validate_file_entry(
                        &directory.fd,
                        OsStr::new(CONFIG_NAME),
                        &config_file,
                        config_identity,
                    )
                });
                if let Err(error) = verification {
                    remove_exact_entry(&directory, OsStr::new(CONFIG_NAME), config_identity);
                    return Err(error);
                }
                Ok(PublishResult::Published)
            }
            Err(NoReplaceError::Exists) => {
                let current = read_config_at(&directory)?;
                if config_is_current(current.as_ref(), identity) {
                    Ok(PublishResult::AlreadyCurrent)
                } else {
                    Err(PortError::conflict(
                        "publish bootstrap config",
                        PortConflict::InstallationIdentity,
                    ))
                }
            }
            Err(NoReplaceError::Other(error)) => Err(error),
        }
    }
}

fn volume_id_for_directory(directory: &ValidatedDirectory) -> Result<VolumeId, PortError> {
    let filesystem = crate::ffi::file_system_type(&directory.fd)
        .map_err(|error| io_error("inspect data-root filesystem", error))?;
    if filesystem != "apfs" {
        return Err(PortError::new(
            PortErrorKind::CapabilityUnavailable,
            "require APFS data root",
        ));
    }
    let bytes = crate::ffi::volume_uuid(&directory.fd)
        .map_err(|error| io_error("read APFS volume UUID", error))?;
    decode_volume_id(bytes)
}

fn decode_volume_id(bytes: Option<[u8; 16]>) -> Result<VolumeId, PortError> {
    let bytes = bytes
        .filter(|bytes| bytes.iter().any(|byte| *byte != 0))
        .ok_or_else(|| {
            PortError::new(
                PortErrorKind::CapabilityUnavailable,
                "require APFS volume UUID",
            )
        })?;
    let encoded = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    );
    VolumeId::from_str(&encoded).map_err(|error| {
        PortError::new(PortErrorKind::InvalidData, "decode APFS volume UUID").with_source(error)
    })
}

impl MacOsHostAdapter {
    fn validate_bootstrap_lock(&self, lock: &MacOsLockGuard) -> Result<(), PortError> {
        if lock.scope() != LifecycleScope::Bootstrap || !lock.belongs_to(self) {
            return Err(PortError::new(
                PortErrorKind::InvalidData,
                "require matching bootstrap lock",
            ));
        }
        lock.revalidate()
    }
}

fn read_config_at(
    directory: &crate::filesystem::ValidatedDirectory,
) -> Result<Option<InstallationIdentity>, PortError> {
    let result = read_private_file(&directory.fd, OsStr::new(CONFIG_NAME))?
        .map(|bytes| decode_bootstrap_config(&bytes).map_err(document_error))
        .transpose()?;
    revalidate_directory(directory)?;
    Ok(result)
}

fn read_marker_at(
    directory: &crate::filesystem::ValidatedDirectory,
    name: &OsStr,
) -> Result<Option<RootMarker>, PortError> {
    let result = read_private_file(&directory.fd, name)?
        .map(|bytes| decode_root_marker(&bytes).map_err(document_error))
        .transpose()?;
    revalidate_directory(directory)?;
    Ok(result)
}

fn remove_exact_entry(
    directory: &crate::filesystem::ValidatedDirectory,
    name: &OsStr,
    expected: FileIdentity,
) {
    if entry_identity(&directory.fd, name).ok() == Some(expected) {
        let _ = unlink_entry(&directory.fd, name);
        let _ = sync_directory(&directory.fd);
    }
}

fn exchanged_pair_matches(
    observed: (Option<FileIdentity>, Option<FileIdentity>),
    expected: (FileIdentity, FileIdentity),
) -> bool {
    observed == (Some(expected.0), Some(expected.1))
}

fn config_is_current(
    current: Option<&InstallationIdentity>,
    requested: &InstallationIdentity,
) -> bool {
    current == Some(requested)
}

fn document_error(error: DocumentError) -> PortError {
    let kind = if error == DocumentError::UnsupportedVersion {
        PortErrorKind::UnsupportedVersion
    } else {
        PortErrorKind::InvalidData
    };
    PortError::new(kind, "decode bootstrap document").with_source(error)
}

fn classify_layout_revalidation(error: PortError) -> PortError {
    if error.kind() == PortErrorKind::InvalidLayout {
        return error;
    }
    if matches!(
        error.kind(),
        PortErrorKind::InvalidData
            | PortErrorKind::NotFound
            | PortErrorKind::Conflict
            | PortErrorKind::CapabilityUnavailable
    ) {
        PortError::new(
            PortErrorKind::InvalidLayout,
            "revalidate controlled data-root layout",
        )
        .with_source(error)
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::str::FromStr;

    use tempfile::Builder;
    use thinws_core::{AbsolutePath, InstanceId, VolumeId};

    use super::*;

    #[test]
    fn exact_entry_cleanup_and_document_error_mapping_are_observable() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-02-macos-tests");
        fs::create_dir_all(&root).unwrap();
        let temp = Builder::new()
            .prefix("exact-entry-cleanup-")
            .tempdir_in(fs::canonicalize(root).unwrap())
            .unwrap();
        let directory_path = temp.path().join("private");
        fs::create_dir(&directory_path).unwrap();
        fs::set_permissions(&directory_path, fs::Permissions::from_mode(0o700)).unwrap();
        let directory = open_private_directory(&directory_path).unwrap();
        for name in ["target", "other"] {
            fs::write(directory_path.join(name), name.as_bytes()).unwrap();
            fs::set_permissions(directory_path.join(name), fs::Permissions::from_mode(0o600))
                .unwrap();
        }
        let target_identity = entry_identity(&directory.fd, OsStr::new("target")).unwrap();
        let other_identity = entry_identity(&directory.fd, OsStr::new("other")).unwrap();

        assert!(exchanged_pair_matches(
            (Some(target_identity), Some(other_identity)),
            (target_identity, other_identity)
        ));
        assert!(!exchanged_pair_matches(
            (Some(other_identity), Some(target_identity)),
            (target_identity, other_identity)
        ));

        remove_exact_entry(&directory, OsStr::new("target"), other_identity);
        assert!(directory_path.join("target").exists());
        remove_exact_entry(&directory, OsStr::new("target"), target_identity);
        assert!(!directory_path.join("target").exists());

        assert_eq!(
            document_error(DocumentError::UnsupportedVersion).kind(),
            PortErrorKind::UnsupportedVersion
        );
        assert_eq!(
            document_error(DocumentError::InvalidToml).kind(),
            PortErrorKind::InvalidData
        );

        let requested = InstallationIdentity::new(
            InstanceId::from_str("01890a5d-ac96-774b-bd5b-55c7b8d09f33").unwrap(),
            AbsolutePath::try_from_bytes(b"/Volumes/data/thinws".to_vec()).unwrap(),
            VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        );
        let conflicting = InstallationIdentity::new(
            InstanceId::from_str("01890a5d-ac96-774b-bd5b-55c7b8d09f34").unwrap(),
            AbsolutePath::try_from_bytes(b"/Volumes/data/thinws".to_vec()).unwrap(),
            VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        );
        assert!(config_is_current(Some(&requested), &requested));
        assert!(!config_is_current(Some(&conflicting), &requested));
        assert!(!config_is_current(None, &requested));
    }

    #[test]
    fn volume_uuid_bytes_require_a_nonzero_value_and_use_canonical_lowercase() {
        assert_eq!(
            decode_volume_id(None).unwrap_err().kind(),
            PortErrorKind::CapabilityUnavailable
        );
        assert_eq!(
            decode_volume_id(Some([0; 16])).unwrap_err().kind(),
            PortErrorKind::CapabilityUnavailable
        );
        assert_eq!(
            decode_volume_id(Some([
                0x1a, 0x42, 0xc8, 0x88, 0x32, 0xe3, 0x48, 0x9c, 0x9b, 0xfa, 0x67, 0xfd, 0x64, 0x0a,
                0x94, 0xe8,
            ]))
            .unwrap()
            .to_string(),
            "1a42c888-32e3-489c-9bfa-67fd640a94e8"
        );
    }

    #[test]
    fn layout_revalidation_classification_preserves_true_io_failures() {
        let io = classify_layout_revalidation(PortError::new(
            PortErrorKind::Io,
            "injected filesystem failure",
        ));
        assert_eq!(io.kind(), PortErrorKind::Io);

        for kind in [
            PortErrorKind::InvalidData,
            PortErrorKind::NotFound,
            PortErrorKind::Conflict,
            PortErrorKind::CapabilityUnavailable,
        ] {
            let error = classify_layout_revalidation(PortError::new(kind, "injected drift"));
            assert_eq!(error.kind(), PortErrorKind::InvalidLayout);
        }
    }

    #[test]
    #[ignore = "requires repository and system temporary directories on different APFS volumes"]
    fn layout_revalidation_rejects_a_real_different_apfs_volume() {
        let repository_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-03-volume-tests");
        fs::create_dir_all(&repository_root).unwrap();
        let data_root = Builder::new()
            .prefix("registered-root-")
            .tempdir_in(fs::canonicalize(repository_root).unwrap())
            .unwrap();
        let foreign = Builder::new().prefix("foreign-volume-").tempdir().unwrap();
        let data_root_path = fs::canonicalize(data_root.path()).unwrap();
        let foreign_path = fs::canonicalize(foreign.path()).unwrap();
        fs::set_permissions(&data_root_path, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&foreign_path, fs::Permissions::from_mode(0o700)).unwrap();
        let data_root = open_private_directory(&data_root_path).unwrap();
        let foreign = open_private_directory(&foreign_path).unwrap();
        let registered_volume = volume_id_for_directory(&data_root).unwrap();
        let foreign_volume = volume_id_for_directory(&foreign).unwrap();
        assert_ne!(registered_volume, foreign_volume);

        let (database_file, database_identity) =
            create_private_file(&foreign, OsStr::new(STATE_DATABASE_NAME)).unwrap();
        let database_path =
            crate::filesystem::absolute_from_path(&foreign_path.join(STATE_DATABASE_NAME)).unwrap();
        let layout = MacOsDataRootLayout {
            data_root,
            controlled_directories: vec![foreign],
            database_file,
            database_identity,
            database_path,
            volume_id: registered_volume,
        };

        assert_eq!(
            layout.revalidate().unwrap_err().kind(),
            PortErrorKind::InvalidLayout
        );
    }
}

use std::ffi::OsStr;
use std::fs::File;

use thinws_core::{InstallationIdentity, RootMarker, RootMarkerState};
use thinws_ports::{
    BootstrapStore, LifecycleLockGuard, LifecycleScope, PortConflict, PortError, PortErrorKind,
    PublishResult,
};

use crate::document::{
    DocumentError, decode_bootstrap_config, decode_root_marker, encode_config, encode_marker,
};
use crate::filesystem::{
    FileIdentity, NoReplaceError, PrivateTemp, ValidatedDirectory, entry_identity,
    open_private_directory, open_private_directory_optional, path_from_absolute, read_private_file,
    revalidate_directory, sync_directory, unlink_entry, validate_file_entry,
};
use crate::{MacOsHostAdapter, MacOsLockGuard};

const CONFIG_NAME: &str = "config.toml";
const MARKER_NAME: &str = ".thinws-root.toml";

/// Non-copyable proof that this process published one exact initializing marker.
pub struct MacOsInitializingProof {
    data_root: ValidatedDirectory,
    marker_file: File,
    marker_identity: FileIdentity,
    marker: RootMarker,
}

impl BootstrapStore for MacOsHostAdapter {
    type LockGuard = MacOsLockGuard;
    type InitializingProof = MacOsInitializingProof;

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
        identity: &InstallationIdentity,
    ) -> Result<Self::InitializingProof, PortError> {
        self.validate_bootstrap_lock(lock)?;
        let directory = open_private_directory(&path_from_absolute(identity.data_root()))?;
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
}

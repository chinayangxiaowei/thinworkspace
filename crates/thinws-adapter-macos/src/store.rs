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
    FileIdentity, NoReplaceError, PrivateTemp, entry_identity, open_private_directory,
    open_private_directory_optional, path_from_absolute, read_private_file, revalidate_directory,
    sync_directory, unlink_entry, validate_file_entry,
};
use crate::{MacOsHostAdapter, MacOsLockGuard};

const CONFIG_NAME: &str = "config.toml";
const MARKER_NAME: &str = ".thinws-root.toml";

/// Non-copyable proof that this process published one exact initializing marker.
pub struct MacOsInitializingProof {
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
        let data_root = path_from_absolute(proof.marker.identity().data_root());
        let directory = open_private_directory(&data_root)?;
        validate_file_entry(
            &directory.fd,
            OsStr::new(MARKER_NAME),
            &proof.marker_file,
            proof.marker_identity,
        )?;
        let current = read_marker_at(&directory, OsStr::new(MARKER_NAME))?;
        if current.as_ref() != Some(&proof.marker)
            || proof.marker.state() != RootMarkerState::Initializing
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
            if entry_identity(&directory.fd, OsStr::new(MARKER_NAME))? != temporary.identity()
                || entry_identity(&directory.fd, temporary.name())? != proof.marker_identity
            {
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
            if read_marker_at(&directory, OsStr::new(MARKER_NAME))?.as_ref() != Some(&ready)
                || read_marker_at(&directory, temporary.name())?.as_ref() != Some(&proof.marker)
            {
                return Err(PortError::new(
                    PortErrorKind::InvalidData,
                    "verify exchanged root marker content",
                ));
            }
            revalidate_directory(&directory)?;
            Ok(())
        })();

        if let Err(error) = verification {
            let can_restore = entry_identity(&directory.fd, OsStr::new(MARKER_NAME)).ok()
                == Some(temporary.identity())
                && entry_identity(&directory.fd, temporary.name()).ok()
                    == Some(proof.marker_identity);
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
            return if current == *identity {
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
            Err(NoReplaceError::Exists) => match read_config_at(&directory)? {
                Some(current) if current == *identity => Ok(PublishResult::AlreadyCurrent),
                _ => Err(PortError::conflict(
                    "publish bootstrap config",
                    PortConflict::InstallationIdentity,
                )),
            },
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

fn document_error(error: DocumentError) -> PortError {
    let kind = if error == DocumentError::UnsupportedVersion {
        PortErrorKind::UnsupportedVersion
    } else {
        PortErrorKind::InvalidData
    };
    PortError::new(kind, "decode bootstrap document").with_source(error)
}

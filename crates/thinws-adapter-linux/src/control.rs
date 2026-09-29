use std::fs::File;
use std::io::Read;
use std::os::fd::AsFd;
use std::os::unix::ffi::OsStrExt;

use rustix::fs::{AtFlags, Mode, OFlags};
use thinws_core::{AbsolutePath, PathResolution, VolumeId};
use thinws_ports::{
    PlatformProbe, PortConflict, PortError, PortErrorKind, PreparedDataRootEvidence,
};

use crate::document::{DocumentError, MAX_DOCUMENT_BYTES, decode_config, decode_marker};
use crate::lock::{
    PrivateDirectory, open_private_directory_optional, prepare_private_directory,
    revalidate_private_directory,
};
use crate::{LinuxHostAdapter, LinuxPlatformProbe};

/// Descriptor-backed evidence for a prepared Linux control root.
#[derive(Debug)]
pub struct LinuxPreparedDataRoot {
    data_root: AbsolutePath,
    volume_id: VolumeId,
    pub(crate) directory: PrivateDirectory,
}

impl PreparedDataRootEvidence for LinuxPreparedDataRoot {
    fn data_root(&self) -> &AbsolutePath {
        &self.data_root
    }

    fn volume_id(&self) -> VolumeId {
        self.volume_id
    }
}

impl LinuxPreparedDataRoot {
    /// Revalidates the held control directory and its verified filesystem identity.
    pub fn revalidate(&self) -> Result<(), PortError> {
        revalidate_private_directory(&self.directory)?;
        let current = control_volume_id(&self.directory, &self.data_root)?;
        if current != self.volume_id {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "control-root filesystem identity changed",
            ));
        }
        Ok(())
    }
}

impl LinuxHostAdapter {
    /// Prepares only the fixed, unclaimed control root and proves its volume identity.
    pub fn prepare_data_root(
        &self,
        data_root: &AbsolutePath,
    ) -> Result<LinuxPreparedDataRoot, PortError> {
        if self.control_root().as_os_str().as_bytes() != data_root.as_bytes() {
            return Err(PortError::conflict(
                "prepare fixed Linux control root",
                PortConflict::InstallationIdentity,
            ));
        }
        let directory = prepare_private_directory(self.control_root())?;
        require_unclaimed_control_root(&directory)?;
        let volume_id = control_volume_id(&directory, data_root)?;
        let prepared = LinuxPreparedDataRoot {
            data_root: data_root.clone(),
            volume_id,
            directory,
        };
        prepared.revalidate()?;
        Ok(prepared)
    }

    /// Reads the fixed bootstrap config without creating a missing control root.
    pub fn read_config(&self) -> Result<Option<thinws_core::InstallationIdentity>, PortError> {
        let Some(directory) = open_private_directory_optional(self.control_root())? else {
            return Ok(None);
        };
        read_private_document(&directory, "config.toml")?
            .map(|bytes| decode_config(&bytes).map_err(document_error))
            .transpose()
    }

    /// Reads the marker under the fixed control root without modifying it.
    pub fn read_root_marker(
        &self,
        data_root: &AbsolutePath,
    ) -> Result<Option<thinws_core::RootMarker>, PortError> {
        if self.control_root().as_os_str().as_bytes() != data_root.as_bytes() {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "read fixed Linux control marker",
            ));
        }
        let Some(directory) = open_private_directory_optional(self.control_root())? else {
            return Ok(None);
        };
        read_private_document(&directory, ".thinws-control.toml")?
            .map(|bytes| decode_marker(&bytes).map_err(document_error))
            .transpose()
    }
}

fn read_private_document(
    directory: &PrivateDirectory,
    name: &str,
) -> Result<Option<Vec<u8>>, PortError> {
    revalidate_private_directory(directory)?;
    let fd = match rustix::fs::openat(
        &directory.fd,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => {
            let named = rustix::fs::statat(&directory.fd, name, AtFlags::SYMLINK_NOFOLLOW);
            if matches!(named, Err(rustix::io::Errno::NOENT)) {
                revalidate_private_directory(directory)?;
                return Ok(None);
            }
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "private control document path changed",
            ));
        }
        Err(rustix::io::Errno::LOOP) => {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "private control document is a link",
            ));
        }
        Err(error) => {
            return Err(
                PortError::new(PortErrorKind::Io, "open private control document")
                    .with_source(error),
            );
        }
    };
    let mut file = File::from(fd);
    let before = rustix::fs::fstat(file.as_fd()).map_err(|error| {
        PortError::new(PortErrorKind::Io, "inspect private control document").with_source(error)
    })?;
    if before.st_mode & libc::S_IFMT != libc::S_IFREG
        || before.st_mode & 0o7777 != 0o600
        || before.st_uid != rustix::process::geteuid().as_raw()
        || before.st_nlink != 1
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "private control document metadata is unsafe",
        ));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take((MAX_DOCUMENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            PortError::new(PortErrorKind::Io, "read private control document").with_source(error)
        })?;
    let after = rustix::fs::fstat(file.as_fd()).map_err(|error| {
        PortError::new(PortErrorKind::Io, "reinspect private control document").with_source(error)
    })?;
    let named =
        rustix::fs::statat(&directory.fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| {
            PortError::new(
                PortErrorKind::InvalidLayout,
                "reinspect named control document",
            )
            .with_source(error)
        })?;
    if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino)
        || (before.st_dev, before.st_ino) != (named.st_dev, named.st_ino)
        || before.st_size != after.st_size
        || after.st_mode != named.st_mode
        || after.st_uid != named.st_uid
        || after.st_nlink != named.st_nlink
        || before.st_mtime != after.st_mtime
        || before.st_mtime_nsec != after.st_mtime_nsec
        || before.st_ctime != after.st_ctime
        || before.st_ctime_nsec != after.st_ctime_nsec
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "private control document changed during read",
        ));
    }
    revalidate_private_directory(directory)?;
    Ok(Some(bytes))
}

fn document_error(error: DocumentError) -> PortError {
    let kind = if error == DocumentError::UnsupportedVersion {
        PortErrorKind::UnsupportedVersion
    } else {
        PortErrorKind::InvalidData
    };
    PortError::new(kind, "decode Linux control document").with_source(error)
}

fn require_unclaimed_control_root(directory: &PrivateDirectory) -> Result<(), PortError> {
    let mut entries = rustix::fs::Dir::read_from(&directory.fd).map_err(|error| {
        PortError::new(PortErrorKind::Io, "inspect unclaimed Linux control root").with_source(error)
    })?;
    for entry in &mut entries {
        let entry = entry.map_err(|error| {
            PortError::new(PortErrorKind::Io, "read unclaimed Linux control root")
                .with_source(error)
        })?;
        if !matches!(
            entry.file_name().to_bytes(),
            b"." | b".." | b"lifecycle.lock"
        ) {
            return Err(PortError::new(
                PortErrorKind::NotEmpty,
                "unclaimed Linux control root contains another entry",
            ));
        }
        if entry.file_name().to_bytes() == b"lifecycle.lock" {
            let lock =
                rustix::fs::statat(&directory.fd, "lifecycle.lock", AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(|error| {
                        PortError::new(PortErrorKind::InvalidLayout, "inspect lifecycle lock entry")
                            .with_source(error)
                    })?;
            if lock.st_mode & libc::S_IFMT != libc::S_IFREG
                || lock.st_mode & 0o7777 != 0o600
                || lock.st_uid != rustix::process::geteuid().as_raw()
                || lock.st_nlink != 1
            {
                return Err(PortError::new(
                    PortErrorKind::InvalidLayout,
                    "unclaimed lifecycle lock metadata is unsafe",
                ));
            }
        }
    }
    revalidate_private_directory(directory)
}

fn control_volume_id(
    directory: &PrivateDirectory,
    data_root: &AbsolutePath,
) -> Result<VolumeId, PortError> {
    revalidate_private_directory(directory)?;
    let report = LinuxPlatformProbe.inspect_path(data_root)?;
    let held = report.ancestry().last().map(|entry| entry.identity());
    if report.resolution() != PathResolution::ExistingDirectory
        || report.mount().mount_id().is_none()
        || held != Some(directory.file_identity())
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "control-root path or mount identity changed",
        ));
    }
    if !matches!(report.filesystem().type_name(), "ext4" | "btrfs") || !report.mount().writable() {
        return Err(PortError::new(
            PortErrorKind::CapabilityUnavailable,
            "control-root filesystem is not verified and writable",
        ));
    }
    let volume = report
        .filesystem()
        .volume_id()
        .known()
        .copied()
        .ok_or_else(|| {
            PortError::new(
                PortErrorKind::CapabilityUnavailable,
                "control-root filesystem identity is unavailable",
            )
        })?;
    revalidate_private_directory(directory)?;
    Ok(volume)
}

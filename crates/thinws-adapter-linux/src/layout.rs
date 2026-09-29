use std::fs::File;
use std::os::fd::AsFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use rustix::fs::{AtFlags, Mode, OFlags};
use thinws_core::{AbsolutePath, InstallationIdentity, VolumeId};
use thinws_ports::{DataRootLayoutEvidence, PortError, PortErrorKind};

use crate::LinuxHostAdapter;
use crate::control::control_filesystem_identity;
use crate::lock::{
    PrivateDirectory, open_private_directory, open_private_directory_optional,
    revalidate_private_directory,
};

/// Descriptor-backed proof of the Linux control directory and SQLite file.
#[derive(Debug)]
pub struct LinuxDataRootLayout {
    data_root: PrivateDirectory,
    metadata: PrivateDirectory,
    logs: PrivateDirectory,
    database_file: File,
    database_identity: (u64, u64),
    database_path: AbsolutePath,
    volume_id: VolumeId,
    mount_id: u64,
}

impl DataRootLayoutEvidence for LinuxDataRootLayout {
    fn database_path(&self) -> &AbsolutePath {
        &self.database_path
    }

    fn revalidate(&self) -> Result<(), PortError> {
        require_filesystem(&self.data_root, self.volume_id, self.mount_id)?;
        for (parent, name, child) in [
            (&self.data_root, "metadata", &self.metadata),
            (&self.data_root, "logs", &self.logs),
        ] {
            revalidate_child(parent, name, child)?;
            require_filesystem(child, self.volume_id, self.mount_id)?;
        }
        if self.database_path != absolute(&self.metadata.path().join("state.db"))? {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "control database path changed",
            ));
        }
        validate_database_file(&self.metadata, &self.database_file, self.database_identity)
    }
}

impl LinuxHostAdapter {
    /// Opens an existing private control layout without creating or repairing it.
    pub fn validate_layout(
        &self,
        identity: &InstallationIdentity,
    ) -> Result<LinuxDataRootLayout, PortError> {
        if self.control_root().as_os_str().as_bytes() != identity.data_root().as_bytes() {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "validate fixed Linux control root",
            ));
        }
        let data_root = open_private_directory_optional(self.control_root())?.ok_or_else(|| {
            PortError::new(
                PortErrorKind::Unavailable,
                "open registered Linux control root",
            )
        })?;
        let (volume_id, mount_id) =
            control_filesystem_identity(&data_root, &absolute(data_root.path())?)?;
        if volume_id != identity.volume_id() {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "registered Linux control volume changed",
            ));
        }
        let metadata = open_child(&data_root, "metadata")?;
        let logs = open_child(&data_root, "logs")?;
        let database_file = open_database_file(&metadata)?;
        let stat = rustix::fs::fstat(database_file.as_fd()).map_err(|error| {
            PortError::new(PortErrorKind::Io, "inspect Linux control database").with_source(error)
        })?;
        let layout = LinuxDataRootLayout {
            database_path: absolute(&metadata.path().join("state.db"))?,
            data_root,
            metadata,
            logs,
            database_file,
            database_identity: (stat.st_dev, stat.st_ino),
            volume_id: identity.volume_id(),
            mount_id,
        };
        layout.revalidate()?;
        Ok(layout)
    }
}

fn absolute(path: &Path) -> Result<AbsolutePath, PortError> {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).map_err(|error| {
        PortError::new(PortErrorKind::InvalidLayout, "derive Linux control path").with_source(error)
    })
}

fn require_filesystem(
    directory: &PrivateDirectory,
    volume_id: VolumeId,
    mount_id: u64,
) -> Result<(), PortError> {
    let actual = control_filesystem_identity(directory, &absolute(directory.path())?)?;
    if actual != (volume_id, mount_id) {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Linux control layout filesystem identity changed",
        ));
    }
    Ok(())
}

fn open_child(parent: &PrivateDirectory, name: &str) -> Result<PrivateDirectory, PortError> {
    revalidate_private_directory(parent)?;
    let child = open_private_directory(&parent.path().join(name)).map_err(|error| {
        PortError::new(
            PortErrorKind::InvalidLayout,
            "open private Linux control child",
        )
        .with_source(error)
    })?;
    revalidate_child(parent, name, &child)?;
    Ok(child)
}

fn revalidate_child(
    parent: &PrivateDirectory,
    name: &str,
    child: &PrivateDirectory,
) -> Result<(), PortError> {
    revalidate_private_directory(parent)?;
    revalidate_private_directory(child)?;
    if child.path() != parent.path().join(name) {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Linux control child path changed",
        ));
    }
    let named =
        rustix::fs::statat(&parent.fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| {
            PortError::new(
                PortErrorKind::InvalidLayout,
                "inspect named Linux control child",
            )
            .with_source(error)
        })?;
    if (named.st_dev, named.st_ino)
        != (
            child.file_identity().device(),
            child.file_identity().inode(),
        )
        || named.st_mode & libc::S_IFMT != libc::S_IFDIR
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Linux control child identity changed",
        ));
    }
    revalidate_private_directory(parent)
}

fn open_database_file(metadata: &PrivateDirectory) -> Result<File, PortError> {
    revalidate_private_directory(metadata)?;
    let fd = rustix::fs::openat(
        &metadata.fd,
        "state.db",
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|error| {
        PortError::new(PortErrorKind::InvalidLayout, "open Linux control database")
            .with_source(error)
    })?;
    let file = File::from(fd);
    let stat = rustix::fs::fstat(file.as_fd()).map_err(|error| {
        PortError::new(PortErrorKind::Io, "inspect Linux control database").with_source(error)
    })?;
    validate_database_file(metadata, &file, (stat.st_dev, stat.st_ino))?;
    Ok(file)
}

fn validate_database_file(
    metadata: &PrivateDirectory,
    file: &File,
    expected: (u64, u64),
) -> Result<(), PortError> {
    revalidate_private_directory(metadata)?;
    let held = rustix::fs::fstat(file.as_fd()).map_err(|error| {
        PortError::new(PortErrorKind::Io, "inspect held Linux control database").with_source(error)
    })?;
    let named = rustix::fs::statat(&metadata.fd, "state.db", AtFlags::SYMLINK_NOFOLLOW).map_err(
        |error| {
            PortError::new(
                PortErrorKind::InvalidLayout,
                "inspect named Linux control database",
            )
            .with_source(error)
        },
    )?;
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
            "Linux control database identity or mode changed",
        ));
    }
    revalidate_private_directory(metadata)
}

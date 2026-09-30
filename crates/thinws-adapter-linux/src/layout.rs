use std::fs::File;
use std::os::fd::AsFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use rustix::fs::{AtFlags, Mode, OFlags};
use thinws_core::{AbsolutePath, FileIdentity, InstallationIdentity, InstanceId, VolumeId};
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
    pub(crate) data_root: PrivateDirectory,
    pub(crate) metadata: PrivateDirectory,
    pub(crate) logs: PrivateDirectory,
    database_file: File,
    database_identity: (u64, u64),
    database_path: AbsolutePath,
    volume_id: VolumeId,
    mount_id: u64,
    pub(crate) instance_id: InstanceId,
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

impl LinuxDataRootLayout {
    pub(crate) fn matches_newly_created(
        &self,
        metadata: &PrivateDirectory,
        logs: &PrivateDirectory,
        database: &File,
    ) -> Result<bool, PortError> {
        let created = rustix::fs::fstat(database.as_fd()).map_err(|error| {
            PortError::new(PortErrorKind::Io, "inspect created Linux database").with_source(error)
        })?;
        Ok(self.metadata.same_identity(metadata)
            && self.logs.same_identity(logs)
            && (created.st_dev, created.st_ino) == self.database_identity)
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
            instance_id: identity.instance_id(),
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
    if child_entry_changed(&named, child.file_identity()) {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Linux control child identity changed",
        ));
    }
    revalidate_private_directory(parent)
}

fn child_entry_changed(named: &rustix::fs::Stat, expected: FileIdentity) -> bool {
    (named.st_dev, named.st_ino) != (expected.device(), expected.inode())
        || named.st_mode & libc::S_IFMT != libc::S_IFDIR
}

fn open_database_file(metadata: &PrivateDirectory) -> Result<File, PortError> {
    revalidate_private_directory(metadata)?;
    let fd = rustix::fs::openat(
        &metadata.fd,
        "state.db",
        database_open_flags(),
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

fn database_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK
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
    if database_metadata_changed(&held, &named, expected, rustix::process::geteuid().as_raw()) {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Linux control database identity or mode changed",
        ));
    }
    revalidate_private_directory(metadata)
}

fn database_metadata_changed(
    held: &rustix::fs::Stat,
    named: &rustix::fs::Stat,
    expected: (u64, u64),
    owner: u32,
) -> bool {
    (held.st_dev, held.st_ino) != expected
        || (named.st_dev, named.st_ino) != expected
        || held.st_mode & libc::S_IFMT != libc::S_IFREG
        || named.st_mode & libc::S_IFMT != libc::S_IFREG
        || held.st_mode & 0o7777 != 0o600
        || named.st_mode & 0o7777 != 0o600
        || held.st_uid != owner
        || named.st_uid != held.st_uid
        || held.st_nlink != 1
        || named.st_nlink != 1
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::lock::prepare_private_directory;

    fn fixture() -> (tempfile::TempDir, LinuxDataRootLayout) {
        let root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
            .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-linux-layout-invariants-")
            .tempdir_in(root)
            .unwrap();
        let data_root = prepare_private_directory(&fixture.path().join("control")).unwrap();
        let metadata = prepare_private_directory(&data_root.path().join("metadata")).unwrap();
        let logs = prepare_private_directory(&data_root.path().join("logs")).unwrap();
        let database_path = metadata.path().join("state.db");
        std::fs::write(&database_path, b"").unwrap();
        std::fs::set_permissions(&database_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let database_file = open_database_file(&metadata).unwrap();
        let stat = rustix::fs::fstat(database_file.as_fd()).unwrap();
        let (volume_id, mount_id) =
            control_filesystem_identity(&data_root, &absolute(data_root.path()).unwrap()).unwrap();
        let layout = LinuxDataRootLayout {
            database_path: absolute(&database_path).unwrap(),
            data_root,
            metadata,
            logs,
            database_file,
            database_identity: (stat.st_dev, stat.st_ino),
            volume_id,
            mount_id,
            instance_id: "01890a5d-ac96-774b-bd5b-55c7b8d09f33".parse().unwrap(),
        };
        layout.revalidate().unwrap();
        (fixture, layout)
    }

    #[test]
    fn newly_created_layout_requires_all_three_created_identities() {
        let (fixture, layout) = fixture();
        assert!(
            layout
                .matches_newly_created(&layout.metadata, &layout.logs, &layout.database_file)
                .unwrap()
        );
        let foreign = prepare_private_directory(&fixture.path().join("foreign")).unwrap();
        assert!(
            !layout
                .matches_newly_created(&foreign, &layout.logs, &layout.database_file)
                .unwrap()
        );
        assert!(
            !layout
                .matches_newly_created(&layout.metadata, &foreign, &layout.database_file)
                .unwrap()
        );
        let other_file = fixture.path().join("other.db");
        std::fs::write(&other_file, b"").unwrap();
        let other_file = File::open(other_file).unwrap();
        assert!(
            !layout
                .matches_newly_created(&layout.metadata, &layout.logs, &other_file)
                .unwrap()
        );
    }

    #[test]
    fn control_layout_rejects_either_filesystem_identity_mismatch() {
        let (_fixture, layout) = fixture();
        require_filesystem(&layout.data_root, layout.volume_id, layout.mount_id).unwrap();
        let foreign_volume = "550e8400-e29b-41d4-a716-446655440001"
            .parse::<VolumeId>()
            .unwrap();
        assert_ne!(foreign_volume, layout.volume_id);
        assert_eq!(
            require_filesystem(&layout.data_root, foreign_volume, layout.mount_id)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        assert_eq!(
            require_filesystem(&layout.data_root, layout.volume_id, layout.mount_id + 1)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
    }

    #[test]
    fn child_identity_rejects_each_independent_named_change() {
        let (_fixture, layout) = fixture();
        let named = rustix::fs::statat(&layout.data_root.fd, "metadata", AtFlags::SYMLINK_NOFOLLOW)
            .unwrap();
        let identity = layout.metadata.file_identity();
        assert!(!child_entry_changed(&named, identity));
        for change in [
            (|stat: &mut rustix::fs::Stat| stat.st_dev += 1) as fn(&mut rustix::fs::Stat),
            |stat| stat.st_ino += 1,
            |stat| stat.st_mode ^= libc::S_IFDIR,
        ] {
            let mut changed =
                rustix::fs::statat(&layout.data_root.fd, "metadata", AtFlags::SYMLINK_NOFOLLOW)
                    .unwrap();
            change(&mut changed);
            assert!(child_entry_changed(&changed, identity));
        }
    }

    #[test]
    fn named_control_child_must_match_the_held_parent_and_path() {
        let (_fixture, layout) = fixture();
        revalidate_child(&layout.data_root, "metadata", &layout.metadata).unwrap();
        assert_eq!(
            revalidate_child(&layout.data_root, "logs", &layout.metadata)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        assert_eq!(
            revalidate_child(&layout.data_root, "metadata", &layout.logs)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
    }

    #[test]
    fn database_identity_rejects_each_independent_held_or_named_change() {
        let (_fixture, layout) = fixture();
        let held = rustix::fs::fstat(layout.database_file.as_fd()).unwrap();
        let named =
            rustix::fs::statat(&layout.metadata.fd, "state.db", AtFlags::SYMLINK_NOFOLLOW).unwrap();
        let expected = layout.database_identity;
        let owner = rustix::process::geteuid().as_raw();
        assert!(!database_metadata_changed(&held, &named, expected, owner));

        for change in [
            (|stat: &mut rustix::fs::Stat| stat.st_dev += 1) as fn(&mut rustix::fs::Stat),
            |stat| stat.st_ino += 1,
            |stat| stat.st_mode ^= libc::S_IFREG,
            |stat| stat.st_mode ^= 0o100,
            |stat| stat.st_uid += 1,
            |stat| stat.st_nlink += 1,
        ] {
            let mut changed = rustix::fs::fstat(layout.database_file.as_fd()).unwrap();
            change(&mut changed);
            assert!(database_metadata_changed(&changed, &named, expected, owner));
        }
        for change in [
            (|stat: &mut rustix::fs::Stat| stat.st_dev += 1) as fn(&mut rustix::fs::Stat),
            |stat| stat.st_ino += 1,
            |stat| stat.st_mode ^= libc::S_IFREG,
            |stat| stat.st_mode ^= 0o100,
            |stat| stat.st_uid += 1,
            |stat| stat.st_nlink += 1,
        ] {
            let mut changed =
                rustix::fs::statat(&layout.metadata.fd, "state.db", AtFlags::SYMLINK_NOFOLLOW)
                    .unwrap();
            change(&mut changed);
            assert!(database_metadata_changed(&held, &changed, expected, owner));
        }
    }

    #[test]
    fn database_open_flags_require_each_syscall_safety_bit() {
        let flags = database_open_flags();
        assert!(flags.contains(OFlags::CLOEXEC));
        assert!(flags.contains(OFlags::NOFOLLOW));
        assert!(flags.contains(OFlags::NONBLOCK));
        assert_eq!(flags & OFlags::ACCMODE, OFlags::RDONLY);
    }
}

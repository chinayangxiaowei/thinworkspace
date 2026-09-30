//! Descriptor-relative deletion inside one historically owned Btrfs root.

use std::ffi::OsStr;
use std::os::fd::OwnedFd;

use rustix::fs::AtFlags;
use thinws_core::FileIdentity;
use thinws_ports::{PortError, PortErrorKind};

use crate::tree::{
    NodeKind, directory_names, mount_id, mount_id_at, node, node_at, open_directory,
};

const MAX_DELETE_DEPTH: usize = 512;

pub(crate) fn remove_root_contents(
    root: &OwnedFd,
    expected: FileIdentity,
    expected_mount: u64,
    verify_scope: &dyn Fn() -> Result<(), PortError>,
) -> Result<usize, PortError> {
    verify_scope()?;
    require_directory(root, expected, expected_mount)?;
    remove_directory_contents(root, expected, expected_mount, 0, verify_scope)
}

fn remove_directory_contents(
    directory: &OwnedFd,
    expected: FileIdentity,
    expected_mount: u64,
    depth: usize,
    verify_scope: &dyn Fn() -> Result<(), PortError>,
) -> Result<usize, PortError> {
    if depth > MAX_DELETE_DEPTH {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace deletion exceeds directory depth limit",
        ));
    }
    require_directory(directory, expected, expected_mount)?;
    let names = directory_names(directory).map_err(|error| error.into_port_error())?;
    let mut removed = 0_usize;
    for name in names {
        verify_scope()?;
        require_directory(directory, expected, expected_mount)?;
        let before = checked_entry(directory, &name, expected, expected_mount)?;
        if before.kind == NodeKind::Unsupported {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "unsupported special entry in Workspace deletion scope",
            ));
        }
        if before.kind == NodeKind::Directory {
            let child =
                open_directory(directory, &name).map_err(|error| error.into_port_error())?;
            require_directory(&child, before.identity(), expected_mount)?;
            let verify_child = || {
                verify_scope()?;
                let named = checked_entry(directory, &name, expected, expected_mount)?;
                if named.identity() != before.identity() || named.kind != before.kind {
                    return Err(stale_scope());
                }
                Ok(())
            };
            removed = removed
                .checked_add(remove_directory_contents(
                    &child,
                    before.identity(),
                    expected_mount,
                    depth + 1,
                    &verify_child,
                )?)
                .ok_or_else(|| {
                    PortError::new(PortErrorKind::InvalidLayout, "entry count overflow")
                })?;
            require_directory(&child, before.identity(), expected_mount)?;
        }
        verify_scope()?;
        let current = checked_entry(directory, &name, expected, expected_mount)?;
        if current.identity() != before.identity() || current.kind != before.kind {
            return Err(stale_scope());
        }
        let flags = if before.kind == NodeKind::Directory {
            AtFlags::REMOVEDIR
        } else {
            AtFlags::empty()
        };
        rustix::fs::unlinkat(directory, &name, flags).map_err(|error| {
            PortError::new(PortErrorKind::Io, "remove proven Workspace entry").with_source(error)
        })?;
        removed = removed
            .checked_add(1)
            .ok_or_else(|| PortError::new(PortErrorKind::InvalidLayout, "entry count overflow"))?;
    }
    verify_scope()?;
    require_directory(directory, expected, expected_mount)?;
    if !directory_names(directory)
        .map_err(|error| error.into_port_error())?
        .is_empty()
    {
        return Err(PortError::new(
            PortErrorKind::NotEmpty,
            "Workspace directory changed during deletion",
        ));
    }
    rustix::fs::fsync(directory).map_err(|error| {
        PortError::new(PortErrorKind::Io, "sync removed Workspace entries").with_source(error)
    })?;
    Ok(removed)
}

fn require_directory(
    directory: &OwnedFd,
    expected: FileIdentity,
    expected_mount: u64,
) -> Result<(), PortError> {
    let current = node(directory).map_err(|error| error.into_port_error())?;
    let current_mount = mount_id(directory).map_err(|error| error.into_port_error())?;
    if current.kind != NodeKind::Directory
        || current.identity() != expected
        || current_mount != expected_mount
    {
        return Err(stale_scope());
    }
    Ok(())
}

fn checked_entry(
    directory: &OwnedFd,
    name: &OsStr,
    expected: FileIdentity,
    expected_mount: u64,
) -> Result<crate::tree::Node, PortError> {
    require_directory(directory, expected, expected_mount)?;
    let entry = node_at(directory, name).map_err(|error| error.into_port_error())?;
    let entry_mount = mount_id_at(directory, name).map_err(|error| error.into_port_error())?;
    if entry.identity().device() != expected.device() || entry_mount != expected_mount {
        return Err(stale_scope());
    }
    Ok(entry)
}

fn stale_scope() -> PortError {
    PortError::new(
        PortErrorKind::InvalidLayout,
        "Workspace deletion scope changed",
    )
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::os::fd::OwnedFd;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use rustix::fs::{Mode, open};

    use super::*;
    use crate::tree::DIRECTORY_FLAGS;

    fn fixture() -> (tempfile::TempDir, OwnedFd, FileIdentity, u64) {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs test mount");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-linux-destroy-")
            .tempdir_in(root)
            .unwrap();
        let copy = fixture.path().join("copy");
        fs::create_dir(&copy).unwrap();
        let fd = directory(&copy);
        let identity = node(&fd)
            .map_err(|error| error.into_port_error())
            .unwrap()
            .identity();
        let mount = mount_id(&fd)
            .map_err(|error| error.into_port_error())
            .unwrap();
        (fixture, fd, identity, mount)
    }

    fn directory(path: &Path) -> OwnedFd {
        open(path, DIRECTORY_FLAGS, Mode::empty()).unwrap()
    }

    #[test]
    fn held_directory_requires_both_identity_and_mount() {
        let (_fixture, fd, identity, mount) = fixture();
        require_directory(&fd, identity, mount).unwrap();
        let wrong_identity = FileIdentity::new(identity.device(), identity.inode() + 1);
        assert_eq!(
            require_directory(&fd, wrong_identity, mount)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        assert_eq!(
            require_directory(&fd, identity, mount + 1)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
    }

    #[test]
    fn changed_file_is_not_unlinked_after_scope_revalidation() {
        let (fixture, fd, identity, mount) = fixture();
        let copy = fixture.path().join("copy");
        fs::write(copy.join("file"), b"original").unwrap();
        let calls = AtomicUsize::new(0);
        let result = remove_root_contents(&fd, identity, mount, &|| {
            if calls.fetch_add(1, Ordering::SeqCst) == 2 {
                fs::rename(copy.join("file"), fixture.path().join("displaced")).unwrap();
                fs::write(copy.join("file"), b"foreign").unwrap();
            }
            Ok(())
        });
        assert_eq!(result.unwrap_err().kind(), PortErrorKind::InvalidLayout);
        assert_eq!(fs::read(copy.join("file")).unwrap(), b"foreign");
        assert_eq!(
            fs::read(fixture.path().join("displaced")).unwrap(),
            b"original"
        );
    }

    #[test]
    fn changed_child_name_does_not_delete_held_old_directory() {
        let (fixture, fd, identity, mount) = fixture();
        let copy = fixture.path().join("copy");
        fs::create_dir(copy.join("nested")).unwrap();
        fs::write(copy.join("nested/file"), b"original").unwrap();
        let calls = AtomicUsize::new(0);
        let result = remove_root_contents(&fd, identity, mount, &|| {
            if calls.fetch_add(1, Ordering::SeqCst) == 2 {
                fs::rename(copy.join("nested"), fixture.path().join("displaced")).unwrap();
                fs::create_dir(copy.join("nested")).unwrap();
            }
            Ok(())
        });
        assert_eq!(result.unwrap_err().kind(), PortErrorKind::InvalidLayout);
        assert_eq!(
            fs::read(fixture.path().join("displaced/file")).unwrap(),
            b"original"
        );
    }

    #[test]
    fn depth_limit_accepts_boundary_and_rejects_next_child() {
        let (fixture, fd, identity, mount) = fixture();
        let copy = fixture.path().join("copy");
        fs::write(copy.join("leaf"), b"data").unwrap();
        assert_eq!(
            remove_directory_contents(&fd, identity, mount, MAX_DELETE_DEPTH, &|| Ok(())).unwrap(),
            1
        );
        fs::create_dir(copy.join("nested")).unwrap();
        fs::write(copy.join("nested/leaf"), b"data").unwrap();
        assert_eq!(
            remove_directory_contents(&fd, identity, mount, MAX_DELETE_DEPTH, &|| Ok(()))
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        assert_eq!(fs::read(copy.join("nested/leaf")).unwrap(), b"data");
    }

    #[test]
    #[ignore = "requires THINWS_LINUX_BIND_MOUNT_CHILD on a distinct bind mount of the same Btrfs filesystem"]
    fn same_device_child_on_different_mount_is_not_in_deletion_scope() {
        let child_path = env::var_os("THINWS_LINUX_BIND_MOUNT_CHILD")
            .expect("set THINWS_LINUX_BIND_MOUNT_CHILD to the bind-mounted Btrfs child");
        let child_path = Path::new(&child_path);
        let parent = directory(child_path.parent().unwrap());
        let expected = node(&parent)
            .map_err(|error| error.into_port_error())
            .unwrap()
            .identity();
        let expected_mount = mount_id(&parent)
            .map_err(|error| error.into_port_error())
            .unwrap();
        let name = child_path.file_name().unwrap();
        let child = node_at(&parent, name)
            .map_err(|error| error.into_port_error())
            .unwrap();
        assert_eq!(child.device, expected.device());
        assert_ne!(
            mount_id_at(&parent, name)
                .map_err(|error| error.into_port_error())
                .unwrap(),
            expected_mount
        );
        assert_eq!(
            checked_entry(&parent, name, expected, expected_mount)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
    }
}

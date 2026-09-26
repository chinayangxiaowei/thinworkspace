//! Descriptor-relative removal of the contents of an already owned Workspace root.

use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;

use thinws_core::FileIdentity;

use crate::ffi::{
    RawFileKind, RawNodeMetadata, c_string, node_metadata, node_metadata_at, open_directory_at,
    read_directory, remove_staged_at,
};

const MAX_DELETE_DEPTH: usize = 512;

pub(crate) fn remove_root_contents(
    root: &OwnedFd,
    expected: FileIdentity,
    verify_scope: &dyn Fn() -> io::Result<()>,
) -> io::Result<usize> {
    verify_scope()?;
    let metadata = node_metadata(root)?;
    require_directory(metadata, expected)?;
    remove_directory_contents(root, expected, expected.device(), 0, verify_scope)
}

fn remove_directory_contents(
    directory: &OwnedFd,
    expected: FileIdentity,
    root_device: u64,
    depth: usize,
    verify_scope: &dyn Fn() -> io::Result<()>,
) -> io::Result<usize> {
    if depth > MAX_DELETE_DEPTH {
        return Err(io::Error::from_raw_os_error(libc::ELOOP));
    }
    require_directory(node_metadata(directory)?, expected)?;
    let mut names = read_directory(directory)?;
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    let mut removed = 0_usize;
    for name in names {
        verify_scope()?;
        let component = c_string(name.as_bytes())?;
        require_directory(node_metadata(directory)?, expected)?;
        let before = node_metadata_at(directory, &component)?;
        if before.device != root_device {
            return Err(io::Error::from_raw_os_error(libc::EXDEV));
        }
        if before.kind == RawFileKind::Directory {
            let child = open_directory_at(directory, &component)?;
            let child_identity = file_identity(before);
            require_directory(node_metadata(&child)?, child_identity)?;
            let verify_child = || {
                verify_scope()?;
                let named = node_metadata_at(directory, &component)?;
                if named.kind != RawFileKind::Directory || file_identity(named) != child_identity {
                    return Err(io::Error::from_raw_os_error(libc::ESTALE));
                }
                Ok(())
            };
            removed = removed
                .checked_add(remove_directory_contents(
                    &child,
                    child_identity,
                    root_device,
                    depth + 1,
                    &verify_child,
                )?)
                .ok_or_else(|| io::Error::other("removed entry count overflow"))?;
            require_directory(node_metadata(&child)?, child_identity)?;
        }
        verify_scope()?;
        require_directory(node_metadata(directory)?, expected)?;
        let current = node_metadata_at(directory, &component)?;
        if file_identity(current) != file_identity(before) || current.kind != before.kind {
            return Err(io::Error::from_raw_os_error(libc::ESTALE));
        }
        remove_staged_at(directory, &component, before.kind)?;
        removed = removed
            .checked_add(1)
            .ok_or_else(|| io::Error::other("removed entry count overflow"))?;
    }
    require_directory(node_metadata(directory)?, expected)?;
    if !read_directory(directory)?.is_empty() {
        return Err(io::Error::from_raw_os_error(libc::ENOTEMPTY));
    }
    Ok(removed)
}

fn require_directory(metadata: RawNodeMetadata, expected: FileIdentity) -> io::Result<()> {
    if metadata.kind != RawFileKind::Directory || file_identity(metadata) != expected {
        return Err(io::Error::from_raw_os_error(libc::ESTALE));
    }
    Ok(())
}

fn file_identity(metadata: RawNodeMetadata) -> FileIdentity {
    FileIdentity::new(metadata.device, metadata.inode)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs::{self, File};
    use std::os::fd::OwnedFd;
    use std::os::unix::fs::{MetadataExt, symlink};

    use tempfile::TempDir;

    use super::*;
    use crate::ffi::node_metadata;

    #[test]
    fn removes_nested_entries_without_following_external_symlinks() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let external = temp.path().join("external");
        fs::create_dir_all(root.join("nested/again")).unwrap();
        fs::create_dir(&external).unwrap();
        fs::write(root.join("nested/again/file"), b"copy").unwrap();
        fs::write(external.join("keep"), b"outside").unwrap();
        symlink(&external, root.join("escape")).unwrap();
        symlink("missing", root.join("dangling")).unwrap();

        let held: OwnedFd = File::open(&root).unwrap().into();
        let metadata = node_metadata(&held).unwrap();
        let removed = remove_root_contents(
            &held,
            FileIdentity::new(metadata.device, metadata.inode),
            &|| Ok(()),
        )
        .unwrap();

        assert_eq!(removed, 5);
        assert!(fs::read_dir(&root).unwrap().next().is_none());
        assert_eq!(fs::read(external.join("keep")).unwrap(), b"outside");
    }

    #[test]
    fn refuses_a_mismatched_root_before_removing_anything() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("keep"), b"copy").unwrap();
        let held: OwnedFd = File::open(&root).unwrap().into();
        let metadata = node_metadata(&held).unwrap();

        assert!(
            remove_root_contents(
                &held,
                FileIdentity::new(metadata.device, metadata.inode + 1),
                &|| Ok(()),
            )
            .is_err()
        );
        assert_eq!(fs::read(root.join("keep")).unwrap(), b"copy");
    }

    #[test]
    fn scope_replacement_before_first_unlink_keeps_old_and_foreign_contents() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let detached = temp.path().join("detached");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("old"), b"old").unwrap();
        let held: OwnedFd = File::open(&root).unwrap().into();
        let metadata = node_metadata(&held).unwrap();
        let checks = Cell::new(0);
        let verify = || {
            checks.set(checks.get() + 1);
            if checks.get() == 2 {
                fs::rename(&root, &detached)?;
                fs::create_dir(&root)?;
                fs::write(root.join("foreign"), b"keep")?;
            }
            let current = fs::metadata(&root)?;
            if (current.dev(), current.ino()) != (metadata.device, metadata.inode) {
                return Err(io::Error::from_raw_os_error(libc::ESTALE));
            }
            Ok(())
        };
        let error = remove_root_contents(
            &held,
            FileIdentity::new(metadata.device, metadata.inode),
            &verify,
        )
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ESTALE));
        assert_eq!(fs::read(detached.join("old")).unwrap(), b"old");
        assert_eq!(fs::read(root.join("foreign")).unwrap(), b"keep");
    }

    #[test]
    fn detached_child_is_not_deleted_through_its_held_descriptor() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let detached = temp.path().join("detached-child");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/keep"), b"outside").unwrap();
        let held: OwnedFd = File::open(&root).unwrap().into();
        let metadata = node_metadata(&held).unwrap();
        let checks = Cell::new(0);
        let verify = || {
            checks.set(checks.get() + 1);
            if checks.get() == 3 {
                fs::rename(root.join("nested"), &detached)?;
            }
            Ok(())
        };
        assert!(
            remove_root_contents(
                &held,
                FileIdentity::new(metadata.device, metadata.inode),
                &verify,
            )
            .is_err()
        );
        assert_eq!(fs::read(detached.join("keep")).unwrap(), b"outside");
    }
}

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

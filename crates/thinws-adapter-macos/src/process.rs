//! Best-effort cwd and open-vnode occupancy inspection via macOS libproc.

use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use thinws_core::{AbsolutePath, ProcessUse, UnixMillis};
use thinws_ports::{PortError, PortErrorKind, ProcessObservation, ProcessProbe};

use crate::MacOsHostAdapter;
use crate::ffi::{
    RawProcessIdentity, RawProcessVnode, fd_kernel_path, process_cwd, process_identity,
    process_vnode_fd, process_vnode_fds, visible_user_pids,
};
use crate::filesystem::{io_error, open_private_directory, revalidate_directory};

const SCAN_BUDGET: Duration = Duration::from_secs(2);

impl ProcessProbe for MacOsHostAdapter {
    fn inspect_workspace(
        &self,
        workspace_container: &AbsolutePath,
    ) -> Result<ProcessObservation, PortError> {
        let container = PathBuf::from(std::ffi::OsString::from_vec(
            workspace_container.as_bytes().to_vec(),
        ));
        let held = open_private_directory(&container)?;
        let kernel_path = PathBuf::from(
            fd_kernel_path(&held.fd)
                .map_err(|error| io_error("resolve Workspace process-scan root", error))?,
        );
        let held_metadata = rustix::fs::fstat(&held.fd)
            .map_err(|error| io_error("inspect held process-scan root", error))?;
        let kernel_metadata = fs::metadata(&kernel_path)
            .map_err(|error| io_error("inspect resolved process-scan root", error))?;
        if !kernel_metadata.is_dir()
            || kernel_metadata.dev() != held_metadata.st_dev as u64
            || kernel_metadata.ino() != held_metadata.st_ino
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace process-scan root identity changed",
            ));
        }
        revalidate_directory(&held)?;
        let use_state = scan_visible_processes(&kernel_path);
        let observed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .and_then(|millis| UnixMillis::new(millis).ok())
            .ok_or_else(|| PortError::new(PortErrorKind::Io, "record process-scan time"))?;
        Ok(ProcessObservation {
            observed_at,
            use_state,
        })
    }
}

fn scan_visible_processes(container: &Path) -> ProcessUse {
    let deadline = Instant::now() + SCAN_BUDGET;
    let Ok((pids, mut incomplete)) = visible_user_pids() else {
        return ProcessUse::ScanIncomplete;
    };
    let own_pid = std::process::id();
    let own_uid = rustix::process::geteuid().as_raw();
    for pid in pids {
        if Instant::now() >= deadline {
            incomplete = true;
            break;
        }
        if u32::try_from(pid).ok() == Some(own_pid) {
            continue;
        }
        let Ok(identity) = process_identity(pid) else {
            incomplete = true;
            continue;
        };
        if identity.pid != pid || identity.uid != own_uid {
            incomplete = true;
            continue;
        }
        match inspect_process(pid, identity, container, deadline) {
            ProcessUse::ConfirmedInUse => return ProcessUse::ConfirmedInUse,
            ProcessUse::ScanIncomplete => incomplete = true,
            ProcessUse::NoEvidence => {}
        }
    }
    if incomplete {
        ProcessUse::ScanIncomplete
    } else {
        ProcessUse::NoEvidence
    }
}

fn inspect_process(
    pid: i32,
    identity: RawProcessIdentity,
    container: &Path,
    deadline: Instant,
) -> ProcessUse {
    let mut incomplete = false;
    match process_cwd(pid) {
        Ok(Some(vnode)) => {
            if vnode_matches_container(&vnode, container, &mut incomplete) {
                if process_identity(pid)
                    .ok()
                    .is_some_and(|current| same_process(identity, current))
                {
                    return ProcessUse::ConfirmedInUse;
                }
                incomplete = true;
            }
        }
        Ok(None) | Err(_) => incomplete = true,
    }
    let fds = process_vnode_fds(pid, identity.open_file_count);
    match fds {
        Ok((fds, truncated)) => {
            incomplete |= truncated;
            for fd in fds {
                if Instant::now() >= deadline {
                    incomplete = true;
                    break;
                }
                match process_vnode_fd(pid, fd) {
                    Ok(Some(vnode)) => {
                        if vnode_matches_container(&vnode, container, &mut incomplete) {
                            if process_identity(pid)
                                .ok()
                                .is_some_and(|current| same_process(identity, current))
                            {
                                return ProcessUse::ConfirmedInUse;
                            }
                            incomplete = true;
                        }
                    }
                    Ok(None) | Err(_) => incomplete = true,
                }
            }
        }
        Err(_) => incomplete = true,
    }
    if incomplete {
        ProcessUse::ScanIncomplete
    } else {
        ProcessUse::NoEvidence
    }
}

fn same_process(before: RawProcessIdentity, after: RawProcessIdentity) -> bool {
    before.pid == after.pid
        && before.uid == after.uid
        && before.start_seconds == after.start_seconds
        && before.start_microseconds == after.start_microseconds
}

fn vnode_matches_container(
    vnode: &RawProcessVnode,
    container: &Path,
    incomplete: &mut bool,
) -> bool {
    let reported = PathBuf::from(&vnode.path);
    if !reported.starts_with(container) {
        return false;
    }
    let Ok(metadata) = fs::metadata(&reported) else {
        *incomplete = true;
        return false;
    };
    if metadata.dev() != vnode.device || metadata.ino() != vnode.inode {
        *incomplete = true;
        return false;
    }
    match fs::canonicalize(reported) {
        Ok(actual) => actual.starts_with(container),
        Err(_) => {
            *incomplete = true;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};
    use std::thread;

    use tempfile::Builder;

    use super::*;

    fn controlled_container(prefix: &str) -> (tempfile::TempDir, PathBuf, AbsolutePath) {
        let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/p1-12-process-probe-tests");
        fs::create_dir_all(&controlled).unwrap();
        let temp = Builder::new()
            .prefix(prefix)
            .tempdir_in(fs::canonicalize(controlled).unwrap())
            .unwrap();
        let container = temp.path().join("workspace");
        fs::create_dir(&container).unwrap();
        fs::set_permissions(&container, fs::Permissions::from_mode(0o700)).unwrap();
        let absolute =
            AbsolutePath::try_from_bytes(container.as_os_str().as_bytes().to_vec()).unwrap();
        (temp, container, absolute)
    }

    fn data_firmlink_alias(path: &Path) -> Option<AbsolutePath> {
        let alias = Path::new("/System/Volumes/Data").join(path.strip_prefix("/").ok()?);
        let original = fs::metadata(path).ok()?;
        let through_alias = fs::metadata(&alias).ok()?;
        if (original.dev(), original.ino()) != (through_alias.dev(), through_alias.ino()) {
            return None;
        }
        AbsolutePath::try_from_bytes(alias.as_os_str().as_bytes().to_vec()).ok()
    }

    #[test]
    fn current_process_identity_is_not_just_a_pid() {
        let current = process_identity(std::process::id() as i32).unwrap();
        assert_eq!(current.pid, std::process::id() as i32);
        assert_eq!(current.uid, rustix::process::geteuid().as_raw());
        assert!(current.start_seconds > 0);
        assert!(current.start_microseconds < 1_000_000);
        let (pids, _) = visible_user_pids().unwrap();
        assert!(pids.contains(&current.pid));
    }

    #[test]
    fn vanished_process_queries_are_errors_not_empty_evidence() {
        assert!(process_identity(i32::MAX).is_err());
        assert!(process_cwd(i32::MAX).is_err());
        assert!(process_vnode_fds(i32::MAX, 1).is_err());
    }

    #[test]
    fn process_identity_survives_fd_count_change_but_not_starttime_change() {
        let before = RawProcessIdentity {
            pid: 123,
            uid: 501,
            start_seconds: 1_700_000_000,
            start_microseconds: 123_456,
            open_file_count: 10,
        };
        let more_fds = RawProcessIdentity {
            open_file_count: 400,
            ..before
        };
        assert!(same_process(before, more_fds));
        let reused_pid = RawProcessIdentity {
            start_microseconds: before.start_microseconds + 1,
            ..before
        };
        assert!(!same_process(before, reused_pid));
    }

    #[test]
    fn child_cwd_inside_workspace_is_confirmed_in_use() {
        let (_temp, container, path) = controlled_container("cwd-");
        let adapter = MacOsHostAdapter::new(container.parent().unwrap().join("bootstrap")).unwrap();
        let mut child = Command::new("/bin/sleep")
            .arg("5")
            .current_dir(&container)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut found = false;
        while Instant::now() < deadline {
            if adapter.inspect_workspace(&path).unwrap().use_state == ProcessUse::ConfirmedInUse {
                found = true;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if container.starts_with("/Volumes/data") {
            assert!(data_firmlink_alias(&container).is_some());
        }
        if let Some(alias) = data_firmlink_alias(&container) {
            assert_eq!(
                adapter.inspect_workspace(&alias).unwrap().use_state,
                ProcessUse::ConfirmedInUse,
                "a physical firmlink alias must not hide cwd occupancy"
            );
        }
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(found, "a same-user child cwd must be detected");
        assert_ne!(
            adapter.inspect_workspace(&path).unwrap().use_state,
            ProcessUse::ConfirmedInUse
        );
    }

    #[test]
    fn child_open_file_inside_workspace_is_confirmed_in_use() {
        let (temp, container, path) = controlled_container("open-vnode-");
        let file = container.join("opened.txt");
        fs::write(&file, b"hold open").unwrap();
        let adapter = MacOsHostAdapter::new(temp.path().join("bootstrap")).unwrap();
        let mut child = Command::new("/usr/bin/tail")
            .arg("-f")
            .arg(&file)
            .current_dir(temp.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut found = false;
        while Instant::now() < deadline {
            if adapter.inspect_workspace(&path).unwrap().use_state == ProcessUse::ConfirmedInUse {
                found = true;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if container.starts_with("/Volumes/data") {
            assert!(data_firmlink_alias(&container).is_some());
        }
        if let Some(alias) = data_firmlink_alias(&container) {
            assert_eq!(
                adapter.inspect_workspace(&alias).unwrap().use_state,
                ProcessUse::ConfirmedInUse,
                "a physical firmlink alias must not hide open-vnode occupancy"
            );
        }
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(found, "a same-user open vnode must be detected");
    }

    #[test]
    fn vnode_path_requires_component_boundary_and_current_inode() {
        let (temp, container, _path) = controlled_container("vnode-boundary-");
        let sibling = temp.path().join("workspace-other");
        fs::write(&sibling, b"outside").unwrap();
        let sibling_metadata = fs::metadata(&sibling).unwrap();
        let sibling_vnode = RawProcessVnode {
            path: sibling.as_os_str().to_os_string(),
            device: sibling_metadata.dev(),
            inode: sibling_metadata.ino(),
        };
        let mut incomplete = false;
        assert!(!vnode_matches_container(
            &sibling_vnode,
            &container,
            &mut incomplete
        ));
        assert!(!incomplete);

        let internal = container.join("entry");
        fs::write(&internal, b"original").unwrap();
        let metadata = fs::metadata(&internal).unwrap();
        let stale = RawProcessVnode {
            path: internal.as_os_str().to_os_string(),
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        fs::rename(&internal, container.join("old-entry")).unwrap();
        fs::write(&internal, b"replacement").unwrap();
        assert!(!vnode_matches_container(
            &stale,
            &container,
            &mut incomplete
        ));
        assert!(incomplete);
    }

    #[test]
    fn vnode_symlink_inside_container_cannot_confirm_an_external_target() {
        let (temp, container, _path) = controlled_container("vnode-symlink-");
        let outside = temp.path().join("outside");
        fs::write(&outside, b"external").unwrap();
        let link = container.join("link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let metadata = fs::metadata(&link).unwrap();
        let vnode = RawProcessVnode {
            path: link.as_os_str().to_os_string(),
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        let mut incomplete = false;
        assert!(!vnode_matches_container(
            &vnode,
            &container,
            &mut incomplete
        ));
        assert!(!incomplete);
    }
}

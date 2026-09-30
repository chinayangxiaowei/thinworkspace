//! Best-effort inspection of Linux processes using cwd and open descriptors.

use std::ffi::OsStr;
use std::fs;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rustix::fs::{AtFlags, Mode, OFlags, StatxFlags};
use thinws_core::{
    AbsolutePath, FileIdentity, PathCapabilityReport, PathResolution, ProcessUse, UnixMillis,
};
use thinws_ports::{PlatformProbe, PortError, PortErrorKind, ProcessObservation, ProcessProbe};

use crate::{LinuxHostAdapter, LinuxPlatformProbe};

const SCAN_BUDGET: Duration = Duration::from_secs(2);
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

impl ProcessProbe for LinuxHostAdapter {
    fn inspect_workspace(
        &self,
        workspace_container: &AbsolutePath,
    ) -> Result<ProcessObservation, PortError> {
        let held = open_bound_root(workspace_container)?;
        let path = PathBuf::from(OsStr::from_bytes(workspace_container.as_bytes()));
        let use_state = scan_visible_processes(&path);
        revalidate_root(workspace_container, &held)?;
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .and_then(|value| UnixMillis::new(value).ok())
            .ok_or_else(|| PortError::new(PortErrorKind::Io, "record process-scan time"))?;
        Ok(ProcessObservation {
            observed_at: millis,
            use_state,
        })
    }
}

struct BoundRoot {
    fd: OwnedFd,
    report: PathCapabilityReport,
}

fn open_bound_root(path: &AbsolutePath) -> Result<BoundRoot, PortError> {
    let report = LinuxPlatformProbe.inspect_path(path)?;
    if report.resolution() != PathResolution::ExistingDirectory {
        return Err(PortError::new(
            PortErrorKind::NotFound,
            "open Workspace process-scan root",
        ));
    }
    let mut directory = rustix::fs::open("/", DIRECTORY_FLAGS, Mode::empty())
        .map_err(|error| io_error("open process-scan filesystem root", error))?;
    let mut ancestry = report.ancestry().iter();
    let root = ancestry.next().ok_or_else(root_changed)?;
    if root.identity() != identity(&directory)? {
        return Err(root_changed());
    }
    for component in path.as_bytes()[1..].split(|byte| *byte == b'/') {
        if component.is_empty() {
            continue;
        }
        directory = rustix::fs::openat(
            &directory,
            OsStr::from_bytes(component),
            DIRECTORY_FLAGS,
            Mode::empty(),
        )
        .map_err(|error| path_error("open process-scan path component", error))?;
        if ancestry.next().map(|part| part.identity()) != Some(identity(&directory)?) {
            return Err(root_changed());
        }
    }
    if ancestry.next().is_some() {
        return Err(root_changed());
    }
    let stat = rustix::fs::fstat(&directory)
        .map_err(|error| io_error("inspect process-scan root", error))?;
    if stat.st_uid != rustix::process::geteuid().as_raw() {
        return Err(root_changed());
    }
    let bound = BoundRoot {
        fd: directory,
        report,
    };
    revalidate_root(path, &bound)?;
    Ok(bound)
}

fn revalidate_root(path: &AbsolutePath, held: &BoundRoot) -> Result<(), PortError> {
    let report = LinuxPlatformProbe.inspect_path(path)?;
    let expected = current_root_identity(&report, path).ok_or_else(root_changed)?;
    let stat = rustix::fs::fstat(&held.fd)
        .map_err(|error| io_error("inspect held process-scan root", error))?;
    let mount = rustix::fs::statx(&held.fd, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)
        .map_err(|error| io_error("inspect held process-scan mount", error))?;
    if root_evidence_changed(
        expected,
        &stat,
        &held.report,
        &report,
        mount.stx_mask,
        mount.stx_mnt_id,
        rustix::process::geteuid().as_raw(),
    ) {
        return Err(root_changed());
    }
    Ok(())
}

fn current_root_identity(
    report: &PathCapabilityReport,
    path: &AbsolutePath,
) -> Option<FileIdentity> {
    report
        .ancestry()
        .last()
        .filter(|entry| {
            entry.path() == path && report.resolution() == PathResolution::ExistingDirectory
        })
        .map(|entry| entry.identity())
}

fn root_evidence_changed(
    expected: FileIdentity,
    held_stat: &rustix::fs::Stat,
    recorded: &PathCapabilityReport,
    current: &PathCapabilityReport,
    mount_mask: u32,
    mounted_id: u64,
    owner: u32,
) -> bool {
    expected != FileIdentity::new(held_stat.st_dev, held_stat.st_ino)
        || held_stat.st_uid != owner
        || recorded.ancestry() != current.ancestry()
        || recorded.filesystem() != current.filesystem()
        || recorded.mount().mount_id() != current.mount().mount_id()
        || (mount_mask & StatxFlags::MNT_ID.bits()) == 0
        || Some(mounted_id) != current.mount().mount_id()
}

fn identity(fd: &OwnedFd) -> Result<FileIdentity, PortError> {
    let stat = rustix::fs::fstat(fd)
        .map_err(|error| io_error("inspect process-scan path identity", error))?;
    Ok(FileIdentity::new(stat.st_dev, stat.st_ino))
}

fn scan_visible_processes(container: &Path) -> ProcessUse {
    let deadline = Instant::now() + SCAN_BUDGET;
    let Ok(processes) = fs::read_dir("/proc") else {
        return ProcessUse::ScanIncomplete;
    };
    let mut incomplete = false;
    let own_pid = std::process::id();
    let own_uid = rustix::process::geteuid().as_raw();
    for entry in processes {
        if Instant::now() >= deadline {
            incomplete = true;
            break;
        }
        let Ok(entry) = entry else {
            incomplete = true;
            continue;
        };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|text| text.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == own_pid {
            continue;
        }
        let proc_dir = entry.path();
        let Some(before) = read_process_identity(&proc_dir) else {
            incomplete = true;
            continue;
        };
        if before.uid != own_uid {
            continue;
        }
        match inspect_process(&proc_dir, before, container, deadline) {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProcessIdentity {
    uid: u32,
    start_ticks: u64,
}

fn read_process_identity(proc_dir: &Path) -> Option<ProcessIdentity> {
    let status = fs::read(proc_dir.join("status")).ok()?;
    let uid = status
        .split(|byte| *byte == b'\n')
        .find_map(|line| line.strip_prefix(b"Uid:"))?
        .split(|byte| *byte == b'\t' || *byte == b' ')
        .filter(|word| !word.is_empty())
        .nth(1)
        .and_then(|word| std::str::from_utf8(word).ok())?
        .parse()
        .ok()?;
    let stat = fs::read(proc_dir.join("stat")).ok()?;
    let after_name = stat.windows(2).rposition(|window| window == b") ")? + 2;
    let start_ticks = stat[after_name..]
        .split(|byte| *byte == b' ')
        .nth(19)
        .and_then(|word| std::str::from_utf8(word).ok())?
        .parse()
        .ok()?;
    Some(ProcessIdentity { uid, start_ticks })
}

fn inspect_process(
    proc_dir: &Path,
    identity: ProcessIdentity,
    container: &Path,
    deadline: Instant,
) -> ProcessUse {
    let mut incomplete = false;
    if matches_process_link(&proc_dir.join("cwd"), container, &mut incomplete)
        && read_process_identity(proc_dir) == Some(identity)
    {
        return ProcessUse::ConfirmedInUse;
    }
    let Ok(fds) = fs::read_dir(proc_dir.join("fd")) else {
        return ProcessUse::ScanIncomplete;
    };
    for entry in fds {
        if Instant::now() >= deadline {
            incomplete = true;
            break;
        }
        let Ok(entry) = entry else {
            incomplete = true;
            continue;
        };
        if matches_process_link(&entry.path(), container, &mut incomplete) {
            if read_process_identity(proc_dir) == Some(identity) {
                return ProcessUse::ConfirmedInUse;
            }
            incomplete = true;
        }
    }
    if read_process_identity(proc_dir) != Some(identity) {
        incomplete = true;
    }
    if incomplete {
        ProcessUse::ScanIncomplete
    } else {
        ProcessUse::NoEvidence
    }
}

fn matches_process_link(link: &Path, container: &Path, incomplete: &mut bool) -> bool {
    let Ok(reported) = fs::read_link(link) else {
        *incomplete = true;
        return false;
    };
    if !reported.starts_with(container) {
        return false;
    }
    let (Ok(through_proc), Ok(through_path), Ok(canonical)) = (
        fs::metadata(link),
        fs::metadata(&reported),
        fs::canonicalize(&reported),
    ) else {
        *incomplete = true;
        return false;
    };
    if process_link_evidence_conflicts(
        (through_proc.dev(), through_proc.ino()),
        (through_path.dev(), through_path.ino()),
        &canonical,
        container,
    ) {
        *incomplete = true;
        return false;
    }
    true
}

fn process_link_evidence_conflicts(
    through_proc: (u64, u64),
    through_path: (u64, u64),
    canonical: &Path,
    container: &Path,
) -> bool {
    through_proc.0 != through_path.0
        || through_proc.1 != through_path.1
        || !canonical.starts_with(container)
}

fn root_changed() -> PortError {
    PortError::new(
        PortErrorKind::InvalidLayout,
        "Workspace process-scan root identity changed",
    )
}

fn io_error(operation: &'static str, error: rustix::io::Errno) -> PortError {
    PortError::new(PortErrorKind::Io, operation).with_source(std::io::Error::from(error))
}

fn path_error(operation: &'static str, error: rustix::io::Errno) -> PortError {
    let kind = match error {
        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => PortErrorKind::InvalidLayout,
        rustix::io::Errno::NOENT => PortErrorKind::NotFound,
        _ => PortErrorKind::Io,
    };
    PortError::new(kind, operation).with_source(std::io::Error::from(error))
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::os::unix::fs::symlink;

    use thinws_core::{DirectoryIdentityEvidence, FileSystemIdentity, MountEvidence};

    use super::*;

    fn rebuilt_report(
        original: &PathCapabilityReport,
        resolution: PathResolution,
        missing: Vec<Vec<u8>>,
        ancestry: Vec<DirectoryIdentityEvidence>,
        filesystem: FileSystemIdentity,
        mount: MountEvidence,
    ) -> PathCapabilityReport {
        PathCapabilityReport::new(
            original.requested_path().clone(),
            resolution,
            original.nearest_existing_ancestor().clone(),
            missing,
            ancestry,
            filesystem,
            mount,
            original.readability(),
            original.writability(),
            original.cow_clone(),
        )
        .unwrap()
    }

    #[test]
    fn proc_stat_identity_uses_start_time_and_effective_uid() {
        let self_dir = PathBuf::from(format!("/proc/{}", std::process::id()));
        let identity = read_process_identity(&self_dir).unwrap();
        assert_eq!(identity.uid, rustix::process::geteuid().as_raw());
        assert!(identity.start_ticks > 0);
        assert_eq!(read_process_identity(&self_dir), Some(identity));
    }

    #[test]
    fn changed_process_start_time_cannot_confirm_use() {
        let self_dir = PathBuf::from(format!("/proc/{}", std::process::id()));
        let identity = read_process_identity(&self_dir).unwrap();
        let cwd = env::current_dir().unwrap();
        assert_eq!(
            inspect_process(&self_dir, identity, &cwd, Instant::now() + SCAN_BUDGET),
            ProcessUse::ConfirmedInUse
        );
        let stale = ProcessIdentity {
            start_ticks: identity.start_ticks + 1,
            ..identity
        };
        assert_eq!(
            inspect_process(&self_dir, stale, &cwd, Instant::now() + SCAN_BUDGET),
            ProcessUse::ScanIncomplete
        );
    }

    #[test]
    fn renamed_workspace_root_does_not_revalidate_against_a_replacement() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-linux-process-replace-")
            .tempdir_in(root)
            .unwrap();
        let workspace = fixture.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let path = AbsolutePath::try_from_bytes(workspace.as_os_str().as_bytes().to_vec()).unwrap();
        let held = open_bound_root(&path).unwrap();
        fs::rename(&workspace, fixture.path().join("displaced")).unwrap();
        fs::create_dir(&workspace).unwrap();
        assert_eq!(
            revalidate_root(&path, &held).unwrap_err().kind(),
            PortErrorKind::InvalidLayout
        );
    }

    #[test]
    fn process_link_evidence_rejects_each_independent_identity_mismatch() {
        let container = Path::new("/container");
        let inside = Path::new("/container/held");
        assert!(!process_link_evidence_conflicts(
            (10, 20),
            (10, 20),
            inside,
            container
        ));
        assert!(process_link_evidence_conflicts(
            (11, 20),
            (10, 20),
            inside,
            container
        ));
        assert!(process_link_evidence_conflicts(
            (10, 21),
            (10, 20),
            inside,
            container
        ));
        assert!(process_link_evidence_conflicts(
            (10, 20),
            (10, 20),
            Path::new("/outside/held"),
            container
        ));
    }

    #[test]
    fn changed_identity_after_an_empty_scan_is_incomplete_not_no_evidence() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-linux-process-identity-")
            .tempdir_in(root)
            .unwrap();
        let proc_dir = fixture.path().join("process");
        fs::create_dir(&proc_dir).unwrap();
        fs::create_dir(proc_dir.join("fd")).unwrap();
        let self_proc = PathBuf::from(format!("/proc/{}", std::process::id()));
        fs::copy(self_proc.join("status"), proc_dir.join("status")).unwrap();
        fs::copy(self_proc.join("stat"), proc_dir.join("stat")).unwrap();
        let outside = fixture.path().join("outside");
        let container = fixture.path().join("container");
        fs::create_dir(&outside).unwrap();
        fs::create_dir(&container).unwrap();
        symlink(&outside, proc_dir.join("cwd")).unwrap();
        let identity = read_process_identity(&proc_dir).unwrap();
        assert_eq!(
            inspect_process(
                &proc_dir,
                identity,
                &container,
                Instant::now() + SCAN_BUDGET
            ),
            ProcessUse::NoEvidence
        );
        let stale = ProcessIdentity {
            start_ticks: identity.start_ticks + 1,
            ..identity
        };
        assert_eq!(
            inspect_process(&proc_dir, stale, &container, Instant::now() + SCAN_BUDGET),
            ProcessUse::ScanIncomplete
        );
    }

    #[test]
    fn process_scan_path_errors_keep_their_failure_class() {
        for errno in [rustix::io::Errno::LOOP, rustix::io::Errno::NOTDIR] {
            assert_eq!(
                path_error("open process-scan test", errno).kind(),
                PortErrorKind::InvalidLayout
            );
        }
        assert_eq!(
            path_error("open process-scan test", rustix::io::Errno::NOENT).kind(),
            PortErrorKind::NotFound
        );
        assert_eq!(
            path_error("open process-scan test", rustix::io::Errno::ACCESS).kind(),
            PortErrorKind::Io
        );
    }

    #[test]
    fn root_report_requires_both_the_requested_path_and_existing_resolution() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-linux-process-root-report-")
            .tempdir_in(root)
            .unwrap();
        let workspace = fixture.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let path = AbsolutePath::try_from_bytes(workspace.as_os_str().as_bytes().to_vec()).unwrap();
        let report = LinuxPlatformProbe.inspect_path(&path).unwrap();
        assert!(current_root_identity(&report, &path).is_some());
        let other = AbsolutePath::try_from_bytes(
            fixture.path().join("other").as_os_str().as_bytes().to_vec(),
        )
        .unwrap();
        assert!(current_root_identity(&report, &other).is_none());
        let missing = rebuilt_report(
            &report,
            PathResolution::MissingTarget,
            vec![b"missing".to_vec()],
            report.ancestry().to_vec(),
            report.filesystem().clone(),
            report.mount(),
        );
        assert!(current_root_identity(&missing, &path).is_none());
    }

    #[test]
    fn root_revalidation_rejects_each_independent_held_or_mount_change() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-linux-process-root-evidence-")
            .tempdir_in(root)
            .unwrap();
        let workspace = fixture.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let path = AbsolutePath::try_from_bytes(workspace.as_os_str().as_bytes().to_vec()).unwrap();
        let held = open_bound_root(&path).unwrap();
        let current = LinuxPlatformProbe.inspect_path(&path).unwrap();
        let expected = current_root_identity(&current, &path).unwrap();
        let stat = rustix::fs::fstat(&held.fd).unwrap();
        let mount =
            rustix::fs::statx(&held.fd, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID).unwrap();
        let owner = rustix::process::geteuid().as_raw();
        let changed = root_evidence_changed;
        assert!(!changed(
            expected,
            &stat,
            &held.report,
            &current,
            mount.stx_mask,
            mount.stx_mnt_id,
            owner,
        ));

        let other = FileIdentity::new(expected.device(), expected.inode() + 1);
        assert!(changed(
            other,
            &stat,
            &held.report,
            &current,
            mount.stx_mask,
            mount.stx_mnt_id,
            owner
        ));
        assert!(changed(
            expected,
            &stat,
            &held.report,
            &current,
            mount.stx_mask,
            mount.stx_mnt_id,
            owner + 1
        ));

        let mut ancestry = current.ancestry().to_vec();
        let last = ancestry.last_mut().unwrap();
        *last = DirectoryIdentityEvidence::new(
            last.path().clone(),
            FileIdentity::new(last.identity().device(), last.identity().inode() + 1),
        );
        let changed_ancestry = rebuilt_report(
            &current,
            current.resolution(),
            current.missing_components().to_vec(),
            ancestry,
            current.filesystem().clone(),
            current.mount(),
        );
        assert!(changed(
            expected,
            &stat,
            &held.report,
            &changed_ancestry,
            mount.stx_mask,
            mount.stx_mnt_id,
            owner
        ));

        let foreign_filesystem = FileSystemIdentity::new(
            "foreign",
            current.filesystem().fsid(),
            current.filesystem().volume_id().clone(),
        );
        let changed_filesystem = rebuilt_report(
            &current,
            current.resolution(),
            current.missing_components().to_vec(),
            current.ancestry().to_vec(),
            foreign_filesystem,
            current.mount(),
        );
        assert!(changed(
            expected,
            &stat,
            &held.report,
            &changed_filesystem,
            mount.stx_mask,
            mount.stx_mnt_id,
            owner
        ));

        let changed_mount = rebuilt_report(
            &current,
            current.resolution(),
            current.missing_components().to_vec(),
            current.ancestry().to_vec(),
            current.filesystem().clone(),
            MountEvidence::new(current.mount().raw_flags(), current.mount().writable())
                .with_mount_id(current.mount().mount_id().unwrap() + 1),
        );
        assert!(changed(
            expected,
            &stat,
            &held.report,
            &changed_mount,
            mount.stx_mask,
            changed_mount.mount().mount_id().unwrap(),
            owner
        ));
        assert!(changed(
            expected,
            &stat,
            &held.report,
            &current,
            0,
            mount.stx_mnt_id,
            owner
        ));
        assert!(changed(
            expected,
            &stat,
            &held.report,
            &current,
            mount.stx_mask,
            mount.stx_mnt_id + 1,
            owner
        ));
    }
}

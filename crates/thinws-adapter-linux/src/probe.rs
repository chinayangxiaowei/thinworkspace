use std::ffi::OsStr;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::str::FromStr;

use rustix::fs::{Access, AtFlags, Mode, OFlags, StatxFlags};
use thinws_core::{
    AbsolutePath, CandidateEvidence, DirectoryIdentityEvidence, Evidence, FileIdentity,
    FileSystemIdentity, HostCapabilityReport, MaterializationPathReport, MaterializerKind,
    MountEvidence, PathCapabilityReport, PathResolution, ProbeEvidenceDigest, SupportState,
    VolumeId,
};
use thinws_ports::{
    MaterializationPathProbeRequest, MaterializationPathRole, PlatformProbe, PortError,
    PortErrorKind,
};

use crate::ffi::btrfs_fsid;
use crate::mountinfo;

const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

/// Read-only Linux host and path capability probe.
pub struct LinuxPlatformProbe;

impl PlatformProbe for LinuxPlatformProbe {
    fn inspect_host(&self) -> Result<HostCapabilityReport, PortError> {
        let uname = rustix::system::uname();
        Ok(HostCapabilityReport::new(
            uname.sysname().to_string_lossy(),
            uname.version().to_string_lossy(),
            uname.release().to_string_lossy(),
            uname.machine().to_string_lossy(),
            "linux-btrfs",
            SupportState::Unknown,
        ))
    }

    fn inspect_path(&self, path: &AbsolutePath) -> Result<PathCapabilityReport, PortError> {
        inspect_path(path, false)
    }

    fn inspect_materialization_paths(
        &self,
        request: &MaterializationPathProbeRequest,
    ) -> Result<MaterializationPathReport, PortError> {
        let source = inspect_path(request.source(), false).map_err(|error| {
            error.with_materialization_path_role(MaterializationPathRole::Source)
        })?;
        let target = inspect_path(request.target_root(), true).map_err(|error| {
            error.with_materialization_path_role(MaterializationPathRole::TargetRoot)
        })?;
        let staging = inspect_path(request.staging(), false).map_err(|error| {
            error.with_materialization_path_role(MaterializationPathRole::Staging)
        })?;
        let trash = inspect_path(request.trash(), false).map_err(|error| {
            error.with_materialization_path_role(MaterializationPathRole::Trash)
        })?;
        let (cow_state, cow_reasons) = combined_support(&source, &target, &staging, &trash);
        let cow = CandidateEvidence::new(MaterializerKind::BtrfsReflink, cow_state, cow_reasons);
        // Phase 1 Linux has no Full Copy executor or cross-filesystem fallback.
        let copy = CandidateEvidence::new(
            MaterializerKind::FullCopy,
            SupportState::Unsupported,
            vec!["linux_full_copy_not_implemented".to_owned()],
        );
        let digest = digest(&source, &target, &staging, &trash, &cow, &copy);
        Ok(MaterializationPathReport::new(
            source, target, staging, trash, cow, copy, digest,
        ))
    }
}

fn inspect_path(
    path: &AbsolutePath,
    occupied_target_leaf: bool,
) -> Result<PathCapabilityReport, PortError> {
    let mut fd = rustix::fs::open("/", DIRECTORY_FLAGS, Mode::empty())
        .map_err(|error| port_io("open filesystem root", error))?;
    let root = AbsolutePath::try_from_bytes(b"/".to_vec())
        .expect("static filesystem root is a canonical absolute path");
    let root_stat = rustix::fs::fstat(&fd).map_err(|error| port_io("stat root", error))?;
    let mut ancestry = vec![DirectoryIdentityEvidence::new(
        root,
        FileIdentity::new(root_stat.st_dev, root_stat.st_ino),
    )];
    let mut current_path = b"/".to_vec();
    let mut missing = Vec::new();
    let components = if path.as_bytes() == b"/" {
        Vec::new()
    } else {
        path.as_bytes()[1..]
            .split(|byte| *byte == b'/')
            .collect::<Vec<_>>()
    };
    for (index, component) in components.iter().enumerate() {
        let name = OsStr::from_bytes(component);
        match rustix::fs::openat(&fd, name, DIRECTORY_FLAGS, Mode::empty()) {
            Ok(next) => {
                let stat = rustix::fs::fstat(&next)
                    .map_err(|error| port_io("stat path component", error))?;
                if current_path.len() > 1 {
                    current_path.push(b'/');
                }
                current_path.extend_from_slice(component);
                let observed = AbsolutePath::try_from_bytes(current_path.clone())
                    .map_err(|error| port_invalid("record path ancestry", error))?;
                ancestry.push(DirectoryIdentityEvidence::new(
                    observed,
                    FileIdentity::new(stat.st_dev, stat.st_ino),
                ));
                fd = next;
            }
            Err(rustix::io::Errno::NOENT) => {
                match rustix::fs::statat(&fd, name, AtFlags::SYMLINK_NOFOLLOW) {
                    Err(rustix::io::Errno::NOENT) => {
                        missing.extend(components[index..].iter().map(|part| part.to_vec()));
                        break;
                    }
                    Ok(_) => {
                        return Err(PortError::new(
                            PortErrorKind::InvalidLayout,
                            "path component changed during probe",
                        ));
                    }
                    Err(error) => return Err(port_io("confirm missing component", error)),
                }
            }
            Err(rustix::io::Errno::NOTDIR) if index + 1 == components.len() => {
                let leaf = rustix::fs::statat(&fd, name, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(|error| port_io("inspect occupied path leaf", error))?;
                if leaf.st_mode & libc::S_IFMT == libc::S_IFLNK {
                    return Err(PortError::new(
                        if occupied_target_leaf {
                            PortErrorKind::NotEmpty
                        } else {
                            PortErrorKind::InvalidLayout
                        },
                        "path leaf is a symbolic link",
                    ));
                }
                return Err(PortError::new(
                    PortErrorKind::NotEmpty,
                    "path final component is occupied",
                ));
            }
            Err(error) => return Err(open_error(error)),
        }
    }
    let nearest = ancestry
        .last()
        .expect("root ancestry is present")
        .path()
        .clone();
    let resolution = if missing.is_empty() {
        PathResolution::ExistingDirectory
    } else {
        PathResolution::MissingTarget
    };
    let stat = rustix::fs::fstat(&fd).map_err(|error| port_io("stat nearest ancestor", error))?;
    let statfs = rustix::fs::fstatfs(&fd).map_err(|error| port_io("stat filesystem", error))?;
    let mount_id = rustix::fs::statx(&fd, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)
        .ok()
        .and_then(|observed| known_mount_id(observed.stx_mask, observed.stx_mnt_id));
    let mount_kind = mount_id.and_then(|id| mountinfo::filesystem_type(id).ok().flatten());
    let fs_type = filesystem_type_from_evidence(mount_kind.as_deref(), statfs.f_type as u64);
    let volume = if fs_type == "btrfs" {
        match btrfs_fsid(&fd) {
            Ok(bytes) => Evidence::Known(
                VolumeId::from_str(&uuid::Uuid::from_bytes(bytes).hyphenated().to_string())
                    .expect("UUID formatter produces canonical lowercase UUID"),
            ),
            Err(error) => Evidence::Unknown {
                reason: "Btrfs FSID unavailable".to_owned(),
                errno: error.raw_os_error(),
            },
        }
    } else if fs_type == "ext4" {
        match ext4_identity(&fd) {
            Ok(id) => Evidence::Known(id),
            Err(error) => Evidence::Unknown {
                reason: "ext4 filesystem identity unavailable".to_owned(),
                errno: Some(error.raw_os_error()),
            },
        }
    } else {
        Evidence::Unknown {
            reason: "unsupported or unverified Linux filesystem identity".to_owned(),
            errno: None,
        }
    };
    let fsid_words = match &volume {
        Evidence::Known(id) => {
            let bytes = id.as_uuid().into_bytes();
            [
                i32::from_le_bytes(bytes[..4].try_into().expect("UUID word length")),
                i32::from_le_bytes(bytes[4..8].try_into().expect("UUID word length")),
            ]
        }
        Evidence::Unknown { .. } => [stat.st_dev as i32, statfs.f_type as i32],
    };
    let mount_writable = (statfs.f_flags as u64 & libc::ST_RDONLY) == 0;
    let mount = mount_id.map_or(
        MountEvidence::new(statfs.f_flags as u32, mount_writable),
        |id| MountEvidence::new(statfs.f_flags as u32, mount_writable).with_mount_id(id),
    );
    let (read, write) = directory_access(&fd, mount_writable);
    let cow = match fs_type {
        // The ioctl result and per-file flags are not known until execution.
        "btrfs" | "unknown" => SupportState::Unknown,
        _ => SupportState::Unsupported,
    };
    PathCapabilityReport::new(
        path.clone(),
        resolution,
        nearest,
        missing,
        ancestry,
        FileSystemIdentity::new(fs_type, fsid_words, volume),
        mount,
        read,
        write,
        cow,
    )
    .map_err(|error| port_invalid("build path report", error))
}

fn directory_access(directory: &OwnedFd, mount_writable: bool) -> (SupportState, SupportState) {
    let read = access_state(rustix::fs::accessat(
        directory,
        ".",
        Access::READ_OK | Access::EXEC_OK,
        AtFlags::EACCESS,
    ));
    let write = if mount_writable {
        access_state(rustix::fs::accessat(
            directory,
            ".",
            Access::WRITE_OK | Access::EXEC_OK,
            AtFlags::EACCESS,
        ))
    } else {
        SupportState::Unsupported
    };
    (read, write)
}

fn known_mount_id(mask: u32, id: u64) -> Option<u64> {
    (mask & StatxFlags::MNT_ID.bits() != 0).then_some(id)
}

fn filesystem_type_from_evidence(mount_kind: Option<&str>, superblock_magic: u64) -> &str {
    match mount_kind {
        Some("btrfs") if superblock_magic == libc::BTRFS_SUPER_MAGIC as u64 => "btrfs",
        Some("ext4") if superblock_magic == libc::EXT4_SUPER_MAGIC as u64 => "ext4",
        Some("btrfs" | "ext4") => "unknown",
        Some(other) => other,
        None => "unknown",
    }
}

fn ext4_identity(directory: &OwnedFd) -> Result<VolumeId, rustix::io::Errno> {
    let statvfs = rustix::fs::fstatvfs(directory)?;
    if statvfs.f_fsid == 0 {
        return Err(rustix::io::Errno::INVAL);
    }
    Ok(ext4_volume_id(statvfs.f_fsid))
}

fn ext4_volume_id(fsid: u64) -> VolumeId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"thinws-linux-ext4-identity-v1\0");
    hasher.update(&fsid.to_le_bytes());
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    VolumeId::from_str(&uuid::Uuid::from_bytes(bytes).hyphenated().to_string())
        .expect("UUID formatter produces canonical lowercase UUID")
}

fn access_state(result: Result<(), rustix::io::Errno>) -> SupportState {
    match result {
        Ok(()) => SupportState::Supported,
        Err(rustix::io::Errno::ACCESS | rustix::io::Errno::PERM) => SupportState::Unsupported,
        Err(_) => SupportState::Unknown,
    }
}

fn open_error(error: rustix::io::Errno) -> PortError {
    let kind = match error {
        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => PortErrorKind::InvalidLayout,
        rustix::io::Errno::ACCESS | rustix::io::Errno::PERM => PortErrorKind::Unavailable,
        _ => PortErrorKind::Io,
    };
    PortError::new(kind, "open path component without following links").with_source(error)
}

fn port_io(
    operation: &'static str,
    error: impl std::error::Error + Send + Sync + 'static,
) -> PortError {
    PortError::new(PortErrorKind::Io, operation).with_source(error)
}

fn port_invalid(
    operation: &'static str,
    error: impl std::error::Error + Send + Sync + 'static,
) -> PortError {
    PortError::new(PortErrorKind::InvalidLayout, operation).with_source(error)
}

fn combined_support(
    source: &PathCapabilityReport,
    target: &PathCapabilityReport,
    staging: &PathCapabilityReport,
    trash: &PathCapabilityReport,
) -> (SupportState, Vec<String>) {
    let paths = [source, target, staging, trash];
    let mut unsupported = Vec::new();
    let mut unknown = Vec::new();
    if source.resolution() != PathResolution::ExistingDirectory {
        unsupported.push("source_missing".to_owned());
    }
    for (role, path) in [("staging", staging), ("trash", trash)] {
        if path.resolution() != PathResolution::ExistingDirectory {
            unknown.push(format!("{role}_not_yet_created"));
        }
    }
    for (left_name, left, right_name, right) in [
        ("source", source, "target", target),
        ("source", source, "staging", staging),
        ("source", source, "trash", trash),
        ("target", target, "staging", staging),
        ("target", target, "trash", trash),
        ("staging", staging, "trash", trash),
    ] {
        if overlap(left, right) {
            unsupported.push(format!("{left_name}_{right_name}_overlap"));
        }
    }
    for (name, state) in [
        ("source_read", source.readability()),
        ("target_write", target.writability()),
        ("staging_write", staging.writability()),
        ("trash_write", trash.writability()),
    ] {
        match state {
            SupportState::Supported => {}
            SupportState::Unsupported => unsupported.push(format!("{name}_unsupported")),
            SupportState::Unknown => unknown.push(format!("{name}_unknown")),
        }
    }
    let first_mount = source.mount().mount_id();
    let first_volume = source.filesystem().volume_id().known().copied();
    for path in paths {
        match path.filesystem().type_name() {
            "btrfs" => {}
            "unknown" => unknown.push("filesystem_unknown".to_owned()),
            _ => unsupported.push("non_btrfs_path".to_owned()),
        }
        match (first_mount, path.mount().mount_id()) {
            (Some(a), Some(b)) if a != b => unsupported.push("different_mount".to_owned()),
            (None, _) | (_, None) => unknown.push("mount_unknown".to_owned()),
            _ => {}
        }
        match (first_volume, path.filesystem().volume_id().known().copied()) {
            (Some(a), Some(b)) if a != b => unsupported.push("different_volume".to_owned()),
            (None, _) | (_, None) => unknown.push("volume_unknown".to_owned()),
            _ => {}
        }
        match path.cow_clone() {
            SupportState::Supported => {}
            SupportState::Unsupported => {
                unsupported.push("clone_capability_unsupported".to_owned())
            }
            SupportState::Unknown => unknown.push("clone_capability_unknown".to_owned()),
        }
    }
    let has_unsupported = !unsupported.is_empty();
    unsupported.extend(unknown.iter().cloned());
    unsupported.sort();
    unsupported.dedup();
    unknown.sort();
    unknown.dedup();
    if has_unsupported {
        (SupportState::Unsupported, unsupported)
    } else if !unknown.is_empty() {
        (SupportState::Unknown, unknown)
    } else {
        (SupportState::Supported, Vec::new())
    }
}

fn overlap(left: &PathCapabilityReport, right: &PathCapabilityReport) -> bool {
    fn contains(parent: &[u8], child: &[u8]) -> bool {
        parent == child
            || (child.starts_with(parent)
                && (parent == b"/" || child.get(parent.len()) == Some(&b'/')))
    }
    let a = left.requested_path().as_bytes();
    let b = right.requested_path().as_bytes();
    if contains(a, b) || contains(b, a) {
        return true;
    }
    let leaf = |path: &PathCapabilityReport| {
        (path.resolution() == PathResolution::ExistingDirectory)
            .then(|| {
                path.ancestry()
                    .last()
                    .map(DirectoryIdentityEvidence::identity)
            })
            .flatten()
    };
    leaf(left).is_some_and(|id| right.ancestry().iter().any(|item| item.identity() == id))
        || leaf(right).is_some_and(|id| left.ancestry().iter().any(|item| item.identity() == id))
}

fn digest(
    source: &PathCapabilityReport,
    target: &PathCapabilityReport,
    staging: &PathCapabilityReport,
    trash: &PathCapabilityReport,
    cow: &CandidateEvidence,
    copy: &CandidateEvidence,
) -> ProbeEvidenceDigest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"thinws-linux-btrfs-probe-v1\0");
    for (role, report) in [
        (b"source".as_slice(), source),
        (b"target".as_slice(), target),
        (b"staging".as_slice(), staging),
        (b"trash".as_slice(), trash),
    ] {
        digest_bytes(&mut hasher, role);
        digest_bytes(&mut hasher, report.requested_path().as_bytes());
        hasher.update(&[match report.resolution() {
            PathResolution::ExistingDirectory => 1,
            PathResolution::MissingTarget => 2,
        }]);
        digest_bytes(&mut hasher, report.nearest_existing_ancestor().as_bytes());
        digest_count(&mut hasher, report.missing_components().len());
        for component in report.missing_components() {
            digest_bytes(&mut hasher, component);
        }
        digest_count(&mut hasher, report.ancestry().len());
        for entry in report.ancestry() {
            digest_bytes(&mut hasher, entry.path().as_bytes());
            hasher.update(&entry.identity().device().to_le_bytes());
            hasher.update(&entry.identity().inode().to_le_bytes());
        }
        digest_bytes(&mut hasher, report.filesystem().type_name().as_bytes());
        for word in report.filesystem().fsid() {
            hasher.update(&word.to_le_bytes());
        }
        match report.filesystem().volume_id() {
            Evidence::Known(volume) => {
                hasher.update(&[1]);
                digest_bytes(&mut hasher, volume.to_string().as_bytes());
            }
            Evidence::Unknown { reason, errno } => {
                hasher.update(&[2]);
                digest_bytes(&mut hasher, reason.as_bytes());
                digest_optional_i32(&mut hasher, *errno);
            }
        }
        hasher.update(&report.mount().raw_flags().to_le_bytes());
        hasher.update(&[u8::from(report.mount().writable())]);
        match report.mount().mount_id() {
            Some(id) => {
                hasher.update(&[1]);
                hasher.update(&id.to_le_bytes());
            }
            None => {
                hasher.update(&[0]);
            }
        }
        for state in [
            report.readability(),
            report.writability(),
            report.cow_clone(),
        ] {
            hasher.update(&[state_byte(state)]);
        }
    }
    for candidate in [cow, copy] {
        hasher.update(&[match candidate.kind() {
            MaterializerKind::ApfsFileClone => 1,
            MaterializerKind::FullCopy => 2,
            MaterializerKind::BtrfsReflink => 3,
        }]);
        hasher.update(&[state_byte(candidate.state())]);
        digest_count(&mut hasher, candidate.reasons().len());
        for reason in candidate.reasons() {
            digest_bytes(&mut hasher, reason.as_bytes());
        }
    }
    ProbeEvidenceDigest::new(*hasher.finalize().as_bytes())
}

fn digest_count(hasher: &mut blake3::Hasher, count: usize) {
    hasher.update(&(count as u64).to_le_bytes());
}

fn digest_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    digest_count(hasher, bytes.len());
    hasher.update(bytes);
}

fn digest_optional_i32(hasher: &mut blake3::Hasher, value: Option<i32>) {
    match value {
        Some(value) => {
            hasher.update(&[1]);
            hasher.update(&value.to_le_bytes());
        }
        None => {
            hasher.update(&[0]);
        }
    }
}

const fn state_byte(value: SupportState) -> u8 {
    match value {
        SupportState::Supported => 1,
        SupportState::Unsupported => 2,
        SupportState::Unknown => 3,
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    fn path(value: &str) -> AbsolutePath {
        AbsolutePath::try_from_bytes(value.as_bytes().to_vec()).expect("valid test path")
    }

    fn known_volume() -> Evidence<VolumeId> {
        Evidence::Known(
            VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").expect("valid test UUID"),
        )
    }

    fn report(
        name: &str,
        resolution: PathResolution,
        mount_id: Option<u64>,
        volume: Evidence<VolumeId>,
    ) -> PathCapabilityReport {
        report_with_type(name, resolution, mount_id, volume, "btrfs")
    }

    fn report_with_type(
        name: &str,
        resolution: PathResolution,
        mount_id: Option<u64>,
        volume: Evidence<VolumeId>,
        filesystem_type: &str,
    ) -> PathCapabilityReport {
        let requested = path(&format!("/sample/{name}"));
        let mut ancestry = vec![
            DirectoryIdentityEvidence::new(path("/"), FileIdentity::new(1, 1)),
            DirectoryIdentityEvidence::new(path("/sample"), FileIdentity::new(1, 2)),
        ];
        let (nearest, missing) = match resolution {
            PathResolution::ExistingDirectory => {
                let inode = match name {
                    "source" => 3,
                    "target" => 4,
                    "staging" => 5,
                    "trash" => 6,
                    _ => panic!("unknown test path"),
                };
                ancestry.push(DirectoryIdentityEvidence::new(
                    requested.clone(),
                    FileIdentity::new(1, inode),
                ));
                (requested.clone(), Vec::new())
            }
            PathResolution::MissingTarget => (path("/sample"), vec![name.as_bytes().to_vec()]),
        };
        let mount = mount_id.map_or(MountEvidence::new(0, true), |id| {
            MountEvidence::new(0, true).with_mount_id(id)
        });
        PathCapabilityReport::new(
            requested,
            resolution,
            nearest,
            missing,
            ancestry,
            FileSystemIdentity::new(filesystem_type, [1, 2], volume),
            mount,
            SupportState::Supported,
            SupportState::Supported,
            if filesystem_type == "unknown" {
                SupportState::Unknown
            } else {
                SupportState::Supported
            },
        )
        .expect("consistent test report")
    }

    fn same_mount_reports() -> [PathCapabilityReport; 4] {
        ["source", "target", "staging", "trash"].map(|name| {
            report(
                name,
                PathResolution::ExistingDirectory,
                Some(7),
                known_volume(),
            )
        })
    }

    fn support(reports: &[PathCapabilityReport; 4]) -> (SupportState, Vec<String>) {
        combined_support(&reports[0], &reports[1], &reports[2], &reports[3])
    }

    #[test]
    fn same_btrfs_mount_and_volume_support_reflink() {
        assert_eq!(
            support(&same_mount_reports()),
            (SupportState::Supported, vec![])
        );
    }

    #[test]
    fn absent_source_is_unsupported() {
        let mut reports = same_mount_reports();
        reports[0] = report(
            "source",
            PathResolution::MissingTarget,
            Some(7),
            known_volume(),
        );
        assert_eq!(
            support(&reports),
            (SupportState::Unsupported, vec!["source_missing".to_owned()])
        );
    }

    #[test]
    fn unknown_mount_identity_does_not_claim_support() {
        let mut reports = same_mount_reports();
        reports[1] = report(
            "target",
            PathResolution::ExistingDirectory,
            None,
            known_volume(),
        );
        assert_eq!(
            support(&reports),
            (SupportState::Unknown, vec!["mount_unknown".to_owned()])
        );
    }

    #[test]
    fn different_mount_is_unsupported_even_with_matching_volume() {
        let mut reports = same_mount_reports();
        reports[1] = report(
            "target",
            PathResolution::ExistingDirectory,
            Some(8),
            known_volume(),
        );
        assert_eq!(
            support(&reports),
            (
                SupportState::Unsupported,
                vec!["different_mount".to_owned()]
            )
        );
    }

    #[test]
    fn unknown_volume_identity_does_not_claim_support() {
        let mut reports = same_mount_reports();
        reports[1] = report(
            "target",
            PathResolution::ExistingDirectory,
            Some(7),
            Evidence::Unknown {
                reason: "test unavailable".to_owned(),
                errno: None,
            },
        );
        assert_eq!(
            support(&reports),
            (SupportState::Unknown, vec!["volume_unknown".to_owned()])
        );
    }

    #[test]
    fn unknown_filesystem_type_keeps_probe_result_unknown() {
        let mut reports = same_mount_reports();
        reports[1] = report_with_type(
            "target",
            PathResolution::ExistingDirectory,
            Some(7),
            Evidence::Unknown {
                reason: "mountinfo unavailable".to_owned(),
                errno: None,
            },
            "unknown",
        );
        assert_eq!(
            support(&reports),
            (
                SupportState::Unknown,
                vec![
                    "clone_capability_unknown".to_owned(),
                    "filesystem_unknown".to_owned(),
                    "volume_unknown".to_owned(),
                ],
            )
        );
    }

    #[test]
    fn known_ext4_path_stays_unsupported() {
        let mut reports = same_mount_reports();
        reports[1] = report_with_type(
            "target",
            PathResolution::ExistingDirectory,
            Some(7),
            known_volume(),
            "ext4",
        );
        assert_eq!(
            support(&reports),
            (SupportState::Unsupported, vec!["non_btrfs_path".to_owned()])
        );
    }

    #[test]
    fn different_volume_is_unsupported_even_with_matching_mount() {
        let mut reports = same_mount_reports();
        reports[1] = report(
            "target",
            PathResolution::ExistingDirectory,
            Some(7),
            Evidence::Known(
                VolumeId::from_str("550e8400-e29b-41d4-a716-446655440001")
                    .expect("valid test UUID"),
            ),
        );
        assert_eq!(
            support(&reports),
            (
                SupportState::Unsupported,
                vec!["different_volume".to_owned()]
            )
        );
    }

    #[test]
    fn occupied_target_leaf_and_intermediate_links_have_distinct_failures() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs mount");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-linux-probe-file-")
            .tempdir_in(root)
            .unwrap();
        let occupied = fixture.path().join("occupied");
        fs::write(&occupied, b"content").unwrap();
        let absolute = |candidate: &std::path::Path| {
            AbsolutePath::try_from_bytes(candidate.as_os_str().as_bytes().to_vec()).unwrap()
        };
        assert_eq!(
            LinuxPlatformProbe
                .inspect_path(&absolute(&occupied))
                .unwrap_err()
                .kind(),
            PortErrorKind::NotEmpty
        );
        assert_eq!(
            LinuxPlatformProbe
                .inspect_path(&absolute(&occupied.join("child")))
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        let link = fixture.path().join("occupied-link");
        symlink("occupied", &link).unwrap();
        assert_eq!(
            inspect_path(&absolute(&link), true).unwrap_err().kind(),
            PortErrorKind::NotEmpty
        );
        assert_eq!(
            LinuxPlatformProbe
                .inspect_path(&absolute(&link))
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        assert_eq!(
            LinuxPlatformProbe
                .inspect_path(&absolute(&link.join("child")))
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
    }

    #[test]
    fn overlap_rejects_lexical_nesting_and_identity_aliases_but_not_siblings() {
        let source = report(
            "source",
            PathResolution::ExistingDirectory,
            Some(7),
            known_volume(),
        );
        let child = PathCapabilityReport::new(
            path("/sample/source/child"),
            PathResolution::MissingTarget,
            source.requested_path().clone(),
            vec![b"child".to_vec()],
            source.ancestry().to_vec(),
            source.filesystem().clone(),
            source.mount(),
            SupportState::Supported,
            SupportState::Supported,
            SupportState::Supported,
        )
        .unwrap();
        let sibling = report(
            "source-backup",
            PathResolution::MissingTarget,
            Some(7),
            known_volume(),
        );
        assert!(overlap(&source, &child));
        assert!(overlap(&child, &source));
        assert!(!overlap(&source, &sibling));

        // A concurrent disappearance can leave the separately probed source
        // and target with inconsistent ancestry. Literal nesting must still
        // reject this combination without relying on a shared leaf identity.
        let lexical_only_child = PathCapabilityReport::new(
            path("/sample/source/child"),
            PathResolution::MissingTarget,
            path("/sample"),
            vec![b"source".to_vec(), b"child".to_vec()],
            source.ancestry()[..2].to_vec(),
            source.filesystem().clone(),
            source.mount(),
            SupportState::Supported,
            SupportState::Supported,
            SupportState::Supported,
        )
        .unwrap();
        assert!(overlap(&source, &lexical_only_child));
        assert!(overlap(&lexical_only_child, &source));
        assert!(overlap(&sibling, &sibling));
        let synthetic_root = PathCapabilityReport::new(
            path("/"),
            PathResolution::ExistingDirectory,
            path("/"),
            vec![],
            vec![DirectoryIdentityEvidence::new(
                path("/"),
                FileIdentity::new(99, 99),
            )],
            source.filesystem().clone(),
            source.mount(),
            SupportState::Supported,
            SupportState::Supported,
            SupportState::Supported,
        )
        .unwrap();
        assert!(overlap(&synthetic_root, &sibling));

        let mut alias_ancestry = source.ancestry().to_vec();
        alias_ancestry
            .last_mut()
            .unwrap()
            .clone_from(&DirectoryIdentityEvidence::new(
                path("/sample/alias"),
                source.ancestry().last().unwrap().identity(),
            ));
        let alias = PathCapabilityReport::new(
            path("/sample/alias"),
            PathResolution::ExistingDirectory,
            path("/sample/alias"),
            vec![],
            alias_ancestry,
            source.filesystem().clone(),
            source.mount(),
            SupportState::Supported,
            SupportState::Supported,
            SupportState::Supported,
        )
        .unwrap();
        assert!(overlap(&source, &alias));
        assert!(overlap(&alias, &source));

        let alias_child = PathCapabilityReport::new(
            path("/sample/alias/child"),
            PathResolution::MissingTarget,
            path("/sample/alias"),
            vec![b"child".to_vec()],
            alias.ancestry().to_vec(),
            source.filesystem().clone(),
            source.mount(),
            SupportState::Supported,
            SupportState::Supported,
            SupportState::Supported,
        )
        .unwrap();
        assert!(overlap(&source, &alias_child));
        assert!(overlap(&alias_child, &source));

        let reports = same_mount_reports();
        let (state, reasons) =
            combined_support(&source, &lexical_only_child, &reports[2], &reports[3]);
        assert_eq!(state, SupportState::Unsupported);
        assert!(
            reasons
                .iter()
                .any(|reason| reason == "source_target_overlap")
        );
    }

    #[test]
    fn probe_digest_commits_to_reason_boundaries_unknown_errno_and_support_state() {
        let reports = same_mount_reports();
        let copy = CandidateEvidence::new(
            MaterializerKind::FullCopy,
            SupportState::Unsupported,
            vec![],
        );
        let candidate =
            |state, reasons| CandidateEvidence::new(MaterializerKind::BtrfsReflink, state, reasons);
        let digest_for = |source: &PathCapabilityReport, cow: &CandidateEvidence| {
            digest(source, &reports[1], &reports[2], &reports[3], cow, &copy)
        };
        let split_a = candidate(SupportState::Unknown, vec!["a".to_owned(), "bc".to_owned()]);
        let split_b = candidate(SupportState::Unknown, vec!["ab".to_owned(), "c".to_owned()]);
        assert_ne!(
            digest_for(&reports[0], &split_a),
            digest_for(&reports[0], &split_b)
        );
        assert_ne!(
            digest_for(&reports[0], &candidate(SupportState::Supported, vec![])),
            digest_for(&reports[0], &candidate(SupportState::Unsupported, vec![]))
        );
        let unknown = |errno| {
            report(
                "source",
                PathResolution::ExistingDirectory,
                Some(7),
                Evidence::Unknown {
                    reason: "test unavailable".to_owned(),
                    errno: Some(errno),
                },
            )
        };
        assert_ne!(
            digest_for(&unknown(3), &split_a),
            digest_for(&unknown(4), &split_a)
        );
    }

    #[test]
    fn mount_and_superblock_evidence_must_agree_before_naming_a_filesystem() {
        assert_eq!(known_mount_id(0, 41), None);
        assert_eq!(known_mount_id(StatxFlags::MNT_ID.bits(), 41), Some(41));
        assert_eq!(
            filesystem_type_from_evidence(Some("btrfs"), libc::BTRFS_SUPER_MAGIC as u64),
            "btrfs"
        );
        assert_eq!(
            filesystem_type_from_evidence(Some("btrfs"), libc::EXT4_SUPER_MAGIC as u64),
            "unknown"
        );
        assert_eq!(
            filesystem_type_from_evidence(Some("ext4"), libc::EXT4_SUPER_MAGIC as u64),
            "ext4"
        );
        assert_eq!(
            filesystem_type_from_evidence(Some("ext4"), libc::BTRFS_SUPER_MAGIC as u64),
            "unknown"
        );
    }

    #[test]
    fn ext4_identity_keeps_the_fixed_uuid_version_and_variant_bits() {
        let fsid = 0x1234_5678_9abc_def0_u64;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"thinws-linux-ext4-identity-v1\0");
        hasher.update(&fsid.to_le_bytes());
        let hashed = hasher.finalize();
        let actual = ext4_volume_id(fsid).as_uuid().into_bytes();
        assert_eq!(actual[6], (hashed.as_bytes()[6] & 0x0f) | 0x80);
        assert_eq!(actual[8], (hashed.as_bytes()[8] & 0x3f) | 0x80);
    }

    #[test]
    fn private_path_probe_checks_actual_read_write_and_search_permissions() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs mount");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-linux-probe-access-")
            .tempdir_in(root)
            .unwrap();
        let readonly = fixture.path().join("readonly");
        let no_search = fixture.path().join("no-search");
        fs::create_dir(&readonly).unwrap();
        fs::create_dir(&no_search).unwrap();
        fs::set_permissions(&readonly, fs::Permissions::from_mode(0o500)).unwrap();
        fs::set_permissions(&no_search, fs::Permissions::from_mode(0o400)).unwrap();
        let absolute = |candidate: &std::path::Path| {
            AbsolutePath::try_from_bytes(candidate.as_os_str().as_bytes().to_vec()).unwrap()
        };
        let readonly_report = LinuxPlatformProbe.inspect_path(&absolute(&readonly));
        let no_search_report = LinuxPlatformProbe.inspect_path(&absolute(&no_search));
        fs::set_permissions(&readonly, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&no_search, fs::Permissions::from_mode(0o700)).unwrap();
        let readonly_report = readonly_report.unwrap();
        assert_eq!(readonly_report.readability(), SupportState::Supported);
        assert_eq!(readonly_report.writability(), SupportState::Unsupported);
        let no_search_report = no_search_report.unwrap();
        assert_eq!(no_search_report.readability(), SupportState::Unsupported);
        assert_eq!(no_search_report.writability(), SupportState::Unsupported);
    }

    #[test]
    fn already_open_search_only_directory_is_not_reported_readable_or_writable() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs mount");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-linux-probe-search-only-")
            .tempdir_in(root)
            .unwrap();
        let directory = fixture.path().join("search-only");
        fs::create_dir(&directory).unwrap();
        let fd = rustix::fs::open(&directory, DIRECTORY_FLAGS, Mode::empty()).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o100)).unwrap();
        let exists = rustix::fs::accessat(&fd, ".", Access::EXISTS, AtFlags::EACCESS);
        let observed = directory_access(&fd, true);
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();

        assert!(
            exists.is_ok(),
            "search permission should permit the existence probe"
        );
        assert_eq!(observed.0, SupportState::Unsupported);
        assert_eq!(observed.1, SupportState::Unsupported);
    }

    #[test]
    fn path_open_error_keeps_link_layout_and_access_denial_distinct() {
        assert_eq!(
            open_error(rustix::io::Errno::LOOP).kind(),
            PortErrorKind::InvalidLayout
        );
        assert_eq!(
            open_error(rustix::io::Errno::NOTDIR).kind(),
            PortErrorKind::InvalidLayout
        );
        assert_eq!(
            open_error(rustix::io::Errno::ACCESS).kind(),
            PortErrorKind::Unavailable
        );
        assert_eq!(
            open_error(rustix::io::Errno::PERM).kind(),
            PortErrorKind::Unavailable
        );
        assert_eq!(
            open_error(rustix::io::Errno::NOENT).kind(),
            PortErrorKind::Io
        );
    }
}

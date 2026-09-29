use std::ffi::OsStr;
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
        inspect_path(path)
    }

    fn inspect_materialization_paths(
        &self,
        request: &MaterializationPathProbeRequest,
    ) -> Result<MaterializationPathReport, PortError> {
        let source = inspect_path(request.source()).map_err(|error| {
            error.with_materialization_path_role(MaterializationPathRole::Source)
        })?;
        let target = inspect_path(request.target_root()).map_err(|error| {
            error.with_materialization_path_role(MaterializationPathRole::TargetRoot)
        })?;
        let staging = inspect_path(request.staging()).map_err(|error| {
            error.with_materialization_path_role(MaterializationPathRole::Staging)
        })?;
        let trash = inspect_path(request.trash()).map_err(|error| {
            error.with_materialization_path_role(MaterializationPathRole::Trash)
        })?;
        let (cow_state, cow_reasons) = combined_support(&source, &target, &staging, &trash, true);
        let (copy_state, copy_reasons) =
            combined_support(&source, &target, &staging, &trash, false);
        let cow = CandidateEvidence::new(MaterializerKind::BtrfsReflink, cow_state, cow_reasons);
        let copy = CandidateEvidence::new(MaterializerKind::FullCopy, copy_state, copy_reasons);
        let digest = digest(&source, &target, &staging, &trash, &cow, &copy);
        Ok(MaterializationPathReport::new(
            source, target, staging, trash, cow, copy, digest,
        ))
    }
}

fn inspect_path(path: &AbsolutePath) -> Result<PathCapabilityReport, PortError> {
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
                        PortErrorKind::InvalidLayout,
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
    let fs_type = if statfs.f_type == libc::BTRFS_SUPER_MAGIC as _ {
        "btrfs"
    } else if statfs.f_type == libc::EXT4_SUPER_MAGIC as _ {
        "ext4"
    } else {
        "other"
    };
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
    } else {
        Evidence::Unknown {
            reason: "non-Btrfs filesystem has no Btrfs FSID".to_owned(),
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
    let mount_id = rustix::fs::statx(&fd, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)
        .ok()
        .and_then(|observed| {
            (observed.stx_mask & StatxFlags::MNT_ID.bits() != 0).then_some(observed.stx_mnt_id)
        });
    let mount_writable = (statfs.f_flags as u64 & libc::ST_RDONLY) == 0;
    let mount = mount_id.map_or(
        MountEvidence::new(statfs.f_flags as u32, mount_writable),
        |id| MountEvidence::new(statfs.f_flags as u32, mount_writable).with_mount_id(id),
    );
    let read = access_state(rustix::fs::accessat(
        &fd,
        "",
        Access::READ_OK | Access::EXEC_OK,
        AtFlags::EMPTY_PATH | AtFlags::EACCESS,
    ));
    let write = if mount_writable {
        access_state(rustix::fs::accessat(
            &fd,
            "",
            Access::WRITE_OK | Access::EXEC_OK,
            AtFlags::EMPTY_PATH | AtFlags::EACCESS,
        ))
    } else {
        SupportState::Unsupported
    };
    let cow = if fs_type == "btrfs" && volume.known().is_some() && mount_id.is_some() {
        // The ioctl result and per-file flags are not known until execution.
        SupportState::Unknown
    } else if fs_type == "btrfs" {
        SupportState::Unknown
    } else {
        SupportState::Unsupported
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
    cow: bool,
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
    if cow {
        let first_mount = source.mount().mount_id();
        let first_volume = source.filesystem().volume_id().known().copied();
        for path in paths {
            if path.filesystem().type_name() != "btrfs" {
                unsupported.push("non_btrfs_path".to_owned());
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

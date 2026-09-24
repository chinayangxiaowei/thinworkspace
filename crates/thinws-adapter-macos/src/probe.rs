use std::io;
use std::os::fd::OwnedFd;

use thinws_core::{
    AbsolutePath, CandidateEvidence, DirectoryIdentityEvidence, Evidence, FileIdentity,
    FileSystemIdentity, HostCapabilityReport, MaterializationPathReport, MaterializerKind,
    MountEvidence, PathCapabilityReport, PathResolution, ProbeEvidenceDigest, SupportState,
};
use thinws_ports::{MaterializationPathProbeRequest, PlatformProbe, PortError, PortErrorKind};

use crate::MacOsHostAdapter;
use crate::ffi::{
    RawCloneCapability, RawFileKind, RawNodeMetadata, c_string, clone_capability,
    effective_read_search_access, effective_write_search_access, file_system_metadata,
    host_identity, node_metadata, node_metadata_at, open_directory_at, open_root_directory,
    product_version, volume_uuid,
};
use crate::volume::decode_volume_id;

const ADAPTER_NAME: &str = "macos-apfs";

impl PlatformProbe for MacOsHostAdapter {
    fn inspect_host(&self) -> Result<HostCapabilityReport, PortError> {
        let identity = host_identity().map_err(|error| probe_io("inspect host identity", error))?;
        let version =
            product_version().map_err(|error| probe_io("inspect macOS product version", error))?;
        let root =
            open_root_directory().map_err(|error| probe_io("open filesystem root", error))?;
        let clone = clone_support(&root);
        Ok(HostCapabilityReport::new(
            identity.system_name,
            version,
            identity.kernel_release,
            identity.architecture,
            ADAPTER_NAME,
            clone,
        ))
    }

    fn inspect_path(&self, path: &AbsolutePath) -> Result<PathCapabilityReport, PortError> {
        inspect_path(path)
    }

    fn inspect_materialization_paths(
        &self,
        request: &MaterializationPathProbeRequest,
    ) -> Result<MaterializationPathReport, PortError> {
        let source = inspect_path(request.source())?;
        let target_root = inspect_path(request.target_root())?;
        let staging = inspect_path(request.staging())?;
        let trash = inspect_path(request.trash())?;
        let (state, reasons) = combined_clone_support(&source, &target_root, &staging, &trash);
        let candidate = CandidateEvidence::new(MaterializerKind::ApfsFileClone, state, reasons);
        let digest = probe_digest(&source, &target_root, &staging, &trash, &candidate);
        Ok(MaterializationPathReport::new(
            source,
            target_root,
            staging,
            trash,
            candidate,
            digest,
        ))
    }
}

fn inspect_path(path: &AbsolutePath) -> Result<PathCapabilityReport, PortError> {
    let components = if path.as_bytes() == b"/" {
        Vec::new()
    } else {
        path.as_bytes()[1..]
            .split(|byte| *byte == b'/')
            .map(<[u8]>::to_vec)
            .collect::<Vec<_>>()
    };
    let mut current = open_root_directory().map_err(|error| probe_io("open path root", error))?;
    let root_metadata =
        node_metadata(&current).map_err(|error| probe_io("inspect path root", error))?;
    require_directory(root_metadata, "require directory path root")?;
    let root_path = AbsolutePath::try_from_bytes(b"/".to_vec())
        .expect("the filesystem root is a canonical absolute path");
    let mut ancestry = vec![DirectoryIdentityEvidence::new(
        root_path,
        identity(root_metadata),
    )];
    let mut current_path = Vec::from(b"/".as_slice());
    let mut missing_components = Vec::new();

    for (index, component) in components.iter().enumerate() {
        let name =
            c_string(component).map_err(|error| probe_invalid("encode path component", error))?;
        match open_directory_at(&current, &name) {
            Ok(next) => {
                let metadata = node_metadata(&next)
                    .map_err(|error| probe_io("inspect path component", error))?;
                require_directory(metadata, "require directory path component")?;
                append_component(&mut current_path, component);
                let observed_path = AbsolutePath::try_from_bytes(current_path.clone())
                    .map_err(|error| probe_invalid("record path ancestry", error))?;
                ancestry.push(DirectoryIdentityEvidence::new(
                    observed_path,
                    identity(metadata),
                ));
                current = next;
            }
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {
                match node_metadata_at(&current, &name) {
                    Err(metadata_error) if metadata_error.raw_os_error() == Some(libc::ENOENT) => {
                        missing_components.extend(components[index..].iter().cloned());
                        break;
                    }
                    Ok(_) => {
                        return Err(PortError::new(
                            PortErrorKind::InvalidLayout,
                            "reject non-directory path component",
                        ));
                    }
                    Err(metadata_error) => {
                        return Err(probe_io("inspect missing path component", metadata_error));
                    }
                }
            }
            Err(error) => return Err(open_component_error(error)),
        }
    }

    let nearest_existing_ancestor = ancestry
        .last()
        .expect("root ancestry is always present")
        .path()
        .clone();
    let resolution = if missing_components.is_empty() {
        PathResolution::ExistingDirectory
    } else {
        PathResolution::MissingTarget
    };
    let raw_filesystem = file_system_metadata(&current)
        .map_err(|error| probe_io("inspect path filesystem", error))?;
    let volume_id = volume_evidence(&current);
    let filesystem =
        FileSystemIdentity::new(raw_filesystem.type_name, raw_filesystem.fsid, volume_id);
    let mount_writable = raw_filesystem.mount_flags & (libc::MNT_RDONLY as u32) == 0;
    let readability = access_support(effective_read_search_access(&current));
    let writability = if mount_writable {
        access_support(effective_write_search_access(&current))
    } else {
        SupportState::Unsupported
    };
    let apfs_clone = path_clone_support(&current, &filesystem);

    PathCapabilityReport::new(
        path.clone(),
        resolution,
        nearest_existing_ancestor,
        missing_components,
        ancestry,
        filesystem,
        MountEvidence::new(raw_filesystem.mount_flags, mount_writable),
        readability,
        writability,
        apfs_clone,
    )
    .map_err(|error| probe_invalid("build path capability report", error))
}

fn combined_clone_support(
    source: &PathCapabilityReport,
    target: &PathCapabilityReport,
    staging: &PathCapabilityReport,
    trash: &PathCapabilityReport,
) -> (SupportState, Vec<String>) {
    let paths = [source, target, staging, trash];
    let mut reasons = Vec::new();
    let mut unsupported = false;
    let mut unknown = false;

    if source.resolution() != PathResolution::ExistingDirectory {
        unsupported = true;
        reasons.push("source_missing".to_owned());
    }
    for (name, report) in [("staging", staging), ("trash", trash)] {
        if report.resolution() != PathResolution::ExistingDirectory {
            unsupported = true;
            reasons.push(format!("{name}_missing"));
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
        if roots_overlap(left, right) {
            unsupported = true;
            reasons.push(format!("{left_name}_{right_name}_overlap"));
        }
    }

    let mut volume = None;
    for report in paths {
        if report.filesystem().type_name() != "apfs" {
            unsupported = true;
            reasons.push("non_apfs_path".to_owned());
        }
        match report.filesystem().volume_id().known().copied() {
            Some(observed) => match volume {
                Some(expected) if expected != observed => {
                    unsupported = true;
                    reasons.push("different_volume".to_owned());
                }
                None => volume = Some(observed),
                _ => {}
            },
            None => {
                unknown = true;
                reasons.push("volume_unknown".to_owned());
            }
        }
        match report.apfs_clone() {
            SupportState::Supported => {}
            SupportState::Unsupported => {
                unsupported = true;
                reasons.push("clone_capability_unsupported".to_owned());
            }
            SupportState::Unknown => {
                unknown = true;
                reasons.push("clone_capability_unknown".to_owned());
            }
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
            SupportState::Unsupported => {
                unsupported = true;
                reasons.push(format!("{name}_unsupported"));
            }
            SupportState::Unknown => {
                unknown = true;
                reasons.push(format!("{name}_unknown"));
            }
        }
    }

    reasons.sort();
    reasons.dedup();
    let state = if unsupported {
        SupportState::Unsupported
    } else if unknown {
        SupportState::Unknown
    } else {
        SupportState::Supported
    };
    (state, reasons)
}

fn roots_overlap(source: &PathCapabilityReport, target: &PathCapabilityReport) -> bool {
    if path_contains(source.requested_path(), target.requested_path())
        || path_contains(target.requested_path(), source.requested_path())
    {
        return true;
    }
    let source_leaf = (source.resolution() == PathResolution::ExistingDirectory)
        .then(|| {
            source
                .ancestry()
                .last()
                .map(DirectoryIdentityEvidence::identity)
        })
        .flatten();
    let target_leaf = (target.resolution() == PathResolution::ExistingDirectory)
        .then(|| {
            target
                .ancestry()
                .last()
                .map(DirectoryIdentityEvidence::identity)
        })
        .flatten();
    source_leaf.is_some_and(|identity| {
        target
            .ancestry()
            .iter()
            .any(|entry| entry.identity() == identity)
    }) || target_leaf.is_some_and(|identity| {
        source
            .ancestry()
            .iter()
            .any(|entry| entry.identity() == identity)
    })
}

fn path_contains(parent: &AbsolutePath, child: &AbsolutePath) -> bool {
    let parent = parent.as_bytes();
    let child = child.as_bytes();
    parent == child
        || (child.starts_with(parent) && (parent == b"/" || child.get(parent.len()) == Some(&b'/')))
}

fn path_clone_support(fd: &OwnedFd, filesystem: &FileSystemIdentity) -> SupportState {
    if filesystem.type_name() != "apfs" {
        return SupportState::Unsupported;
    }
    if filesystem.volume_id().known().is_none() {
        return SupportState::Unknown;
    }
    clone_support(fd)
}

fn clone_support(fd: &OwnedFd) -> SupportState {
    match clone_capability(fd) {
        Ok(Some(RawCloneCapability {
            interface_capabilities,
            interface_valid,
        })) if interface_valid & libc::VOL_CAP_INT_CLONE != 0 => {
            if interface_capabilities & libc::VOL_CAP_INT_CLONE != 0 {
                SupportState::Supported
            } else {
                SupportState::Unsupported
            }
        }
        Ok(Some(_)) | Ok(None) | Err(_) => SupportState::Unknown,
    }
}

fn volume_evidence(fd: &OwnedFd) -> Evidence<thinws_core::VolumeId> {
    match volume_uuid(fd) {
        Ok(Some(bytes)) if bytes.iter().any(|byte| *byte != 0) => {
            match decode_volume_id(Some(bytes)) {
                Ok(volume_id) => Evidence::Known(volume_id),
                Err(_) => Evidence::Unknown {
                    reason: "APFS volume UUID was not canonical".to_owned(),
                    errno: None,
                },
            }
        }
        Ok(Some(_)) => Evidence::Unknown {
            reason: "APFS volume UUID was the all-zero sentinel".to_owned(),
            errno: None,
        },
        Ok(None) => Evidence::Unknown {
            reason: "filesystem did not return a Volume UUID".to_owned(),
            errno: None,
        },
        Err(error) => Evidence::Unknown {
            reason: "reading the Volume UUID failed".to_owned(),
            errno: error.raw_os_error(),
        },
    }
}

fn access_support(result: io::Result<()>) -> SupportState {
    match result {
        Ok(()) => SupportState::Supported,
        Err(error) if matches!(error.raw_os_error(), Some(libc::EACCES) | Some(libc::EPERM)) => {
            SupportState::Unsupported
        }
        Err(_) => SupportState::Unknown,
    }
}

fn probe_digest(
    source: &PathCapabilityReport,
    target: &PathCapabilityReport,
    staging: &PathCapabilityReport,
    trash: &PathCapabilityReport,
    candidate: &CandidateEvidence,
) -> ProbeEvidenceDigest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"thinws-materialization-probe-v2\0");
    for (role, report) in [
        (b"source".as_slice(), source),
        (b"target".as_slice(), target),
        (b"staging".as_slice(), staging),
        (b"trash".as_slice(), trash),
    ] {
        update_bytes(&mut hasher, role);
        update_path_report(&mut hasher, report);
    }
    hasher.update(&[
        materializer_kind_byte(candidate.kind()),
        enum_byte(candidate.state()),
    ]);
    update_count(&mut hasher, candidate.reasons().len());
    for reason in candidate.reasons() {
        update_bytes(&mut hasher, reason.as_bytes());
    }
    ProbeEvidenceDigest::new(*hasher.finalize().as_bytes())
}

fn update_path_report(hasher: &mut blake3::Hasher, report: &PathCapabilityReport) {
    update_bytes(hasher, report.requested_path().as_bytes());
    hasher.update(&[match report.resolution() {
        PathResolution::ExistingDirectory => 1,
        PathResolution::MissingTarget => 2,
    }]);
    update_bytes(hasher, report.nearest_existing_ancestor().as_bytes());
    update_count(hasher, report.missing_components().len());
    for component in report.missing_components() {
        update_bytes(hasher, component);
    }
    update_count(hasher, report.ancestry().len());
    for entry in report.ancestry() {
        update_bytes(hasher, entry.path().as_bytes());
        hasher.update(&entry.identity().device().to_le_bytes());
        hasher.update(&entry.identity().inode().to_le_bytes());
    }
    update_bytes(hasher, report.filesystem().type_name().as_bytes());
    for word in report.filesystem().fsid() {
        hasher.update(&word.to_le_bytes());
    }
    match report.filesystem().volume_id() {
        Evidence::Known(volume_id) => {
            hasher.update(&[1]);
            update_bytes(hasher, volume_id.to_string().as_bytes());
        }
        Evidence::Unknown { .. } => {
            hasher.update(&[2]);
            update_bytes(
                hasher,
                report
                    .filesystem()
                    .volume_id()
                    .unknown_reason()
                    .unwrap_or_default()
                    .as_bytes(),
            );
            match report.filesystem().volume_id().unknown_errno() {
                Some(errno) => {
                    hasher.update(&[1]);
                    hasher.update(&errno.to_le_bytes());
                }
                None => {
                    hasher.update(&[0]);
                }
            }
        }
    }
    hasher.update(&report.mount().raw_flags().to_le_bytes());
    hasher.update(&[u8::from(report.mount().writable())]);
    hasher.update(&[
        enum_byte(report.readability()),
        enum_byte(report.writability()),
        enum_byte(report.apfs_clone()),
    ]);
}

fn update_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    update_count(hasher, bytes.len());
    hasher.update(bytes);
}

fn update_count(hasher: &mut blake3::Hasher, count: usize) {
    hasher.update(&(count as u64).to_le_bytes());
}

const fn materializer_kind_byte(kind: MaterializerKind) -> u8 {
    match kind {
        MaterializerKind::ApfsFileClone => 1,
        MaterializerKind::FullCopy => 2,
    }
}

const fn enum_byte(state: SupportState) -> u8 {
    match state {
        SupportState::Supported => 1,
        SupportState::Unsupported => 2,
        SupportState::Unknown => 3,
    }
}

fn append_component(path: &mut Vec<u8>, component: &[u8]) {
    if path.len() > 1 {
        path.push(b'/');
    }
    path.extend_from_slice(component);
}

fn identity(metadata: RawNodeMetadata) -> FileIdentity {
    FileIdentity::new(metadata.device, metadata.inode)
}

fn require_directory(metadata: RawNodeMetadata, operation: &'static str) -> Result<(), PortError> {
    if metadata.kind == RawFileKind::Directory {
        Ok(())
    } else {
        Err(PortError::new(PortErrorKind::InvalidLayout, operation))
    }
}

fn open_component_error(error: io::Error) -> PortError {
    let kind = if matches!(
        error.raw_os_error(),
        Some(libc::ELOOP) | Some(libc::ENOTDIR)
    ) {
        PortErrorKind::InvalidLayout
    } else if matches!(error.raw_os_error(), Some(libc::EACCES) | Some(libc::EPERM)) {
        PortErrorKind::Unavailable
    } else {
        PortErrorKind::Io
    };
    PortError::new(kind, "open path component without following links").with_source(error)
}

fn probe_io(
    operation: &'static str,
    error: impl std::error::Error + Send + Sync + 'static,
) -> PortError {
    PortError::new(PortErrorKind::Io, operation).with_source(error)
}

fn probe_invalid(
    operation: &'static str,
    error: impl std::error::Error + Send + Sync + 'static,
) -> PortError {
    PortError::new(PortErrorKind::InvalidLayout, operation).with_source(error)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use thinws_core::VolumeId;

    use super::*;

    fn absolute(value: &str) -> AbsolutePath {
        AbsolutePath::try_from_bytes(value.as_bytes().to_vec()).unwrap()
    }

    fn report(
        path: &str,
        volume_id: Evidence<VolumeId>,
        clone_support: SupportState,
    ) -> PathCapabilityReport {
        let requested = absolute(path);
        let inode = u64::from(*path.as_bytes().last().unwrap());
        PathCapabilityReport::new(
            requested.clone(),
            PathResolution::ExistingDirectory,
            requested.clone(),
            Vec::new(),
            vec![DirectoryIdentityEvidence::new(
                requested,
                FileIdentity::new(1, inode),
            )],
            FileSystemIdentity::new("apfs", [7, 8], volume_id),
            MountEvidence::new(0, true),
            SupportState::Supported,
            SupportState::Supported,
            clone_support,
        )
        .unwrap()
    }

    fn digest_for(
        report: &PathCapabilityReport,
        candidate: &CandidateEvidence,
    ) -> ProbeEvidenceDigest {
        probe_digest(report, report, report, report, candidate)
    }

    #[test]
    fn probe_digest_binds_the_candidate_backend_kind() {
        let report = report(
            "/Volumes/data/source",
            Evidence::Known(VolumeId::from_str("1a42c888-32e3-489c-9bfa-67fd640a94e8").unwrap()),
            SupportState::Supported,
        );
        let apfs = CandidateEvidence::new(
            MaterializerKind::ApfsFileClone,
            SupportState::Supported,
            Vec::new(),
        );
        let full_copy = CandidateEvidence::new(
            MaterializerKind::FullCopy,
            SupportState::Supported,
            Vec::new(),
        );

        assert_ne!(digest_for(&report, &apfs), digest_for(&report, &full_copy));
    }

    #[test]
    fn probe_digest_distinguishes_absent_errno_from_errno_zero() {
        let without_errno = report(
            "/Volumes/data/source",
            Evidence::Unknown {
                reason: "missing volume identity".to_owned(),
                errno: None,
            },
            SupportState::Unknown,
        );
        let errno_zero = report(
            "/Volumes/data/source",
            Evidence::Unknown {
                reason: "missing volume identity".to_owned(),
                errno: Some(0),
            },
            SupportState::Unknown,
        );
        let candidate = CandidateEvidence::new(
            MaterializerKind::ApfsFileClone,
            SupportState::Unknown,
            vec!["volume_unknown".to_owned()],
        );

        assert_ne!(
            digest_for(&without_errno, &candidate),
            digest_for(&errno_zero, &candidate)
        );
    }

    #[test]
    fn probe_digest_binds_reason_bytes_and_list_cardinality() {
        let report = report(
            "/Volumes/data/source",
            Evidence::Known(VolumeId::from_str("1a42c888-32e3-489c-9bfa-67fd640a94e8").unwrap()),
            SupportState::Supported,
        );
        let alpha = CandidateEvidence::new(
            MaterializerKind::ApfsFileClone,
            SupportState::Supported,
            vec!["alpha".to_owned()],
        );
        let bravo = CandidateEvidence::new(
            MaterializerKind::ApfsFileClone,
            SupportState::Supported,
            vec!["bravo".to_owned()],
        );
        let no_reasons = CandidateEvidence::new(
            MaterializerKind::ApfsFileClone,
            SupportState::Supported,
            Vec::new(),
        );
        let one_empty_reason = CandidateEvidence::new(
            MaterializerKind::ApfsFileClone,
            SupportState::Supported,
            vec![String::new()],
        );

        assert_ne!(digest_for(&report, &alpha), digest_for(&report, &bravo));
        assert_ne!(
            digest_for(&report, &no_reasons),
            digest_for(&report, &one_empty_reason)
        );
    }

    #[test]
    fn combined_clone_support_preserves_an_unknown_path_fact() {
        let volume = VolumeId::from_str("1a42c888-32e3-489c-9bfa-67fd640a94e8").unwrap();
        let source = report(
            "/Volumes/data/source",
            Evidence::Known(volume),
            SupportState::Unknown,
        );
        let target = report(
            "/Volumes/data/target",
            Evidence::Known(volume),
            SupportState::Supported,
        );
        let staging = report(
            "/Volumes/data/staging",
            Evidence::Known(volume),
            SupportState::Supported,
        );
        let trash = report(
            "/Volumes/data/trash",
            Evidence::Known(volume),
            SupportState::Supported,
        );

        let (state, reasons) = combined_clone_support(&source, &target, &staging, &trash);

        assert_eq!(state, SupportState::Unknown);
        assert_eq!(reasons, ["clone_capability_unknown"]);
    }
}

use std::collections::BTreeMap;
use std::ffi::{CString, OsString};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::time::Instant;

use thinws_core::{
    AbsolutePath, CreatedObjectEvidence, FileIdentity, MaterializationAttemptEvidence,
    MaterializationFailureKind, MaterializationMode, MaterializationPlan, MaterializationReceipt,
    MaterializeRequest, MaterializedEntryKind, MaterializerKind, PathCapabilityReport,
    PathResolution, RelativePath, RollbackEvidence, RollbackStatus, SupportState, TreeDigest,
};
use thinws_ports::{
    MaterializationFailure, MaterializationPathProbeRequest, PlatformProbe, PortError,
    PortErrorKind, WorkspaceMaterializer,
};

use crate::MacOsHostAdapter;
use crate::ffi::{
    RawFileKind, RawNodeMetadata, c_string, clone_file_at, create_directory_at, create_symlink_at,
    file_system_metadata, node_metadata, node_metadata_at, open_directory_at, open_file_read_at,
    open_root_directory, read_directory, read_link_at, rename_exclusive_at, set_mode,
    set_modified_time, volume_uuid,
};
use crate::volume::decode_volume_id;

/// macOS APFS implementation of the Phase 1 clone-only materializer.
pub struct ApfsCloneMaterializer {
    probe: MacOsHostAdapter,
}

impl ApfsCloneMaterializer {
    /// Creates a clone materializer using the same host Adapter for revalidation.
    #[must_use]
    pub const fn new(probe: MacOsHostAdapter) -> Self {
        Self { probe }
    }
}

impl WorkspaceMaterializer for ApfsCloneMaterializer {
    fn kind(&self) -> MaterializerKind {
        MaterializerKind::ApfsFileClone
    }

    fn materialize(
        &self,
        request: &MaterializeRequest,
        plan: &MaterializationPlan,
    ) -> Result<MaterializationReceipt, MaterializationFailure> {
        self.materialize_with_hook(request, plan, &NoopHook)
    }
}

impl ApfsCloneMaterializer {
    fn materialize_with_hook(
        &self,
        request: &MaterializeRequest,
        plan: &MaterializationPlan,
        hook: &dyn ExecutionHook,
    ) -> Result<MaterializationReceipt, MaterializationFailure> {
        let started = Instant::now();
        match self.materialize_inner(request, plan, hook) {
            Ok(success) => MaterializationReceipt::successful_apfs_clone(
                plan,
                success.regular_files,
                success.clone_calls,
                success.created,
                success.source_digest,
                success.target_digest,
                elapsed_millis(started),
                success.logical_bytes,
                Some(success.physical_bytes),
            )
            .map_err(|error| {
                failure_without_writes(
                    plan,
                    started,
                    Failure::with_source(
                        MaterializationFailureKind::ManifestMismatch,
                        PortErrorKind::InvalidData,
                        "construct APFS materialization receipt",
                        error,
                    ),
                )
            }),
            Err(mut failed) => {
                let rollback = match (failed.target.as_ref(), failed.trash.as_ref()) {
                    (Some(target), Some(trash)) if failed.target_modified => rollback_created(
                        request.target(),
                        target,
                        request.trash(),
                        trash,
                        failed.target_baseline,
                        &failed.created,
                        hook,
                    ),
                    _ => RollbackEvidence::new(
                        RollbackStatus::NotNeeded,
                        Vec::new(),
                        failed
                            .created
                            .iter()
                            .map(TrackedCreated::evidence)
                            .collect(),
                    ),
                };
                let created = failed
                    .created
                    .drain(..)
                    .map(|entry| entry.evidence())
                    .collect();
                let failure_kind = failed.error.kind;
                let port_error = failed.error.into_port_error();
                Err(MaterializationFailure::new(
                    port_error,
                    MaterializationReceipt::failed_apfs_clone(
                        plan,
                        failure_kind,
                        created,
                        failed.target_modified,
                        rollback,
                        failed.evidence,
                        elapsed_millis(started),
                    ),
                ))
            }
        }
    }
    fn materialize_inner(
        &self,
        request: &MaterializeRequest,
        plan: &MaterializationPlan,
        hook: &dyn ExecutionHook,
    ) -> Result<Success, Box<AttemptFailure>> {
        validate_request_plan(request, plan).map_err(AttemptFailure::before_write)?;
        let fresh = self
            .probe
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(request))
            .map_err(|error| {
                AttemptFailure::before_write(Failure::with_source(
                    MaterializationFailureKind::PlanStale,
                    PortErrorKind::InvalidLayout,
                    "revalidate APFS materialization paths",
                    error,
                ))
            })?;
        if fresh.evidence_digest() != plan.probe_evidence_digest()
            || fresh.apfs_clone().state() == SupportState::Unsupported
        {
            return Err(AttemptFailure::before_write(Failure::new(
                MaterializationFailureKind::PlanStale,
                PortErrorKind::InvalidLayout,
                "reject stale APFS materialization plan",
            )));
        }

        hook.after_probe().map_err(AttemptFailure::before_write)?;

        let source = open_bound_directory(request.source(), fresh.source())
            .map_err(AttemptFailure::before_write)?;
        let target = open_bound_directory(request.target(), fresh.target_root())
            .map_err(AttemptFailure::before_write)?;
        let staging = open_bound_directory(request.staging(), fresh.staging())
            .map_err(AttemptFailure::before_write)?;
        let trash = open_bound_directory(request.trash(), fresh.trash())
            .map_err(AttemptFailure::before_write)?;
        let target_baseline = node_metadata(&target).map_err(|error| {
            AttemptFailure::before_write(Failure::io(
                "inspect target root before materialization",
                error,
            ))
        })?;
        ensure_directory(target_baseline, "require target root directory")
            .map_err(AttemptFailure::before_write)?;
        ensure_empty(&target).map_err(AttemptFailure::before_write)?;

        let snapshot = snapshot_tree(&source).map_err(AttemptFailure::before_write)?;
        let mut context = ExecutionContext::default();
        if let Err(error) = clone_directory(
            &source,
            &target,
            &[],
            snapshot.root.device,
            &snapshot.entries,
            &mut context,
            hook,
        ) {
            let target_modified = !context.created.is_empty();
            return Err(AttemptFailure::after_write(
                error,
                FailedExecution {
                    source: &source,
                    target,
                    trash,
                    target_baseline,
                    context,
                    target_modified,
                    source_snapshot: &snapshot,
                },
            ));
        }

        if let Err(error) = hook.before_root_metadata(&target) {
            return Err(AttemptFailure::after_write(
                error,
                FailedExecution {
                    source: &source,
                    target,
                    trash,
                    target_baseline,
                    context,
                    target_modified: true,
                    source_snapshot: &snapshot,
                },
            ));
        }
        if let Err(error) =
            set_preserved_metadata(&target, target_baseline.identity(), snapshot.root)
        {
            return Err(AttemptFailure::after_write(
                error,
                FailedExecution {
                    source: &source,
                    target,
                    trash,
                    target_baseline,
                    context,
                    target_modified: true,
                    source_snapshot: &snapshot,
                },
            ));
        }

        if let Err(error) = hook.before_final_validation() {
            return Err(AttemptFailure::after_write(
                error,
                FailedExecution {
                    source: &source,
                    target,
                    trash,
                    target_baseline,
                    context,
                    target_modified: true,
                    source_snapshot: &snapshot,
                },
            ));
        }

        let source_after = match snapshot_tree(&source) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Err(AttemptFailure::after_write(
                    error.into_source_revalidation(),
                    FailedExecution {
                        source: &source,
                        target,
                        trash,
                        target_baseline,
                        context,
                        target_modified: true,
                        source_snapshot: &snapshot,
                    },
                ));
            }
        };
        if source_after != snapshot {
            return Err(AttemptFailure::after_write(
                source_changed(),
                FailedExecution {
                    source: &source,
                    target,
                    trash,
                    target_baseline,
                    context,
                    target_modified: true,
                    source_snapshot: &snapshot,
                },
            ));
        }
        let target_snapshot = match snapshot_tree(&target) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Err(AttemptFailure::after_write(
                    error.into_target_revalidation(),
                    FailedExecution {
                        source: &source,
                        target,
                        trash,
                        target_baseline,
                        context,
                        target_modified: true,
                        source_snapshot: &snapshot,
                    },
                ));
            }
        };
        if !target_snapshot
            .manifest
            .matches_promised(&snapshot.manifest)
        {
            return Err(AttemptFailure::after_write(
                Failure::new(
                    MaterializationFailureKind::ManifestMismatch,
                    PortErrorKind::InvalidData,
                    "verify APFS target manifest",
                ),
                FailedExecution {
                    source: &source,
                    target,
                    trash,
                    target_baseline,
                    context,
                    target_modified: true,
                    source_snapshot: &snapshot,
                },
            ));
        }
        if !target_snapshot_matches_created(&target_snapshot, &context.created) {
            return Err(AttemptFailure::after_write(
                target_changed(),
                FailedExecution {
                    source: &source,
                    target,
                    trash,
                    target_baseline,
                    context,
                    target_modified: true,
                    source_snapshot: &snapshot,
                },
            ));
        }
        let final_paths = revalidate_bound_path(request.source(), fresh.source(), &source)
            .and_then(|()| revalidate_bound_path(request.target(), fresh.target_root(), &target))
            .and_then(|()| revalidate_bound_path(request.staging(), fresh.staging(), &staging))
            .and_then(|()| revalidate_bound_path(request.trash(), fresh.trash(), &trash));
        if let Err(error) = final_paths {
            return Err(AttemptFailure::after_write(
                error,
                FailedExecution {
                    source: &source,
                    target,
                    trash,
                    target_baseline,
                    context,
                    target_modified: true,
                    source_snapshot: &snapshot,
                },
            ));
        }

        Ok(Success {
            regular_files: snapshot.manifest.regular_files,
            clone_calls: context.clone_calls,
            created: context
                .created
                .into_iter()
                .map(|entry| entry.evidence())
                .collect(),
            source_digest: snapshot.manifest.digest(),
            target_digest: target_snapshot.manifest.digest(),
            logical_bytes: snapshot.manifest.logical_bytes,
            physical_bytes: target_snapshot.manifest.physical_bytes,
        })
    }
}

fn validate_request_plan(
    request: &MaterializeRequest,
    plan: &MaterializationPlan,
) -> Result<(), Failure> {
    if plan.selected_adapter() != MaterializerKind::ApfsFileClone
        || plan.effective_mode() != MaterializationMode::CowClone
        || request.source() != plan.source_path()
        || request.target() != plan.target_path()
        || request.staging() != plan.staging_path()
        || request.trash() != plan.trash_path()
    {
        return Err(Failure::new(
            MaterializationFailureKind::PlanStale,
            PortErrorKind::InvalidLayout,
            "validate APFS materialization request against plan",
        ));
    }
    Ok(())
}

fn open_absolute_directory(path: &AbsolutePath) -> Result<OwnedFd, Failure> {
    let mut current =
        open_root_directory().map_err(|error| Failure::io("open APFS path root", error))?;
    for component in path.as_bytes()[1..].split(|byte| *byte == b'/') {
        if component.is_empty() {
            continue;
        }
        let name = c_string(component)
            .map_err(|error| Failure::invalid("encode APFS path component", error))?;
        current = open_directory_at(&current, &name)
            .map_err(|error| Failure::path_open("open APFS path component", error))?;
    }
    Ok(current)
}

fn open_bound_directory(
    path: &AbsolutePath,
    report: &PathCapabilityReport,
) -> Result<OwnedFd, Failure> {
    if report.requested_path() != path || report.resolution() != PathResolution::ExistingDirectory {
        return Err(stale_path_binding());
    }
    let directory = open_absolute_directory(path)?;
    verify_directory_evidence(&directory, report)?;
    Ok(directory)
}

fn revalidate_bound_path(
    path: &AbsolutePath,
    report: &PathCapabilityReport,
    held: &OwnedFd,
) -> Result<(), Failure> {
    let reopened = open_bound_directory(path, report)?;
    let held_metadata =
        node_metadata(held).map_err(|error| Failure::io("inspect held APFS directory", error))?;
    let reopened_metadata = node_metadata(&reopened)
        .map_err(|error| Failure::io("reinspect APFS directory path", error))?;
    if held_metadata.identity() != reopened_metadata.identity()
        || held_metadata.kind != RawFileKind::Directory
        || reopened_metadata.kind != RawFileKind::Directory
    {
        return Err(stale_path_binding());
    }
    Ok(())
}

fn verify_directory_evidence(
    directory: &OwnedFd,
    report: &PathCapabilityReport,
) -> Result<(), Failure> {
    let expected_identity = report
        .ancestry()
        .last()
        .filter(|entry| entry.path() == report.requested_path())
        .map(|entry| entry.identity())
        .ok_or_else(stale_path_binding)?;
    let metadata = node_metadata(directory)
        .map_err(|error| Failure::io("inspect APFS directory identity", error))?;
    let filesystem = file_system_metadata(directory)
        .map_err(|error| Failure::io("inspect APFS directory filesystem", error))?;
    let volume = volume_uuid(directory)
        .map_err(|error| Failure::io("inspect APFS directory volume UUID", error))?;
    let volume = decode_volume_id(volume).map_err(|error| {
        Failure::with_source(
            MaterializationFailureKind::PlanStale,
            PortErrorKind::InvalidLayout,
            "decode APFS directory volume UUID",
            error,
        )
    })?;
    let expected_volume = report
        .filesystem()
        .volume_id()
        .known()
        .ok_or_else(stale_path_binding)?;
    if metadata.kind != RawFileKind::Directory
        || metadata.identity() != expected_identity
        || filesystem.type_name != report.filesystem().type_name()
        || filesystem.fsid != report.filesystem().fsid()
        || filesystem.mount_flags != report.mount().raw_flags()
        || &volume != expected_volume
    {
        return Err(stale_path_binding());
    }
    Ok(())
}

fn stale_path_binding() -> Failure {
    Failure::new(
        MaterializationFailureKind::PlanStale,
        PortErrorKind::InvalidLayout,
        "bind APFS directory descriptors to plan evidence",
    )
}

fn ensure_empty(target: &OwnedFd) -> Result<(), Failure> {
    let names =
        read_directory(target).map_err(|error| Failure::io("enumerate target root", error))?;
    if names.is_empty() {
        Ok(())
    } else {
        Err(Failure::new(
            MaterializationFailureKind::InvalidLayout,
            PortErrorKind::NotEmpty,
            "require empty target root",
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TreeSnapshot {
    root: RawNodeMetadata,
    entries: BTreeMap<Vec<u8>, SnapshotEntry>,
    manifest: TreeManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SnapshotEntry {
    metadata: RawNodeMetadata,
    link_text: Option<Vec<u8>>,
    content_digest: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TreeManifest {
    root_mode: u32,
    root_mtime: (i64, i64),
    entries: Vec<ManifestEntry>,
    regular_files: u64,
    logical_bytes: u64,
    physical_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManifestEntry {
    path: Vec<u8>,
    kind: RawFileKind,
    mode: Option<u32>,
    mtime: Option<(i64, i64)>,
    length: u64,
    content_digest: Option<[u8; 32]>,
}

impl TreeManifest {
    fn matches_promised(&self, source: &Self) -> bool {
        self.root_mode == source.root_mode
            && self.root_mtime == source.root_mtime
            && self.entries == source.entries
            && self.regular_files == source.regular_files
            && self.logical_bytes == source.logical_bytes
    }

    fn digest(&self) -> TreeDigest {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"thinws-tree-manifest-v1\0");
        hasher.update(&self.root_mode.to_le_bytes());
        update_time(&mut hasher, self.root_mtime);
        for entry in &self.entries {
            update_bytes(&mut hasher, &entry.path);
            hasher.update(&[kind_byte(entry.kind)]);
            match entry.mode {
                Some(mode) => {
                    hasher.update(&[1]);
                    hasher.update(&mode.to_le_bytes());
                }
                None => {
                    hasher.update(&[0]);
                }
            }
            match entry.mtime {
                Some(time) => {
                    hasher.update(&[1]);
                    update_time(&mut hasher, time);
                }
                None => {
                    hasher.update(&[0]);
                }
            }
            hasher.update(&entry.length.to_le_bytes());
            match entry.content_digest {
                Some(digest) => {
                    hasher.update(&[1]);
                    hasher.update(&digest);
                }
                None => {
                    hasher.update(&[0]);
                }
            }
        }
        TreeDigest::new(*hasher.finalize().as_bytes())
    }
}

fn snapshot_tree(root: &OwnedFd) -> Result<TreeSnapshot, Failure> {
    let root_before =
        node_metadata(root).map_err(|error| Failure::io("inspect manifest root", error))?;
    ensure_directory(root_before, "require manifest root directory")?;
    let mut snapshot = TreeSnapshot {
        root: root_before,
        entries: BTreeMap::new(),
        manifest: TreeManifest {
            root_mode: root_before.mode,
            root_mtime: root_before.mtime(),
            entries: Vec::new(),
            regular_files: 0,
            logical_bytes: 0,
            physical_bytes: 0,
        },
    };
    snapshot_directory(root, &[], root_before.device, &mut snapshot)?;
    let root_after =
        node_metadata(root).map_err(|error| Failure::io("reinspect manifest root", error))?;
    if root_after != root_before {
        return Err(source_changed());
    }
    Ok(snapshot)
}

fn snapshot_directory(
    directory: &OwnedFd,
    relative: &[OsString],
    root_device: u64,
    snapshot: &mut TreeSnapshot,
) -> Result<(), Failure> {
    let mut names = read_directory(directory)
        .map_err(|error| Failure::io("enumerate manifest directory", error))?;
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    for name in names {
        let component = component_name(&name)?;
        let mut child = relative.to_vec();
        child.push(name);
        let path = relative_bytes(&child);
        let before = node_metadata_at(directory, &component)
            .map_err(|error| Failure::io("inspect manifest entry", error))?;
        if before.device != root_device {
            return Err(Failure::new(
                MaterializationFailureKind::UnsupportedSourceEntry,
                PortErrorKind::InvalidLayout,
                "reject source submount",
            ));
        }

        let mut entry = SnapshotEntry {
            metadata: before,
            link_text: None,
            content_digest: None,
        };
        let (mode, mtime, length, digest) = match before.kind {
            RawFileKind::Directory => {
                let child_fd = open_directory_at(directory, &component)
                    .map_err(|error| Failure::path_open("open manifest directory", error))?;
                if node_metadata(&child_fd)
                    .map_err(|error| Failure::io("reinspect manifest directory", error))?
                    != before
                {
                    return Err(source_changed());
                }
                snapshot_directory(&child_fd, &child, root_device, snapshot)?;
                (Some(before.mode), Some(before.mtime()), 0, None)
            }
            RawFileKind::RegularFile => {
                let file = open_file_read_at(directory, &component)
                    .map_err(|error| Failure::path_open("open manifest file", error))?;
                let (length, digest) = digest_file(file, before)?;
                entry.content_digest = Some(digest);
                snapshot.manifest.regular_files += 1;
                snapshot.manifest.logical_bytes =
                    snapshot.manifest.logical_bytes.saturating_add(length);
                snapshot.manifest.physical_bytes = snapshot
                    .manifest
                    .physical_bytes
                    .saturating_add(before.allocated_bytes);
                (
                    Some(before.mode),
                    Some(before.mtime()),
                    length,
                    Some(digest),
                )
            }
            RawFileKind::SymbolicLink => {
                let text = read_link_at(directory, &component)
                    .map_err(|error| Failure::io("read manifest symlink", error))?;
                if node_metadata_at(directory, &component)
                    .map_err(|error| Failure::io("reinspect manifest symlink", error))?
                    != before
                {
                    return Err(source_changed());
                }
                let digest = *blake3::hash(&text).as_bytes();
                let length = u64::try_from(text.len()).unwrap_or(u64::MAX);
                entry.link_text = Some(text);
                entry.content_digest = Some(digest);
                (None, None, length, Some(digest))
            }
            RawFileKind::Fifo
            | RawFileKind::Socket
            | RawFileKind::CharacterDevice
            | RawFileKind::BlockDevice
            | RawFileKind::Unknown => {
                return Err(Failure::new(
                    MaterializationFailureKind::UnsupportedSourceEntry,
                    PortErrorKind::InvalidData,
                    "reject unsupported source entry",
                ));
            }
        };
        snapshot.entries.insert(path.clone(), entry);
        snapshot.manifest.entries.push(ManifestEntry {
            path,
            kind: before.kind,
            mode,
            mtime,
            length,
            content_digest: digest,
        });
    }
    Ok(())
}

fn digest_file(file: OwnedFd, expected: RawNodeMetadata) -> Result<(u64, [u8; 32]), Failure> {
    let held =
        node_metadata(&file).map_err(|error| Failure::io("inspect opened manifest file", error))?;
    if held != expected {
        return Err(source_changed());
    }
    let mut file = File::from(file);
    let mut hasher = blake3::Hasher::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| Failure::io("read manifest file", error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        length = length.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
    }
    let after =
        node_metadata(&file).map_err(|error| Failure::io("reinspect manifest file", error))?;
    if after != expected || length != expected.size {
        return Err(source_changed());
    }
    Ok((length, *hasher.finalize().as_bytes()))
}

fn clone_directory(
    source: &OwnedFd,
    target: &OwnedFd,
    relative: &[OsString],
    source_root_device: u64,
    expected: &BTreeMap<Vec<u8>, SnapshotEntry>,
    context: &mut ExecutionContext,
    hook: &dyn ExecutionHook,
) -> Result<(), Failure> {
    let mut names = read_directory(source)
        .map_err(|error| Failure::io("enumerate source during clone", error))?;
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    for name in names {
        let component = component_name(&name)?;
        let mut child = relative.to_vec();
        child.push(name);
        let path = relative_bytes(&child);
        let expected_entry = expected.get(&path).ok_or_else(source_changed)?;
        let before = node_metadata_at(source, &component)
            .map_err(|error| Failure::io("inspect source entry before clone", error))?;
        if before != expected_entry.metadata || before.device != source_root_device {
            return Err(source_changed());
        }

        match before.kind {
            RawFileKind::Directory => {
                let source_child = open_directory_at(source, &component)
                    .map_err(|error| Failure::path_open("open source directory", error))?;
                if node_metadata(&source_child)
                    .map_err(|error| Failure::io("inspect held source directory", error))?
                    != before
                {
                    return Err(source_changed());
                }
                create_directory_at(target, &component, 0o700)
                    .map_err(|error| Failure::io("create target directory", error))?;
                let identity = register_created(target, &component, &child, before.kind, context)?;
                let target_child = open_directory_at(target, &component)
                    .map_err(|error| Failure::path_open("open created target directory", error))?;
                ensure_identity(
                    node_metadata(&target_child)
                        .map_err(|error| Failure::io("inspect held target directory", error))?,
                    identity,
                    before.kind,
                )?;
                clone_directory(
                    &source_child,
                    &target_child,
                    &child,
                    source_root_device,
                    expected,
                    context,
                    hook,
                )?;
                set_preserved_metadata(&target_child, identity, before)?;
                hook.after_created(&child, context.created.len())?;
                if node_metadata(&source_child)
                    .map_err(|error| Failure::io("reinspect source directory", error))?
                    != before
                {
                    return Err(source_changed());
                }
            }
            RawFileKind::RegularFile => {
                let source_file = open_file_read_at(source, &component)
                    .map_err(|error| Failure::path_open("open source file", error))?;
                if node_metadata(&source_file)
                    .map_err(|error| Failure::io("inspect held source file", error))?
                    != before
                {
                    return Err(source_changed());
                }
                clone_file_at(&source_file, target, &component)
                    .map_err(|error| Failure::clone_io("clone target file", error))?;
                context.clone_calls = context.clone_calls.saturating_add(1);
                let identity = register_created(target, &component, &child, before.kind, context)?;
                let target_file = open_file_read_at(target, &component)
                    .map_err(|error| Failure::path_open("open cloned target file", error))?;
                ensure_identity(
                    node_metadata(&target_file)
                        .map_err(|error| Failure::io("inspect held target file", error))?,
                    identity,
                    before.kind,
                )?;
                set_preserved_metadata(&target_file, identity, before)?;
                hook.after_created(&child, context.created.len())?;
                if node_metadata(&source_file)
                    .map_err(|error| Failure::io("reinspect source file", error))?
                    != before
                {
                    return Err(source_changed());
                }
            }
            RawFileKind::SymbolicLink => {
                let text = read_link_at(source, &component)
                    .map_err(|error| Failure::io("read source symlink", error))?;
                if expected_entry.link_text.as_deref() != Some(text.as_slice())
                    || node_metadata_at(source, &component)
                        .map_err(|error| Failure::io("reinspect source symlink", error))?
                        != before
                {
                    return Err(source_changed());
                }
                let text = CString::new(text)
                    .map_err(|error| Failure::invalid("encode source symlink", error))?;
                create_symlink_at(&text, target, &component)
                    .map_err(|error| Failure::io("create target symlink", error))?;
                register_created(target, &component, &child, before.kind, context)?;
                hook.after_created(&child, context.created.len())?;
            }
            RawFileKind::Fifo
            | RawFileKind::Socket
            | RawFileKind::CharacterDevice
            | RawFileKind::BlockDevice
            | RawFileKind::Unknown => {
                return Err(Failure::new(
                    MaterializationFailureKind::UnsupportedSourceEntry,
                    PortErrorKind::InvalidData,
                    "reject unsupported source entry during clone",
                ));
            }
        }
    }
    Ok(())
}

fn register_created(
    parent: &OwnedFd,
    component: &CString,
    relative: &[OsString],
    kind: RawFileKind,
    context: &mut ExecutionContext,
) -> Result<FileIdentity, Failure> {
    context.created.push(TrackedCreated {
        relative: relative.to_vec(),
        kind,
        identity: None,
    });
    let metadata = node_metadata_at(parent, component)
        .map_err(|error| Failure::io("register created target identity", error))?;
    if metadata.kind != kind {
        return Err(target_changed());
    }
    let identity = metadata.identity();
    context
        .created
        .last_mut()
        .expect("the just-created target is recorded")
        .identity = Some(identity);
    Ok(identity)
}

fn set_preserved_metadata(
    target: &OwnedFd,
    expected_identity: FileIdentity,
    source: RawNodeMetadata,
) -> Result<(), Failure> {
    ensure_identity(
        node_metadata(target)
            .map_err(|error| Failure::io("inspect target before metadata update", error))?,
        expected_identity,
        source.kind,
    )?;
    set_modified_time(target, source.modified_seconds, source.modified_nanoseconds)
        .map_err(|error| Failure::io("preserve target modification time", error))?;
    set_mode(target, source.mode)
        .map_err(|error| Failure::io("preserve target permissions", error))?;
    let after = node_metadata(target)
        .map_err(|error| Failure::io("inspect target after metadata update", error))?;
    ensure_identity(after, expected_identity, source.kind)?;
    if after.mode != source.mode || after.mtime() != source.mtime() {
        return Err(Failure::new(
            MaterializationFailureKind::ManifestMismatch,
            PortErrorKind::InvalidData,
            "verify preserved target metadata",
        ));
    }
    Ok(())
}

trait ExecutionHook {
    fn after_probe(&self) -> Result<(), Failure> {
        Ok(())
    }

    fn after_created(&self, _relative: &[OsString], _created_count: usize) -> Result<(), Failure> {
        Ok(())
    }

    fn before_root_metadata(&self, _target: &OwnedFd) -> Result<(), Failure> {
        Ok(())
    }

    fn before_final_validation(&self) -> Result<(), Failure> {
        Ok(())
    }

    fn before_rollback_detach(&self, _relative: &[OsString]) -> Result<(), Failure> {
        Ok(())
    }
}

struct NoopHook;

impl ExecutionHook for NoopHook {}

#[derive(Default)]
struct ExecutionContext {
    created: Vec<TrackedCreated>,
    clone_calls: u64,
}

#[derive(Clone)]
struct TrackedCreated {
    relative: Vec<OsString>,
    kind: RawFileKind,
    identity: Option<FileIdentity>,
}

impl TrackedCreated {
    fn evidence(&self) -> CreatedObjectEvidence {
        let path = RelativePath::try_from_bytes(relative_bytes(&self.relative))
            .expect("filesystem components form a safe relative path");
        CreatedObjectEvidence::new(path, materialized_kind(self.kind), self.identity)
    }
}

fn rollback_created(
    target_path: &AbsolutePath,
    target: &OwnedFd,
    trash_path: &AbsolutePath,
    trash: &OwnedFd,
    baseline: Option<RawNodeMetadata>,
    created: &[TrackedCreated],
    hook: &dyn ExecutionHook,
) -> RollbackEvidence {
    let Some(baseline) = baseline else {
        return RollbackEvidence::new(
            RollbackStatus::Incomplete,
            Vec::new(),
            created.iter().map(TrackedCreated::evidence).collect(),
        );
    };
    let current_root = match node_metadata(target) {
        Ok(metadata)
            if metadata.identity() == baseline.identity()
                && metadata.kind == RawFileKind::Directory =>
        {
            metadata
        }
        _ => {
            return RollbackEvidence::new(
                RollbackStatus::Incomplete,
                Vec::new(),
                created.iter().map(TrackedCreated::evidence).collect(),
            );
        }
    };
    if current_root.mode != baseline.mode && set_mode(target, baseline.mode).is_err() {
        return RollbackEvidence::new(
            RollbackStatus::Incomplete,
            Vec::new(),
            created.iter().map(TrackedCreated::evidence).collect(),
        );
    }
    if !path_points_to_held(target_path, target) || !path_points_to_held(trash_path, trash) {
        return RollbackEvidence::new(
            RollbackStatus::Incomplete,
            Vec::new(),
            created.iter().map(TrackedCreated::evidence).collect(),
        );
    }

    let mut removed = Vec::new();
    let mut quarantined = Vec::new();
    let mut active_count = created.len();
    for entry in created.iter().rev() {
        if !rollback_set_matches(target, &created[..active_count]) {
            break;
        }
        if !path_points_to_held(target_path, target) || !path_points_to_held(trash_path, trash) {
            break;
        }
        let Some(expected_identity) = entry.identity else {
            break;
        };
        let Ok((parent, component)) = open_rollback_parent(target, entry, created) else {
            break;
        };
        let Ok(current) = node_metadata_at(&parent, &component) else {
            break;
        };
        if current.identity() != expected_identity || current.kind != entry.kind {
            break;
        }
        if hook.before_rollback_detach(&entry.relative).is_err() {
            break;
        }
        let Some(quarantine) =
            detach_created_to_trash(&parent, &component, trash, entry, removed.len())
        else {
            break;
        };
        quarantined.push(quarantine);
        removed.push(entry.evidence().path().clone());
        active_count -= 1;
    }

    let exact_remaining = rollback_set_matches(target, &created[..active_count]);
    let root_restored = exact_remaining
        && node_metadata(target).is_ok_and(|metadata| {
            metadata.identity() == baseline.identity()
                && metadata.kind == RawFileKind::Directory
                && set_modified_time(
                    target,
                    baseline.modified_seconds,
                    baseline.modified_nanoseconds,
                )
                .is_ok()
                && set_mode(target, baseline.mode).is_ok()
                && node_metadata(target).is_ok_and(|after| {
                    after.identity() == baseline.identity()
                        && after.kind == RawFileKind::Directory
                        && after.mode == baseline.mode
                        && after.mtime() == baseline.mtime()
                })
        });
    let remaining = created[..active_count]
        .iter()
        .map(TrackedCreated::evidence)
        .collect::<Vec<_>>();
    let status = if remaining.is_empty() && root_restored {
        RollbackStatus::ConfirmedBaseline
    } else {
        RollbackStatus::Incomplete
    };
    RollbackEvidence::new(status, removed, remaining).with_quarantined(quarantined)
}

fn path_points_to_held(path: &AbsolutePath, held: &OwnedFd) -> bool {
    let Ok(reopened) = open_absolute_directory(path) else {
        return false;
    };
    let Ok(expected) = node_metadata(held) else {
        return false;
    };
    node_metadata(&reopened).is_ok_and(|observed| {
        observed.kind == RawFileKind::Directory
            && expected.kind == RawFileKind::Directory
            && observed.identity() == expected.identity()
    })
}

fn detach_created_to_trash(
    parent: &OwnedFd,
    component: &CString,
    trash: &OwnedFd,
    entry: &TrackedCreated,
    sequence: usize,
) -> Option<RelativePath> {
    let expected_identity = entry.identity?;
    for collision in 0..128_u32 {
        let quarantine = CString::new(format!(
            ".thinws-rollback-{}-{}-{sequence}-{collision}",
            expected_identity.device(),
            expected_identity.inode(),
        ))
        .expect("rollback quarantine names contain no NUL");
        match rename_exclusive_at(parent, component, trash, &quarantine) {
            Ok(()) => {
                let moved_matches = node_metadata_at(trash, &quarantine).is_ok_and(|moved| {
                    moved.identity() == expected_identity && moved.kind == entry.kind
                });
                if moved_matches {
                    return RelativePath::try_from_bytes(quarantine.as_bytes().to_vec()).ok();
                }
                let _ = rename_exclusive_at(trash, &quarantine, parent, component);
                return None;
            }
            Err(error) if error.raw_os_error() == Some(libc::EEXIST) => continue,
            Err(_) => return None,
        }
    }
    None
}

fn rollback_set_matches(target: &OwnedFd, created: &[TrackedCreated]) -> bool {
    let Ok(root) = node_metadata(target) else {
        return false;
    };
    let mut observed = BTreeMap::new();
    if observe_target_entries(target, &[], root.device, &mut observed).is_err()
        || observed.len() != created.len()
    {
        return false;
    }
    created.iter().all(|entry| {
        let Some(identity) = entry.identity else {
            return false;
        };
        observed.get(&relative_bytes(&entry.relative)) == Some(&(identity, entry.kind))
    })
}

fn target_snapshot_matches_created(snapshot: &TreeSnapshot, created: &[TrackedCreated]) -> bool {
    snapshot.entries.len() == created.len()
        && created.iter().all(|entry| {
            let Some(identity) = entry.identity else {
                return false;
            };
            snapshot
                .entries
                .get(&relative_bytes(&entry.relative))
                .is_some_and(|observed| {
                    observed.metadata.identity() == identity && observed.metadata.kind == entry.kind
                })
        })
}

fn observe_target_entries(
    directory: &OwnedFd,
    relative: &[OsString],
    root_device: u64,
    observed: &mut BTreeMap<Vec<u8>, (FileIdentity, RawFileKind)>,
) -> io::Result<()> {
    let mut names = read_directory(directory)?;
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    for name in names {
        let component = component_name_io(&name)?;
        let mut child = relative.to_vec();
        child.push(name);
        let metadata = node_metadata_at(directory, &component)?;
        if metadata.device != root_device {
            return Err(io::Error::from_raw_os_error(libc::EXDEV));
        }
        observed.insert(relative_bytes(&child), (metadata.identity(), metadata.kind));
        if metadata.kind == RawFileKind::Directory {
            let child_fd = open_directory_at(directory, &component)?;
            let held = node_metadata(&child_fd)?;
            if held.identity() != metadata.identity() || held.kind != RawFileKind::Directory {
                return Err(io::Error::from_raw_os_error(libc::ESTALE));
            }
            observe_target_entries(&child_fd, &child, root_device, observed)?;
        }
    }
    Ok(())
}

fn open_rollback_parent(
    target: &OwnedFd,
    entry: &TrackedCreated,
    created: &[TrackedCreated],
) -> Result<(OwnedFd, CString), io::Error> {
    let (name, parents) = entry
        .relative
        .split_last()
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    let mut current = target.try_clone()?;
    let mut prefix = Vec::new();
    for parent_name in parents {
        prefix.push(parent_name.clone());
        let component = component_name_io(parent_name)?;
        current = open_directory_at(&current, &component)?;
        let expected = created
            .iter()
            .find(|candidate| candidate.relative == prefix)
            .and_then(|candidate| {
                (candidate.kind == RawFileKind::Directory)
                    .then_some(candidate.identity)
                    .flatten()
            })
            .ok_or_else(|| io::Error::from_raw_os_error(libc::ESTALE))?;
        let observed = node_metadata(&current)?;
        if observed.identity() != expected || observed.kind != RawFileKind::Directory {
            return Err(io::Error::from_raw_os_error(libc::ESTALE));
        }
    }
    Ok((current, component_name_io(name)?))
}

struct Success {
    regular_files: u64,
    clone_calls: u64,
    created: Vec<CreatedObjectEvidence>,
    source_digest: TreeDigest,
    target_digest: TreeDigest,
    logical_bytes: u64,
    physical_bytes: u64,
}

struct AttemptFailure {
    error: Failure,
    target: Option<OwnedFd>,
    trash: Option<OwnedFd>,
    target_baseline: Option<RawNodeMetadata>,
    created: Vec<TrackedCreated>,
    target_modified: bool,
    evidence: MaterializationAttemptEvidence,
}

struct FailedExecution<'a> {
    source: &'a OwnedFd,
    target: OwnedFd,
    trash: OwnedFd,
    target_baseline: RawNodeMetadata,
    context: ExecutionContext,
    target_modified: bool,
    source_snapshot: &'a TreeSnapshot,
}

impl AttemptFailure {
    fn before_write(error: Failure) -> Box<Self> {
        Box::new(Self {
            error,
            target: None,
            trash: None,
            target_baseline: None,
            created: Vec::new(),
            target_modified: false,
            evidence: MaterializationAttemptEvidence::default(),
        })
    }

    fn after_write(error: Failure, failed: FailedExecution<'_>) -> Box<Self> {
        let source_observation = snapshot_tree(failed.source).ok();
        let target_snapshot = snapshot_tree(&failed.target).ok();
        let evidence = MaterializationAttemptEvidence::new(
            Some(failed.source_snapshot.manifest.logical_bytes),
            target_snapshot
                .as_ref()
                .map(|snapshot| snapshot.manifest.physical_bytes),
            Some(failed.source_snapshot.manifest.regular_files),
            failed.context.clone_calls,
            source_observation
                .as_ref()
                .map(|snapshot| snapshot.manifest.digest()),
            target_snapshot
                .as_ref()
                .map(|snapshot| snapshot.manifest.digest()),
        );
        Box::new(Self {
            error,
            target: Some(failed.target),
            trash: Some(failed.trash),
            target_baseline: Some(failed.target_baseline),
            created: failed.context.created,
            target_modified: failed.target_modified,
            evidence,
        })
    }
}

#[derive(Debug)]
struct Failure {
    kind: MaterializationFailureKind,
    port_kind: PortErrorKind,
    operation: &'static str,
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl Failure {
    const fn new(
        kind: MaterializationFailureKind,
        port_kind: PortErrorKind,
        operation: &'static str,
    ) -> Self {
        Self {
            kind,
            port_kind,
            operation,
            source: None,
        }
    }

    fn with_source(
        kind: MaterializationFailureKind,
        port_kind: PortErrorKind,
        operation: &'static str,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind,
            port_kind,
            operation,
            source: Some(Box::new(source)),
        }
    }

    fn io(operation: &'static str, error: io::Error) -> Self {
        let kind = if error.raw_os_error() == Some(libc::ENOSPC) {
            MaterializationFailureKind::NoSpace
        } else {
            MaterializationFailureKind::Filesystem
        };
        Self::with_source(kind, PortErrorKind::Io, operation, error)
    }

    fn clone_io(operation: &'static str, error: io::Error) -> Self {
        let (kind, port_kind) = match error.raw_os_error() {
            Some(libc::ENOTSUP) => (
                MaterializationFailureKind::CowUnavailable,
                PortErrorKind::CapabilityUnavailable,
            ),
            Some(libc::EXDEV) => (
                MaterializationFailureKind::PlanStale,
                PortErrorKind::InvalidLayout,
            ),
            Some(libc::ENOSPC) => (MaterializationFailureKind::NoSpace, PortErrorKind::Io),
            _ => (MaterializationFailureKind::Filesystem, PortErrorKind::Io),
        };
        Self::with_source(kind, port_kind, operation, error)
    }

    fn invalid(
        operation: &'static str,
        error: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::with_source(
            MaterializationFailureKind::InvalidLayout,
            PortErrorKind::InvalidLayout,
            operation,
            error,
        )
    }

    fn path_open(operation: &'static str, error: io::Error) -> Self {
        if matches!(
            error.raw_os_error(),
            Some(libc::ELOOP) | Some(libc::ENOTDIR)
        ) {
            Self::with_source(
                MaterializationFailureKind::PlanStale,
                PortErrorKind::InvalidLayout,
                operation,
                error,
            )
        } else {
            Self::io(operation, error)
        }
    }

    fn into_port_error(self) -> PortError {
        let error = PortError::new(self.port_kind, self.operation);
        match self.source {
            Some(source) => error.with_boxed_source(source),
            None => error,
        }
    }

    fn into_source_revalidation(mut self) -> Self {
        if matches!(
            self.kind,
            MaterializationFailureKind::InvalidLayout
                | MaterializationFailureKind::UnsupportedSourceEntry
        ) {
            self.kind = MaterializationFailureKind::SourceChanged;
            self.port_kind = PortErrorKind::Conflict;
            self.operation = "source changed during final APFS revalidation";
        }
        self
    }

    fn into_target_revalidation(mut self) -> Self {
        if matches!(
            self.kind,
            MaterializationFailureKind::InvalidLayout
                | MaterializationFailureKind::SourceChanged
                | MaterializationFailureKind::UnsupportedSourceEntry
        ) {
            self.kind = MaterializationFailureKind::TargetChanged;
            self.port_kind = PortErrorKind::Conflict;
            self.operation = "target changed during final APFS revalidation";
        }
        self
    }
}

fn failure_without_writes(
    plan: &MaterializationPlan,
    started: Instant,
    error: Failure,
) -> MaterializationFailure {
    let kind = error.kind;
    MaterializationFailure::new(
        error.into_port_error(),
        MaterializationReceipt::failed_apfs_clone(
            plan,
            kind,
            Vec::new(),
            false,
            RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new()),
            MaterializationAttemptEvidence::default(),
            elapsed_millis(started),
        ),
    )
}

fn ensure_directory(metadata: RawNodeMetadata, operation: &'static str) -> Result<(), Failure> {
    if metadata.kind == RawFileKind::Directory {
        Ok(())
    } else {
        Err(Failure::new(
            MaterializationFailureKind::InvalidLayout,
            PortErrorKind::InvalidLayout,
            operation,
        ))
    }
}

fn ensure_identity(
    metadata: RawNodeMetadata,
    expected: FileIdentity,
    kind: RawFileKind,
) -> Result<(), Failure> {
    if metadata.identity() == expected && metadata.kind == kind {
        Ok(())
    } else {
        Err(target_changed())
    }
}

fn component_name(name: &OsString) -> Result<CString, Failure> {
    component_name_io(name)
        .map_err(|error| Failure::invalid("encode materialization path component", error))
}

fn component_name_io(name: &OsString) -> io::Result<CString> {
    c_string(name.as_bytes())
}

fn relative_bytes(components: &[OsString]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (index, component) in components.iter().enumerate() {
        if index != 0 {
            bytes.push(b'/');
        }
        bytes.extend_from_slice(component.as_bytes());
    }
    bytes
}

fn materialized_kind(kind: RawFileKind) -> MaterializedEntryKind {
    match kind {
        RawFileKind::Directory => MaterializedEntryKind::Directory,
        RawFileKind::RegularFile => MaterializedEntryKind::RegularFile,
        RawFileKind::SymbolicLink => MaterializedEntryKind::SymbolicLink,
        RawFileKind::Fifo
        | RawFileKind::Socket
        | RawFileKind::CharacterDevice
        | RawFileKind::BlockDevice
        | RawFileKind::Unknown => unreachable!("unsupported entries are never registered"),
    }
}

fn source_changed() -> Failure {
    Failure::new(
        MaterializationFailureKind::SourceChanged,
        PortErrorKind::Conflict,
        "source changed during APFS materialization",
    )
}

fn target_changed() -> Failure {
    Failure::new(
        MaterializationFailureKind::TargetChanged,
        PortErrorKind::Conflict,
        "target changed during APFS materialization",
    )
}

fn update_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn update_time(hasher: &mut blake3::Hasher, time: (i64, i64)) {
    hasher.update(&time.0.to_le_bytes());
    hasher.update(&time.1.to_le_bytes());
}

const fn kind_byte(kind: RawFileKind) -> u8 {
    match kind {
        RawFileKind::Directory => 1,
        RawFileKind::RegularFile => 2,
        RawFileKind::SymbolicLink => 3,
        RawFileKind::Fifo => 4,
        RawFileKind::Socket => 5,
        RawFileKind::CharacterDevice => 6,
        RawFileKind::BlockDevice => 7,
        RawFileKind::Unknown => 8,
    }
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

impl RawNodeMetadata {
    const fn identity(self) -> FileIdentity {
        FileIdentity::new(self.device, self.inode)
    }

    const fn mtime(self) -> (i64, i64) {
        (self.modified_seconds, self.modified_nanoseconds)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::error::Error;
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    use tempfile::{Builder, TempDir};
    use thinws_core::{FallbackPolicy, MaterializationOutcome, MaterializationPlan};

    use super::*;

    fn absolute(path: &Path) -> AbsolutePath {
        AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
    }

    fn fixture(
        prefix: &str,
    ) -> (
        TempDir,
        MaterializeRequest,
        MaterializationPlan,
        MacOsHostAdapter,
    ) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/p1-06-materializer-unit-tests");
        fs::create_dir_all(&root).unwrap();
        let temp = Builder::new()
            .prefix(prefix)
            .tempdir_in(fs::canonicalize(root).unwrap())
            .unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        let staging = temp.path().join("staging");
        let trash = temp.path().join("trash");
        for directory in [&source, &target, &staging, &trash] {
            fs::create_dir(directory).unwrap();
        }
        fs::write(source.join("file.txt"), b"source bytes").unwrap();
        let adapter = MacOsHostAdapter::new(temp.path().join("bootstrap")).unwrap();
        let request = MaterializeRequest::new(
            absolute(&source),
            absolute(&target),
            absolute(&staging),
            absolute(&trash),
        );
        let report = adapter
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
            .unwrap();
        let plan = MaterializationPlan::for_apfs_clone(&report, FallbackPolicy::Deny).unwrap();
        (temp, request, plan, adapter)
    }

    struct MutateSourceHook {
        source_file: PathBuf,
        fired: Cell<bool>,
    }

    impl ExecutionHook for MutateSourceHook {
        fn after_created(
            &self,
            _relative: &[OsString],
            _created_count: usize,
        ) -> Result<(), Failure> {
            if !self.fired.replace(true) {
                fs::write(&self.source_file, b"source changed while cloning").unwrap();
            }
            Ok(())
        }
    }

    #[test]
    fn source_change_after_clone_returns_partial_receipt_and_rolls_back() {
        let (temp, request, plan, adapter) = fixture("source-change-");
        let hook = MutateSourceHook {
            source_file: temp.path().join("source/file.txt"),
            fired: Cell::new(false),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::SourceChanged)
        );
        assert_eq!(failure.receipt().created().len(), 1);
        assert_eq!(failure.receipt().regular_file_count(), Some(1));
        assert_eq!(failure.receipt().clone_calls_succeeded(), 1);
        assert!(failure.receipt().logical_bytes().is_some());
        assert!(failure.receipt().source_manifest_digest().is_some());
        assert!(failure.receipt().target_manifest_digest().is_some());
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::ConfirmedBaseline
        );
        assert_eq!(failure.receipt().rollback().quarantined().len(), 1);
        assert!(
            fs::read_dir(temp.path().join("target"))
                .unwrap()
                .next()
                .is_none()
        );
    }

    struct ReplaceAfterProbeHook {
        target: PathBuf,
        displaced: PathBuf,
    }

    impl ExecutionHook for ReplaceAfterProbeHook {
        fn after_probe(&self) -> Result<(), Failure> {
            fs::rename(&self.target, &self.displaced).unwrap();
            fs::create_dir(&self.target).unwrap();
            Ok(())
        }
    }

    #[test]
    fn target_replacement_after_probe_is_rejected_before_writes() {
        let (temp, request, plan, adapter) = fixture("post-probe-root-replacement-");
        let target = temp.path().join("target");
        let displaced = temp.path().join("displaced-target");
        let hook = ReplaceAfterProbeHook {
            target: target.clone(),
            displaced: displaced.clone(),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::PlanStale)
        );
        assert_eq!(failure.receipt().outcome(), MaterializationOutcome::Failed);
        assert!(fs::read_dir(target).unwrap().next().is_none());
        assert!(fs::read_dir(displaced).unwrap().next().is_none());
    }

    struct MoveTargetBeforeFinalValidationHook {
        target: PathBuf,
        displaced: PathBuf,
    }

    impl ExecutionHook for MoveTargetBeforeFinalValidationHook {
        fn before_final_validation(&self) -> Result<(), Failure> {
            fs::rename(&self.target, &self.displaced).unwrap();
            fs::create_dir(&self.target).unwrap();
            Ok(())
        }
    }

    #[test]
    fn target_root_move_during_execution_cannot_return_success_or_touch_replacement() {
        let (temp, request, plan, adapter) = fixture("mid-execution-root-move-");
        let target = temp.path().join("target");
        let displaced = temp.path().join("displaced-target");
        let hook = MoveTargetBeforeFinalValidationHook {
            target: target.clone(),
            displaced: displaced.clone(),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::PlanStale)
        );
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert!(fs::read_dir(target).unwrap().next().is_none());
        assert_eq!(
            fs::read(displaced.join("file.txt")).unwrap(),
            b"source bytes"
        );
    }

    struct ReplaceCloneWithSourceHardLinkHook {
        source_file: PathBuf,
        target_file: PathBuf,
        fired: Cell<bool>,
    }

    impl ExecutionHook for ReplaceCloneWithSourceHardLinkHook {
        fn after_created(
            &self,
            _relative: &[OsString],
            _created_count: usize,
        ) -> Result<(), Failure> {
            if !self.fired.replace(true) {
                fs::remove_file(&self.target_file).unwrap();
                fs::hard_link(&self.source_file, &self.target_file).unwrap();
            }
            Ok(())
        }
    }

    #[test]
    fn source_hard_link_replacement_is_detected_without_claiming_cow_success() {
        let (temp, request, plan, adapter) = fixture("source-hard-link-");
        let source_file = temp.path().join("source/file.txt");
        let target_file = temp.path().join("target/file.txt");
        let hook = ReplaceCloneWithSourceHardLinkHook {
            source_file: source_file.clone(),
            target_file: target_file.clone(),
            fired: Cell::new(false),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::TargetChanged)
        );
        assert_eq!(
            failure.receipt().cow_evidence(),
            thinws_core::CowEvidence::Unknown
        );
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert_eq!(
            fs::metadata(source_file).unwrap().ino(),
            fs::metadata(target_file).unwrap().ino()
        );
    }

    struct ReplaceTargetHook {
        target_file: PathBuf,
        fired: Cell<bool>,
    }

    impl ExecutionHook for ReplaceTargetHook {
        fn after_created(
            &self,
            _relative: &[OsString],
            _created_count: usize,
        ) -> Result<(), Failure> {
            if !self.fired.replace(true) {
                fs::remove_file(&self.target_file).unwrap();
                fs::write(&self.target_file, b"replacement owned by another actor").unwrap();
                return Err(target_changed());
            }
            Ok(())
        }
    }

    #[test]
    fn rollback_preserves_a_replacement_object_and_reports_incomplete() {
        let (temp, request, plan, adapter) = fixture("replacement-");
        let target_file = temp.path().join("target/file.txt");
        let hook = ReplaceTargetHook {
            target_file: target_file.clone(),
            fired: Cell::new(false),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::TargetChanged)
        );
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert_eq!(failure.receipt().rollback().remaining().len(), 1);
        assert_eq!(
            fs::read(target_file).unwrap(),
            b"replacement owned by another actor"
        );
    }

    struct AddUnknownTargetHook {
        unknown_file: PathBuf,
        fired: Cell<bool>,
    }

    impl ExecutionHook for AddUnknownTargetHook {
        fn after_created(
            &self,
            _relative: &[OsString],
            _created_count: usize,
        ) -> Result<(), Failure> {
            if !self.fired.replace(true) {
                fs::write(&self.unknown_file, b"external object").unwrap();
                return Err(target_changed());
            }
            Ok(())
        }
    }

    #[test]
    fn rollback_does_not_delete_anything_when_an_unknown_object_appears() {
        let (temp, request, plan, adapter) = fixture("unknown-object-");
        let hook = AddUnknownTargetHook {
            unknown_file: temp.path().join("target/unknown.txt"),
            fired: Cell::new(false),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert!(temp.path().join("target/file.txt").exists());
        assert!(temp.path().join("target/unknown.txt").exists());
        assert!(failure.receipt().rollback().removed().is_empty());
    }

    struct ReplaceBetweenRollbackCheckAndDetachHook {
        target_file: PathBuf,
        fired: Cell<bool>,
    }

    impl ExecutionHook for ReplaceBetweenRollbackCheckAndDetachHook {
        fn after_created(
            &self,
            _relative: &[OsString],
            _created_count: usize,
        ) -> Result<(), Failure> {
            Err(target_changed())
        }

        fn before_rollback_detach(&self, _relative: &[OsString]) -> Result<(), Failure> {
            if !self.fired.replace(true) {
                fs::remove_file(&self.target_file).unwrap();
                fs::write(&self.target_file, b"replacement during rollback").unwrap();
            }
            Ok(())
        }
    }

    #[test]
    fn rollback_never_deletes_an_object_replaced_after_its_last_identity_check() {
        let (temp, request, plan, adapter) = fixture("rollback-detach-race-");
        let target_file = temp.path().join("target/file.txt");
        let hook = ReplaceBetweenRollbackCheckAndDetachHook {
            target_file: target_file.clone(),
            fired: Cell::new(false),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert!(failure.receipt().rollback().removed().is_empty());
        assert_eq!(
            fs::read(target_file).unwrap(),
            b"replacement during rollback"
        );
    }

    struct RootMetadataFailureHook;

    impl ExecutionHook for RootMetadataFailureHook {
        fn before_root_metadata(&self, target: &OwnedFd) -> Result<(), Failure> {
            let before = node_metadata(target).unwrap();
            set_modified_time(
                target,
                before.modified_seconds.saturating_sub(1),
                before.modified_nanoseconds,
            )
            .unwrap();
            Err(Failure::io(
                "injected target root metadata failure",
                io::Error::from_raw_os_error(libc::EIO),
            ))
        }
    }

    #[test]
    fn empty_tree_root_metadata_failure_is_partial_and_restores_the_baseline() {
        let (temp, request, plan, adapter) = fixture("root-metadata-");
        fs::remove_file(temp.path().join("source/file.txt")).unwrap();
        let target = open_absolute_directory(&absolute(&temp.path().join("target"))).unwrap();
        let baseline = node_metadata(&target).unwrap();

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &RootMetadataFailureHook)
            .unwrap_err();

        assert_eq!(failure.receipt().outcome(), MaterializationOutcome::Partial);
        assert!(failure.receipt().created().is_empty());
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::ConfirmedBaseline
        );
        assert_eq!(node_metadata(&target).unwrap(), baseline);
    }

    #[test]
    fn submount_guard_rejects_a_directory_entry_from_another_device() {
        let (temp, _, _, _) = fixture("submount-guard-");
        let source = open_absolute_directory(&absolute(&temp.path().join("source"))).unwrap();
        let mut snapshot = TreeSnapshot {
            root: node_metadata(&source).unwrap(),
            entries: BTreeMap::new(),
            manifest: TreeManifest {
                root_mode: 0,
                root_mtime: (0, 0),
                entries: Vec::new(),
                regular_files: 0,
                logical_bytes: 0,
                physical_bytes: 0,
            },
        };

        let error = snapshot_directory(&source, &[], u64::MAX, &mut snapshot).unwrap_err();
        assert_eq!(
            error.kind,
            MaterializationFailureKind::UnsupportedSourceEntry
        );
    }

    #[test]
    fn port_error_chain_preserves_original_errno_values() {
        for (failure, expected_errno) in [
            (
                Failure::io("injected EIO", io::Error::from_raw_os_error(libc::EIO)),
                libc::EIO,
            ),
            (
                Failure::io(
                    "injected ENOSPC",
                    io::Error::from_raw_os_error(libc::ENOSPC),
                ),
                libc::ENOSPC,
            ),
            (
                Failure::clone_io("injected EXDEV", io::Error::from_raw_os_error(libc::EXDEV)),
                libc::EXDEV,
            ),
        ] {
            let error = failure.into_port_error();
            let source = error.source().unwrap().downcast_ref::<io::Error>().unwrap();
            assert_eq!(source.raw_os_error(), Some(expected_errno));
        }
    }
}

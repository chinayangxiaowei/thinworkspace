//! Linux Btrfs implementation of the frozen Workspace materialization Port.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use rustix::fs::{AtFlags, Mode, OFlags, RenameFlags, Timespec, Timestamps, UTIME_OMIT};
use thinws_core::{
    AbsolutePath, CreatedObjectEvidence, FileIdentity, MaterializationAttemptEvidence,
    MaterializationFailureKind, MaterializationMode, MaterializationPathReport,
    MaterializationPlan, MaterializationReceipt, MaterializeRequest, MaterializedEntryKind,
    MaterializerKind, PathCapabilityReport, PathResolution, RelativePath, RollbackEvidence,
    RollbackStatus, SupportState, TreeDigest, VolumeId,
};
use thinws_ports::{
    MaterializationFailure, MaterializationPathProbeRequest, PlatformProbe, PortErrorKind,
    WorkspaceMaterializer,
};

use crate::LinuxPlatformProbe;
use crate::ffi::{btrfs_fsid, reflink_clone};
use crate::tree::{
    DIRECTORY_FLAGS, Node, NodeKind, TreeFailure, TreeSnapshot, directory_names, mount_id,
    mount_id_at, node, node_at, open_directory, open_file, snapshot,
};

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Real `FICLONE` materializer for one proven same-mount Btrfs path set.
#[derive(Clone, Copy, Default)]
pub struct BtrfsReflinkMaterializer;

impl BtrfsReflinkMaterializer {
    /// Creates a materializer; capability and identity are rechecked per call.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    fn materialize_with_hook(
        &self,
        request: &MaterializeRequest,
        plan: &MaterializationPlan,
        hook: &dyn ExecutionHook,
    ) -> Result<MaterializationReceipt, MaterializationFailure> {
        materialize_with_hook(request, plan, hook)
    }
}

impl WorkspaceMaterializer for BtrfsReflinkMaterializer {
    fn kind(&self) -> MaterializerKind {
        MaterializerKind::BtrfsReflink
    }

    fn materialize(
        &self,
        request: &MaterializeRequest,
        plan: &MaterializationPlan,
    ) -> Result<MaterializationReceipt, MaterializationFailure> {
        self.materialize_with_hook(request, plan, &NoopHook)
    }
}

trait ExecutionHook {
    fn before_staged_publish(&self, _relative: &[u8]) -> Result<(), TreeFailure> {
        Ok(())
    }

    fn after_published(&self, _relative: &[u8], _count: usize) -> Result<(), TreeFailure> {
        Ok(())
    }

    #[cfg(test)]
    fn before_rollback_entry_identity_check(&self, _relative: &[u8]) {}

    #[cfg(test)]
    fn before_rollback_final_path_check(&self) {}

    #[cfg(test)]
    fn before_rollback_final_root_check(&self) {}
}

struct NoopHook;
impl ExecutionHook for NoopHook {}

fn materialize_with_hook(
    request: &MaterializeRequest,
    plan: &MaterializationPlan,
    hook: &dyn ExecutionHook,
) -> Result<MaterializationReceipt, MaterializationFailure> {
    let started = Instant::now();
    let mut context = ExecutionContext::default();
    let mut bound = None;
    let result = (|| {
        validate_request_plan(request, plan)?;
        let fresh = LinuxPlatformProbe
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(request))
            .map_err(|error| {
                TreeFailure::with_source(
                    MaterializationFailureKind::PlanStale,
                    PortErrorKind::InvalidLayout,
                    "revalidate Btrfs materialization paths",
                    error,
                )
            })?;
        if fresh.evidence_digest() != plan.probe_evidence_digest()
            || fresh.cow_clone().state() == SupportState::Unsupported
        {
            return Err(stale_plan());
        }
        let paths = BoundPaths::open(request, fresh)?;
        ensure_empty(&paths.target)?;
        ensure_empty(&paths.staging)?;
        ensure_empty(&paths.trash)?;
        let source_snapshot = snapshot(&paths.source)?;
        context.source_manifest = Some(source_snapshot.manifest.digest());
        context.regular_files = Some(source_snapshot.manifest.regular_files);
        context.logical_bytes = Some(source_snapshot.manifest.logical_bytes);
        bound = Some(paths);
        let paths = bound.as_ref().expect("just-bound paths exist");
        materialize_directory(
            &paths.source,
            &paths.target,
            &paths.staging,
            &[],
            &source_snapshot,
            &mut context,
            hook,
        )?;
        context.target_modified = true;
        set_preserved_metadata(
            &paths.target,
            node(&paths.target)?.identity(),
            source_snapshot.root,
        )?;
        let source_after = snapshot(&paths.source).map_err(|error| {
            TreeFailure::with_source(
                MaterializationFailureKind::SourceChanged,
                PortErrorKind::InvalidData,
                "revalidate Btrfs source tree",
                error.into_port_error(),
            )
        })?;
        if source_after != source_snapshot {
            return Err(TreeFailure::source_changed());
        }
        let target_after = snapshot(&paths.target).map_err(|error| {
            TreeFailure::with_source(
                MaterializationFailureKind::TargetChanged,
                PortErrorKind::InvalidLayout,
                "revalidate Btrfs target tree",
                error.into_port_error(),
            )
        })?;
        context.target_manifest = Some(target_after.manifest.digest());
        context.physical_bytes = Some(target_after.manifest.physical_bytes);
        if !target_after
            .manifest
            .matches_promised(&source_snapshot.manifest)
            || !created_matches_target(&target_after, &context.created)
        {
            return Err(TreeFailure::new(
                MaterializationFailureKind::ManifestMismatch,
                PortErrorKind::InvalidData,
                "verify Btrfs target manifest",
            ));
        }
        paths.revalidate(request)?;
        MaterializationReceipt::successful_cow_clone(
            plan,
            source_snapshot.manifest.regular_files,
            context.clone_calls,
            context
                .created
                .iter()
                .map(TrackedCreated::evidence)
                .collect(),
            source_snapshot.manifest.digest(),
            target_after.manifest.digest(),
            elapsed_millis(started),
            source_snapshot.manifest.logical_bytes,
            Some(target_after.manifest.physical_bytes),
        )
        .map_err(|error| {
            TreeFailure::with_source(
                MaterializationFailureKind::ManifestMismatch,
                PortErrorKind::InvalidData,
                "construct Btrfs materialization receipt",
                error,
            )
        })
    })();
    match result {
        Ok(receipt) => Ok(receipt),
        Err(error) => {
            if let Some(paths) = bound.as_ref() {
                cleanup_active_staging(paths, request, &mut context);
            }
            let rollback = bound.as_ref().map_or_else(
                || RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new()),
                |paths| rollback_created(paths, request, &context, hook),
            );
            let evidence = MaterializationAttemptEvidence::new(
                context.logical_bytes,
                context.physical_bytes,
                context.regular_files,
                context.clone_calls,
                context.source_manifest,
                context.target_manifest,
            );
            let mut receipt = MaterializationReceipt::failed_cow_clone(
                plan,
                error.kind,
                context
                    .created
                    .iter()
                    .map(TrackedCreated::evidence)
                    .collect(),
                context.target_modified,
                rollback,
                evidence,
                elapsed_millis(started),
            );
            if let Some(staged) = context.active_staging {
                receipt = receipt.with_unconfirmed_staging(staged.evidence());
            }
            Err(MaterializationFailure::new(
                error.into_port_error(),
                receipt,
            ))
        }
    }
}

fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

fn validate_request_plan(
    request: &MaterializeRequest,
    plan: &MaterializationPlan,
) -> Result<(), TreeFailure> {
    if plan.selected_adapter() != MaterializerKind::BtrfsReflink
        || plan.effective_mode() != MaterializationMode::CowClone
        || request.source() != plan.source_path()
        || request.target() != plan.target_path()
        || request.staging() != plan.staging_path()
        || request.trash() != plan.trash_path()
    {
        return Err(stale_plan());
    }
    Ok(())
}

fn stale_plan() -> TreeFailure {
    TreeFailure::new(
        MaterializationFailureKind::PlanStale,
        PortErrorKind::InvalidLayout,
        "Btrfs materialization plan changed",
    )
}

struct BoundPaths {
    source: OwnedFd,
    target: OwnedFd,
    staging: OwnedFd,
    trash: OwnedFd,
    target_baseline: Node,
    report: MaterializationPathReport,
}

impl BoundPaths {
    fn open(
        request: &MaterializeRequest,
        report: MaterializationPathReport,
    ) -> Result<Self, TreeFailure> {
        let source = open_bound_directory(request.source(), report.source())?;
        let target = open_bound_directory(request.target(), report.target_root())?;
        let staging = open_bound_directory(request.staging(), report.staging())?;
        let trash = open_bound_directory(request.trash(), report.trash())?;
        let target_baseline = node(&target)?;
        let expected_mount = mount_id(&source)?;
        if [mount_id(&target)?, mount_id(&staging)?, mount_id(&trash)?]
            .iter()
            .any(|actual| *actual != expected_mount)
        {
            return Err(stale_plan());
        }
        Ok(Self {
            source,
            target,
            staging,
            trash,
            target_baseline,
            report,
        })
    }

    fn revalidate(&self, request: &MaterializeRequest) -> Result<(), TreeFailure> {
        for (path, report, held) in [
            (request.source(), self.report.source(), &self.source),
            (request.target(), self.report.target_root(), &self.target),
            (request.staging(), self.report.staging(), &self.staging),
            (request.trash(), self.report.trash(), &self.trash),
        ] {
            let reopened = open_bound_directory(path, report)?;
            if node(&reopened)?.identity() != node(held)?.identity() {
                return Err(stale_plan());
            }
        }
        Ok(())
    }

    fn revalidate_target_trash(&self, request: &MaterializeRequest) -> Result<(), TreeFailure> {
        for (path, report, held) in [
            (request.target(), self.report.target_root(), &self.target),
            (request.trash(), self.report.trash(), &self.trash),
        ] {
            let reopened = open_bound_directory(path, report)?;
            if node(&reopened)?.identity() != node(held)?.identity() {
                return Err(stale_plan());
            }
        }
        Ok(())
    }
}

fn open_bound_directory(
    path: &AbsolutePath,
    report: &PathCapabilityReport,
) -> Result<OwnedFd, TreeFailure> {
    if report.requested_path() != path || report.resolution() != PathResolution::ExistingDirectory {
        return Err(stale_plan());
    }
    let mut current = rustix::fs::open("/", DIRECTORY_FLAGS, Mode::empty())
        .map_err(|error| TreeFailure::io("open Btrfs filesystem root", error))?;
    let mut ancestry = report.ancestry().iter();
    if ancestry.next().map(|entry| entry.identity()) != Some(node(&current)?.identity()) {
        return Err(stale_plan());
    }
    for part in path.as_bytes()[1..].split(|byte| *byte == b'/') {
        if part.is_empty() {
            continue;
        }
        current = open_directory(&current, OsStr::from_bytes(part))?;
        if ancestry.next().map(|entry| entry.identity()) != Some(node(&current)?.identity()) {
            return Err(stale_plan());
        }
    }
    if ancestry.next().is_some()
        || mount_id(&current)? != report.mount().mount_id().ok_or_else(stale_plan)?
    {
        return Err(stale_plan());
    }
    let statfs = rustix::fs::fstatfs(&current)
        .map_err(|error| TreeFailure::io("inspect Btrfs filesystem type", error))?;
    if statfs.f_type as u64 != libc::BTRFS_SUPER_MAGIC as u64
        || report.filesystem().type_name() != "btrfs"
    {
        return Err(stale_plan());
    }
    let fsid = btrfs_fsid(&current).map_err(|error| {
        TreeFailure::with_source(
            MaterializationFailureKind::PlanStale,
            PortErrorKind::InvalidLayout,
            "inspect Btrfs FSID",
            error,
        )
    })?;
    let volume = VolumeId::from_str(&uuid::Uuid::from_bytes(fsid).hyphenated().to_string())
        .expect("UUID formatting is canonical");
    if report.filesystem().volume_id().known() != Some(&volume) {
        return Err(stale_plan());
    }
    Ok(current)
}

fn ensure_empty(directory: &OwnedFd) -> Result<(), TreeFailure> {
    if directory_names(directory)?.is_empty() {
        Ok(())
    } else {
        Err(TreeFailure::new(
            MaterializationFailureKind::InvalidLayout,
            PortErrorKind::NotEmpty,
            "require empty Btrfs target",
        ))
    }
}

#[derive(Default)]
struct ExecutionContext {
    created: Vec<TrackedCreated>,
    active_staging: Option<TrackedCreated>,
    clone_calls: u64,
    target_modified: bool,
    source_manifest: Option<TreeDigest>,
    target_manifest: Option<TreeDigest>,
    regular_files: Option<u64>,
    logical_bytes: Option<u64>,
    physical_bytes: Option<u64>,
}

#[derive(Clone)]
struct TrackedCreated {
    path: Vec<u8>,
    kind: NodeKind,
    identity: Option<FileIdentity>,
}

impl TrackedCreated {
    fn evidence(&self) -> CreatedObjectEvidence {
        let path = RelativePath::try_from_bytes(self.path.clone())
            .expect("verified tree relative path is valid");
        let kind = match self.kind {
            NodeKind::Directory => MaterializedEntryKind::Directory,
            NodeKind::File => MaterializedEntryKind::RegularFile,
            NodeKind::Symlink => MaterializedEntryKind::SymbolicLink,
            NodeKind::Unsupported => unreachable!("unsupported entry is never created"),
        };
        CreatedObjectEvidence::new(path, kind, self.identity)
    }
}

fn materialize_directory(
    source: &OwnedFd,
    target: &OwnedFd,
    staging: &OwnedFd,
    prefix: &[u8],
    expected: &TreeSnapshot,
    context: &mut ExecutionContext,
    hook: &dyn ExecutionHook,
) -> Result<(), TreeFailure> {
    for name in directory_names(source)? {
        let mut relative = prefix.to_vec();
        if !relative.is_empty() {
            relative.push(b'/');
        }
        relative.extend_from_slice(name.as_bytes());
        let entry = expected
            .entries
            .get(&relative)
            .ok_or_else(TreeFailure::source_changed)?;
        let before = node_at(source, &name)?;
        if before != entry.node {
            return Err(TreeFailure::source_changed());
        }
        match before.kind {
            NodeKind::Directory => {
                let source_child = open_directory(source, &name)?;
                if node(&source_child)? != before {
                    return Err(TreeFailure::source_changed());
                }
                let identity = stage_and_publish(
                    staging,
                    target,
                    &name,
                    relative.clone(),
                    NodeKind::Directory,
                    context,
                    hook,
                    |parent, stage_name| {
                        rustix::fs::mkdirat(parent, stage_name, Mode::from_bits_retain(0o700))
                    },
                    |_, _| Ok(0),
                )?;
                let target_child = open_directory(target, &name)?;
                ensure_identity(node(&target_child)?, identity, NodeKind::Directory)?;
                materialize_directory(
                    &source_child,
                    &target_child,
                    staging,
                    &relative,
                    expected,
                    context,
                    hook,
                )?;
                set_preserved_metadata(&target_child, identity, before)?;
                if node(&source_child)? != before {
                    return Err(TreeFailure::source_changed());
                }
            }
            NodeKind::File => {
                let source_file = open_file(source, &name)?;
                if node(&source_file)? != before {
                    return Err(TreeFailure::source_changed());
                }
                let identity = stage_and_publish(
                    staging,
                    target,
                    &name,
                    relative,
                    NodeKind::File,
                    context,
                    hook,
                    |parent, stage_name| {
                        rustix::fs::openat(
                            parent,
                            stage_name,
                            OFlags::CREATE
                                | OFlags::EXCL
                                | OFlags::RDWR
                                | OFlags::NOFOLLOW
                                | OFlags::CLOEXEC,
                            Mode::from_bits_retain(0o600),
                        )
                        .map(|_| ())
                    },
                    |parent, stage_name| {
                        let destination = rustix::fs::openat(
                            parent,
                            stage_name,
                            OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                            Mode::empty(),
                        )
                        .map_err(|error| TreeFailure::io("open staged Btrfs file", error))?;
                        reflink_clone(&source_file, &destination).map_err(classify_clone_error)?;
                        Ok(1)
                    },
                )?;
                let target_file = open_file(target, &name)?;
                ensure_identity(node(&target_file)?, identity, NodeKind::File)?;
                set_preserved_metadata(&target_file, identity, before)?;
                if node(&source_file)? != before {
                    return Err(TreeFailure::source_changed());
                }
            }
            NodeKind::Symlink => {
                let text = rustix::fs::readlinkat(source, &name, Vec::new())
                    .map_err(|error| TreeFailure::io("read source symbolic link", error))?;
                if entry.link_text.as_deref() != Some(text.to_bytes())
                    || node_at(source, &name)? != before
                {
                    return Err(TreeFailure::source_changed());
                }
                stage_and_publish(
                    staging,
                    target,
                    &name,
                    relative,
                    NodeKind::Symlink,
                    context,
                    hook,
                    |parent, stage_name| rustix::fs::symlinkat(&text, parent, stage_name),
                    |_, _| Ok(0),
                )?;
            }
            NodeKind::Unsupported => {
                return Err(TreeFailure::new(
                    MaterializationFailureKind::UnsupportedSourceEntry,
                    PortErrorKind::InvalidData,
                    "reject unsupported source entry",
                ));
            }
        }
    }
    Ok(())
}

fn classify_clone_error(error: std::io::Error) -> TreeFailure {
    let (kind, port_kind) = match error.raw_os_error() {
        Some(libc::EOPNOTSUPP | libc::ENOTTY | libc::EINVAL) => (
            MaterializationFailureKind::CowUnavailable,
            PortErrorKind::CapabilityUnavailable,
        ),
        Some(libc::EXDEV) => (
            MaterializationFailureKind::InvalidLayout,
            PortErrorKind::InvalidLayout,
        ),
        Some(libc::ENOSPC) => (MaterializationFailureKind::NoSpace, PortErrorKind::Io),
        _ => (MaterializationFailureKind::Filesystem, PortErrorKind::Io),
    };
    TreeFailure::with_source(kind, port_kind, "clone Btrfs file with FICLONE", error)
}

#[allow(clippy::too_many_arguments)]
fn stage_and_publish(
    staging: &OwnedFd,
    target: &OwnedFd,
    target_name: &OsStr,
    relative: Vec<u8>,
    kind: NodeKind,
    context: &mut ExecutionContext,
    hook: &dyn ExecutionHook,
    mut create: impl FnMut(&OwnedFd, &OsStr) -> Result<(), rustix::io::Errno>,
    mut prepare: impl FnMut(&OwnedFd, &OsStr) -> Result<u64, TreeFailure>,
) -> Result<FileIdentity, TreeFailure> {
    let mut selected = None;
    for _ in 0..128 {
        let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = OsString::from(format!(
            ".thinws-materialize-{}-{sequence}",
            std::process::id()
        ));
        match create(staging, &name) {
            Ok(()) => {
                selected = Some(name);
                break;
            }
            Err(rustix::io::Errno::EXIST) => continue,
            Err(error) => return Err(TreeFailure::io("create staged Btrfs entry", error)),
        }
    }
    let staged_name = selected.ok_or_else(|| {
        TreeFailure::new(
            MaterializationFailureKind::Filesystem,
            PortErrorKind::Io,
            "reserve Btrfs staging name",
        )
    })?;
    context.active_staging = Some(TrackedCreated {
        path: staged_name.as_bytes().to_vec(),
        kind,
        identity: None,
    });
    let staged = node_at(staging, &staged_name)?;
    if staged.kind != kind {
        return Err(TreeFailure::target_changed());
    }
    let identity = staged.identity();
    context
        .active_staging
        .as_mut()
        .expect("staging just recorded")
        .identity = Some(identity);
    let clone_calls = prepare(staging, &staged_name)?;
    context.clone_calls = context.clone_calls.saturating_add(clone_calls);
    ensure_identity(node_at(staging, &staged_name)?, identity, kind)?;
    hook.before_staged_publish(&relative)?;
    if let Err(error) = rustix::fs::renameat_with(
        staging,
        &staged_name,
        target,
        target_name,
        RenameFlags::NOREPLACE,
    ) {
        return Err(if error == rustix::io::Errno::EXIST {
            TreeFailure::target_changed()
        } else {
            TreeFailure::io("publish staged Btrfs entry", error)
        });
    }
    context.active_staging = None;
    context.created.push(TrackedCreated {
        path: relative,
        kind,
        identity: Some(identity),
    });
    context.target_modified = true;
    ensure_identity(node_at(target, target_name)?, identity, kind)?;
    hook.after_published(
        &context
            .created
            .last()
            .expect("published entry is recorded")
            .path,
        context.created.len(),
    )?;
    Ok(identity)
}

fn ensure_identity(
    observed: Node,
    expected: FileIdentity,
    kind: NodeKind,
) -> Result<(), TreeFailure> {
    if observed.identity() != expected || observed.kind != kind {
        Err(TreeFailure::target_changed())
    } else {
        Ok(())
    }
}

fn set_preserved_metadata(
    target: &OwnedFd,
    identity: FileIdentity,
    source: Node,
) -> Result<(), TreeFailure> {
    ensure_identity(node(target)?, identity, source.kind)?;
    let times = Timestamps {
        last_access: Timespec {
            tv_sec: 0,
            tv_nsec: UTIME_OMIT,
        },
        last_modification: Timespec {
            tv_sec: source.mtime_seconds,
            tv_nsec: source.mtime_nanoseconds,
        },
    };
    rustix::fs::futimens(target, &times)
        .map_err(|error| TreeFailure::io("preserve Btrfs entry modification time", error))?;
    rustix::fs::fchmod(target, Mode::from_bits_retain(source.mode))
        .map_err(|error| TreeFailure::io("preserve Btrfs entry permissions", error))?;
    let after = node(target)?;
    ensure_identity(after, identity, source.kind)?;
    if after.mode != source.mode || after.mtime() != source.mtime() {
        return Err(TreeFailure::new(
            MaterializationFailureKind::ManifestMismatch,
            PortErrorKind::InvalidData,
            "verify Btrfs entry metadata",
        ));
    }
    Ok(())
}

fn created_matches_target(target: &TreeSnapshot, created: &[TrackedCreated]) -> bool {
    target.entries.len() == created.len()
        && created.iter().all(|entry| {
            target.entries.get(&entry.path).is_some_and(|observed| {
                observed.node.kind == entry.kind && entry.identity == Some(observed.node.identity())
            })
        })
}

fn cleanup_active_staging(
    paths: &BoundPaths,
    request: &MaterializeRequest,
    context: &mut ExecutionContext,
) {
    let Some(staged) = context.active_staging.as_ref() else {
        return;
    };
    let Some(identity) = staged.identity else {
        return;
    };
    if open_bound_directory(request.staging(), paths.report.staging())
        .ok()
        .and_then(|reopened| node(&reopened).ok())
        .map(Node::identity)
        != node(&paths.staging).ok().map(Node::identity)
    {
        return;
    }
    let name = OsStr::from_bytes(&staged.path);
    if node_at(&paths.staging, name)
        .ok()
        .is_none_or(|observed| observed.identity() != identity || observed.kind != staged.kind)
    {
        return;
    }
    let flags = if staged.kind == NodeKind::Directory {
        AtFlags::REMOVEDIR
    } else {
        AtFlags::empty()
    };
    if rustix::fs::unlinkat(&paths.staging, name, flags).is_ok() {
        context.active_staging = None;
    }
}

fn rollback_created(
    paths: &BoundPaths,
    request: &MaterializeRequest,
    context: &ExecutionContext,
    _hook: &dyn ExecutionHook,
) -> RollbackEvidence {
    if !context.target_modified && context.created.is_empty() {
        return RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new());
    }
    let baseline = paths.target_baseline;
    if paths.revalidate_target_trash(request).is_err()
        || node(&paths.target).ok().is_none_or(|root| {
            root.identity() != baseline.identity() || root.kind != NodeKind::Directory
        })
    {
        return incomplete_rollback(&context.created);
    }
    if node(&paths.target)
        .ok()
        .is_some_and(|root| root.mode != baseline.mode)
        && rustix::fs::fchmod(&paths.target, Mode::from_bits_retain(baseline.mode)).is_err()
    {
        return incomplete_rollback(&context.created);
    }
    let mut removed = Vec::new();
    let mut quarantined = Vec::new();
    let mut unconfirmed_quarantined = Vec::new();
    let mut active_count = context.created.len();
    for entry in context.created.iter().rev() {
        if !rollback_set_matches(&paths.target, &context.created[..active_count])
            || paths.revalidate_target_trash(request).is_err()
        {
            break;
        }
        let Some(identity) = entry.identity else {
            break;
        };
        let Ok((parent, name)) = open_rollback_parent(&paths.target, entry, &context.created)
        else {
            break;
        };
        #[cfg(test)]
        _hook.before_rollback_entry_identity_check(&entry.path);
        if node_at(&parent, &name)
            .ok()
            .is_none_or(|current| current.identity() != identity || current.kind != entry.kind)
        {
            break;
        }
        match detach_to_trash(&parent, &name, &paths.trash, entry, removed.len()) {
            DetachOutcome::Confirmed(path) => {
                quarantined.push(path);
                removed.push(entry.evidence().path().clone());
                active_count -= 1;
            }
            DetachOutcome::Unconfirmed(path) => {
                unconfirmed_quarantined.push(path);
                break;
            }
            DetachOutcome::NotDetached => break,
        }
    }
    let exact_remaining = rollback_set_matches(&paths.target, &context.created[..active_count]);
    let restored = exact_remaining
        && {
            #[cfg(test)]
            _hook.before_rollback_final_path_check();
            paths.revalidate_target_trash(request).is_ok()
        }
        && set_preserved_metadata(&paths.target, baseline.identity(), baseline).is_ok()
        && {
            #[cfg(test)]
            _hook.before_rollback_final_root_check();
            node(&paths.target).is_ok_and(|root| root_baseline_matches(root, baseline))
        };
    let remaining = context.created[..active_count]
        .iter()
        .map(TrackedCreated::evidence)
        .collect::<Vec<_>>();
    let status = if remaining.is_empty() && restored {
        RollbackStatus::ConfirmedBaseline
    } else {
        RollbackStatus::Incomplete
    };
    RollbackEvidence::new(status, removed, remaining)
        .with_quarantined(quarantined)
        .with_unconfirmed_quarantined(unconfirmed_quarantined)
}

fn incomplete_rollback(created: &[TrackedCreated]) -> RollbackEvidence {
    RollbackEvidence::new(
        RollbackStatus::Incomplete,
        Vec::new(),
        created.iter().map(TrackedCreated::evidence).collect(),
    )
}

fn root_baseline_matches(current: Node, baseline: Node) -> bool {
    current.identity() == baseline.identity()
        && current.kind == NodeKind::Directory
        && current.mode == baseline.mode
        && current.mtime() == baseline.mtime()
}

fn rollback_set_matches(target: &OwnedFd, created: &[TrackedCreated]) -> bool {
    let Ok(root_mount) = mount_id(target) else {
        return false;
    };
    let mut observed = BTreeMap::new();
    if observe_target_entries(target, &[], root_mount, &mut observed).is_err()
        || observed.len() != created.len()
    {
        return false;
    }
    created.iter().all(|entry| {
        entry
            .identity
            .is_some_and(|identity| observed.get(&entry.path) == Some(&(identity, entry.kind)))
    })
}

fn observe_target_entries(
    directory: &OwnedFd,
    prefix: &[u8],
    root_mount: u64,
    observed: &mut BTreeMap<Vec<u8>, (FileIdentity, NodeKind)>,
) -> Result<(), TreeFailure> {
    for name in directory_names(directory)? {
        let mut path = prefix.to_vec();
        if !path.is_empty() {
            path.push(b'/');
        }
        path.extend_from_slice(name.as_bytes());
        let current = node_at(directory, &name)?;
        if mount_id_at(directory, &name)? != root_mount {
            return Err(TreeFailure::target_changed());
        }
        observed.insert(path.clone(), (current.identity(), current.kind));
        if current.kind == NodeKind::Directory {
            let child = open_directory(directory, &name)?;
            ensure_identity(node(&child)?, current.identity(), NodeKind::Directory)?;
            observe_target_entries(&child, &path, root_mount, observed)?;
        }
    }
    Ok(())
}

fn open_rollback_parent(
    target: &OwnedFd,
    entry: &TrackedCreated,
    created: &[TrackedCreated],
) -> Result<(OwnedFd, OsString), TreeFailure> {
    let mut parts = entry.path.split(|byte| *byte == b'/').collect::<Vec<_>>();
    let leaf = parts.pop().ok_or_else(TreeFailure::target_changed)?;
    let mut parent = rustix::io::dup(target)
        .map_err(|error| TreeFailure::io("duplicate rollback target", error))?;
    let mut prefix = Vec::new();
    for component in parts {
        if !prefix.is_empty() {
            prefix.push(b'/');
        }
        prefix.extend_from_slice(component);
        parent = open_directory(&parent, OsStr::from_bytes(component))?;
        let expected = created
            .iter()
            .find(|candidate| candidate.path == prefix && candidate.kind == NodeKind::Directory)
            .and_then(|candidate| candidate.identity)
            .ok_or_else(TreeFailure::target_changed)?;
        ensure_identity(node(&parent)?, expected, NodeKind::Directory)?;
    }
    Ok((parent, OsString::from_vec(leaf.to_vec())))
}

enum DetachOutcome {
    Confirmed(RelativePath),
    Unconfirmed(RelativePath),
    NotDetached,
}

fn detach_to_trash(
    parent: &OwnedFd,
    name: &OsStr,
    trash: &OwnedFd,
    entry: &TrackedCreated,
    sequence: usize,
) -> DetachOutcome {
    let Some(identity) = entry.identity else {
        return DetachOutcome::NotDetached;
    };
    for collision in 0..128 {
        let quarantine = OsString::from(format!(
            ".thinws-rollback-{}-{}-{sequence}-{collision}",
            identity.device(),
            identity.inode()
        ));
        match rustix::fs::renameat_with(parent, name, trash, &quarantine, RenameFlags::NOREPLACE) {
            Ok(()) => {
                let relative = RelativePath::try_from_bytes(quarantine.as_bytes().to_vec())
                    .expect("generated quarantine name is valid");
                let matches = node_at(trash, &quarantine)
                    .is_ok_and(|moved| moved.identity() == identity && moved.kind == entry.kind)
                    && (entry.kind != NodeKind::Directory
                        || open_directory(trash, &quarantine)
                            .and_then(|directory| {
                                ensure_identity(node(&directory)?, identity, NodeKind::Directory)?;
                                directory_names(&directory)
                            })
                            .is_ok_and(|names| names.is_empty()));
                if matches {
                    return DetachOutcome::Confirmed(relative);
                }
                return if rustix::fs::renameat_with(
                    trash,
                    &quarantine,
                    parent,
                    name,
                    RenameFlags::NOREPLACE,
                )
                .is_ok()
                {
                    DetachOutcome::NotDetached
                } else {
                    DetachOutcome::Unconfirmed(relative)
                };
            }
            Err(rustix::io::Errno::EXIST) => continue,
            Err(_) => return DetachOutcome::NotDetached,
        }
    }
    DetachOutcome::NotDetached
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs::{self, File, FileTimes};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use tempfile::TempDir;
    use thinws_core::{CandidateEvidence, CowEvidence, FallbackPolicy, FileSystemIdentity};

    use super::*;

    fn absolute(path: &Path) -> AbsolutePath {
        AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
    }

    fn fixture(
        prefix: &str,
    ) -> (
        TempDir,
        [PathBuf; 4],
        MaterializeRequest,
        MaterializationPlan,
    ) {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs directory");
        let fixture = tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(root)
            .unwrap();
        let paths = ["source", "target", "staging", "trash"].map(|name| fixture.path().join(name));
        for path in &paths {
            fs::create_dir(path).unwrap();
        }
        fs::write(paths[0].join("a"), b"first file").unwrap();
        fs::write(paths[0].join("b"), b"second file").unwrap();
        let request = MaterializeRequest::new(
            absolute(&paths[0]),
            absolute(&paths[1]),
            absolute(&paths[2]),
            absolute(&paths[3]),
        );
        let report = LinuxPlatformProbe
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
            .unwrap();
        let plan = MaterializationPlan::for_cow_clone(&report, FallbackPolicy::Deny).unwrap();
        (fixture, paths, request, plan)
    }

    fn injected_failure() -> TreeFailure {
        TreeFailure::new(
            MaterializationFailureKind::Filesystem,
            PortErrorKind::Io,
            "injected Btrfs failure",
        )
    }

    #[test]
    fn clone_errno_classification_keeps_cross_device_and_no_space_distinct() {
        for (errno, failure, port) in [
            (
                libc::EXDEV,
                MaterializationFailureKind::InvalidLayout,
                PortErrorKind::InvalidLayout,
            ),
            (
                libc::ENOSPC,
                MaterializationFailureKind::NoSpace,
                PortErrorKind::Io,
            ),
            (
                libc::EOPNOTSUPP,
                MaterializationFailureKind::CowUnavailable,
                PortErrorKind::CapabilityUnavailable,
            ),
            (
                libc::EIO,
                MaterializationFailureKind::Filesystem,
                PortErrorKind::Io,
            ),
        ] {
            let classified = classify_clone_error(std::io::Error::from_raw_os_error(errno));
            assert_eq!(classified.kind, failure, "errno {errno}");
            assert_eq!(classified.port_kind, port, "errno {errno}");
        }
    }

    #[test]
    fn frozen_plan_rejects_each_changed_request_path() {
        let (fixture, paths, request, plan) = fixture("thinws-btrfs-plan-roles-");
        assert!(validate_request_plan(&request, &plan).is_ok());
        for role in 0..4 {
            let mut changed = paths.clone();
            changed[role] = fixture.path().join(format!("other-{role}"));
            let changed_request = MaterializeRequest::new(
                absolute(&changed[0]),
                absolute(&changed[1]),
                absolute(&changed[2]),
                absolute(&changed[3]),
            );
            assert_eq!(
                validate_request_plan(&changed_request, &plan)
                    .unwrap_err()
                    .kind,
                MaterializationFailureKind::PlanStale,
                "changed role {role} must invalidate the frozen plan"
            );
        }
    }

    struct ReplaceSourcePathAfterFirstPublish {
        source: PathBuf,
    }

    impl ExecutionHook for ReplaceSourcePathAfterFirstPublish {
        fn after_published(&self, _relative: &[u8], count: usize) -> Result<(), TreeFailure> {
            if count == 1 {
                fs::rename(&self.source, self.source.with_file_name("displaced-source"))
                    .map_err(|error| TreeFailure::io("displace source path for test", error))?;
                fs::create_dir(&self.source)
                    .map_err(|error| TreeFailure::io("replace source path for test", error))?;
                fs::write(self.source.join("foreign"), b"unrelated source")
                    .map_err(|error| TreeFailure::io("write replacement source for test", error))?;
            }
            Ok(())
        }
    }

    #[test]
    fn source_path_replacement_after_clone_cannot_produce_a_success_receipt() {
        let (_fixture, paths, request, plan) = fixture("thinws-btrfs-source-replaced-");
        let failure = BtrfsReflinkMaterializer::new()
            .materialize_with_hook(
                &request,
                &plan,
                &ReplaceSourcePathAfterFirstPublish {
                    source: paths[0].clone(),
                },
            )
            .unwrap_err();
        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::PlanStale)
        );
        assert_eq!(failure.receipt().clone_calls_succeeded(), 2);
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::ConfirmedBaseline
        );
        assert_eq!(
            fs::read(paths[0].join("foreign")).unwrap(),
            b"unrelated source"
        );
        assert_eq!(
            fs::read(paths[0].with_file_name("displaced-source").join("a")).unwrap(),
            b"first file"
        );
        assert!(fs::read_dir(&paths[1]).unwrap().next().is_none());
    }

    #[test]
    fn bound_paths_revalidation_rejects_replacement_of_each_directory_role() {
        for role in 0..4 {
            let (_fixture, paths, request, _plan) = fixture("thinws-btrfs-bound-roles-");
            let report = LinuxPlatformProbe
                .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
                .unwrap();
            let bound = BoundPaths::open(&request, report)
                .unwrap_or_else(|_| panic!("valid Btrfs fixture must bind"));
            let displaced = paths[role].with_file_name(format!("displaced-{role}"));
            fs::rename(&paths[role], &displaced).unwrap();
            fs::create_dir(&paths[role]).unwrap();
            assert_eq!(
                bound.revalidate(&request).unwrap_err().kind,
                MaterializationFailureKind::PlanStale,
                "replaced role {role} must invalidate the bound path"
            );
            assert!(displaced.is_dir());
        }
    }

    #[test]
    fn bound_directory_rejects_each_inconsistent_path_mount_and_filesystem_fact() {
        let (_fixture, paths, request, _plan) = fixture("thinws-btrfs-bound-facts-");
        let report = LinuxPlatformProbe.inspect_path(request.source()).unwrap();
        assert!(open_bound_directory(request.source(), &report).is_ok());
        let variant = |requested_path: AbsolutePath,
                       filesystem: FileSystemIdentity,
                       mount: thinws_core::MountEvidence| {
            PathCapabilityReport::new(
                requested_path,
                report.resolution(),
                report.nearest_existing_ancestor().clone(),
                report.missing_components().to_vec(),
                report.ancestry().to_vec(),
                filesystem,
                mount,
                report.readability(),
                report.writability(),
                report.cow_clone(),
            )
            .unwrap()
        };
        let original_filesystem = report.filesystem().clone();
        let original_mount = report.mount();
        let wrong_requested_path = variant(
            absolute(&paths[1]),
            original_filesystem.clone(),
            original_mount,
        );
        let wrong_mount = variant(
            request.source().clone(),
            original_filesystem.clone(),
            thinws_core::MountEvidence::new(original_mount.raw_flags(), original_mount.writable())
                .with_mount_id(original_mount.mount_id().unwrap() + 1),
        );
        let wrong_filesystem = variant(
            request.source().clone(),
            FileSystemIdentity::new(
                "ext4",
                original_filesystem.fsid(),
                original_filesystem.volume_id().clone(),
            ),
            original_mount,
        );
        for (case, altered) in [
            ("requested path", wrong_requested_path),
            ("mount ID", wrong_mount),
            ("filesystem type", wrong_filesystem),
        ] {
            assert_eq!(
                open_bound_directory(request.source(), &altered)
                    .unwrap_err()
                    .kind,
                MaterializationFailureKind::PlanStale,
                "inconsistent {case} must stale the plan"
            );
        }
    }

    #[test]
    fn btrfs_materializer_rejects_a_plan_for_another_backend() {
        let (_fixture, _paths, request, _plan) = fixture("thinws-btrfs-wrong-backend-");
        let report = LinuxPlatformProbe
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
            .unwrap();
        let as_apfs = |path: &PathCapabilityReport| {
            PathCapabilityReport::new(
                path.requested_path().clone(),
                path.resolution(),
                path.nearest_existing_ancestor().clone(),
                path.missing_components().to_vec(),
                path.ancestry().to_vec(),
                FileSystemIdentity::new(
                    "apfs",
                    path.filesystem().fsid(),
                    path.filesystem().volume_id().clone(),
                ),
                path.mount(),
                path.readability(),
                path.writability(),
                path.cow_clone(),
            )
            .unwrap()
        };
        let other_backend = MaterializationPathReport::new(
            as_apfs(report.source()),
            as_apfs(report.target_root()),
            as_apfs(report.staging()),
            as_apfs(report.trash()),
            CandidateEvidence::new(
                MaterializerKind::ApfsFileClone,
                SupportState::Supported,
                Vec::new(),
            ),
            report.full_copy().clone(),
            report.evidence_digest(),
        );
        let wrong_plan =
            MaterializationPlan::for_cow_clone(&other_backend, FallbackPolicy::Deny).unwrap();
        assert_eq!(
            validate_request_plan(&request, &wrong_plan)
                .unwrap_err()
                .kind,
            MaterializationFailureKind::PlanStale
        );
    }

    #[test]
    fn occupied_target_is_rejected_without_touching_the_foreign_entry() {
        let (_fixture, paths, request, plan) = fixture("thinws-btrfs-target-not-empty-");
        fs::write(paths[1].join("foreign"), b"keep").unwrap();
        let failure = BtrfsReflinkMaterializer::new()
            .materialize(&request, &plan)
            .unwrap_err();
        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::InvalidLayout)
        );
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::NotNeeded
        );
        assert!(failure.receipt().created().is_empty());
        assert_eq!(fs::read(paths[1].join("foreign")).unwrap(), b"keep");
    }

    #[test]
    fn created_entry_identity_and_type_must_both_match() {
        let (_fixture, paths, _request, _plan) = fixture("thinws-btrfs-created-match-");
        let target = rustix::fs::open(&paths[1], DIRECTORY_FLAGS, Mode::empty()).unwrap();
        let observed = node(&target).ok().unwrap();
        assert!(ensure_identity(observed, observed.identity(), NodeKind::Directory).is_ok());
        assert!(
            ensure_identity(
                observed,
                FileIdentity::new(observed.device, observed.inode + 1),
                NodeKind::Directory
            )
            .is_err()
        );
        assert!(ensure_identity(observed, observed.identity(), NodeKind::File).is_err());

        fs::write(paths[1].join("owned"), b"clone").unwrap();
        let tree = snapshot(&target).ok().unwrap();
        let file = tree.entries.get(b"owned".as_slice()).unwrap();
        let created = vec![TrackedCreated {
            path: b"owned".to_vec(),
            kind: NodeKind::File,
            identity: Some(file.node.identity()),
        }];
        assert!(created_matches_target(&tree, &created));
        assert!(rollback_set_matches(&target, &created));

        let mut wrong_kind = created.clone();
        wrong_kind[0].kind = NodeKind::Directory;
        assert!(!created_matches_target(&tree, &wrong_kind));
        assert!(!rollback_set_matches(&target, &wrong_kind));

        let mut wrong_identity = created.clone();
        wrong_identity[0].identity = Some(FileIdentity::new(file.node.device, file.node.inode + 1));
        assert!(!created_matches_target(&tree, &wrong_identity));
        assert!(!rollback_set_matches(&target, &wrong_identity));

        let mut missing_identity = created.clone();
        missing_identity[0].identity = None;
        assert!(!created_matches_target(&tree, &missing_identity));
        assert!(!rollback_set_matches(&target, &missing_identity));

        let mut wrong_path = created.clone();
        wrong_path[0].path = b"other".to_vec();
        assert!(!created_matches_target(&tree, &wrong_path));
        assert!(!rollback_set_matches(&target, &wrong_path));

        fs::write(paths[1].join("foreign"), b"keep").unwrap();
        let enlarged = snapshot(&target).ok().unwrap();
        assert!(!created_matches_target(&enlarged, &created));
        assert!(!rollback_set_matches(&target, &created));
    }

    #[test]
    fn restored_root_requires_every_baseline_fact() {
        let (_fixture, paths, _request, _plan) = fixture("thinws-btrfs-root-baseline-");
        let target = rustix::fs::open(&paths[1], DIRECTORY_FLAGS, Mode::empty()).unwrap();
        let baseline = node(&target).ok().unwrap();
        assert!(root_baseline_matches(baseline, baseline));
        assert!(!root_baseline_matches(
            Node {
                device: baseline.device + 1,
                ..baseline
            },
            baseline
        ));
        assert!(!root_baseline_matches(
            Node {
                inode: baseline.inode + 1,
                ..baseline
            },
            baseline
        ));
        assert!(!root_baseline_matches(
            Node {
                kind: NodeKind::File,
                ..baseline
            },
            baseline
        ));
        assert!(!root_baseline_matches(
            Node {
                mode: baseline.mode ^ 0o100,
                ..baseline
            },
            baseline
        ));
        assert!(!root_baseline_matches(
            Node {
                mtime_seconds: baseline.mtime_seconds + 1,
                ..baseline
            },
            baseline
        ));
        assert!(!root_baseline_matches(
            Node {
                mtime_nanoseconds: baseline.mtime_nanoseconds + 1,
                ..baseline
            },
            baseline
        ));
    }

    struct FailAfterOne;
    impl ExecutionHook for FailAfterOne {
        fn after_published(&self, _relative: &[u8], count: usize) -> Result<(), TreeFailure> {
            if count == 1 {
                Err(injected_failure())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn partial_clone_detaches_only_registered_identity_and_restores_empty_target() {
        let (_fixture, paths, request, plan) = fixture("thinws-btrfs-rollback-");
        let failure = BtrfsReflinkMaterializer::new()
            .materialize_with_hook(&request, &plan, &FailAfterOne)
            .unwrap_err();
        let receipt = failure.receipt();
        assert_eq!(
            receipt.failure_kind(),
            Some(MaterializationFailureKind::Filesystem)
        );
        assert_eq!(receipt.clone_calls_succeeded(), 1);
        assert_eq!(receipt.cow_evidence(), CowEvidence::Unknown);
        assert_eq!(receipt.created().len(), 1);
        assert_eq!(
            receipt.rollback().status(),
            RollbackStatus::ConfirmedBaseline
        );
        assert_eq!(receipt.rollback().removed().len(), 1);
        assert_eq!(receipt.rollback().quarantined().len(), 1);
        assert!(receipt.rollback().remaining().is_empty());
        assert!(fs::read_dir(&paths[1]).unwrap().next().is_none());
        assert_eq!(fs::read_dir(&paths[3]).unwrap().count(), 1);
    }

    struct FailBeforePublish;
    impl ExecutionHook for FailBeforePublish {
        fn before_staged_publish(&self, _relative: &[u8]) -> Result<(), TreeFailure> {
            Err(injected_failure())
        }
    }

    #[test]
    fn failed_staged_clone_is_removed_without_claiming_target_creation() {
        let (_fixture, paths, request, plan) = fixture("thinws-btrfs-stage-fail-");
        let failure = BtrfsReflinkMaterializer::new()
            .materialize_with_hook(&request, &plan, &FailBeforePublish)
            .unwrap_err();
        let receipt = failure.receipt();
        assert_eq!(receipt.clone_calls_succeeded(), 1);
        assert_eq!(receipt.rollback().status(), RollbackStatus::NotNeeded);
        assert!(receipt.created().is_empty());
        assert!(receipt.unconfirmed_staging().is_none());
        assert!(fs::read_dir(&paths[1]).unwrap().next().is_none());
        assert!(fs::read_dir(&paths[2]).unwrap().next().is_none());
    }

    struct OccupyTargetBeforePublish {
        target: PathBuf,
    }

    impl ExecutionHook for OccupyTargetBeforePublish {
        fn before_staged_publish(&self, relative: &[u8]) -> Result<(), TreeFailure> {
            fs::write(
                self.target.join(OsStr::from_bytes(relative)),
                b"foreign target",
            )
            .map_err(|error| TreeFailure::io("occupy Btrfs target name for test", error))
        }
    }

    #[test]
    fn occupied_name_at_publish_is_target_changed_and_preserves_foreign_data() {
        let (_fixture, paths, request, plan) = fixture("thinws-btrfs-publish-race-");
        let failure = BtrfsReflinkMaterializer::new()
            .materialize_with_hook(
                &request,
                &plan,
                &OccupyTargetBeforePublish {
                    target: paths[1].clone(),
                },
            )
            .unwrap_err();
        let receipt = failure.receipt();
        assert_eq!(
            receipt.failure_kind(),
            Some(MaterializationFailureKind::TargetChanged)
        );
        assert_eq!(receipt.clone_calls_succeeded(), 1);
        assert!(receipt.created().is_empty());
        assert_eq!(receipt.rollback().status(), RollbackStatus::NotNeeded);
        assert!(receipt.unconfirmed_staging().is_none());
        assert_eq!(fs::read(paths[1].join("a")).unwrap(), b"foreign target");
        assert!(fs::read_dir(&paths[2]).unwrap().next().is_none());
        assert!(fs::read_dir(&paths[3]).unwrap().next().is_none());
    }

    struct ReplacePublished {
        target: PathBuf,
    }
    impl ExecutionHook for ReplacePublished {
        fn after_published(&self, relative: &[u8], _count: usize) -> Result<(), TreeFailure> {
            let name = OsStr::from_bytes(relative);
            fs::rename(self.target.join(name), self.target.join("displaced")).map_err(|error| {
                TreeFailure::io("replace published Btrfs entry for test", error)
            })?;
            fs::write(self.target.join(name), b"foreign")
                .map_err(|error| TreeFailure::io("create foreign Btrfs entry for test", error))?;
            Err(TreeFailure::target_changed())
        }
    }

    #[test]
    fn rollback_never_deletes_a_foreign_replacement() {
        let (_fixture, paths, request, plan) = fixture("thinws-btrfs-replacement-");
        let hook = ReplacePublished {
            target: paths[1].clone(),
        };
        let failure = BtrfsReflinkMaterializer::new()
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();
        let receipt = failure.receipt();
        assert_eq!(receipt.rollback().status(), RollbackStatus::Incomplete);
        assert_eq!(receipt.rollback().remaining().len(), 1);
        assert_eq!(fs::read(paths[1].join("a")).unwrap(), b"foreign");
        assert_eq!(fs::read(paths[1].join("displaced")).unwrap(), b"first file");
        assert!(fs::read_dir(&paths[3]).unwrap().next().is_none());
    }

    struct ReplaceEntryAfterRollbackSetCheck {
        target: PathBuf,
        trash: PathBuf,
        frozen_trash_mtime: SystemTime,
    }
    impl ExecutionHook for ReplaceEntryAfterRollbackSetCheck {
        fn after_published(&self, _relative: &[u8], _count: usize) -> Result<(), TreeFailure> {
            Err(injected_failure())
        }

        fn before_rollback_entry_identity_check(&self, relative: &[u8]) {
            let name = OsStr::from_bytes(relative);
            fs::rename(self.target.join(name), self.target.join("displaced")).unwrap();
            fs::write(self.target.join(name), b"foreign replacement").unwrap();
            File::open(&self.trash)
                .unwrap()
                .set_times(FileTimes::new().set_modified(self.frozen_trash_mtime))
                .unwrap();
        }
    }

    #[test]
    fn rollback_never_quarantines_a_foreign_entry_replaced_after_set_check() {
        let (_fixture, paths, request, plan) = fixture("thinws-btrfs-entry-race-");
        let frozen_trash_mtime = UNIX_EPOCH + Duration::from_secs(946_684_800);
        let failure = BtrfsReflinkMaterializer::new()
            .materialize_with_hook(
                &request,
                &plan,
                &ReplaceEntryAfterRollbackSetCheck {
                    target: paths[1].clone(),
                    trash: paths[3].clone(),
                    frozen_trash_mtime,
                },
            )
            .unwrap_err();
        let rollback = failure.receipt().rollback();
        assert_eq!(rollback.status(), RollbackStatus::Incomplete);
        assert!(rollback.removed().is_empty());
        assert_eq!(rollback.remaining().len(), 1);
        assert_eq!(
            fs::read(paths[1].join("a")).unwrap(),
            b"foreign replacement"
        );
        assert_eq!(fs::read(paths[1].join("displaced")).unwrap(), b"first file");
        assert!(fs::read_dir(&paths[3]).unwrap().next().is_none());
        assert_eq!(
            fs::metadata(&paths[3]).unwrap().modified().unwrap(),
            frozen_trash_mtime
        );
    }

    #[test]
    fn rollback_restores_a_modified_empty_target_instead_of_claiming_no_work() {
        let (_fixture, paths, request, _plan) = fixture("thinws-btrfs-empty-rollback-");
        let report = LinuxPlatformProbe
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
            .unwrap();
        let bound = BoundPaths::open(&request, report)
            .unwrap_or_else(|_| panic!("valid Btrfs fixture must bind"));
        assert_eq!(
            rollback_created(&bound, &request, &ExecutionContext::default(), &NoopHook).status(),
            RollbackStatus::NotNeeded
        );
        rustix::fs::fchmod(&bound.target, Mode::from_bits_retain(0o500)).unwrap();
        assert_ne!(
            node(&bound.target)
                .unwrap_or_else(|_| panic!("bound target must remain readable"))
                .mode,
            bound.target_baseline.mode
        );
        let modified = ExecutionContext {
            target_modified: true,
            ..ExecutionContext::default()
        };
        let rollback = rollback_created(&bound, &request, &modified, &NoopHook);
        assert_eq!(rollback.status(), RollbackStatus::ConfirmedBaseline);
        assert!(rollback.remaining().is_empty());
        assert!(root_baseline_matches(
            node(&bound.target).unwrap_or_else(|_| panic!("bound target must remain readable")),
            bound.target_baseline
        ));
        assert!(fs::read_dir(&paths[3]).unwrap().next().is_none());
    }

    #[test]
    fn rollback_cannot_confirm_an_empty_ledger_with_a_foreign_target_entry() {
        let (_fixture, paths, request, _plan) = fixture("thinws-btrfs-empty-foreign-");
        let report = LinuxPlatformProbe
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
            .unwrap();
        let bound = BoundPaths::open(&request, report)
            .unwrap_or_else(|_| panic!("valid Btrfs fixture must bind"));
        fs::write(paths[1].join("foreign"), b"unrelated data").unwrap();
        let modified = ExecutionContext {
            target_modified: true,
            ..ExecutionContext::default()
        };

        let rollback = rollback_created(&bound, &request, &modified, &NoopHook);
        assert_eq!(rollback.status(), RollbackStatus::Incomplete);
        assert!(rollback.removed().is_empty());
        assert!(rollback.remaining().is_empty());
        assert_eq!(
            fs::read(paths[1].join("foreign")).unwrap(),
            b"unrelated data"
        );
        assert!(fs::read_dir(&paths[3]).unwrap().next().is_none());
    }

    #[test]
    fn rollback_detach_restores_a_file_with_a_different_registered_identity() {
        let (_fixture, paths, _request, _plan) = fixture("thinws-btrfs-detach-identity-");
        fs::write(paths[1].join("owned"), b"foreign identity").unwrap();
        let target = rustix::fs::open(&paths[1], DIRECTORY_FLAGS, Mode::empty()).unwrap();
        let trash = rustix::fs::open(&paths[3], DIRECTORY_FLAGS, Mode::empty()).unwrap();
        let observed = node_at(&target, OsStr::new("owned"))
            .unwrap_or_else(|_| panic!("target file must be inspectable"));
        let entry = TrackedCreated {
            path: b"owned".to_vec(),
            kind: NodeKind::File,
            identity: Some(FileIdentity::new(observed.device, observed.inode + 1)),
        };
        assert!(matches!(
            detach_to_trash(&target, OsStr::new("owned"), &trash, &entry, 0),
            DetachOutcome::NotDetached
        ));
        assert_eq!(
            fs::read(paths[1].join("owned")).unwrap(),
            b"foreign identity"
        );
        assert!(fs::read_dir(&paths[3]).unwrap().next().is_none());
    }

    struct ReplaceTargetBeforeFinalPathCheck {
        target: PathBuf,
    }
    impl ExecutionHook for ReplaceTargetBeforeFinalPathCheck {
        fn before_rollback_final_path_check(&self) {
            fs::rename(
                &self.target,
                self.target.with_file_name("displaced-at-final-check"),
            )
            .unwrap();
            fs::create_dir(&self.target).unwrap();
            fs::write(self.target.join("foreign"), b"unrelated data").unwrap();
        }
    }

    #[test]
    fn rollback_cannot_confirm_a_target_replaced_before_final_path_check() {
        let (_fixture, paths, request, _plan) = fixture("thinws-btrfs-final-path-");
        let report = LinuxPlatformProbe
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
            .unwrap();
        let bound = BoundPaths::open(&request, report)
            .unwrap_or_else(|_| panic!("valid Btrfs fixture must bind"));
        rustix::fs::fchmod(&bound.target, Mode::from_bits_retain(0o500)).unwrap();
        let modified = ExecutionContext {
            target_modified: true,
            ..ExecutionContext::default()
        };

        let rollback = rollback_created(
            &bound,
            &request,
            &modified,
            &ReplaceTargetBeforeFinalPathCheck {
                target: paths[1].clone(),
            },
        );
        assert_eq!(rollback.status(), RollbackStatus::Incomplete);
        assert_eq!(
            fs::read(paths[1].join("foreign")).unwrap(),
            b"unrelated data"
        );
        assert!(paths[1].with_file_name("displaced-at-final-check").is_dir());
        assert!(fs::read_dir(&paths[3]).unwrap().next().is_none());
    }

    struct ChangeTargetBeforeFinalRootCheck {
        target: PathBuf,
    }
    impl ExecutionHook for ChangeTargetBeforeFinalRootCheck {
        fn before_rollback_final_root_check(&self) {
            fs::set_permissions(&self.target, fs::Permissions::from_mode(0o500)).unwrap();
        }
    }

    #[test]
    fn rollback_cannot_confirm_root_metadata_changed_after_restoration() {
        let (_fixture, paths, request, _plan) = fixture("thinws-btrfs-final-root-");
        let report = LinuxPlatformProbe
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
            .unwrap();
        let bound = BoundPaths::open(&request, report)
            .unwrap_or_else(|_| panic!("valid Btrfs fixture must bind"));
        rustix::fs::fchmod(&bound.target, Mode::from_bits_retain(0o500)).unwrap();
        let modified = ExecutionContext {
            target_modified: true,
            ..ExecutionContext::default()
        };

        let rollback = rollback_created(
            &bound,
            &request,
            &modified,
            &ChangeTargetBeforeFinalRootCheck {
                target: paths[1].clone(),
            },
        );
        let observed_mode = fs::metadata(&paths[1]).unwrap().permissions().mode() & 0o777;
        fs::set_permissions(
            &paths[1],
            fs::Permissions::from_mode(bound.target_baseline.mode),
        )
        .unwrap();
        assert_eq!(rollback.status(), RollbackStatus::Incomplete);
        assert_eq!(observed_mode, 0o500);
        assert!(fs::read_dir(&paths[3]).unwrap().next().is_none());
    }

    struct RemoveTargetSearchPermission {
        target: PathBuf,
    }
    impl ExecutionHook for RemoveTargetSearchPermission {
        fn after_published(&self, _relative: &[u8], _count: usize) -> Result<(), TreeFailure> {
            fs::set_permissions(&self.target, fs::Permissions::from_mode(0o400)).map_err(
                |error| TreeFailure::io("remove target search permission for test", error),
            )?;
            Err(injected_failure())
        }
    }

    #[test]
    fn rollback_restores_target_search_permission_before_examining_created_entries() {
        let (_fixture, paths, request, plan) = fixture("thinws-btrfs-root-mode-rollback-");
        let baseline_mode = fs::metadata(&paths[1]).unwrap().permissions().mode() & 0o777;
        let failure = BtrfsReflinkMaterializer::new()
            .materialize_with_hook(
                &request,
                &plan,
                &RemoveTargetSearchPermission {
                    target: paths[1].clone(),
                },
            )
            .unwrap_err();
        let rollback_status = failure.receipt().rollback().status();
        let result_mode = fs::metadata(&paths[1]).unwrap().permissions().mode() & 0o777;
        fs::set_permissions(&paths[1], fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(rollback_status, RollbackStatus::ConfirmedBaseline);
        assert_eq!(result_mode, baseline_mode);
        assert!(fs::read_dir(&paths[1]).unwrap().next().is_none());
        assert_eq!(fs::read_dir(&paths[3]).unwrap().count(), 1);
    }

    struct ReplaceTargetRoot {
        target: PathBuf,
    }
    impl ExecutionHook for ReplaceTargetRoot {
        fn after_published(&self, _relative: &[u8], _count: usize) -> Result<(), TreeFailure> {
            let displaced = self.target.with_file_name("displaced-target");
            fs::rename(&self.target, &displaced)
                .map_err(|error| TreeFailure::io("displace target root for test", error))?;
            fs::set_permissions(&displaced, fs::Permissions::from_mode(0o500))
                .map_err(|error| TreeFailure::io("change displaced root mode for test", error))?;
            fs::create_dir(&self.target)
                .map_err(|error| TreeFailure::io("replace target root for test", error))?;
            fs::write(self.target.join("foreign"), b"unrelated data")
                .map_err(|error| TreeFailure::io("write replacement target for test", error))?;
            Err(TreeFailure::target_changed())
        }
    }

    #[test]
    fn rollback_does_not_touch_a_replaced_target_root() {
        let (_fixture, paths, request, plan) = fixture("thinws-btrfs-root-replacement-");
        let failure = BtrfsReflinkMaterializer::new()
            .materialize_with_hook(
                &request,
                &plan,
                &ReplaceTargetRoot {
                    target: paths[1].clone(),
                },
            )
            .unwrap_err();
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert_eq!(failure.receipt().rollback().remaining().len(), 1);
        assert_eq!(
            fs::read(paths[1].join("foreign")).unwrap(),
            b"unrelated data"
        );
        assert_eq!(
            fs::read(paths[1].with_file_name("displaced-target").join("a")).unwrap(),
            b"first file"
        );
        assert_eq!(
            fs::metadata(paths[1].with_file_name("displaced-target"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o500
        );
        assert!(fs::read_dir(&paths[3]).unwrap().next().is_none());
    }

    struct SaturateRollbackQuarantine {
        target: PathBuf,
        trash: PathBuf,
    }
    impl ExecutionHook for SaturateRollbackQuarantine {
        fn after_published(&self, relative: &[u8], _count: usize) -> Result<(), TreeFailure> {
            let published = fs::metadata(self.target.join(OsStr::from_bytes(relative)))
                .map_err(|error| TreeFailure::io("stat published entry for test", error))?;
            for collision in 0..128 {
                let name = format!(
                    ".thinws-rollback-{}-{}-0-{collision}",
                    published.dev(),
                    published.ino()
                );
                fs::write(self.trash.join(name), b"existing quarantine")
                    .map_err(|error| TreeFailure::io("occupy quarantine name for test", error))?;
            }
            Err(injected_failure())
        }
    }

    #[test]
    fn exhausted_quarantine_names_cannot_claim_a_completed_rollback() {
        let (_fixture, paths, request, plan) = fixture("thinws-btrfs-quarantine-full-");
        let failure = BtrfsReflinkMaterializer::new()
            .materialize_with_hook(
                &request,
                &plan,
                &SaturateRollbackQuarantine {
                    target: paths[1].clone(),
                    trash: paths[3].clone(),
                },
            )
            .unwrap_err();
        let rollback = failure.receipt().rollback();
        assert_eq!(rollback.status(), RollbackStatus::Incomplete);
        assert_eq!(rollback.remaining().len(), 1);
        assert!(rollback.removed().is_empty());
        assert_eq!(fs::read_dir(&paths[1]).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&paths[3]).unwrap().count(), 128);
    }

    struct AddForeignChild {
        target: PathBuf,
    }
    impl ExecutionHook for AddForeignChild {
        fn after_published(&self, _relative: &[u8], _count: usize) -> Result<(), TreeFailure> {
            fs::write(self.target.join("nested/foreign"), b"unrelated child")
                .map_err(|error| TreeFailure::io("write foreign child for test", error))?;
            Err(injected_failure())
        }
    }

    #[test]
    fn rollback_preserves_an_unregistered_child_in_a_created_directory() {
        let (_fixture, paths, _request, _plan) = fixture("thinws-btrfs-foreign-child-");
        fs::remove_file(paths[0].join("a")).unwrap();
        fs::remove_file(paths[0].join("b")).unwrap();
        fs::create_dir(paths[0].join("nested")).unwrap();
        fs::write(paths[0].join("nested/file"), b"source file").unwrap();
        let request = MaterializeRequest::new(
            absolute(&paths[0]),
            absolute(&paths[1]),
            absolute(&paths[2]),
            absolute(&paths[3]),
        );
        let report = LinuxPlatformProbe
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
            .unwrap();
        let plan = MaterializationPlan::for_cow_clone(&report, FallbackPolicy::Deny).unwrap();
        let failure = BtrfsReflinkMaterializer::new()
            .materialize_with_hook(
                &request,
                &plan,
                &AddForeignChild {
                    target: paths[1].clone(),
                },
            )
            .unwrap_err();
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert_eq!(failure.receipt().rollback().remaining().len(), 1);
        assert_eq!(
            fs::read(paths[1].join("nested/foreign")).unwrap(),
            b"unrelated child"
        );
        assert!(fs::read_dir(&paths[3]).unwrap().next().is_none());
    }

    struct AddForeignSibling {
        target: PathBuf,
    }
    impl ExecutionHook for AddForeignSibling {
        fn after_published(&self, _relative: &[u8], _count: usize) -> Result<(), TreeFailure> {
            fs::write(self.target.join("foreign"), b"unrelated sibling")
                .map_err(|error| TreeFailure::io("write foreign sibling for test", error))?;
            Err(injected_failure())
        }
    }

    #[test]
    fn rollback_stops_before_detaching_a_registered_file_beside_a_foreign_sibling() {
        let (_fixture, paths, request, plan) = fixture("thinws-btrfs-foreign-sibling-");
        let failure = BtrfsReflinkMaterializer::new()
            .materialize_with_hook(
                &request,
                &plan,
                &AddForeignSibling {
                    target: paths[1].clone(),
                },
            )
            .unwrap_err();
        let rollback = failure.receipt().rollback();
        assert_eq!(rollback.status(), RollbackStatus::Incomplete);
        assert!(rollback.removed().is_empty());
        assert_eq!(rollback.remaining().len(), 1);
        assert_eq!(fs::read(paths[1].join("a")).unwrap(), b"first file");
        assert_eq!(
            fs::read(paths[1].join("foreign")).unwrap(),
            b"unrelated sibling"
        );
        assert!(fs::read_dir(&paths[3]).unwrap().next().is_none());
    }

    #[test]
    fn nested_partial_clone_is_quarantined_child_before_parent() {
        let (_fixture, paths, _request, _plan) = fixture("thinws-btrfs-nested-rollback-");
        fs::remove_file(paths[0].join("a")).unwrap();
        fs::remove_file(paths[0].join("b")).unwrap();
        fs::create_dir(paths[0].join("nested")).unwrap();
        fs::write(paths[0].join("nested/file"), b"nested bytes").unwrap();
        let request = MaterializeRequest::new(
            absolute(&paths[0]),
            absolute(&paths[1]),
            absolute(&paths[2]),
            absolute(&paths[3]),
        );
        let report = LinuxPlatformProbe
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
            .unwrap();
        let plan = MaterializationPlan::for_cow_clone(&report, FallbackPolicy::Deny).unwrap();
        struct FailAfterTwo;
        impl ExecutionHook for FailAfterTwo {
            fn after_published(&self, _relative: &[u8], count: usize) -> Result<(), TreeFailure> {
                if count == 2 {
                    Err(injected_failure())
                } else {
                    Ok(())
                }
            }
        }
        let failure = BtrfsReflinkMaterializer::new()
            .materialize_with_hook(&request, &plan, &FailAfterTwo)
            .unwrap_err();
        let rollback = failure.receipt().rollback();
        assert_eq!(rollback.status(), RollbackStatus::ConfirmedBaseline);
        assert_eq!(rollback.removed().len(), 2);
        assert_eq!(rollback.quarantined().len(), 2);
        assert!(fs::read_dir(&paths[1]).unwrap().next().is_none());
        assert_eq!(fs::read_dir(&paths[3]).unwrap().count(), 2);
    }

    struct ReplaceStaged {
        staging: PathBuf,
    }
    impl ExecutionHook for ReplaceStaged {
        fn before_staged_publish(&self, _relative: &[u8]) -> Result<(), TreeFailure> {
            let name = fs::read_dir(&self.staging)
                .map_err(|error| TreeFailure::io("enumerate test staging", error))?
                .next()
                .expect("one staged entry exists")
                .map_err(|error| TreeFailure::io("read test staging", error))?
                .file_name();
            fs::rename(self.staging.join(&name), self.staging.join("displaced"))
                .map_err(|error| TreeFailure::io("displace staged entry for test", error))?;
            fs::write(self.staging.join(&name), b"foreign")
                .map_err(|error| TreeFailure::io("replace staged entry for test", error))?;
            Err(injected_failure())
        }
    }

    #[test]
    fn unknown_staging_replacement_is_preserved_and_reported() {
        let (_fixture, paths, request, plan) = fixture("thinws-btrfs-stage-replace-");
        let hook = ReplaceStaged {
            staging: paths[2].clone(),
        };
        let failure = BtrfsReflinkMaterializer::new()
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();
        let receipt = failure.receipt();
        assert!(receipt.unconfirmed_staging().is_some());
        assert_eq!(receipt.rollback().status(), RollbackStatus::NotNeeded);
        assert_eq!(fs::read_dir(&paths[2]).unwrap().count(), 2);
        assert!(fs::read_dir(&paths[1]).unwrap().next().is_none());
    }

    struct MutateSource {
        source: PathBuf,
    }
    impl ExecutionHook for MutateSource {
        fn after_published(&self, _relative: &[u8], count: usize) -> Result<(), TreeFailure> {
            if count == 1 {
                fs::write(self.source.join("a"), b"changed while cloning")
                    .map_err(|error| TreeFailure::io("mutate test source", error))?;
            }
            Ok(())
        }
    }

    #[test]
    fn changed_source_is_not_reported_as_a_successful_clone() {
        let (_fixture, paths, request, plan) = fixture("thinws-btrfs-source-change-");
        let hook = MutateSource {
            source: paths[0].clone(),
        };
        let failure = BtrfsReflinkMaterializer::new()
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();
        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::SourceChanged)
        );
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::ConfirmedBaseline
        );
        assert!(fs::read_dir(&paths[1]).unwrap().next().is_none());
    }
}

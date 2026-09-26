//! Explicit Workspace removal over the existing ownership, policy, and storage Ports.

use std::str::FromStr;

use thinws_core::{
    AbsolutePath, DiscoveryCompleteness, ErrorCode, GitState, InstallationRecord, OperationId,
    ProcessUse, RemovalDecision, RemovalMode, RemovalRefusal, RemovalWarning, UnixMillis,
    WorkspaceId, WorkspaceName, WorkspaceState, decide_removal,
};
use thinws_ports::{
    BootstrapStore, GitInspection, GitInspector, LifecycleLock, LifecycleLockGuard,
    MetadataStoreFactory, ProcessProbe, RemovalLogEvent, RemovalLogRecord, WorkspaceRemoval,
};

use crate::{Stage, ThinWorkspaceService, UseCaseError, map_port, semantic_error};

#[derive(Clone, Debug, Eq, PartialEq)]
enum RemoveTarget {
    Name(WorkspaceName),
    Id(WorkspaceId),
    Ambiguous(WorkspaceName, WorkspaceId),
}

/// One explicit user request to remove a named or exact-ID Workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoveRequest {
    target: RemoveTarget,
    mode: RemovalMode,
    now: UnixMillis,
}

impl RemoveRequest {
    /// Parses the public target, explicit force flag, and caller-supplied UTC time.
    pub fn try_from_raw(target: &str, force: bool, now_ms: i64) -> Result<Self, UseCaseError> {
        let target =
            if let Some(value) = target.strip_prefix("name:") {
                RemoveTarget::Name(WorkspaceName::from_str(value).map_err(|_| {
                    semantic_error(ErrorCode::Usage, "invalid explicit Workspace name")
                })?)
            } else if let Some(value) = target.strip_prefix("id:") {
                RemoveTarget::Id(WorkspaceId::from_str(value).map_err(|_| {
                    semantic_error(ErrorCode::Usage, "invalid explicit Workspace ID")
                })?)
            } else {
                match (
                    WorkspaceName::from_str(target).ok(),
                    WorkspaceId::from_str(target).ok(),
                ) {
                    (Some(name), Some(id)) => RemoveTarget::Ambiguous(name, id),
                    (Some(name), None) => RemoveTarget::Name(name),
                    (None, Some(id)) => RemoveTarget::Id(id),
                    (None, None) => {
                        return Err(semantic_error(
                            ErrorCode::Usage,
                            "invalid Workspace name or full ID",
                        ));
                    }
                }
            };
        let now = UnixMillis::new(now_ms).map_err(|_| {
            semantic_error(ErrorCode::Usage, "system clock predates the Unix epoch")
        })?;
        Ok(Self {
            target,
            mode: if force {
                RemovalMode::Force
            } else {
                RemovalMode::Normal
            },
            now,
        })
    }
}

/// Stable successful result of an explicit cleanup attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoveResult {
    /// A controlled Workspace container and active record were removed.
    Removed,
    /// The exact ID has an existing deletion tombstone.
    AlreadyRemoved,
}

impl RemoveResult {
    /// Returns the stable public result name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::AlreadyRemoved => "already-removed",
        }
    }
}

/// Successful cleanup facts for the CLI renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoveOutcome {
    workspace_id: WorkspaceId,
    operation_id: OperationId,
    forced: bool,
    result: RemoveResult,
    log_path: AbsolutePath,
    warning: Option<RemovalWarning>,
}

impl RemoveOutcome {
    /// Returns the exact ID affected by the request.
    #[must_use]
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    /// Returns the per-attempt log correlation identifier.
    #[must_use]
    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    /// Returns whether the request explicitly used force.
    #[must_use]
    pub const fn forced(&self) -> bool {
        self.forced
    }

    /// Returns the stable cleanup result.
    #[must_use]
    pub const fn result(&self) -> RemoveResult {
        self.result
    }

    /// Returns the synchronously written log location outside the copy.
    #[must_use]
    pub const fn log_path(&self) -> &AbsolutePath {
        &self.log_path
    }

    /// Returns a non-blocking process-scan warning, when applicable.
    #[must_use]
    pub const fn warning(&self) -> Option<RemovalWarning> {
        self.warning
    }
}

impl<B, M> ThinWorkspaceService<B, M>
where
    B: BootstrapStore + LifecycleLock<Guard = <B as BootstrapStore>::LockGuard>,
    M: MetadataStoreFactory<<B as BootstrapStore>::DataRootLayout>,
{
    /// Removes only a registered Workspace with explicit policy and durable logging.
    pub fn remove<G: GitInspector, P: ProcessProbe>(
        &self,
        request: RemoveRequest,
        git: &G,
        process: &P,
    ) -> Result<RemoveOutcome, UseCaseError> {
        let identity = self
            .bootstrap
            .read_config()
            .map_err(|error| map_port(Stage::Bootstrap, error))?
            .ok_or_else(|| {
                semantic_error(
                    ErrorCode::NotInitialized,
                    "ThinWorkspace is not initialized",
                )
            })?;
        self.bootstrap
            .validate_layout(&identity)
            .map_err(|error| map_port(Stage::Layout, error))?;
        let lock = self
            .bootstrap
            .acquire_data_root(identity.data_root(), self.lock_timeout)
            .map_err(|error| match self.bootstrap.validate_layout(&identity) {
                Err(layout_error) => map_port(Stage::Layout, layout_error),
                Ok(_) => map_port(Stage::Lock, error),
            })?;
        lock.revalidate()
            .map_err(|error| map_port(Stage::Layout, error))?;
        let layout = self
            .bootstrap
            .validate_layout(&identity)
            .map_err(|error| map_port(Stage::Layout, error))?;
        self.require_ready_marker(&identity)?;
        let mut metadata = self
            .metadata
            .open_existing(
                &layout,
                &InstallationRecord::new(identity.clone(), request.now),
                self.sqlite_timeout,
            )
            .map_err(|error| map_port(Stage::Metadata, error))?;
        let active = match &request.target {
            RemoveTarget::Name(name) => metadata
                .workspaces()
                .map_err(|error| map_port(Stage::Metadata, error))?
                .into_iter()
                .find(|workspace| workspace.reservation().name() == name),
            RemoveTarget::Id(id) => metadata
                .workspace(*id)
                .map_err(|error| map_port(Stage::Metadata, error))?,
            RemoveTarget::Ambiguous(name, id) => {
                let by_name = metadata
                    .workspaces()
                    .map_err(|error| map_port(Stage::Metadata, error))?
                    .into_iter()
                    .find(|workspace| workspace.reservation().name() == name);
                let by_id = metadata
                    .workspace(*id)
                    .map_err(|error| map_port(Stage::Metadata, error))?;
                let old_id = if by_id.is_none() {
                    metadata
                        .deletion_tombstone(*id)
                        .map_err(|error| map_port(Stage::Metadata, error))?
                        .is_some()
                } else {
                    false
                };
                if by_name.as_ref().is_some_and(|named| {
                    old_id
                        || by_id.as_ref().is_some_and(|identified| {
                            named.reservation().workspace_id()
                                != identified.reservation().workspace_id()
                        })
                }) {
                    return Err(semantic_error(
                        ErrorCode::Usage,
                        "Workspace name and ID select different records",
                    )
                    .with_remediation(
                        "Use name:<name> or id:<full-workspace-id> to select the intended Workspace explicitly.",
                    ));
                }
                by_name.or(by_id)
            }
        };
        let Some(active) = active else {
            let id = match &request.target {
                RemoveTarget::Id(id) | RemoveTarget::Ambiguous(_, id) => *id,
                RemoveTarget::Name(_) => {
                    return Err(semantic_error(
                        ErrorCode::WorkspaceNotFound,
                        "Workspace not found",
                    ));
                }
            };
            let tombstone = metadata
                .deletion_tombstone(id)
                .map_err(|error| map_port(Stage::Metadata, error))?
                .ok_or_else(|| {
                    semantic_error(ErrorCode::WorkspaceNotFound, "Workspace not found")
                })?;
            if tombstone.instance_id() != identity.instance_id() {
                return Err(semantic_error(
                    ErrorCode::Metadata,
                    "deletion record belongs to another installation",
                ));
            }
            let operation_id = OperationId::new();
            let mut log_path = None;
            for (event, outcome) in [
                (RemovalLogEvent::Started, None),
                (
                    RemovalLogEvent::Completed,
                    Some(WorkspaceRemoval::AlreadyAbsent),
                ),
            ] {
                let path = self
                    .bootstrap
                    .append_removal_log(
                        &lock,
                        &layout,
                        &RemovalLogRecord {
                            occurred_at: request.now,
                            operation_id,
                            workspace_id: id,
                            event,
                            mode: request.mode,
                            git_state: GitState::Unknown,
                            git_check_complete: false,
                            repositories: &[],
                            process_use: None,
                            protection: None,
                            error_code: None,
                            outcome,
                        },
                    )
                    .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))?;
                log_path = Some(path);
            }
            return Ok(RemoveOutcome {
                workspace_id: id,
                operation_id,
                forced: request.mode == RemovalMode::Force,
                result: RemoveResult::AlreadyRemoved,
                log_path: log_path.expect("two successfully appended events have a log path"),
                warning: None,
            });
        };
        let id = active.reservation().workspace_id();
        let state = active.state();
        let operation_id = OperationId::new();
        let preflight =
            (|| -> Result<(Option<GitInspection>, Option<ProcessUse>), UseCaseError> {
                let container = self
                    .bootstrap
                    .inspect_removal_container(&lock, &layout, id)
                    .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))?;
                let inspection = if state == WorkspaceState::Ready
                    && request.mode == RemovalMode::Normal
                    && container.is_some()
                {
                    let path = self
                        .bootstrap
                        .validate_ready_workspace(&layout, id)
                        .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))?;
                    if path != *active.reservation().target_path() {
                        return Err(semantic_error(
                            ErrorCode::DataRootLayout,
                            "Ready Workspace path does not match metadata",
                        )
                        .with_workspace_id(id));
                    }
                    let result = git.inspect(&path);
                    self.bootstrap
                        .validate_ready_workspace(&layout, id)
                        .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))?;
                    Some(result)
                } else {
                    None
                };
                let container = self
                    .bootstrap
                    .inspect_removal_container(&lock, &layout, id)
                    .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))?;
                let process_use = container
                    .as_ref()
                    .map(|path| {
                        process
                            .inspect_workspace(path)
                            .map(|observation| observation.use_state)
                            .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))
                    })
                    .transpose()?;
                Ok((inspection, process_use))
            })();
        let (inspection, process_use) = match preflight {
            Ok(facts) => facts,
            Err(failure) => {
                let logged = self.bootstrap.append_removal_log(
                    &lock,
                    &layout,
                    &RemovalLogRecord {
                        occurred_at: request.now,
                        operation_id,
                        workspace_id: id,
                        event: RemovalLogEvent::Failed,
                        mode: request.mode,
                        git_state: GitState::Unknown,
                        git_check_complete: false,
                        repositories: &[],
                        process_use: None,
                        protection: None,
                        error_code: Some(failure.diagnostic().code()),
                        outcome: None,
                    },
                );
                if let Err(log_error) = logged {
                    return Err(map_port(Stage::Layout, log_error)
                        .with_workspace_id(id)
                        .with_public_context(
                            "preflight_error_code",
                            failure.diagnostic().code().as_str(),
                        ));
                }
                return Err(failure);
            }
        };
        let git_state = inspection
            .as_ref()
            .map_or(GitState::Unknown, |result| result.aggregate());
        let git_check_complete = inspection.as_ref().is_some_and(|result| {
            result.discovery() == DiscoveryCompleteness::Complete
                && result.aggregate() != GitState::Unknown
        });
        let repositories = inspection
            .as_ref()
            .map_or(&[][..], |result| result.repositories());
        let append = |event, protection, error_code, outcome| {
            self.bootstrap.append_removal_log(
                &lock,
                &layout,
                &RemovalLogRecord {
                    occurred_at: request.now,
                    operation_id,
                    workspace_id: id,
                    event,
                    mode: request.mode,
                    git_state,
                    git_check_complete,
                    repositories,
                    process_use,
                    protection,
                    error_code,
                    outcome,
                },
            )
        };

        if state != WorkspaceState::Ready && request.mode == RemovalMode::Normal {
            append(
                RemovalLogEvent::Failed,
                None,
                Some(ErrorCode::WorkspaceIncomplete),
                None,
            )
            .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))?;
            let failure = semantic_error(
                ErrorCode::WorkspaceIncomplete,
                "Workspace cleanup is incomplete; explicit force is required",
            )
            .with_workspace_id(id)
            .with_remediation(
                "Inspect the incomplete copy, then use workspace remove <name-or-id> --force only to discard it.",
            );
            return Err(with_process_scan_warning(failure, process_use));
        }
        let decision = decide_removal(
            git_state,
            process_use.unwrap_or(ProcessUse::NoEvidence),
            request.mode,
        );
        let warning = match decision {
            RemovalDecision::Proceed { warning } => warning,
            RemovalDecision::Refuse(reason) => {
                let code = refusal_code(reason);
                append(RemovalLogEvent::Refused, Some(reason), Some(code), None)
                    .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))?;
                let failure = semantic_error(code, refusal_message(reason))
                    .with_workspace_id(id)
                    .with_remediation(refusal_remediation(reason))
                    .with_git_inspection(inspection.clone());
                return Err(with_process_scan_warning(failure, process_use));
            }
        };
        let log_path = append(RemovalLogEvent::Started, None, None, None)
            .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))?;
        if let Err(error) = metadata.begin_removal(id, state, request.mode, request.now) {
            let failure = map_port(Stage::Metadata, error).with_workspace_id(id);
            append(
                RemovalLogEvent::Failed,
                None,
                Some(failure.diagnostic().code()),
                None,
            )
            .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))?;
            return Err(failure);
        }
        let removal = self
            .bootstrap
            .remove_workspace(&lock, &layout, id)
            .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))
            .and_then(|outcome| {
                match self
                    .bootstrap
                    .inspect_removal_container(&lock, &layout, id)
                    .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))?
                {
                    None => Ok(outcome),
                    Some(_) => Err(semantic_error(
                        ErrorCode::DataRootLayout,
                        "Workspace container reappeared before deletion completion",
                    )
                    .with_workspace_id(id)),
                }
            });
        let removed = match removal {
            Ok(removed) => removed,
            Err(failure) => {
                let failure = match metadata.record_failure(
                    id,
                    WorkspaceState::Deleting,
                    failure.diagnostic().code(),
                    request.now,
                ) {
                    Ok(_) => failure,
                    Err(error) => map_port(Stage::Metadata, error).with_workspace_id(id),
                };
                append(
                    RemovalLogEvent::Failed,
                    None,
                    Some(failure.diagnostic().code()),
                    None,
                )
                .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))?;
                return Err(failure);
            }
        };
        if let Err(error) = metadata.complete_deletion(id, identity.instance_id(), request.now) {
            let failure = map_port(Stage::Metadata, error).with_workspace_id(id);
            append(
                RemovalLogEvent::Failed,
                None,
                Some(failure.diagnostic().code()),
                None,
            )
            .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))?;
            return Err(failure);
        }
        append(RemovalLogEvent::Completed, None, None, Some(removed))
            .map_err(|error| map_port(Stage::Layout, error).with_workspace_id(id))?;
        Ok(RemoveOutcome {
            workspace_id: id,
            operation_id,
            forced: request.mode == RemovalMode::Force,
            result: RemoveResult::Removed,
            log_path,
            warning,
        })
    }
}

fn with_process_scan_warning(error: UseCaseError, process_use: Option<ProcessUse>) -> UseCaseError {
    if process_use == Some(ProcessUse::ScanIncomplete) {
        error.with_public_context("process_use", "scan-incomplete")
    } else {
        error
    }
}

fn refusal_code(reason: RemovalRefusal) -> ErrorCode {
    match reason {
        RemovalRefusal::ConfirmedInUse => ErrorCode::WorkspaceBusy,
        RemovalRefusal::GitCheckIncomplete => ErrorCode::GitCheckIncomplete,
        RemovalRefusal::TrackedChanges => ErrorCode::WorkspaceDirty,
    }
}

fn refusal_message(reason: RemovalRefusal) -> &'static str {
    match reason {
        RemovalRefusal::ConfirmedInUse => "Workspace is occupied by another process",
        RemovalRefusal::GitCheckIncomplete => "tracked-change check is incomplete",
        RemovalRefusal::TrackedChanges => "Workspace has tracked changes",
    }
}

fn refusal_remediation(reason: RemovalRefusal) -> &'static str {
    match reason {
        RemovalRefusal::ConfirmedInUse => {
            "Stop processes using the Workspace and retry; --force does not bypass confirmed use."
        }
        RemovalRefusal::GitCheckIncomplete => {
            "Resolve the Git check or use workspace remove <name-or-id> --force to discard the copy."
        }
        RemovalRefusal::TrackedChanges => {
            "Preserve and commit required work, or use workspace remove <name-or-id> --force to discard the copy."
        }
    }
}

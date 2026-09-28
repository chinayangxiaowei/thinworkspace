//! Read-only Workspace listing, validated path lookup, and tracked-change status.

use std::str::FromStr;

use thinws_core::{
    AbsolutePath, ErrorCode, WorkspaceName, WorkspaceRecord, WorkspaceReservation, WorkspaceState,
};
use thinws_ports::{
    BootstrapStore, DataRootLayoutEvidence, FinalMaterializationSummary, GitInspection,
    GitInspector, LifecycleLock, MetadataSnapshot, MetadataStoreFactory, WorkspaceSpace,
};

use crate::{Stage, ThinWorkspaceService, UseCaseError, map_port, semantic_error};

/// One durable Workspace record and its immutable final materialization facts, if present.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceQuery {
    record: WorkspaceRecord,
    materialization: Option<FinalMaterializationSummary>,
}

impl WorkspaceQuery {
    /// Returns the active metadata row observed in a read-only snapshot.
    #[must_use]
    pub const fn record(&self) -> &WorkspaceRecord {
        &self.record
    }

    /// Returns successful final receipt facts when the record has such a receipt.
    #[must_use]
    pub const fn materialization(&self) -> Option<&FinalMaterializationSummary> {
        self.materialization.as_ref()
    }
}

/// Status result that keeps non-Ready diagnostics separate from a completed Git inspection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceStatus {
    workspace: WorkspaceQuery,
    git: Option<GitInspection>,
    space: Option<WorkspaceSpace>,
}

impl WorkspaceStatus {
    /// Returns the durable Workspace and receipt facts.
    #[must_use]
    pub const fn workspace(&self) -> &WorkspaceQuery {
        &self.workspace
    }

    /// Returns Git evidence only when the Workspace was verified Ready.
    #[must_use]
    pub const fn git(&self) -> Option<&GitInspection> {
        self.git.as_ref()
    }

    /// Returns current space evidence only for a verified Ready Workspace.
    #[must_use]
    pub const fn space(&self) -> Option<&WorkspaceSpace> {
        self.space.as_ref()
    }
}

impl<B, M> ThinWorkspaceService<B, M>
where
    B: BootstrapStore + LifecycleLock<Guard = <B as BootstrapStore>::LockGuard>,
    M: MetadataStoreFactory<<B as BootstrapStore>::DataRootLayout>,
{
    /// Lists active records by exact Workspace name without starting Git.
    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceQuery>, UseCaseError> {
        let snapshot = self.inspect_current()?;
        let mut records = snapshot.workspaces().to_vec();
        records.sort_by(|left, right| {
            left.reservation()
                .name()
                .cmp(right.reservation().name())
                .then_with(|| {
                    left.reservation()
                        .workspace_id()
                        .cmp(&right.reservation().workspace_id())
                })
        });
        records
            .into_iter()
            .map(|record| query_from_snapshot(record, &snapshot))
            .collect()
    }

    /// Returns one ordinary path only after Ready, receipt, and controlled-root validation.
    pub fn workspace_path(&self, name: &str) -> Result<AbsolutePath, UseCaseError> {
        let name = parse_name(name)?;
        let (workspace, path) = self.inspect_workspace(&name)?;
        path.ok_or_else(|| {
            semantic_error(ErrorCode::WorkspaceNotReady, "Workspace is not Ready")
                .with_workspace_id(workspace.record().reservation().workspace_id())
        })
    }

    /// Reads current metadata and performs bounded Git inspection only for Ready copies.
    pub fn workspace_status<G: GitInspector>(
        &self,
        name: &str,
        git: &G,
    ) -> Result<WorkspaceStatus, UseCaseError> {
        let name = parse_name(name)?;
        let (workspace, path) = self.inspect_workspace(&name)?;
        let Some(path) = path else {
            return Ok(WorkspaceStatus {
                workspace,
                git: None,
                space: None,
            });
        };
        let inspection = git.inspect(&path);
        // Git inspection can take time. Do not publish a stale Ready path if a
        // lifecycle operation changed the record or controlled root meanwhile.
        let (current, current_path) = self.inspect_workspace(&name)?;
        let Some(_) = current_path else {
            return Ok(WorkspaceStatus {
                workspace: current,
                git: None,
                space: None,
            });
        };
        // Both validations require their path to equal the record's target;
        // equal records therefore imply equal validated paths.
        if current != workspace {
            return Err(semantic_error(
                ErrorCode::TargetLayout,
                "Ready Workspace changed during Git inspection",
            ));
        }
        let space = self.measure_workspace_space(current.record().reservation())?;
        // The read-only space scan can also outlive a concurrent lifecycle
        // mutation; recheck before returning a usable Ready path.
        let (after_space, after_space_path) = self.inspect_workspace(&name)?;
        let Some(_) = after_space_path else {
            return Ok(WorkspaceStatus {
                workspace: after_space,
                git: None,
                space: None,
            });
        };
        if after_space != current {
            return Err(semantic_error(
                ErrorCode::TargetLayout,
                "Ready Workspace changed during space scan",
            ));
        }
        Ok(WorkspaceStatus {
            workspace: after_space,
            git: Some(inspection),
            space: Some(space),
        })
    }

    fn measure_workspace_space(
        &self,
        reservation: &WorkspaceReservation,
    ) -> Result<WorkspaceSpace, UseCaseError> {
        let identity = self
            .bootstrap
            .read_config()
            .map_err(|error| map_port(Stage::Control, error))?
            .ok_or_else(|| {
                semantic_error(
                    ErrorCode::NotInitialized,
                    "ThinWorkspace is not initialized",
                )
            })?;
        let layout = self
            .bootstrap
            .validate_layout(&identity)
            .map_err(|error| map_port(Stage::Control, error))?;
        self.require_ready_marker(&identity)?;
        self.bootstrap
            .measure_ready_workspace_space(&layout, reservation)
            .map_err(|error| map_port(Stage::Layout, error))
    }

    fn inspect_workspace(
        &self,
        name: &WorkspaceName,
    ) -> Result<(WorkspaceQuery, Option<AbsolutePath>), UseCaseError> {
        let snapshot = self.inspect_current()?;
        let workspace = find_workspace(&snapshot, name)?;
        if workspace.record().state() != WorkspaceState::Ready {
            return Ok((workspace, None));
        }
        let identity = self
            .bootstrap
            .read_config()
            .map_err(|error| map_port(Stage::Control, error))?
            .ok_or_else(|| {
                semantic_error(
                    ErrorCode::NotInitialized,
                    "ThinWorkspace is not initialized",
                )
            })?;
        let layout = self
            .bootstrap
            .validate_layout(&identity)
            .map_err(|error| map_port(Stage::Control, error))?;
        self.require_ready_marker(&identity)?;
        let path = self
            .bootstrap
            .validate_ready_workspace(&layout, workspace.record().reservation())
            .map_err(|error| map_port(Stage::Layout, error))?;
        if &path != workspace.record().reservation().target_path() {
            return Err(semantic_error(
                ErrorCode::TargetLayout,
                "Ready Workspace path does not match metadata",
            ));
        }
        layout
            .revalidate()
            .map_err(|error| map_port(Stage::Layout, error))?;
        let current = find_workspace(&self.inspect_current()?, name)?;
        if current.record().state() != WorkspaceState::Ready {
            return Ok((current, None));
        }
        if current != workspace {
            return Err(semantic_error(
                ErrorCode::TargetLayout,
                "Ready Workspace changed during path verification",
            ));
        }
        Ok((current, Some(path)))
    }
}

fn parse_name(name: &str) -> Result<WorkspaceName, UseCaseError> {
    WorkspaceName::from_str(name)
        .map_err(|_| semantic_error(ErrorCode::Usage, "invalid Workspace name"))
}

fn find_workspace(
    snapshot: &MetadataSnapshot,
    name: &WorkspaceName,
) -> Result<WorkspaceQuery, UseCaseError> {
    let record = snapshot
        .workspaces()
        .iter()
        .find(|record| record.reservation().name() == name)
        .cloned()
        .ok_or_else(|| semantic_error(ErrorCode::WorkspaceNotFound, "Workspace not found"))?;
    query_from_snapshot(record, snapshot)
}

fn query_from_snapshot(
    record: WorkspaceRecord,
    snapshot: &MetadataSnapshot,
) -> Result<WorkspaceQuery, UseCaseError> {
    let materialization = snapshot
        .final_materialization(record.reservation().workspace_id())
        .cloned();
    if record.state() == WorkspaceState::Ready && materialization.is_none() {
        return Err(semantic_error(
            ErrorCode::Metadata,
            "Ready Workspace has no final receipt",
        ));
    }
    Ok(WorkspaceQuery {
        record,
        materialization,
    })
}

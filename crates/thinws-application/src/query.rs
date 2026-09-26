//! Read-only Workspace listing, validated path lookup, and tracked-change status.

use std::str::FromStr;

use thinws_core::{
    AbsolutePath, ErrorCode, UnixMillis, WorkspaceName, WorkspaceRecord, WorkspaceState,
};
use thinws_ports::{
    BootstrapStore, DataRootLayoutEvidence, FinalMaterializationSummary, GitInspection,
    GitInspector, LifecycleLock, LifecycleLockGuard, MetadataSnapshot, MetadataStoreFactory,
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
        Ok(WorkspaceStatus {
            workspace,
            git: path.map(|path| git.inspect(&path)),
        })
    }

    fn inspect_workspace(
        &self,
        name: &WorkspaceName,
    ) -> Result<(WorkspaceQuery, Option<AbsolutePath>), UseCaseError> {
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
        let lock = self
            .bootstrap
            .acquire_data_root(identity.data_root(), self.lock_timeout)
            .map_err(|error| map_port(Stage::Lock, error))?;
        lock.revalidate()
            .map_err(|error| map_port(Stage::Lock, error))?;
        let epoch = UnixMillis::new(0).expect("zero is a valid Unix millisecond timestamp");
        let snapshot = self.inspect_existing(&identity, epoch)?;
        let workspace = find_workspace(&snapshot, name)?;
        if workspace.record().state() != WorkspaceState::Ready {
            lock.revalidate()
                .map_err(|error| map_port(Stage::Lock, error))?;
            return Ok((workspace, None));
        }
        let layout = self
            .bootstrap
            .validate_layout(&identity)
            .map_err(|error| map_port(Stage::Layout, error))?;
        self.require_ready_marker(&identity)?;
        let path = self
            .bootstrap
            .validate_ready_workspace(
                &lock,
                &layout,
                workspace.record().reservation().workspace_id(),
            )
            .map_err(|error| map_port(Stage::Layout, error))?;
        if &path != workspace.record().reservation().target_path() {
            return Err(semantic_error(
                ErrorCode::DataRootLayout,
                "Ready Workspace path does not match metadata",
            ));
        }
        layout
            .revalidate()
            .map_err(|error| map_port(Stage::Layout, error))?;
        lock.revalidate()
            .map_err(|error| map_port(Stage::Lock, error))?;
        Ok((workspace, Some(path)))
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

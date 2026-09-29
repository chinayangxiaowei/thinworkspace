//! Existing Phase 1 bootstrap Port wired to the Linux platform operations.

use thinws_core::{
    AbsolutePath, InstallationIdentity, RootMarker, WorkspaceId, WorkspaceReservation,
};
use thinws_ports::{
    BootstrapStore, PortError, PublishResult, RemovalLogRecord, WorkspaceRemoval, WorkspaceSpace,
};

use crate::{
    LinuxDataRootLayout, LinuxHostAdapter, LinuxInitializingProof, LinuxLockGuard,
    LinuxPreparedDataRoot, LinuxPreparedWorkspace,
};

impl BootstrapStore for LinuxHostAdapter {
    type LockGuard = LinuxLockGuard;
    type PreparedDataRoot = LinuxPreparedDataRoot;
    type InitializingProof = LinuxInitializingProof;
    type DataRootLayout = LinuxDataRootLayout;
    type PreparedWorkspace = LinuxPreparedWorkspace;

    fn prepare_bootstrap(&self) -> Result<(), PortError> {
        LinuxHostAdapter::prepare_bootstrap(self)
    }

    fn prepare_data_root(
        &self,
        data_root: &AbsolutePath,
    ) -> Result<Self::PreparedDataRoot, PortError> {
        LinuxHostAdapter::prepare_data_root(self, data_root)
    }

    fn read_config(&self) -> Result<Option<InstallationIdentity>, PortError> {
        LinuxHostAdapter::read_config(self)
    }

    fn read_root_marker(&self, data_root: &AbsolutePath) -> Result<Option<RootMarker>, PortError> {
        LinuxHostAdapter::read_root_marker(self, data_root)
    }

    fn create_initializing(
        &self,
        lock: &Self::LockGuard,
        prepared: Self::PreparedDataRoot,
        identity: &InstallationIdentity,
    ) -> Result<Self::InitializingProof, PortError> {
        LinuxHostAdapter::create_initializing(self, lock, prepared, identity)
    }

    fn initialize_layout(
        &self,
        lock: &Self::LockGuard,
        proof: &Self::InitializingProof,
    ) -> Result<Self::DataRootLayout, PortError> {
        LinuxHostAdapter::initialize_layout(self, lock, proof)
    }

    fn validate_layout(
        &self,
        identity: &InstallationIdentity,
    ) -> Result<Self::DataRootLayout, PortError> {
        LinuxHostAdapter::validate_layout(self, identity)
    }

    fn prepare_workspace(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        workspace_id: WorkspaceId,
        target: &AbsolutePath,
    ) -> Result<Self::PreparedWorkspace, PortError> {
        LinuxHostAdapter::prepare_workspace(self, lock, layout, workspace_id, target)
    }

    fn clear_workspace_incomplete(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        prepared: Self::PreparedWorkspace,
    ) -> Result<(), PortError> {
        LinuxHostAdapter::clear_workspace_incomplete(self, lock, layout, prepared)
    }

    fn validate_ready_workspace(
        &self,
        layout: &Self::DataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<AbsolutePath, PortError> {
        LinuxHostAdapter::validate_ready_workspace(self, layout, reservation)
    }

    fn measure_ready_workspace_space(
        &self,
        layout: &Self::DataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<WorkspaceSpace, PortError> {
        LinuxHostAdapter::measure_ready_workspace_space(self, layout, reservation)
    }

    fn inspect_removal_container(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<Option<AbsolutePath>, PortError> {
        LinuxHostAdapter::inspect_removal_container(self, lock, layout, reservation)
    }

    fn append_removal_log(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        record: &RemovalLogRecord<'_>,
    ) -> Result<AbsolutePath, PortError> {
        LinuxHostAdapter::append_removal_log(self, lock, layout, record)
    }

    fn remove_workspace(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<WorkspaceRemoval, PortError> {
        LinuxHostAdapter::remove_workspace(self, lock, layout, reservation)
    }

    fn publish_ready(
        &self,
        lock: &Self::LockGuard,
        proof: Self::InitializingProof,
    ) -> Result<RootMarker, PortError> {
        LinuxHostAdapter::publish_ready(self, lock, proof)
    }

    fn publish_config(
        &self,
        lock: &Self::LockGuard,
        identity: &InstallationIdentity,
    ) -> Result<PublishResult, PortError> {
        LinuxHostAdapter::publish_config(self, lock, identity)
    }
}

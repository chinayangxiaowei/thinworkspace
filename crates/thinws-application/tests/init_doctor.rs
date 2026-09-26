use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr;
use std::time::Duration;

use thinws_application::{InitRequest, InitResult, ThinWorkspaceService};
use thinws_core::{
    AbsolutePath, CowEvidence, DiscoveryCompleteness, ErrorCode, InstallationIdentity,
    InstallationRecord, InstanceId, MaterializationMode, MaterializerKind, RootMarker,
    RootMarkerState, UnixMillis, VolumeId, WorkspaceId, WorkspaceName, WorkspaceRecord,
    WorkspaceReservation, WorkspaceState,
};
use thinws_ports::{
    BootstrapStore, DataRootLayoutEvidence, FinalMaterializationSummary, GitInspection,
    GitInspector, LifecycleLock, LifecycleLockGuard, LifecycleScope, MetadataSnapshot,
    MetadataStoreFactory, PortError, PortErrorKind, PreparedDataRootEvidence,
    PreparedWorkspaceEvidence, PublishResult,
};

const INSTANCE_ID: &str = "01890a5d-ac96-774b-bd5b-55c7b8d09f33";
const VOLUME_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

#[derive(Default)]
struct State {
    events: Vec<&'static str>,
    config: Option<InstallationIdentity>,
    marker: Option<RootMarker>,
    installation: Option<InstallationRecord>,
    prepare_error: Option<PortErrorKind>,
    layout_error: Option<PortErrorKind>,
    metadata_error: Option<PortErrorKind>,
    lock_error: Option<PortErrorKind>,
    lock_revalidate_error: Option<PortErrorKind>,
    workspaces: Vec<WorkspaceRecord>,
    final_materializations: Vec<(WorkspaceId, FinalMaterializationSummary)>,
    change_ready_during_validation: bool,
}

struct FakePrepared {
    data_root: AbsolutePath,
    volume_id: VolumeId,
}

impl PreparedDataRootEvidence for FakePrepared {
    fn data_root(&self) -> &AbsolutePath {
        &self.data_root
    }

    fn volume_id(&self) -> VolumeId {
        self.volume_id
    }
}

struct FakeLayout {
    state: Rc<RefCell<State>>,
    database_path: AbsolutePath,
}

struct FakePreparedWorkspace;

impl PreparedWorkspaceEvidence for FakePreparedWorkspace {
    fn target_root(&self) -> &AbsolutePath {
        unreachable!("init/doctor never prepares a Workspace")
    }

    fn target_identity(&self) -> thinws_core::FileIdentity {
        unreachable!("init/doctor never inspects a prepared Workspace")
    }

    fn revalidate(&self) -> Result<(), PortError> {
        unreachable!("init/doctor never revalidates a Workspace")
    }
}

impl DataRootLayoutEvidence for FakeLayout {
    fn database_path(&self) -> &AbsolutePath {
        &self.database_path
    }

    fn revalidate(&self) -> Result<(), PortError> {
        self.state.borrow_mut().events.push("layout.revalidate");
        Ok(())
    }
}

struct FakeGuard {
    state: Rc<RefCell<State>>,
}

impl LifecycleLockGuard for FakeGuard {
    fn scope(&self) -> LifecycleScope {
        LifecycleScope::Bootstrap
    }

    fn revalidate(&self) -> Result<(), PortError> {
        self.state.borrow_mut().events.push("lock.revalidate");
        if let Some(kind) = self.state.borrow().lock_revalidate_error {
            return Err(PortError::new(kind, "fake lock revalidation"));
        }
        Ok(())
    }
}

struct FakeBootstrap {
    state: Rc<RefCell<State>>,
}

struct StateChangingGit {
    state: Rc<RefCell<State>>,
}

impl GitInspector for StateChangingGit {
    fn inspect(&self, _copy_root: &AbsolutePath) -> GitInspection {
        self.state.borrow_mut().events.push("git.inspect");
        change_first_workspace_to_error(&self.state);
        GitInspection::new(DiscoveryCompleteness::Complete, Vec::new(), Vec::new())
    }
}

fn change_first_workspace_to_error(state: &Rc<RefCell<State>>) {
    let record = state.borrow().workspaces[0].clone();
    let changed = WorkspaceRecord::new(
        record.reservation().clone(),
        WorkspaceState::Error,
        Some(ErrorCode::Filesystem),
        UnixMillis::new(record.updated_at().get() + 1).unwrap(),
    )
    .unwrap();
    state.borrow_mut().workspaces = vec![changed];
}

impl LifecycleLock for FakeBootstrap {
    type Guard = FakeGuard;

    fn acquire_bootstrap(&self, _timeout: Duration) -> Result<Self::Guard, PortError> {
        self.state.borrow_mut().events.push("lock.acquire");
        if let Some(kind) = self.state.borrow().lock_error {
            return Err(PortError::new(kind, "fake lock"));
        }
        Ok(FakeGuard {
            state: Rc::clone(&self.state),
        })
    }

    fn acquire_data_root(
        &self,
        _data_root: &AbsolutePath,
        _timeout: Duration,
    ) -> Result<Self::Guard, PortError> {
        panic!("P1-03 init/doctor must not acquire a data-root lifecycle lock")
    }
}

impl BootstrapStore for FakeBootstrap {
    type LockGuard = FakeGuard;
    type PreparedDataRoot = FakePrepared;
    type InitializingProof = InstallationIdentity;
    type DataRootLayout = FakeLayout;
    type PreparedWorkspace = FakePreparedWorkspace;

    fn prepare_bootstrap(&self) -> Result<(), PortError> {
        self.state.borrow_mut().events.push("bootstrap.prepare");
        Ok(())
    }

    fn prepare_data_root(
        &self,
        data_root: &AbsolutePath,
    ) -> Result<Self::PreparedDataRoot, PortError> {
        self.state.borrow_mut().events.push("data_root.prepare");
        if let Some(kind) = self.state.borrow().prepare_error {
            return Err(PortError::new(kind, "fake prepare"));
        }
        Ok(FakePrepared {
            data_root: data_root.clone(),
            volume_id: VolumeId::from_str(VOLUME_ID).unwrap(),
        })
    }

    fn read_config(&self) -> Result<Option<InstallationIdentity>, PortError> {
        self.state.borrow_mut().events.push("config.read");
        Ok(self.state.borrow().config.clone())
    }

    fn read_root_marker(&self, _data_root: &AbsolutePath) -> Result<Option<RootMarker>, PortError> {
        self.state.borrow_mut().events.push("marker.read");
        Ok(self.state.borrow().marker.clone())
    }

    fn create_initializing(
        &self,
        _lock: &Self::LockGuard,
        _prepared: Self::PreparedDataRoot,
        identity: &InstallationIdentity,
    ) -> Result<Self::InitializingProof, PortError> {
        self.state.borrow_mut().events.push("marker.initializing");
        self.state.borrow_mut().marker = Some(RootMarker::new(
            identity.clone(),
            RootMarkerState::Initializing,
        ));
        Ok(identity.clone())
    }

    fn initialize_layout(
        &self,
        _lock: &Self::LockGuard,
        _proof: &Self::InitializingProof,
    ) -> Result<Self::DataRootLayout, PortError> {
        self.state.borrow_mut().events.push("layout.initialize");
        Ok(FakeLayout {
            state: Rc::clone(&self.state),
            database_path: AbsolutePath::try_from_bytes(b"/data/metadata/state.db".to_vec())
                .unwrap(),
        })
    }

    fn validate_layout(
        &self,
        _identity: &InstallationIdentity,
    ) -> Result<Self::DataRootLayout, PortError> {
        self.state.borrow_mut().events.push("layout.validate");
        if let Some(kind) = self.state.borrow().layout_error {
            return Err(PortError::new(kind, "fake layout"));
        }
        Ok(FakeLayout {
            state: Rc::clone(&self.state),
            database_path: AbsolutePath::try_from_bytes(b"/data/metadata/state.db".to_vec())
                .unwrap(),
        })
    }

    fn prepare_workspace(
        &self,
        _lock: &Self::LockGuard,
        _layout: &Self::DataRootLayout,
        _workspace_id: WorkspaceId,
    ) -> Result<Self::PreparedWorkspace, PortError> {
        unreachable!("init/doctor never prepares a Workspace")
    }

    fn clear_workspace_incomplete(
        &self,
        _lock: &Self::LockGuard,
        _layout: &Self::DataRootLayout,
        _prepared: Self::PreparedWorkspace,
    ) -> Result<(), PortError> {
        unreachable!("init/doctor never clears an incomplete marker")
    }

    fn validate_ready_workspace(
        &self,
        _layout: &Self::DataRootLayout,
        workspace_id: WorkspaceId,
    ) -> Result<AbsolutePath, PortError> {
        self.state.borrow_mut().events.push("workspace.validate");
        let record = self
            .state
            .borrow()
            .workspaces
            .iter()
            .find(|record| record.reservation().workspace_id() == workspace_id)
            .cloned()
            .expect("controlled Ready fixture");
        if self.state.borrow().change_ready_during_validation {
            change_first_workspace_to_error(&self.state);
        }
        Ok(record.reservation().target_path().clone())
    }

    fn remove_workspace(
        &self,
        _lock: &Self::LockGuard,
        _layout: &Self::DataRootLayout,
        _workspace_id: WorkspaceId,
    ) -> Result<thinws_ports::WorkspaceRemoval, PortError> {
        unreachable!("init/doctor never removes a Workspace")
    }

    fn publish_ready(
        &self,
        _lock: &Self::LockGuard,
        proof: Self::InitializingProof,
    ) -> Result<RootMarker, PortError> {
        self.state.borrow_mut().events.push("marker.ready");
        let marker = RootMarker::new(proof, RootMarkerState::Ready);
        self.state.borrow_mut().marker = Some(marker.clone());
        Ok(marker)
    }

    fn publish_config(
        &self,
        _lock: &Self::LockGuard,
        identity: &InstallationIdentity,
    ) -> Result<PublishResult, PortError> {
        self.state.borrow_mut().events.push("config.publish");
        self.state.borrow_mut().config = Some(identity.clone());
        Ok(PublishResult::Published)
    }
}

struct FakeMetadata {
    state: Rc<RefCell<State>>,
}

impl MetadataStoreFactory<FakeLayout> for FakeMetadata {
    fn initialize(
        &self,
        _layout: &FakeLayout,
        expected: &InstallationRecord,
        _busy_timeout: Duration,
    ) -> Result<InstallationRecord, PortError> {
        self.state.borrow_mut().events.push("metadata.initialize");
        if let Some(kind) = self.state.borrow().metadata_error {
            return Err(PortError::new(kind, "fake metadata"));
        }
        self.state.borrow_mut().installation = Some(expected.clone());
        Ok(expected.clone())
    }

    fn inspect(
        &self,
        _layout: &FakeLayout,
        expected: &InstallationRecord,
        _busy_timeout: Duration,
    ) -> Result<MetadataSnapshot, PortError> {
        self.state.borrow_mut().events.push("metadata.inspect");
        if let Some(kind) = self.state.borrow().metadata_error {
            return Err(PortError::new(kind, "fake metadata"));
        }
        let installation = self
            .state
            .borrow()
            .installation
            .clone()
            .unwrap_or_else(|| expected.clone());
        Ok(
            MetadataSnapshot::new(installation, self.state.borrow().workspaces.clone())
                .with_final_materializations(self.state.borrow().final_materializations.clone()),
        )
    }

    fn open_existing(
        &self,
        _layout: &FakeLayout,
        _expected: &InstallationRecord,
        _busy_timeout: Duration,
    ) -> Result<Box<dyn thinws_ports::MetadataStore>, PortError> {
        unreachable!("init/doctor never opens a lifecycle writer")
    }
}

fn service(state: Rc<RefCell<State>>) -> ThinWorkspaceService<FakeBootstrap, FakeMetadata> {
    ThinWorkspaceService::new(
        FakeBootstrap {
            state: Rc::clone(&state),
        },
        FakeMetadata { state },
        Duration::from_secs(5),
        Duration::from_secs(5),
    )
}

fn identity(path: &[u8]) -> InstallationIdentity {
    InstallationIdentity::new(
        InstanceId::from_str(INSTANCE_ID).unwrap(),
        AbsolutePath::try_from_bytes(path.to_vec()).unwrap(),
        VolumeId::from_str(VOLUME_ID).unwrap(),
    )
}

fn ready_state() -> Rc<RefCell<State>> {
    let state = Rc::new(RefCell::new(State::default()));
    let installation = InstallationRecord::new(identity(b"/data"), UnixMillis::new(10).unwrap());
    state.borrow_mut().config = Some(installation.identity().clone());
    state.borrow_mut().marker = Some(RootMarker::new(
        installation.identity().clone(),
        RootMarkerState::Ready,
    ));
    state.borrow_mut().installation = Some(installation);
    state
}

fn workspace(state: WorkspaceState, suffix: u8) -> WorkspaceRecord {
    let reservation = WorkspaceReservation::new(
        WorkspaceId::from_str(&format!("ws_01890a5d-ac96-774b-bd5b-55c7b8d09f4{suffix}")).unwrap(),
        InstanceId::from_str(INSTANCE_ID).unwrap(),
        WorkspaceName::from_str(&format!("workspace-{suffix}")).unwrap(),
        AbsolutePath::try_from_bytes(format!("/source/{suffix}").into_bytes()).unwrap(),
        AbsolutePath::try_from_bytes(format!("/data/workspaces/{suffix}").into_bytes()).unwrap(),
        VolumeId::from_str(VOLUME_ID).unwrap(),
        VolumeId::from_str(VOLUME_ID).unwrap(),
        false,
        UnixMillis::new(11).unwrap(),
    );
    WorkspaceRecord::new(reservation, state, None, UnixMillis::new(11).unwrap()).unwrap()
}

#[test]
fn first_init_uses_the_frozen_publication_order() {
    let state = Rc::new(RefCell::new(State::default()));
    let outcome = service(Rc::clone(&state))
        .init(InitRequest::new(
            AbsolutePath::try_from_bytes(b"/data".to_vec()).unwrap(),
            UnixMillis::new(123).unwrap(),
        ))
        .unwrap();

    assert_eq!(outcome.result(), InitResult::Initialized);
    assert_eq!(outcome.installation().created_at().get(), 123);
    assert_eq!(
        state.borrow().events,
        [
            "bootstrap.prepare",
            "lock.acquire",
            "config.read",
            "lock.revalidate",
            "data_root.prepare",
            "marker.initializing",
            "layout.initialize",
            "metadata.initialize",
            "layout.revalidate",
            "marker.ready",
            "config.publish",
        ]
    );
}

#[test]
fn init_revalidates_the_bootstrap_lock_after_config_read_before_touching_data_root() {
    let state = Rc::new(RefCell::new(State {
        lock_revalidate_error: Some(PortErrorKind::InvalidData),
        ..State::default()
    }));

    let error = service(Rc::clone(&state))
        .init(InitRequest::new(
            AbsolutePath::try_from_bytes(b"/data".to_vec()).unwrap(),
            UnixMillis::new(123).unwrap(),
        ))
        .unwrap_err();

    assert_eq!(error.diagnostic().code(), ErrorCode::DataRootLayout);
    assert_eq!(
        state.borrow().events,
        [
            "bootstrap.prepare",
            "lock.acquire",
            "config.read",
            "lock.revalidate",
        ]
    );
    assert!(!state.borrow().events.contains(&"data_root.prepare"));
}

#[test]
fn repeated_init_and_doctor_only_use_existing_state_paths() {
    let state = Rc::new(RefCell::new(State::default()));
    let installation = InstallationRecord::new(identity(b"/data"), UnixMillis::new(10).unwrap());
    state.borrow_mut().config = Some(installation.identity().clone());
    state.borrow_mut().marker = Some(RootMarker::new(
        installation.identity().clone(),
        RootMarkerState::Ready,
    ));
    state.borrow_mut().installation = Some(installation.clone());

    let app = service(Rc::clone(&state));
    let outcome = app
        .init(InitRequest::new(
            AbsolutePath::try_from_bytes(b"/data".to_vec()).unwrap(),
            UnixMillis::new(99).unwrap(),
        ))
        .unwrap();
    assert_eq!(outcome.result(), InitResult::AlreadyInitialized);
    assert_eq!(outcome.installation(), &installation);
    assert!(!state.borrow().events.contains(&"data_root.prepare"));
    assert!(!state.borrow().events.contains(&"config.publish"));

    state.borrow_mut().events.clear();
    let doctor = app.doctor().unwrap();
    assert_eq!(doctor.installation(), &installation);
    assert_eq!(doctor.incomplete_workspaces(), 0);
    assert!(!state.borrow().events.contains(&"lock.acquire"));
    assert!(!state.borrow().events.contains(&"bootstrap.prepare"));
    assert!(!state.borrow().events.contains(&"config.publish"));
}

#[test]
fn a_different_requested_data_root_fails_before_touching_it() {
    let state = Rc::new(RefCell::new(State::default()));
    state.borrow_mut().config = Some(identity(b"/data"));

    let error = service(Rc::clone(&state))
        .init(InitRequest::new(
            AbsolutePath::try_from_bytes(b"/other".to_vec()).unwrap(),
            UnixMillis::new(99).unwrap(),
        ))
        .unwrap_err();
    assert_eq!(
        error.diagnostic().code(),
        ErrorCode::DataRootChangeUnsupported
    );
    assert!(!state.borrow().events.contains(&"data_root.prepare"));
    assert!(!state.borrow().events.contains(&"layout.validate"));
}

#[test]
fn port_failure_classes_map_to_the_frozen_public_codes() {
    for (kind, expected) in [
        (PortErrorKind::NotEmpty, ErrorCode::DataRootNotEmpty),
        (
            PortErrorKind::CapabilityUnavailable,
            ErrorCode::CapabilityUnavailable,
        ),
    ] {
        let state = Rc::new(RefCell::new(State {
            prepare_error: Some(kind),
            ..State::default()
        }));
        let error = service(state)
            .init(InitRequest::new(
                AbsolutePath::try_from_bytes(b"/data".to_vec()).unwrap(),
                UnixMillis::new(99).unwrap(),
            ))
            .unwrap_err();
        assert_eq!(error.diagnostic().code(), expected);
    }

    let state = Rc::new(RefCell::new(State {
        lock_error: Some(PortErrorKind::Timeout),
        ..State::default()
    }));
    let error = service(state)
        .init(InitRequest::new(
            AbsolutePath::try_from_bytes(b"/data".to_vec()).unwrap(),
            UnixMillis::new(99).unwrap(),
        ))
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::LockTimeout);

    let state = ready_state();
    state.borrow_mut().layout_error = Some(PortErrorKind::Unavailable);
    let error = service(state).doctor().unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::DataRootUnavailable);

    let state = ready_state();
    state.borrow_mut().metadata_error = Some(PortErrorKind::Storage);
    let error = service(state).doctor().unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::Metadata);

    let state = ready_state();
    state.borrow_mut().metadata_error = Some(PortErrorKind::InvalidLayout);
    let error = service(state).doctor().unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::DataRootLayout);

    let state = ready_state();
    state.borrow_mut().metadata_error = Some(PortErrorKind::Io);
    let error = service(state).doctor().unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::Filesystem);

    let state = ready_state();
    let installation = state.borrow().installation.clone().unwrap();
    state.borrow_mut().marker = Some(RootMarker::new(
        installation.identity().clone(),
        RootMarkerState::Initializing,
    ));
    let error = service(state).doctor().unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::DataRootLayout);
}

#[test]
fn doctor_counts_only_active_non_ready_workspaces() {
    let state = ready_state();
    state.borrow_mut().workspaces = vec![
        workspace(WorkspaceState::Creating, 0),
        workspace(WorkspaceState::Ready, 1),
    ];

    let outcome = service(state).doctor().unwrap();
    assert_eq!(outcome.incomplete_workspaces(), 1);
}

#[test]
fn path_does_not_publish_ready_when_state_changes_during_read_only_validation() {
    let state = ready_state();
    let record = workspace(WorkspaceState::Ready, 0);
    let id = record.reservation().workspace_id();
    state.borrow_mut().workspaces = vec![record];
    state.borrow_mut().final_materializations = vec![(
        id,
        FinalMaterializationSummary::new(
            MaterializationMode::CowClone,
            MaterializationMode::CowClone,
            MaterializationMode::CowClone,
            MaterializerKind::ApfsFileClone,
            CowEvidence::Confirmed,
            None,
            0,
        ),
    )];
    state.borrow_mut().change_ready_during_validation = true;

    let error = service(Rc::clone(&state))
        .workspace_path("workspace-0")
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::WorkspaceNotReady);
    let events = &state.borrow().events;
    assert_eq!(
        events
            .iter()
            .filter(|event| **event == "metadata.inspect")
            .count(),
        2
    );
    assert!(events.contains(&"workspace.validate"));
    assert!(!events.contains(&"lock.acquire"));
}

#[test]
fn status_does_not_publish_ready_when_git_inspection_changes_the_state() {
    let state = ready_state();
    let record = workspace(WorkspaceState::Ready, 0);
    let id = record.reservation().workspace_id();
    state.borrow_mut().workspaces = vec![record];
    state.borrow_mut().final_materializations = vec![(
        id,
        FinalMaterializationSummary::new(
            MaterializationMode::CowClone,
            MaterializationMode::CowClone,
            MaterializationMode::CowClone,
            MaterializerKind::ApfsFileClone,
            CowEvidence::Confirmed,
            None,
            0,
        ),
    )];
    let git = StateChangingGit {
        state: Rc::clone(&state),
    };

    let status = service(Rc::clone(&state))
        .workspace_status("workspace-0", &git)
        .unwrap();
    assert_eq!(status.workspace().record().state(), WorkspaceState::Error);
    assert!(status.git().is_none());
    assert!(state.borrow().events.contains(&"git.inspect"));
}

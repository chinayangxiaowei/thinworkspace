use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr;
use std::time::Duration;

use thinws_application::{InitRequest, InitResult, ThinWorkspaceService};
use thinws_core::{
    AbsolutePath, ErrorCode, InstallationIdentity, InstallationRecord, InstanceId, RootMarker,
    RootMarkerState, UnixMillis, VolumeId, WorkspaceId, WorkspaceName, WorkspaceRecord,
    WorkspaceReservation, WorkspaceState,
};
use thinws_ports::{
    BootstrapStore, DataRootLayoutEvidence, LifecycleLock, LifecycleLockGuard, LifecycleScope,
    MetadataSnapshot, MetadataStoreFactory, PortError, PortErrorKind, PreparedDataRootEvidence,
    PublishResult,
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
        Ok(MetadataSnapshot::new(
            installation,
            self.state.borrow().workspaces.clone(),
        ))
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

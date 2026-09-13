//! P0-07 test-only connection of removal policy, persistent events, and a
//! controlled no-follow deletion fixture. This is not a Phase 1 lifecycle API.

use std::ffi::CStr;
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use rustix::fs::{Mode, OFlags, fcntl_getfl, fcntl_setfl};
use thinws_p0_cleanup::removal_log::{
    AppendError, CheckCompleteness, EventOutcome, ProcessUseEvidence, ProtectionReason,
    RemovalLogEvent, RemovalResult, RepositoryEvidence, RepositoryRelativePath, SafeId,
    append_event,
};
use thinws_p0_cleanup::{
    DiscoveryCompleteness, GitState, PathValidation, ProcessUse, RemovalDecision, RemovalMode,
    RemovalPreflight, RemovalRefusal, RemovalWarning, VolumeValidation, decide_removal,
};

#[path = "../../materialize/tests/support/mod.rs"]
mod support;

use support::{ControlledTree, TrackedIdentity, tracked_identity};

const COPY_FILE_A: &str = "copy/tracked.txt";
const COPY_FILE_B: &str = "copy/generated.tmp";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AfterStartedAction {
    Continue,
    Interrupt,
    MoveFixtureParent,
    MoveFixtureParentAndDisableAppend,
}

#[derive(Debug, Eq, PartialEq)]
enum FlowResult {
    Refused(RemovalRefusal),
    StartedOnly { warning: Option<RemovalWarning> },
    Removed { warning: Option<RemovalWarning> },
}

#[derive(Debug)]
enum FlowError {
    Log(AppendError),
    Remove {
        removal_error: io::Error,
        failed_log_error: Option<AppendError>,
    },
}

impl fmt::Display for FlowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Log(error) => write!(formatter, "removal event persistence failed: {error}"),
            Self::Remove {
                removal_error,
                failed_log_error: None,
            } => write!(formatter, "controlled removal failed: {removal_error}"),
            Self::Remove {
                removal_error,
                failed_log_error: Some(log_error),
            } => write!(
                formatter,
                "controlled removal failed: {removal_error}; failed event persistence also failed: {log_error}"
            ),
        }
    }
}

impl std::error::Error for FlowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Log(error) => Some(error),
            Self::Remove { removal_error, .. } => Some(removal_error),
        }
    }
}

/// All policy and process facts passed here are deliberate test injections.
fn run_flow(
    tree: &mut ControlledTree,
    log: &mut File,
    preflight: RemovalPreflight,
    mode: RemovalMode,
    completeness: DiscoveryCompleteness,
    repositories: &[RepositoryEvidence<'_>],
    after_started: AfterStartedAction,
) -> Result<FlowResult, FlowError> {
    let force = mode == RemovalMode::Force;
    let process_use = preflight.process_use;
    let warning = match decide_removal(preflight, mode) {
        RemovalDecision::Refuse(reason) => {
            persist_event(
                log,
                force,
                completeness,
                process_use,
                repositories,
                EventOutcome::Refused {
                    protection_reason: ProtectionReason::from(reason),
                },
            )
            .map_err(FlowError::Log)?;
            return Ok(FlowResult::Refused(reason));
        }
        RemovalDecision::Proceed { warning } => warning,
    };

    persist_event(
        log,
        force,
        completeness,
        process_use,
        repositories,
        EventOutcome::Started,
    )
    .map_err(FlowError::Log)?;
    match after_started {
        AfterStartedAction::Continue => {}
        AfterStartedAction::Interrupt => return Ok(FlowResult::StartedOnly { warning }),
        AfterStartedAction::MoveFixtureParent
        | AfterStartedAction::MoveFixtureParentAndDisableAppend => {
            let moved = tree
                .move_fixture_parent_exclusive()
                .expect("controlled failure injection moves only its fixture parent");
            eprintln!(
                "injected P0-07 fixture-parent move after started: {}",
                moved.to_string_lossy()
            );
            if after_started == AfterStartedAction::MoveFixtureParentAndDisableAppend {
                let mut flags = fcntl_getfl(&*log).expect("inspect controlled append descriptor");
                flags.remove(OFlags::APPEND);
                fcntl_setfl(&*log, flags)
                    .expect("disable append only on controlled log descriptor");
            }
        }
    }

    if let Err(error) = tree.verify_exact_tree() {
        return Err(record_removal_failure(
            log,
            force,
            completeness,
            process_use,
            repositories,
            error,
        ));
    }
    let root = tree.root_identity();
    eprintln!(
        "authorized P0-07 removal: root={} dev={} ino={}; exact=[{COPY_FILE_A}, {COPY_FILE_B}, copy]",
        tree.root_path().display(),
        root.device,
        root.inode
    );
    for relative in [COPY_FILE_A, COPY_FILE_B, "copy"] {
        if let Err(error) = tree.remove_tracked_exact(Path::new(relative)) {
            return Err(record_removal_failure(
                log,
                force,
                completeness,
                process_use,
                repositories,
                error,
            ));
        }
    }

    persist_event(
        log,
        force,
        completeness,
        process_use,
        repositories,
        EventOutcome::Completed {
            result: RemovalResult::Removed,
        },
    )
    .map_err(FlowError::Log)?;
    Ok(FlowResult::Removed { warning })
}

fn persist_event(
    log: &mut File,
    force: bool,
    completeness: DiscoveryCompleteness,
    process_use: ProcessUse,
    repositories: &[RepositoryEvidence<'_>],
    outcome: EventOutcome,
) -> Result<(), AppendError> {
    let event = RemovalLogEvent {
        utc_unix_ms: 1_789_300_100_000,
        operation_id: SafeId::new("op_removal_flow").expect("static operation ID is valid"),
        workspace_id: SafeId::new("ws_removal_flow").expect("static workspace ID is valid"),
        force,
        check_completeness: CheckCompleteness::from(completeness),
        process_use: ProcessUseEvidence::from(process_use),
        repositories,
        outcome,
    };
    append_event(log, &event)
}

fn record_removal_failure(
    log: &mut File,
    force: bool,
    completeness: DiscoveryCompleteness,
    process_use: ProcessUse,
    repositories: &[RepositoryEvidence<'_>],
    removal_error: io::Error,
) -> FlowError {
    let failed_log_error = persist_event(
        log,
        force,
        completeness,
        process_use,
        repositories,
        EventOutcome::Failed {
            error: thinws_p0_cleanup::removal_log::RemovalFailure::Filesystem,
        },
    )
    .err();
    FlowError::Remove {
        removal_error,
        failed_log_error,
    }
}

fn repository_target() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("cleanup crate is nested below the repository root")
        .join("target")
}

fn new_case(label: &str) -> ControlledTree {
    let mut tree = ControlledTree::create_in(&repository_target(), label)
        .expect("create retained controlled removal fixture");
    tree.create_directory("copy", 0o700)
        .expect("create controlled copy");
    tree.create_file(COPY_FILE_A, b"tracked change\n", 0o600)
        .expect("create first controlled copy file");
    tree.create_file(COPY_FILE_B, b"untracked output\n", 0o600)
        .expect("create second controlled copy file");
    tree.create_directory("logs", 0o700)
        .expect("create external log directory");
    tree.create_file("logs/operations.jsonl", b"", 0o600)
        .expect("create external log file");
    tree.create_file("source", b"source remains\n", 0o600)
        .expect("create external source sentinel");
    tree.create_file("sentinel", b"outside copy\n", 0o600)
        .expect("create external boundary sentinel");
    tree.verify_exact_tree().expect("verify controlled case");
    let identity = tree.root_identity();
    eprintln!(
        "retained P0-07 removal-flow case: {} (dev {}, ino {}, kind {:?})",
        tree.root_path().display(),
        identity.device,
        identity.inode,
        identity.kind
    );
    tree
}

fn open_log(tree: &ControlledTree, append_writable: bool) -> io::Result<File> {
    let logs = open_registered(
        tree,
        tree.root_fd(),
        c"logs",
        Path::new("logs"),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
    )?;
    let flags = if append_writable {
        OFlags::RDWR | OFlags::APPEND | OFlags::NOFOLLOW | OFlags::CLOEXEC
    } else {
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC
    };
    let log = open_registered(
        tree,
        &logs,
        c"operations.jsonl",
        Path::new("logs/operations.jsonl"),
        flags,
    )?;
    Ok(File::from(log))
}

fn open_registered(
    tree: &ControlledTree,
    parent: &impl std::os::fd::AsFd,
    name: &CStr,
    relative: &Path,
    flags: OFlags,
) -> io::Result<std::os::fd::OwnedFd> {
    let expected = tree
        .tracked()
        .get(relative)
        .copied()
        .ok_or_else(|| io::Error::other("fixed test file is not registered"))?;
    let descriptor = rustix::fs::openat(parent, name, flags, Mode::empty())
        .map_err(|error| io::Error::from_raw_os_error(error.raw_os_error()))?;
    let observed = tracked_identity(
        rustix::fs::fstat(&descriptor)
            .map_err(|error| io::Error::from_raw_os_error(error.raw_os_error()))?,
    )?;
    if observed != expected {
        return Err(io::Error::other(
            "fixed test file identity differs from the controlled ledger",
        ));
    }
    Ok(descriptor)
}

fn read_log(tree: &ControlledTree) -> Vec<serde_json::Value> {
    let mut log = open_log(tree, false).expect("reopen registered log without following links");
    let mut text = String::new();
    log.read_to_string(&mut text).expect("read retained log");
    text.lines()
        .map(|line| serde_json::from_str(line).expect("parse typed JSONL event"))
        .collect()
}

fn read_copy_files(tree: &ControlledTree) -> [Vec<u8>; 2] {
    let copy = open_registered(
        tree,
        tree.root_fd(),
        c"copy",
        Path::new("copy"),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
    )
    .expect("open registered copy directory");
    [
        read_registered(tree, &copy, c"tracked.txt", Path::new(COPY_FILE_A)),
        read_registered(tree, &copy, c"generated.tmp", Path::new(COPY_FILE_B)),
    ]
}

fn read_registered(
    tree: &ControlledTree,
    parent: &impl std::os::fd::AsFd,
    name: &CStr,
    relative: &Path,
) -> Vec<u8> {
    let descriptor = open_registered(
        tree,
        parent,
        name,
        relative,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
    )
    .expect("open registered file without following links");
    let mut file = File::from(descriptor);
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .expect("read registered file");
    contents
}

fn base_preflight(git: GitState) -> RemovalPreflight {
    RemovalPreflight {
        path: PathValidation::Valid,
        volume: VolumeValidation::Valid,
        process_use: ProcessUse::NoEvidence,
        git,
    }
}

fn scan_incomplete_preflight(git: GitState) -> RemovalPreflight {
    RemovalPreflight {
        process_use: ProcessUse::ScanIncomplete,
        ..base_preflight(git)
    }
}

fn dirty_repository() -> [RepositoryEvidence<'static>; 1] {
    [RepositoryEvidence {
        relative_path: RepositoryRelativePath::new(b"").expect("root repository path"),
        tracked_changes: Some(1),
    }]
}

fn unknown_repository() -> [RepositoryEvidence<'static>; 1] {
    [RepositoryEvidence {
        relative_path: RepositoryRelativePath::new(b"").expect("root repository path"),
        tracked_changes: None,
    }]
}

#[derive(Debug, Eq, PartialEq)]
struct OutsideEvidence {
    identities: [TrackedIdentity; 3],
    source: Vec<u8>,
    sentinel: Vec<u8>,
}

fn outside_evidence(tree: &ControlledTree) -> OutsideEvidence {
    OutsideEvidence {
        identities: [
            tree.identity_at(Path::new("logs/operations.jsonl"))
                .expect("log identity"),
            tree.identity_at(Path::new("source"))
                .expect("source identity"),
            tree.identity_at(Path::new("sentinel"))
                .expect("sentinel identity"),
        ],
        source: read_registered(tree, tree.root_fd(), c"source", Path::new("source")),
        sentinel: read_registered(tree, tree.root_fd(), c"sentinel", Path::new("sentinel")),
    }
}

fn assert_outside_unchanged(tree: &ControlledTree, expected: OutsideEvidence) {
    tree.verify_exact_tree().expect("remaining tree is exact");
    assert_eq!(outside_evidence(tree), expected);
}

fn assert_copy_removed(tree: &ControlledTree) {
    let error = tree
        .identity_at(Path::new("copy"))
        .expect_err("controlled copy must be absent");
    assert_eq!(error.raw_os_error(), Some(libc::ENOENT));
}

fn assert_started_completed(
    events: &[serde_json::Value],
    force: bool,
    completeness: DiscoveryCompleteness,
    process_use: &str,
) {
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "started");
    assert_eq!(events[1]["event"], "completed");
    assert_eq!(events[0]["force"], force);
    assert_eq!(events[1]["force"], force);
    assert_eq!(events[1]["result"], "removed");
    let expected_completeness = match completeness {
        DiscoveryCompleteness::Complete => "complete",
        DiscoveryCompleteness::Incomplete => "incomplete",
    };
    assert_eq!(events[0]["check_completeness"], expected_completeness);
    assert_eq!(events[1]["check_completeness"], expected_completeness);
    assert_eq!(events[0]["process_use"], process_use);
    assert_eq!(events[1]["process_use"], process_use);
    assert_eq!(events[0]["operation_id"], events[1]["operation_id"]);
}

#[test]
fn injected_dirty_and_unknown_normal_removal_are_refused_without_deletion() {
    for (label, git, completeness, repositories, refusal) in [
        (
            "normal-dirty",
            GitState::Dirty,
            DiscoveryCompleteness::Complete,
            dirty_repository(),
            RemovalRefusal::TrackedChanges,
        ),
        (
            "normal-unknown",
            GitState::Unknown,
            DiscoveryCompleteness::Incomplete,
            unknown_repository(),
            RemovalRefusal::GitCheckIncomplete,
        ),
    ] {
        let mut tree = new_case(label);
        let before = tree.observed_tree().expect("observe complete tree");
        let copy_contents = read_copy_files(&tree);
        let mut log = open_log(&tree, true).expect("open registered append log");
        let result = run_flow(
            &mut tree,
            &mut log,
            base_preflight(git),
            RemovalMode::Normal,
            completeness,
            &repositories,
            AfterStartedAction::Continue,
        )
        .expect("normal refusal is a completed flow result");

        assert_eq!(result, FlowResult::Refused(refusal));
        assert_eq!(
            tree.observed_tree().expect("observe preserved tree"),
            before
        );
        assert_eq!(read_copy_files(&tree), copy_contents);
        let events = read_log(&tree);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "refused");
        assert_eq!(events[0]["force"], false);
    }
}

#[test]
fn injected_force_bypasses_dirty_and_unknown_content_checks_only() {
    for (label, git, completeness, repositories) in [
        (
            "force-dirty",
            GitState::Dirty,
            DiscoveryCompleteness::Complete,
            dirty_repository(),
        ),
        (
            "force-unknown",
            GitState::Unknown,
            DiscoveryCompleteness::Incomplete,
            unknown_repository(),
        ),
    ] {
        let mut tree = new_case(label);
        let outside = outside_evidence(&tree);
        let mut log = open_log(&tree, true).expect("open registered append log");
        let result = run_flow(
            &mut tree,
            &mut log,
            base_preflight(git),
            RemovalMode::Force,
            completeness,
            &repositories,
            AfterStartedAction::Continue,
        )
        .expect("forced content-check bypass succeeds");

        assert_eq!(result, FlowResult::Removed { warning: None });
        assert_copy_removed(&tree);
        assert_outside_unchanged(&tree, outside);
        assert_started_completed(&read_log(&tree), true, completeness, "no_evidence");
    }
}

#[test]
fn injected_clean_force_still_persists_started_and_completed() {
    let mut tree = new_case("force-clean");
    let outside = outside_evidence(&tree);
    let repositories = [RepositoryEvidence {
        relative_path: RepositoryRelativePath::new(b"").expect("root repository path"),
        tracked_changes: Some(0),
    }];
    let mut log = open_log(&tree, true).expect("open registered append log");

    let result = run_flow(
        &mut tree,
        &mut log,
        base_preflight(GitState::Clean),
        RemovalMode::Force,
        DiscoveryCompleteness::Complete,
        &repositories,
        AfterStartedAction::Continue,
    )
    .expect("clean forced flow succeeds");

    assert_eq!(result, FlowResult::Removed { warning: None });
    assert_copy_removed(&tree);
    assert_outside_unchanged(&tree, outside);
    assert_started_completed(
        &read_log(&tree),
        true,
        DiscoveryCompleteness::Complete,
        "no_evidence",
    );
}

fn assert_scan_incomplete_clean_proceeds(mode: RemovalMode, label: &str) {
    let mut tree = new_case(label);
    let outside = outside_evidence(&tree);
    let repositories = [RepositoryEvidence {
        relative_path: RepositoryRelativePath::new(b"").expect("root repository path"),
        tracked_changes: Some(0),
    }];
    let mut log = open_log(&tree, true).expect("open registered append log");

    let result = run_flow(
        &mut tree,
        &mut log,
        scan_incomplete_preflight(GitState::Clean),
        mode,
        DiscoveryCompleteness::Complete,
        &repositories,
        AfterStartedAction::Continue,
    )
    .expect("incomplete process scan warns but does not refuse clean content");

    assert_eq!(
        result,
        FlowResult::Removed {
            warning: Some(RemovalWarning::ProcessScanIncomplete),
        }
    );
    assert_copy_removed(&tree);
    assert_outside_unchanged(&tree, outside);
    assert_started_completed(
        &read_log(&tree),
        mode == RemovalMode::Force,
        DiscoveryCompleteness::Complete,
        "scan_incomplete",
    );
}

#[test]
fn injected_scan_incomplete_clean_normal_proceeds_with_warning_and_log() {
    assert_scan_incomplete_clean_proceeds(RemovalMode::Normal, "scan-incomplete-clean-normal");
}

#[test]
fn injected_scan_incomplete_clean_force_proceeds_with_warning_and_log() {
    assert_scan_incomplete_clean_proceeds(RemovalMode::Force, "scan-incomplete-clean-force");
}

#[test]
fn injected_scan_incomplete_dirty_normal_refusal_still_records_scan_fact() {
    let mut tree = new_case("scan-incomplete-dirty-normal");
    let before = tree.observed_tree().expect("observe complete tree");
    let copy_contents = read_copy_files(&tree);
    let repositories = dirty_repository();
    let mut log = open_log(&tree, true).expect("open registered append log");

    let result = run_flow(
        &mut tree,
        &mut log,
        scan_incomplete_preflight(GitState::Dirty),
        RemovalMode::Normal,
        DiscoveryCompleteness::Complete,
        &repositories,
        AfterStartedAction::Continue,
    )
    .expect("tracked dirty remains the normal-removal refusal");

    assert_eq!(result, FlowResult::Refused(RemovalRefusal::TrackedChanges));
    assert_eq!(
        tree.observed_tree().expect("observe preserved tree"),
        before
    );
    assert_eq!(read_copy_files(&tree), copy_contents);
    let events = read_log(&tree);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "refused");
    assert_eq!(events[0]["protection_reason"], "tracked_changes");
    assert_eq!(events[0]["process_use"], "scan_incomplete");
    assert_eq!(events[0]["force"], false);
}

#[test]
fn injected_unwritable_started_log_preserves_the_complete_copy() {
    let mut tree = new_case("unwritable-log");
    let before = tree.observed_tree().expect("observe complete tree");
    let repositories = dirty_repository();
    let mut log = open_log(&tree, false).expect("open registered read-only log");

    let result = run_flow(
        &mut tree,
        &mut log,
        base_preflight(GitState::Dirty),
        RemovalMode::Force,
        DiscoveryCompleteness::Complete,
        &repositories,
        AfterStartedAction::Continue,
    );

    assert!(matches!(
        result,
        Err(FlowError::Log(AppendError::NotAppendWritable))
    ));
    assert_eq!(
        tree.observed_tree().expect("observe preserved tree"),
        before
    );
    assert!(read_log(&tree).is_empty());
}

#[test]
fn injected_interruption_after_started_preserves_copy_and_has_no_completion() {
    let mut tree = new_case("interrupt-after-started");
    let before = tree.observed_tree().expect("observe complete tree");
    let repositories = unknown_repository();
    let mut log = open_log(&tree, true).expect("open registered append log");

    let result = run_flow(
        &mut tree,
        &mut log,
        base_preflight(GitState::Unknown),
        RemovalMode::Force,
        DiscoveryCompleteness::Incomplete,
        &repositories,
        AfterStartedAction::Interrupt,
    )
    .expect("test interruption is an explained result");

    assert_eq!(result, FlowResult::StartedOnly { warning: None });
    assert_eq!(
        tree.observed_tree().expect("observe preserved tree"),
        before
    );
    let events = read_log(&tree);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "started");
}

#[test]
fn injected_parent_move_after_started_logs_failed_without_deletion() {
    let mut tree = new_case("failure-after-started");
    let before = tree.observed_tree().expect("observe complete tree");
    let copy_contents = read_copy_files(&tree);
    let outside = outside_evidence(&tree);
    let repositories = dirty_repository();
    let mut log = open_log(&tree, true).expect("open registered append log");

    let error = run_flow(
        &mut tree,
        &mut log,
        base_preflight(GitState::Dirty),
        RemovalMode::Force,
        DiscoveryCompleteness::Complete,
        &repositories,
        AfterStartedAction::MoveFixtureParent,
    )
    .expect_err("moved fixture parent must stop before deletion");

    let display = error.to_string();
    let source_text = std::error::Error::source(&error)
        .expect("original removal error is the primary source")
        .to_string();
    assert!(!source_text.is_empty());
    assert!(display.contains(&source_text));
    match error {
        FlowError::Remove {
            removal_error,
            failed_log_error,
        } => {
            assert_eq!(removal_error.to_string(), source_text);
            assert!(failed_log_error.is_none());
        }
        FlowError::Log(_) => panic!("expected the original controlled-tree failure"),
    }
    assert_eq!(
        tree.observed_tree().expect("observe preserved tree"),
        before
    );
    assert_eq!(read_copy_files(&tree), copy_contents);
    assert_eq!(outside_evidence(&tree), outside);
    let events = read_log(&tree);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "started");
    assert_eq!(events[1]["event"], "failed");
    assert_eq!(events[1]["error"], "filesystem");
}

#[test]
fn injected_failed_event_write_preserves_both_errors_and_the_copy() {
    let mut tree = new_case("failure-log-failure");
    let before = tree.observed_tree().expect("observe complete tree");
    let copy_contents = read_copy_files(&tree);
    let outside = outside_evidence(&tree);
    let repositories = dirty_repository();
    let mut log = open_log(&tree, true).expect("open registered append log");

    let error = run_flow(
        &mut tree,
        &mut log,
        base_preflight(GitState::Dirty),
        RemovalMode::Force,
        DiscoveryCompleteness::Complete,
        &repositories,
        AfterStartedAction::MoveFixtureParentAndDisableAppend,
    )
    .expect_err("identity and failed-log injections must both remain visible");

    let display = error.to_string();
    let source_text = std::error::Error::source(&error)
        .expect("original removal error is the primary source")
        .to_string();
    assert!(display.contains(&source_text));
    match error {
        FlowError::Remove {
            removal_error,
            failed_log_error,
        } => {
            assert_eq!(removal_error.to_string(), source_text);
            assert!(matches!(
                failed_log_error,
                Some(AppendError::NotAppendWritable)
            ));
            assert!(display.contains("failed event persistence also failed"));
        }
        FlowError::Log(_) => panic!("expected both post-started errors"),
    }
    assert_eq!(
        tree.observed_tree().expect("observe preserved tree"),
        before
    );
    assert_eq!(read_copy_files(&tree), copy_contents);
    assert_eq!(outside_evidence(&tree), outside);
    let events = read_log(&tree);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "started");
}

#[test]
fn injected_force_cannot_bypass_path_volume_or_confirmed_use() {
    let cases = [
        (
            "force-unsafe-path",
            RemovalPreflight {
                path: PathValidation::Failed,
                ..base_preflight(GitState::Clean)
            },
            RemovalRefusal::UnsafePath,
            "no_evidence",
        ),
        (
            "force-volume-mismatch",
            RemovalPreflight {
                volume: VolumeValidation::Failed,
                ..base_preflight(GitState::Clean)
            },
            RemovalRefusal::VolumeMismatch,
            "no_evidence",
        ),
        (
            "force-confirmed-use",
            RemovalPreflight {
                process_use: ProcessUse::ConfirmedInUse,
                ..base_preflight(GitState::Clean)
            },
            RemovalRefusal::ConfirmedInUse,
            "confirmed_in_use",
        ),
    ];

    for (label, preflight, refusal, process_use) in cases {
        let mut tree = new_case(label);
        let before = tree.observed_tree().expect("observe complete tree");
        let mut log = open_log(&tree, true).expect("open registered append log");
        let result = run_flow(
            &mut tree,
            &mut log,
            preflight,
            RemovalMode::Force,
            DiscoveryCompleteness::Complete,
            &[],
            AfterStartedAction::Continue,
        )
        .expect("non-bypassable refusal is an explained result");

        assert_eq!(result, FlowResult::Refused(refusal));
        assert_eq!(
            tree.observed_tree().expect("observe preserved tree"),
            before
        );
        let events = read_log(&tree);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "refused");
        assert_eq!(events[0]["force"], true);
        assert_eq!(events[0]["process_use"], process_use);
    }
}

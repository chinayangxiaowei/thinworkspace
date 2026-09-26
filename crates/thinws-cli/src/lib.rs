#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Phase 1 command parsing and human/JSON rendering.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand, error::ErrorKind};
use serde_json::{Map, Value, json};
use thinws_adapter_git_cli::SystemGitInspector;
use thinws_adapter_macos::{ApfsCloneMaterializer, FullCopyMaterializer, MacOsHostAdapter};
use thinws_application::{
    CowEvidence, CreateRequest, DiscoveryCompleteness, FallbackReason, GitInspectionIssue,
    GitQueryFailureKind, GitState, InitRequest, MaterializationMode, MaterializerKind,
    RemoveRequest, RepositoryState, ThinWorkspaceService, WorkspaceQuery, WorkspaceStatus,
};
use thinws_metadata_sqlite::SqliteMetadataStoreFactory;

/// Renderer-facing successful init data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitView {
    /// Stable result enum.
    pub result: String,
    /// UUIDv7 installation identifier.
    pub instance_id: String,
    /// Lossless canonical data-root bytes.
    pub data_root: Vec<u8>,
    /// APFS volume UUID.
    pub volume_id: String,
}

/// Renderer-facing successful doctor data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoctorView {
    /// UUIDv7 installation identifier.
    pub instance_id: String,
    /// Lossless canonical data-root bytes.
    pub data_root: Vec<u8>,
    /// APFS volume UUID.
    pub volume_id: String,
    /// Number of active non-Ready Workspace records.
    pub incomplete_workspaces: usize,
}

/// Renderer-facing completed creation data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateReadyView {
    /// UUIDv7 Workspace identifier.
    pub workspace_id: String,
    /// Exact public Workspace name.
    pub name: String,
    /// Lossless canonical source path.
    pub source: Vec<u8>,
    /// Lossless ordinary target path.
    pub path: Vec<u8>,
    /// Whether this invocation performed a new materialization.
    pub created: bool,
    /// Original requested materialization mode.
    pub requested_mode: String,
    /// Effective planned mode after any allowed fallback.
    pub effective_mode: String,
    /// Mode actually executed.
    pub actual_mode: String,
    /// Concrete successful backend.
    pub adapter: String,
    /// Actual CoW evidence.
    pub cow: String,
    /// Stable fallback reason, when used.
    pub fallback_reason: Option<String>,
    /// Earlier failed attempts retained by a runtime fallback.
    pub failed_attempt_count: usize,
}

/// Renderer-facing read-only creation preview without a Workspace ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatePreviewView {
    /// Exact public Workspace name.
    pub name: String,
    /// Lossless canonical source path.
    pub source: Vec<u8>,
    /// Existing controlled parent for a future ID-derived target.
    pub target_parent: Vec<u8>,
    /// Source APFS Volume UUID.
    pub source_volume_id: String,
    /// Target APFS Volume UUID.
    pub target_volume_id: String,
    /// Mode currently selected without execution.
    pub effective_mode: String,
    /// Backend currently selected without execution.
    pub selected_adapter: String,
    /// Preflight fallback reason, when required.
    pub fallback_reason: Option<String>,
}

/// Distinguishes a completed Workspace from a non-executable preview.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreateView {
    /// Ready Workspace with a durable final Receipt.
    Ready(CreateReadyView),
    /// Read-only plan without an allocated Workspace ID.
    Preview(CreatePreviewView),
}

/// Renderer-facing final receipt facts for a Ready Workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializationView {
    /// Original requested materialization mode.
    pub requested_mode: String,
    /// Effective planned mode after permitted fallback.
    pub effective_mode: String,
    /// Mode actually executed.
    pub actual_mode: String,
    /// Concrete successful backend.
    pub adapter: String,
    /// Actual CoW evidence.
    pub cow: String,
    /// Stable fallback reason, if any.
    pub fallback_reason: Option<String>,
    /// Failed attempts retained before final success.
    pub failed_attempt_count: usize,
}

/// Renderer-facing active Workspace record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceView {
    /// UUIDv7 Workspace identifier.
    pub workspace_id: String,
    /// Exact user-facing name.
    pub name: String,
    /// Durable lifecycle state.
    pub state: String,
    /// Lossless canonical source path bytes.
    pub source: Vec<u8>,
    /// Lossless ID-derived target path bytes, which may not be usable when non-Ready.
    pub path: Vec<u8>,
    /// Stable last error code for an Error record.
    pub last_error_code: Option<String>,
    /// Final receipt facts when a successful materialization occurred.
    pub materialization: Option<MaterializationView>,
}

/// One repository's tracked-change summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryView {
    /// Lossless path relative to the Workspace root; `.` denotes root.
    pub relative_path: Vec<u8>,
    /// Stable clean, dirty, or unknown state.
    pub state: String,
    /// Distinct tracked path count, absent when unknown.
    pub tracked_changes: Option<usize>,
    /// Stable reasons for unknown state.
    pub issues: Vec<String>,
}

/// Renderer-facing Git discovery and tracked-change evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitView {
    /// Whether the bounded repository scan completed.
    pub scan_complete: bool,
    /// Stable aggregate state.
    pub state: String,
    /// Whole-scan reasons for unknown state.
    pub issues: Vec<String>,
    /// Repositories in stable relative-path order.
    pub repositories: Vec<RepositoryView>,
}

/// Renderer-facing status of one active Workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusView {
    /// Durable Workspace and final receipt facts.
    pub workspace: WorkspaceView,
    /// Git evidence only for a verified Ready Workspace.
    pub git: Option<GitView>,
}

/// Renderer-facing result of one explicit Workspace cleanup attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoveView {
    /// Exact Workspace identifier.
    pub workspace_id: String,
    /// Per-attempt correlation identifier.
    pub operation_id: String,
    /// Whether force was explicitly requested.
    pub forced: bool,
    /// Stable removed or already-removed result.
    pub result: String,
    /// Lossless log path bytes outside the Workspace copy.
    pub log_path: Vec<u8>,
    /// Non-blocking process-scan warning, when applicable.
    pub warning: Option<String>,
}

/// Renderer-facing stable failure data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorView {
    /// Stable symbolic code.
    pub code: String,
    /// Human-readable implementation message.
    pub message: String,
    /// Stable string context fields.
    pub context: Map<String, Value>,
    /// Optional remediation.
    pub remediation: Option<String>,
}

/// CLI seam that contains no renderer or business policy.
pub trait Commands {
    /// Executes init from lossless path bytes and an injected wall clock.
    fn init(&self, data_root: Vec<u8>, now_ms: i64) -> Result<InitView, ErrorView>;

    /// Executes product-state diagnosis.
    fn doctor(&self) -> Result<DoctorView, ErrorView>;

    /// Executes or previews direct directory mirroring.
    fn create(
        &self,
        source: Vec<u8>,
        name: String,
        allow_copy: bool,
        dry_run: bool,
        now_ms: i64,
    ) -> Result<CreateView, ErrorView>;

    /// Lists active Workspaces without starting Git.
    fn list(&self) -> Result<Vec<WorkspaceView>, ErrorView>;

    /// Returns one verified Ready Workspace path as lossless bytes.
    fn path(&self, name: String) -> Result<Vec<u8>, ErrorView>;

    /// Returns one Workspace's metadata and on-demand tracked-change evidence.
    fn status(&self, name: String) -> Result<StatusView, ErrorView>;

    /// Removes one registered Workspace after policy checks and durable logging.
    fn remove(&self, target: String, force: bool, now_ms: i64) -> Result<RemoveView, ErrorView>;
}

/// Local composition root for the macOS/APFS Phase 1 command set.
pub struct LocalCommands {
    bootstrap_dir: Option<PathBuf>,
    lock_timeout: Duration,
    sqlite_timeout: Duration,
}

impl LocalCommands {
    /// Selects the fixed bootstrap location without creating it.
    #[must_use]
    pub const fn new(bootstrap_dir: Option<PathBuf>) -> Self {
        Self {
            bootstrap_dir,
            lock_timeout: Duration::from_secs(5),
            sqlite_timeout: Duration::from_secs(5),
        }
    }

    /// Replaces production timeouts for deterministic integration tests.
    #[must_use]
    pub const fn with_timeouts(mut self, lock_timeout: Duration, sqlite_timeout: Duration) -> Self {
        self.lock_timeout = lock_timeout;
        self.sqlite_timeout = sqlite_timeout;
        self
    }

    fn adapter(&self) -> Result<MacOsHostAdapter, ErrorView> {
        let path = self.bootstrap_dir.as_ref().ok_or_else(|| ErrorView {
            code: "E_FILESYSTEM".to_owned(),
            message: "the current user data directory is unavailable".to_owned(),
            context: Map::new(),
            remediation: None,
        })?;
        MacOsHostAdapter::new(path).map_err(|_| ErrorView {
            code: "E_FILESYSTEM".to_owned(),
            message: "the ThinWorkspace bootstrap path is invalid".to_owned(),
            context: Map::new(),
            remediation: None,
        })
    }
}

impl Commands for LocalCommands {
    fn init(&self, data_root: Vec<u8>, now_ms: i64) -> Result<InitView, ErrorView> {
        let request = InitRequest::try_from_raw(data_root, now_ms).map_err(use_case_error_view)?;
        let service = ThinWorkspaceService::new(
            self.adapter()?,
            SqliteMetadataStoreFactory,
            self.lock_timeout,
            self.sqlite_timeout,
        );
        let outcome = service.init(request).map_err(use_case_error_view)?;
        let identity = outcome.installation().identity();
        Ok(InitView {
            result: outcome.result().as_str().to_owned(),
            instance_id: identity.instance_id().to_string(),
            data_root: identity.data_root().as_bytes().to_vec(),
            volume_id: identity.volume_id().to_string(),
        })
    }

    fn doctor(&self) -> Result<DoctorView, ErrorView> {
        let service = ThinWorkspaceService::new(
            self.adapter()?,
            SqliteMetadataStoreFactory,
            self.lock_timeout,
            self.sqlite_timeout,
        );
        let outcome = service.doctor().map_err(use_case_error_view)?;
        let identity = outcome.installation().identity();
        Ok(DoctorView {
            instance_id: identity.instance_id().to_string(),
            data_root: identity.data_root().as_bytes().to_vec(),
            volume_id: identity.volume_id().to_string(),
            incomplete_workspaces: outcome.incomplete_workspaces(),
        })
    }

    fn create(
        &self,
        source: Vec<u8>,
        name: String,
        allow_copy: bool,
        dry_run: bool,
        now_ms: i64,
    ) -> Result<CreateView, ErrorView> {
        let request = CreateRequest::try_from_raw(source, &name, allow_copy, now_ms)
            .map_err(use_case_error_view)?;
        let adapter = self.adapter()?;
        let service = ThinWorkspaceService::new(
            adapter.clone(),
            SqliteMetadataStoreFactory,
            self.lock_timeout,
            self.sqlite_timeout,
        );
        if dry_run {
            let preview = service
                .preview_create(&request)
                .map_err(use_case_error_view)?;
            Ok(CreateView::Preview(CreatePreviewView {
                name: request.name().to_string(),
                source: request.source().as_bytes().to_vec(),
                target_parent: preview.target_parent().as_bytes().to_vec(),
                source_volume_id: preview.source_volume_id().to_string(),
                target_volume_id: preview.target_volume_id().to_string(),
                effective_mode: mode_name(preview.effective_mode()).to_owned(),
                selected_adapter: adapter_name(preview.selected_adapter()).to_owned(),
                fallback_reason: preview
                    .fallback_reason()
                    .map(|reason| fallback_name(reason).to_owned()),
            }))
        } else {
            let clone = ApfsCloneMaterializer::new(adapter.clone());
            let copy = FullCopyMaterializer::new(adapter);
            let outcome = service
                .create(request, &clone, &copy)
                .map_err(use_case_error_view)?;
            let reservation = outcome.record().reservation();
            let facts = outcome.materialization();
            Ok(CreateView::Ready(CreateReadyView {
                workspace_id: reservation.workspace_id().to_string(),
                name: reservation.name().to_string(),
                source: reservation.source_path().as_bytes().to_vec(),
                path: reservation.target_path().as_bytes().to_vec(),
                created: outcome.created(),
                requested_mode: mode_name(facts.requested_mode()).to_owned(),
                effective_mode: mode_name(facts.effective_mode()).to_owned(),
                actual_mode: mode_name(facts.actual_mode()).to_owned(),
                adapter: adapter_name(facts.adapter()).to_owned(),
                cow: cow_name(facts.cow()).to_owned(),
                fallback_reason: facts
                    .fallback_reason()
                    .map(|reason| fallback_name(reason).to_owned()),
                failed_attempt_count: facts.failed_attempt_count(),
            }))
        }
    }

    fn list(&self) -> Result<Vec<WorkspaceView>, ErrorView> {
        let service = ThinWorkspaceService::new(
            self.adapter()?,
            SqliteMetadataStoreFactory,
            self.lock_timeout,
            self.sqlite_timeout,
        );
        service
            .list_workspaces()
            .map(|workspaces| workspaces.iter().map(workspace_view).collect())
            .map_err(use_case_error_view)
    }

    fn path(&self, name: String) -> Result<Vec<u8>, ErrorView> {
        let service = ThinWorkspaceService::new(
            self.adapter()?,
            SqliteMetadataStoreFactory,
            self.lock_timeout,
            self.sqlite_timeout,
        );
        service
            .workspace_path(&name)
            .map(|path| path.as_bytes().to_vec())
            .map_err(use_case_error_view)
    }

    fn status(&self, name: String) -> Result<StatusView, ErrorView> {
        let service = ThinWorkspaceService::new(
            self.adapter()?,
            SqliteMetadataStoreFactory,
            self.lock_timeout,
            self.sqlite_timeout,
        );
        service
            .workspace_status(&name, &SystemGitInspector)
            .map(|status| status_view(&status))
            .map_err(use_case_error_view)
    }

    fn remove(&self, target: String, force: bool, now_ms: i64) -> Result<RemoveView, ErrorView> {
        let request =
            RemoveRequest::try_from_raw(&target, force, now_ms).map_err(use_case_error_view)?;
        let adapter = self.adapter()?;
        let service = ThinWorkspaceService::new(
            adapter.clone(),
            SqliteMetadataStoreFactory,
            self.lock_timeout,
            self.sqlite_timeout,
        );
        let outcome = service
            .remove(request, &SystemGitInspector, &adapter)
            .map_err(use_case_error_view)?;
        Ok(RemoveView {
            workspace_id: outcome.workspace_id().to_string(),
            operation_id: outcome.operation_id().to_string(),
            forced: outcome.forced(),
            result: outcome.result().as_str().to_owned(),
            log_path: outcome.log_path().as_bytes().to_vec(),
            warning: outcome
                .warning()
                .map(|_| "process-scan-incomplete".to_owned()),
        })
    }
}

fn workspace_view(query: &WorkspaceQuery) -> WorkspaceView {
    let record = query.record();
    let reservation = record.reservation();
    WorkspaceView {
        workspace_id: reservation.workspace_id().to_string(),
        name: reservation.name().to_string(),
        state: record.state().as_str().to_owned(),
        source: reservation.source_path().as_bytes().to_vec(),
        path: reservation.target_path().as_bytes().to_vec(),
        last_error_code: record.last_error_code().map(|code| code.to_string()),
        materialization: query.materialization().map(|facts| MaterializationView {
            requested_mode: mode_name(facts.requested_mode()).to_owned(),
            effective_mode: mode_name(facts.effective_mode()).to_owned(),
            actual_mode: mode_name(facts.actual_mode()).to_owned(),
            adapter: adapter_name(facts.adapter()).to_owned(),
            cow: cow_name(facts.cow()).to_owned(),
            fallback_reason: facts
                .fallback_reason()
                .map(|reason| fallback_name(reason).to_owned()),
            failed_attempt_count: facts.failed_attempt_count(),
        }),
    }
}

fn status_view(status: &WorkspaceStatus) -> StatusView {
    StatusView {
        workspace: workspace_view(status.workspace()),
        git: status.git().map(|inspection| GitView {
            scan_complete: inspection.discovery() == DiscoveryCompleteness::Complete,
            state: git_state_name(inspection.aggregate()).to_owned(),
            issues: inspection
                .issues()
                .iter()
                .map(|issue| git_issue_name(issue).to_owned())
                .collect(),
            repositories: inspection
                .repositories()
                .iter()
                .map(|repository| RepositoryView {
                    relative_path: if repository.relative_path().as_os_str().is_empty() {
                        b".".to_vec()
                    } else {
                        repository.relative_path().as_os_str().as_bytes().to_vec()
                    },
                    state: repository_state_name(repository.state()).to_owned(),
                    tracked_changes: repository.state().tracked_change_count(),
                    issues: repository
                        .issues()
                        .iter()
                        .map(|issue| git_issue_name(issue).to_owned())
                        .collect(),
                })
                .collect(),
        }),
    }
}

const fn git_state_name(state: GitState) -> &'static str {
    match state {
        GitState::NotApplicable => "not-applicable",
        GitState::Clean => "clean",
        GitState::Dirty => "dirty",
        GitState::Unknown => "unknown",
    }
}

const fn repository_state_name(state: RepositoryState) -> &'static str {
    match state {
        RepositoryState::Clean => "clean",
        RepositoryState::Dirty { .. } => "dirty",
        RepositoryState::Unknown => "unknown",
    }
}

const fn git_issue_name(issue: &GitInspectionIssue) -> &'static str {
    match issue {
        GitInspectionIssue::InvalidCopyRoot => "invalid-copy-root",
        GitInspectionIssue::EnvironmentUnsupported => "environment-unsupported",
        GitInspectionIssue::InvalidExecPath => "invalid-exec-path",
        GitInspectionIssue::ScanFailed => "scan-failed",
        GitInspectionIssue::ScanLimitReached => "scan-limit-reached",
        GitInspectionIssue::DepthLimitReached => "depth-limit-reached",
        GitInspectionIssue::RepositoryLimitReached => "repository-limit-reached",
        GitInspectionIssue::UnsafeRepositoryMetadata => "unsafe-repository-metadata",
        GitInspectionIssue::ExternalRepositoryMetadata => "external-repository-metadata",
        GitInspectionIssue::UnsupportedConfiguration => "unsupported-configuration",
        GitInspectionIssue::UnsafeAttributes => "unsafe-attributes",
        GitInspectionIssue::HiddenIndexFlags => "hidden-index-flags",
        GitInspectionIssue::SparseCheckout => "sparse-checkout",
        GitInspectionIssue::QueryFailed { kind, .. } => match kind {
            GitQueryFailureKind::InvalidInput { .. } => "query-invalid-input",
            GitQueryFailureKind::Start { .. } => "query-start-failed",
            GitQueryFailureKind::Io { .. } => "query-io-failed",
            GitQueryFailureKind::TimedOut => "query-timed-out",
            GitQueryFailureKind::OutputLimitExceeded { .. } => "query-output-limit",
        },
        GitInspectionIssue::QueryNonZeroExit(_) => "query-nonzero-exit",
        GitInspectionIssue::InvalidGitOutput => "invalid-git-output",
        GitInspectionIssue::EvidenceChanged => "evidence-changed",
        GitInspectionIssue::UnsafeDescendant => "unsafe-descendant",
        GitInspectionIssue::BudgetExpired => "budget-expired",
    }
}

/// Parses one argv vector, executes a command, renders output and returns its exit status.
pub fn run<I, C, O, E>(args: I, commands: &C, now_ms: i64, stdout: &mut O, stderr: &mut E) -> i32
where
    I: IntoIterator<Item = OsString>,
    C: Commands,
    O: Write,
    E: Write,
{
    let args: Vec<OsString> = args.into_iter().collect();
    let prescan = prescan(&args);
    if prescan.json && prescan.help_or_version {
        let error = usage_error("--json cannot be combined with --help or --version");
        return render_error(&error, true, stdout, stderr);
    }

    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            if prescan.json {
                return render_error(
                    &usage_error("invalid command or arguments"),
                    true,
                    stdout,
                    stderr,
                );
            }
            let success = matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            );
            if success {
                if stdout.write_all(error.to_string().as_bytes()).is_err() {
                    return 31;
                }
                return 0;
            }
            return render_error(
                &usage_error("invalid command or arguments"),
                false,
                stdout,
                stderr,
            );
        }
    };

    if cli.json
        && matches!(
            &cli.command,
            Command::Workspace {
                command: WorkspaceCommand::Path { .. }
            }
        )
    {
        return render_error(
            &usage_error("workspace path does not support --json"),
            true,
            stdout,
            stderr,
        );
    }

    let result = match cli.command {
        Command::Init { data_root } => commands
            .init(data_root.as_os_str().as_bytes().to_vec(), now_ms)
            .map(Success::Init),
        Command::Doctor => commands.doctor().map(Success::Doctor),
        Command::Workspace {
            command:
                WorkspaceCommand::Create {
                    source,
                    name,
                    allow_copy,
                    dry_run,
                },
        } => commands
            .create(
                source.as_os_str().as_bytes().to_vec(),
                name,
                allow_copy,
                dry_run,
                now_ms,
            )
            .map(Success::Create),
        Command::Workspace {
            command: WorkspaceCommand::List,
        } => commands.list().map(Success::List),
        Command::Workspace {
            command: WorkspaceCommand::Path { name },
        } => commands.path(name).map(Success::Path),
        Command::Workspace {
            command: WorkspaceCommand::Status { name },
        } => commands.status(name).map(Success::Status),
        Command::Workspace {
            command: WorkspaceCommand::Remove { target, force },
        } => commands.remove(target, force, now_ms).map(Success::Remove),
    };
    match result {
        Ok(success) => {
            let rendered = if cli.json {
                render_success_json(&success, stdout)
            } else {
                render_success_human(&success, stdout)
            };
            if rendered.is_ok() { 0 } else { 31 }
        }
        Err(error) => render_error(&error, cli.json, stdout, stderr),
    }
}

#[derive(Parser)]
#[command(
    name = "thinws",
    version,
    about = "Space-efficient local development workspaces"
)]
struct Cli {
    #[arg(long, global = true, help = "Emit one versioned JSON document")]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initializes one private ThinWorkspace data root.
    Init {
        /// Canonical absolute APFS directory used for ThinWorkspace data.
        #[arg(long)]
        data_root: PathBuf,
    },
    /// Validates installation and metadata state without repairing it.
    Doctor,
    /// Manages ordinary local Workspace directories.
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },
}

#[derive(Subcommand)]
enum WorkspaceCommand {
    /// Mirrors one source directory into a new ordinary Workspace path.
    Create {
        /// Canonical absolute source directory on the data-root APFS volume.
        #[arg(long)]
        source: PathBuf,
        /// Unique Workspace name.
        #[arg(long)]
        name: String,
        /// Allow Full Copy only if APFS clone is proven unavailable.
        #[arg(long)]
        allow_copy: bool,
        /// Probe current facts without allocating an ID or writing product state.
        #[arg(long)]
        dry_run: bool,
    },
    /// Lists all active Workspace records without scanning Git.
    List,
    /// Prints one verified Ready Workspace path as a single line.
    Path {
        /// Exact Workspace name.
        name: String,
    },
    /// Inspects one Workspace and its tracked Git changes when Ready.
    Status {
        /// Exact Workspace name.
        name: String,
    },
    /// Removes an exact named or ID-selected Workspace copy.
    Remove {
        /// Workspace name or full Workspace ID.
        target: String,
        /// Explicitly discard tracked changes and bypass incomplete Git checks.
        #[arg(long)]
        force: bool,
    },
}

enum Success {
    Init(InitView),
    Doctor(DoctorView),
    Create(CreateView),
    List(Vec<WorkspaceView>),
    Path(Vec<u8>),
    Status(StatusView),
    Remove(RemoveView),
}

#[derive(Clone, Copy, Default)]
struct Prescan {
    json: bool,
    help_or_version: bool,
}

fn prescan(args: &[OsString]) -> Prescan {
    let mut result = Prescan::default();
    for argument in args.iter().skip(1) {
        if argument == OsStr::new("--") {
            break;
        }
        if argument == OsStr::new("--json") {
            result.json = true;
        }
        if matches!(
            argument.as_os_str().as_bytes(),
            b"--help" | b"-h" | b"--version" | b"-V"
        ) {
            result.help_or_version = true;
        }
    }
    result
}

fn render_success_human(success: &Success, output: &mut dyn Write) -> io::Result<()> {
    match success {
        Success::Init(view) => {
            writeln!(output, "ThinWorkspace initialized")?;
            writeln!(output, "Result:      {}", view.result)?;
            output.write_all(b"Data root:   ")?;
            output.write_all(&view.data_root)?;
            output.write_all(b"\n")?;
            writeln!(output, "Volume ID:   {}", view.volume_id)
        }
        Success::Doctor(view) => {
            writeln!(output, "ThinWorkspace doctor")?;
            writeln!(output, "Status:              ready")?;
            writeln!(
                output,
                "Host:                {}/{}",
                platform_name(),
                std::env::consts::ARCH
            )?;
            output.write_all(b"Data root:           ")?;
            output.write_all(&view.data_root)?;
            output.write_all(b"\n")?;
            writeln!(
                output,
                "Incomplete workspaces: {}",
                view.incomplete_workspaces
            )?;
            writeln!(output, "Git check:           available")
        }
        Success::Create(CreateView::Ready(view)) => {
            writeln!(output, "Workspace ready")?;
            writeln!(
                output,
                "Result:          {}",
                if view.created {
                    "created"
                } else {
                    "already-ready"
                }
            )?;
            writeln!(output, "Name:            {}", view.name)?;
            writeln!(output, "Workspace ID:    {}", view.workspace_id)?;
            output.write_all(b"Source:          ")?;
            output.write_all(&view.source)?;
            output.write_all(b"\nPath:            ")?;
            output.write_all(&view.path)?;
            output.write_all(b"\n")?;
            writeln!(output, "Actual mode:     {}", view.actual_mode)?;
            writeln!(output, "CoW:             {}", view.cow)?;
            writeln!(
                output,
                "Fallback:        {}",
                view.fallback_reason.as_deref().unwrap_or("none")
            )?;
            writeln!(output, "Git setup:       not performed")
        }
        Success::Create(CreateView::Preview(view)) => {
            writeln!(output, "Workspace creation preview")?;
            writeln!(output, "Name:            {}", view.name)?;
            output.write_all(b"Source:          ")?;
            output.write_all(&view.source)?;
            output.write_all(b"\nTarget parent:   ")?;
            output.write_all(&view.target_parent)?;
            output.write_all(b"\n")?;
            writeln!(output, "Target mode:     ID-derived under target parent")?;
            writeln!(output, "Source volume:   {}", view.source_volume_id)?;
            writeln!(output, "Target volume:   {}", view.target_volume_id)?;
            writeln!(output, "Planned mode:    {}", view.effective_mode)?;
            writeln!(output, "Adapter:         {}", view.selected_adapter)?;
            writeln!(
                output,
                "Fallback:        {}",
                view.fallback_reason.as_deref().unwrap_or("none")
            )?;
            writeln!(output, "Workspace ID:    not allocated")
        }
        Success::List(workspaces) => {
            writeln!(output, "Workspaces")?;
            for workspace in workspaces {
                write!(
                    output,
                    "{}  {}  {}  ",
                    workspace.name, workspace.workspace_id, workspace.state
                )?;
                output.write_all(&workspace.source)?;
                writeln!(
                    output,
                    "  {}",
                    workspace
                        .materialization
                        .as_ref()
                        .map_or("not-ready", |facts| facts.actual_mode.as_str())
                )?;
            }
            Ok(())
        }
        Success::Path(path) => {
            output.write_all(path)?;
            output.write_all(b"\n")
        }
        Success::Status(status) => {
            let workspace = &status.workspace;
            writeln!(output, "Workspace status")?;
            writeln!(output, "Name:          {}", workspace.name)?;
            writeln!(output, "Workspace ID:  {}", workspace.workspace_id)?;
            writeln!(output, "State:         {}", workspace.state)?;
            output.write_all(b"Source:        ")?;
            output.write_all(&workspace.source)?;
            output.write_all(b"\n")?;
            if workspace.state == "ready" {
                output.write_all(b"Path:          ")?;
                output.write_all(&workspace.path)?;
                output.write_all(b"\n")?;
            }
            if let Some(git) = &status.git {
                writeln!(output, "Git:           {}", git.state)?;
                for issue in &git.issues {
                    writeln!(output, "Git issue:     {issue}")?;
                }
                for repository in &git.repositories {
                    output.write_all(b"Repository:    ")?;
                    output.write_all(&repository.relative_path)?;
                    writeln!(
                        output,
                        "  {}  {}",
                        repository.state,
                        repository
                            .tracked_changes
                            .map_or_else(|| "unknown".to_owned(), |count| count.to_string())
                    )?;
                    for issue in &repository.issues {
                        writeln!(output, "Repo issue:    {issue}")?;
                    }
                }
            } else {
                writeln!(output, "Git:           not run (Workspace not Ready)")?;
            }
            Ok(())
        }
        Success::Remove(view) => {
            writeln!(output, "Workspace removed")?;
            writeln!(output, "Result: {}", view.result)?;
            writeln!(output, "Workspace ID: {}", view.workspace_id)?;
            writeln!(output, "Forced: {}", if view.forced { "yes" } else { "no" })?;
            writeln!(output, "Operation ID: {}", view.operation_id)?;
            output.write_all(b"Log: ")?;
            output.write_all(&view.log_path)?;
            output.write_all(b"\n")?;
            if let Some(warning) = &view.warning {
                writeln!(output, "Warning: {warning}")?;
            }
            writeln!(output, "Delivery verification: not performed")
        }
    }
}

fn render_success_json(success: &Success, output: &mut dyn Write) -> io::Result<()> {
    let data = match success {
        Success::Init(view) => json!({
            "command": "init",
            "result": view.result,
            "instance_id": view.instance_id,
            "data_root": String::from_utf8_lossy(&view.data_root),
            "data_root_hex": hex(&view.data_root),
            "volume_id": view.volume_id,
        }),
        Success::Doctor(view) => json!({
            "command": "doctor",
            "status": "ready",
            "host": {
                "platform": platform_name(),
                "architecture": std::env::consts::ARCH,
            },
            "instance_id": view.instance_id,
            "data_root": String::from_utf8_lossy(&view.data_root),
            "data_root_hex": hex(&view.data_root),
            "volume_id": view.volume_id,
            "incomplete_workspaces": view.incomplete_workspaces,
            "git_check": {
                "available": true,
                "reason": Value::Null,
            },
        }),
        Success::Create(CreateView::Ready(view)) => json!({
            "command": "workspace create",
            "dry_run": false,
            "result": if view.created { "created" } else { "already-ready" },
            "workspace_id": view.workspace_id,
            "name": view.name,
            "state": "ready",
            "source": String::from_utf8_lossy(&view.source),
            "source_hex": hex(&view.source),
            "path": String::from_utf8_lossy(&view.path),
            "path_hex": hex(&view.path),
            "materialization": {
                "requested_mode": view.requested_mode,
                "effective_planned_mode": view.effective_mode,
                "actual_mode": view.actual_mode,
                "adapter": view.adapter,
                "outcome": "succeeded",
                "cow": view.cow,
                "fallback": {
                    "used": view.fallback_reason.is_some(),
                    "reason": view.fallback_reason,
                },
                "failed_attempt_count": view.failed_attempt_count,
            },
        }),
        Success::Create(CreateView::Preview(view)) => json!({
            "command": "workspace create",
            "dry_run": true,
            "workspace_id": Value::Null,
            "name": view.name,
            "source": String::from_utf8_lossy(&view.source),
            "source_hex": hex(&view.source),
            "target_parent": String::from_utf8_lossy(&view.target_parent),
            "target_parent_hex": hex(&view.target_parent),
            "target_path_mode": "id-derived-under-target-parent",
            "source_volume_id": view.source_volume_id,
            "target_volume_id": view.target_volume_id,
            "same_volume": view.source_volume_id == view.target_volume_id,
            "materialization": {
                "requested_mode": "cow-clone",
                "effective_planned_mode": view.effective_mode,
                "adapter": view.selected_adapter,
                "fallback": {
                    "used": view.fallback_reason.is_some(),
                    "reason": view.fallback_reason,
                },
            },
        }),
        Success::List(workspaces) => json!({
            "command": "workspace list",
            "workspaces": workspaces.iter().map(list_workspace_json).collect::<Vec<_>>(),
        }),
        Success::Path(_) => return Err(io::Error::other("workspace path has no JSON output")),
        Success::Status(status) => {
            let mut data = workspace_json(&status.workspace);
            let object = data
                .as_object_mut()
                .expect("workspace JSON is always an object");
            object.insert("command".to_owned(), json!("workspace status"));
            let git = if let Some(git) = &status.git {
                json!({
                    "scan_complete": git.scan_complete,
                    "state": git.state,
                    "issues": git.issues,
                    "repositories": git.repositories.iter().map(|repository| json!({
                        "relative_path": String::from_utf8_lossy(&repository.relative_path),
                        "relative_path_hex": hex(&repository.relative_path),
                        "state": repository.state,
                        "tracked_changes": repository.tracked_changes,
                        "issues": repository.issues,
                    })).collect::<Vec<_>>(),
                })
            } else {
                json!({
                    "scan_complete": false,
                    "state": "unknown",
                    "issues": ["workspace-not-ready"],
                    "repositories": [],
                })
            };
            object.insert("git".to_owned(), git);
            data
        }
        Success::Remove(view) => json!({
            "command": "workspace remove",
            "workspace_id": view.workspace_id,
            "operation_id": view.operation_id,
            "forced": view.forced,
            "result": view.result,
            "log": String::from_utf8_lossy(&view.log_path),
            "log_hex": hex(&view.log_path),
            "warning": view.warning,
            "delivery_verification": "not-performed",
        }),
    };
    serde_json::to_writer(
        &mut *output,
        &json!({
            "schema_version": 1,
            "ok": true,
            "data": data,
        }),
    )?;
    output.write_all(b"\n")
}

fn workspace_json(view: &WorkspaceView) -> Value {
    let materialization = view.materialization.as_ref().map(|facts| {
        json!({
            "requested_mode": facts.requested_mode,
            "effective_planned_mode": facts.effective_mode,
            "actual_mode": facts.actual_mode,
            "adapter": facts.adapter,
            "outcome": "succeeded",
            "cow": facts.cow,
            "fallback": {
                "used": facts.fallback_reason.is_some(),
                "reason": facts.fallback_reason,
            },
            "failed_attempt_count": facts.failed_attempt_count,
        })
    });
    let ready_path = (view.state == "ready").then(|| String::from_utf8_lossy(&view.path));
    let ready_path_hex = (view.state == "ready").then(|| hex(&view.path));
    json!({
        "workspace_id": view.workspace_id,
        "name": view.name,
        "state": view.state,
        "source": String::from_utf8_lossy(&view.source),
        "source_hex": hex(&view.source),
        "path": ready_path,
        "path_hex": ready_path_hex,
        "last_error_code": view.last_error_code,
        "materialization": materialization,
    })
}

fn list_workspace_json(view: &WorkspaceView) -> Value {
    let mut value = workspace_json(view);
    let object = value
        .as_object_mut()
        .expect("workspace JSON is always an object");
    // Listing does not revalidate Ready path ownership; only path/status may publish it.
    object.remove("path");
    object.remove("path_hex");
    value
}

fn render_error(
    error: &ErrorView,
    json_mode: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let rendered = if json_mode {
        let document = json!({
            "schema_version": 1,
            "ok": false,
            "error": {
                "code": error.code,
                "message": error.message,
                "context": error.context,
                "remediation": error.remediation,
            },
        });
        serde_json::to_writer(&mut *stdout, &document)
            .map_err(io::Error::other)
            .and_then(|()| stdout.write_all(b"\n"))
    } else {
        writeln!(stderr, "Error: {}", error.message)
            .and_then(|()| writeln!(stderr, "Code: {}", error.code))
            .and_then(|()| {
                if let Some(id) = error.context.get("workspace_id").and_then(Value::as_str) {
                    writeln!(stderr, "Workspace ID: {id}")
                } else {
                    Ok(())
                }
            })
            .and_then(|()| {
                if let Some(issues) = error.context.get("issues").and_then(Value::as_array) {
                    for issue in issues.iter().filter_map(Value::as_str) {
                        writeln!(stderr, "Git issue: {issue}")?;
                    }
                }
                if let Some(repositories) =
                    error.context.get("repositories").and_then(Value::as_array)
                {
                    for repository in repositories {
                        if let Some(path) = repository.get("relative_path").and_then(Value::as_str)
                        {
                            writeln!(stderr, "Repository: {path}")?;
                        }
                        if let Some(count) =
                            repository.get("tracked_changes").and_then(Value::as_u64)
                        {
                            writeln!(stderr, "Tracked changes: {count}")?;
                        }
                        if let Some(issues) = repository.get("issues").and_then(Value::as_array) {
                            for issue in issues.iter().filter_map(Value::as_str) {
                                writeln!(stderr, "Repo issue: {issue}")?;
                            }
                        }
                    }
                }
                Ok(())
            })
            .and_then(|()| {
                if let Some(remediation) = &error.remediation {
                    writeln!(stderr, "{remediation}")
                } else {
                    Ok(())
                }
            })
            .and_then(|()| {
                if matches!(
                    error.code.as_str(),
                    "E_WORKSPACE_DIRTY"
                        | "E_GIT_CHECK_INCOMPLETE"
                        | "E_WORKSPACE_BUSY"
                        | "E_WORKSPACE_INCOMPLETE"
                ) {
                    writeln!(stderr, "No files were removed.")
                } else {
                    Ok(())
                }
            })
    };
    if rendered.is_err() {
        31
    } else {
        exit_status(&error.code)
    }
}

fn usage_error(message: &str) -> ErrorView {
    ErrorView {
        code: "E_USAGE".to_owned(),
        message: message.to_owned(),
        context: Map::new(),
        remediation: None,
    }
}

fn exit_status(code: &str) -> i32 {
    BTreeMap::from([
        ("E_USAGE", 2),
        ("E_NOT_INITIALIZED", 10),
        ("E_CAPABILITY_UNAVAILABLE", 11),
        ("E_COW_UNAVAILABLE", 12),
        ("E_NAME_CONFLICT", 15),
        ("E_DATA_ROOT_CHANGE_UNSUPPORTED", 16),
        ("E_WORKSPACE_NOT_FOUND", 20),
        ("E_WORKSPACE_NOT_READY", 21),
        ("E_WORKSPACE_DIRTY", 22),
        ("E_WORKSPACE_BUSY", 23),
        ("E_GIT_CHECK_INCOMPLETE", 25),
        ("E_GIT", 30),
        ("E_FILESYSTEM", 31),
        ("E_DATA_ROOT_UNAVAILABLE", 32),
        ("E_DATA_ROOT_LAYOUT", 33),
        ("E_METADATA", 35),
        ("E_DATA_ROOT_NOT_EMPTY", 36),
        ("E_WORKSPACE_INCOMPLETE", 40),
        ("E_LOCK_TIMEOUT", 41),
    ])
    .get(code)
    .copied()
    .unwrap_or(31)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

const fn platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else {
        std::env::consts::OS
    }
}

fn use_case_error_view(error: thinws_application::UseCaseError) -> ErrorView {
    let diagnostic = error.diagnostic();
    let mut context: Map<String, Value> = diagnostic
        .context()
        .iter()
        .map(|(key, value)| {
            (
                (*key).to_owned(),
                Value::String(value.user_value().to_owned()),
            )
        })
        .collect();
    if let Some(inspection) = error.git_inspection() {
        context.insert(
            "issues".to_owned(),
            json!(
                inspection
                    .issues()
                    .iter()
                    .map(git_issue_name)
                    .collect::<Vec<_>>()
            ),
        );
        context.insert(
            "repositories".to_owned(),
            json!(
                inspection
                    .repositories()
                    .iter()
                    .map(|repository| {
                        let bytes = repository.relative_path().as_os_str().as_bytes();
                        let bytes = if bytes.is_empty() {
                            b".".as_slice()
                        } else {
                            bytes
                        };
                        let display = match std::str::from_utf8(bytes) {
                            Ok(value) if !value.chars().any(char::is_control) => value.to_owned(),
                            _ => format!("hex:{}", hex(bytes)),
                        };
                        json!({
                            "relative_path": display,
                            "relative_path_hex": hex(bytes),
                            "state": repository_state_name(repository.state()),
                            "tracked_changes": repository.state().tracked_change_count(),
                            "issues": repository.issues().iter().map(git_issue_name).collect::<Vec<_>>(),
                        })
                    })
                    .collect::<Vec<_>>()
            ),
        );
    }
    ErrorView {
        code: diagnostic.code().to_string(),
        message: diagnostic.message().to_owned(),
        context,
        remediation: diagnostic.remediation().map(str::to_owned),
    }
}

const fn mode_name(mode: MaterializationMode) -> &'static str {
    match mode {
        MaterializationMode::CowClone => "cow-clone",
        MaterializationMode::FullCopy => "full-copy",
    }
}

const fn adapter_name(adapter: MaterializerKind) -> &'static str {
    match adapter {
        MaterializerKind::ApfsFileClone => "apfs-file-clone",
        MaterializerKind::FullCopy => "full-copy",
    }
}

const fn cow_name(cow: CowEvidence) -> &'static str {
    match cow {
        CowEvidence::Confirmed => "confirmed",
        CowEvidence::NotUsed => "not-used",
        CowEvidence::Unknown => "unknown",
    }
}

const fn fallback_name(reason: FallbackReason) -> &'static str {
    match reason {
        FallbackReason::CloneUnsupportedAtPreflight => "clone-unsupported-at-preflight",
        FallbackReason::CloneUnavailableAtRuntime => "clone-unavailable-at-runtime",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_renderers_keep_raw_path_bytes_and_lossless_hex() {
        let path = b"/data-\xff/workspaces/ws_1/root".to_vec();
        let mut plain = Vec::new();
        render_success_human(&Success::Path(path.clone()), &mut plain).unwrap();
        assert_eq!(plain, [path.as_slice(), b"\n"].concat());

        let mut encoded = Vec::new();
        render_success_json(
            &Success::Status(StatusView {
                workspace: WorkspaceView {
                    workspace_id: "ws_1".to_owned(),
                    name: "one".to_owned(),
                    state: "ready".to_owned(),
                    source: b"/source-\xfe".to_vec(),
                    path,
                    last_error_code: None,
                    materialization: Some(MaterializationView {
                        requested_mode: "cow-clone".to_owned(),
                        effective_mode: "cow-clone".to_owned(),
                        actual_mode: "cow-clone".to_owned(),
                        adapter: "apfs-file-clone".to_owned(),
                        cow: "confirmed".to_owned(),
                        fallback_reason: None,
                        failed_attempt_count: 0,
                    }),
                },
                git: Some(GitView {
                    scan_complete: true,
                    state: "not-applicable".to_owned(),
                    issues: Vec::new(),
                    repositories: Vec::new(),
                }),
            }),
            &mut encoded,
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            value["data"]["path_hex"],
            "2f646174612dff2f776f726b7370616365732f77735f312f726f6f74"
        );
        assert_eq!(value["data"]["source_hex"], "2f736f757263652dfe");
        assert!(value["data"]["path"].as_str().unwrap().contains('�'));
    }
}

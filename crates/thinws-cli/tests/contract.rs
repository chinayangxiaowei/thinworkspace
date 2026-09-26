use std::ffi::OsString;

use serde_json::Value;
use thinws_cli::{
    Commands, CreatePreviewView, CreateReadyView, CreateView, DoctorView, ErrorView, GitView,
    InitView, LocalCommands, MaterializationView, RepositoryView, StatusView, WorkspaceView, run,
};

struct FakeCommands;

fn fixture_workspace_view() -> WorkspaceView {
    WorkspaceView {
        workspace_id: "ws_019a0000-0000-7000-8000-000000000001".to_owned(),
        name: "one".to_owned(),
        state: "ready".to_owned(),
        source: b"/Volumes/data/source".to_vec(),
        path: b"/Volumes/data/thinws-data/workspaces/ws_019a0000-0000-7000-8000-000000000001/root"
            .to_vec(),
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
    }
}

impl Commands for FakeCommands {
    fn init(&self, data_root: Vec<u8>, _now_ms: i64) -> Result<InitView, ErrorView> {
        Ok(InitView {
            result: "initialized".to_owned(),
            instance_id: "01890a5d-ac96-774b-bd5b-55c7b8d09f33".to_owned(),
            data_root,
            volume_id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
        })
    }

    fn doctor(&self) -> Result<DoctorView, ErrorView> {
        Ok(DoctorView {
            instance_id: "01890a5d-ac96-774b-bd5b-55c7b8d09f33".to_owned(),
            data_root: b"/Volumes/data/thinws-data".to_vec(),
            volume_id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
            incomplete_workspaces: 2,
        })
    }

    fn create(
        &self,
        source: Vec<u8>,
        name: String,
        _allow_copy: bool,
        dry_run: bool,
        _now_ms: i64,
    ) -> Result<CreateView, ErrorView> {
        if dry_run {
            Ok(CreateView::Preview(CreatePreviewView {
                name,
                source,
                target_parent: b"/Volumes/data/thinws-data/workspaces".to_vec(),
                source_volume_id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
                target_volume_id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
                effective_mode: "cow-clone".to_owned(),
                selected_adapter: "apfs-file-clone".to_owned(),
                fallback_reason: None,
            }))
        } else {
            Ok(CreateView::Ready(CreateReadyView {
                workspace_id: "ws_019a0000-0000-7000-8000-000000000001".to_owned(),
                name,
                source,
                path: b"/Volumes/data/thinws-data/workspaces/ws_019a0000-0000-7000-8000-000000000001/root".to_vec(),
                created: true,
                requested_mode: "cow-clone".to_owned(),
                effective_mode: "cow-clone".to_owned(),
                actual_mode: "cow-clone".to_owned(),
                adapter: "apfs-file-clone".to_owned(),
                cow: "confirmed".to_owned(),
                fallback_reason: None,
                failed_attempt_count: 0,
            }))
        }
    }

    fn list(&self) -> Result<Vec<WorkspaceView>, ErrorView> {
        Ok(vec![fixture_workspace_view()])
    }

    fn path(&self, _name: String) -> Result<Vec<u8>, ErrorView> {
        Ok(fixture_workspace_view().path)
    }

    fn status(&self, _name: String) -> Result<StatusView, ErrorView> {
        Ok(StatusView {
            workspace: fixture_workspace_view(),
            git: Some(GitView {
                scan_complete: true,
                state: "clean".to_owned(),
                issues: Vec::new(),
                repositories: vec![RepositoryView {
                    relative_path: b".".to_vec(),
                    state: "clean".to_owned(),
                    tracked_changes: Some(0),
                    issues: Vec::new(),
                }],
            }),
        })
    }
}

fn execute(args: &[&str]) -> (i32, Vec<u8>, Vec<u8>) {
    execute_with(&FakeCommands, args)
}

fn execute_with(commands: &impl Commands, args: &[&str]) -> (i32, Vec<u8>, Vec<u8>) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let status = run(
        args.iter().map(OsString::from),
        commands,
        1_700_000_000_000,
        &mut stdout,
        &mut stderr,
    );
    (status, stdout, stderr)
}

struct FailingCommands(&'static str);

impl Commands for FailingCommands {
    fn init(&self, _data_root: Vec<u8>, _now_ms: i64) -> Result<InitView, ErrorView> {
        Err(error(self.0))
    }

    fn doctor(&self) -> Result<DoctorView, ErrorView> {
        Err(error(self.0))
    }

    fn create(
        &self,
        _source: Vec<u8>,
        _name: String,
        _allow_copy: bool,
        _dry_run: bool,
        _now_ms: i64,
    ) -> Result<CreateView, ErrorView> {
        Err(error(self.0))
    }

    fn list(&self) -> Result<Vec<WorkspaceView>, ErrorView> {
        Err(error(self.0))
    }

    fn path(&self, _name: String) -> Result<Vec<u8>, ErrorView> {
        Err(error(self.0))
    }

    fn status(&self, _name: String) -> Result<StatusView, ErrorView> {
        Err(error(self.0))
    }
}

fn error(code: &str) -> ErrorView {
    ErrorView {
        code: code.to_owned(),
        message: "failure".to_owned(),
        context: Vec::new(),
        remediation: None,
    }
}

// Concrete path validation is exercised through LocalCommands in E2E tests.

#[test]
fn init_human_and_json_outputs_match_the_frozen_contract() {
    let (status, stdout, stderr) =
        execute(&["thinws", "init", "--data-root", "/Volumes/data/thinws-data"]);
    assert_eq!(status, 0);
    assert!(stderr.is_empty());
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "ThinWorkspace initialized\nResult:      initialized\nData root:   /Volumes/data/thinws-data\nVolume ID:   550e8400-e29b-41d4-a716-446655440000\n"
    );

    let (status, stdout, stderr) = execute(&[
        "thinws",
        "--json",
        "init",
        "--data-root",
        "/Volumes/data/thinws-data",
    ]);
    assert_eq!(status, 0);
    assert!(stderr.is_empty());
    let json: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["command"], "init");
    assert_eq!(json["data"]["result"], "initialized");
    assert_eq!(json["data"]["data_root"], "/Volumes/data/thinws-data");
    assert_eq!(
        json["data"]["data_root_hex"],
        "2f566f6c756d65732f646174612f7468696e77732d64617461"
    );
}

#[test]
fn doctor_human_and_json_outputs_expose_current_git_status_capability() {
    let (status, stdout, stderr) = execute(&["thinws", "doctor"]);
    assert_eq!(status, 0);
    assert!(stderr.is_empty());
    let human = String::from_utf8(stdout).unwrap();
    assert!(human.contains("ThinWorkspace doctor\n"));
    assert!(human.contains("Status:              ready\n"));
    assert!(human.contains("Incomplete workspaces: 2\n"));
    assert!(human.contains("Git check:           available\n"));

    let (status, stdout, stderr) = execute(&["thinws", "--json", "doctor"]);
    assert_eq!(status, 0);
    assert!(stderr.is_empty());
    let json: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(json["data"]["command"], "doctor");
    assert_eq!(json["data"]["status"], "ready");
    assert_eq!(json["data"]["incomplete_workspaces"], 2);
    assert_eq!(json["data"]["git_check"]["available"], true);
    assert_eq!(json["data"]["git_check"]["reason"], Value::Null);
}

#[test]
fn json_usage_is_an_envelope_help_is_mutually_exclusive_and_double_dash_stops_prescan() {
    let (status, stdout, stderr) = execute(&["thinws", "--json", "unknown"]);
    assert_eq!(status, 2);
    assert!(stderr.is_empty());
    let json: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "E_USAGE");

    let (status, stdout, stderr) = execute(&["thinws", "--json", "--help"]);
    assert_eq!(status, 2);
    assert!(stderr.is_empty());
    let json: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(json["error"]["code"], "E_USAGE");

    let (status, stdout, stderr) = execute(&["thinws", "doctor", "--", "--json"]);
    assert_eq!(status, 2);
    assert!(stdout.is_empty());
    assert!(!stderr.is_empty());
}

#[test]
fn human_usage_errors_use_the_same_stable_error_renderer() {
    let (status, stdout, stderr) = execute(&["thinws", "unknown"]);
    assert_eq!(status, 2);
    assert!(stdout.is_empty());
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: invalid command or arguments\nCode: E_USAGE\n"
    );
}

#[test]
fn p1_03_public_errors_keep_their_frozen_exit_statuses() {
    for (code, expected_status) in [
        ("E_NOT_INITIALIZED", 10),
        ("E_CAPABILITY_UNAVAILABLE", 11),
        ("E_DATA_ROOT_CHANGE_UNSUPPORTED", 16),
        ("E_FILESYSTEM", 31),
        ("E_DATA_ROOT_UNAVAILABLE", 32),
        ("E_DATA_ROOT_LAYOUT", 33),
        ("E_METADATA", 35),
        ("E_DATA_ROOT_NOT_EMPTY", 36),
        ("E_LOCK_TIMEOUT", 41),
    ] {
        let commands = FailingCommands(code);
        let (status, stdout, stderr) = execute_with(&commands, &["thinws", "--json", "doctor"]);
        assert_eq!(status, expected_status, "wrong status for {code}");
        assert!(stderr.is_empty());
        let json: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(json["error"]["code"], code);
    }
}

#[test]
fn application_command_adapter_rejects_noncanonical_path_before_the_use_case() {
    let commands = LocalCommands::new(None);
    let error = commands.init(b"relative/path".to_vec(), 1).unwrap_err();
    assert_eq!(error.code, "E_USAGE");
    let error = commands
        .create(b"relative/path".to_vec(), "one".to_owned(), false, false, 1)
        .unwrap_err();
    assert_eq!(error.code, "E_USAGE");
}

#[test]
fn workspace_create_and_preview_render_distinct_execution_facts() {
    let (status, stdout, stderr) = execute(&[
        "thinws",
        "workspace",
        "create",
        "--source",
        "/Volumes/data/source",
        "--name",
        "one",
    ]);
    assert_eq!(status, 0);
    assert!(stderr.is_empty());
    let human = String::from_utf8(stdout).unwrap();
    assert!(human.contains("Workspace ready\n"));
    assert!(human.contains("Actual mode:     cow-clone\n"));
    assert!(human.contains("CoW:             confirmed\n"));
    assert!(human.contains("Git setup:       not performed\n"));

    let (status, stdout, stderr) = execute(&[
        "thinws",
        "--json",
        "workspace",
        "create",
        "--source",
        "/Volumes/data/source",
        "--name",
        "one",
        "--dry-run",
    ]);
    assert_eq!(status, 0);
    assert!(stderr.is_empty());
    let preview: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(preview["data"]["workspace_id"], Value::Null);
    assert_eq!(preview["data"]["dry_run"], true);
    assert_eq!(
        preview["data"]["materialization"]["effective_planned_mode"],
        "cow-clone"
    );
    assert!(
        preview["data"]["materialization"]
            .get("actual_mode")
            .is_none()
    );
}

#[test]
fn workspace_query_commands_preserve_path_stdout_and_json_shapes() {
    let (code, stdout, stderr) = execute(&["thinws", "--json", "workspace", "list"]);
    assert_eq!(code, 0);
    assert!(stderr.is_empty());
    let listed: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(listed["data"]["command"], "workspace list");
    assert_eq!(listed["data"]["workspaces"][0]["name"], "one");
    assert_eq!(
        listed["data"]["workspaces"][0]["materialization"]["actual_mode"],
        "cow-clone"
    );
    assert!(listed["data"]["workspaces"][0].get("git").is_none());
    assert!(listed["data"]["workspaces"][0].get("path").is_none());

    let (code, stdout, stderr) = execute(&["thinws", "workspace", "path", "one"]);
    assert_eq!(code, 0);
    assert!(stderr.is_empty());
    assert_eq!(
        stdout,
        [fixture_workspace_view().path, b"\n".to_vec()].concat()
    );

    let (code, stdout, stderr) = execute(&["thinws", "--json", "workspace", "path", "one"]);
    assert_eq!(code, 2);
    assert!(stderr.is_empty());
    let rejected: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(rejected["error"]["code"], "E_USAGE");

    let (code, stdout, stderr) = execute(&["thinws", "--json", "workspace", "status", "one"]);
    assert_eq!(code, 0);
    assert!(stderr.is_empty());
    let status: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(status["data"]["command"], "workspace status");
    assert_eq!(status["data"]["state"], "ready");
    assert_eq!(status["data"]["git"]["scan_complete"], true);
    assert_eq!(status["data"]["git"]["state"], "clean");
    assert_eq!(
        status["data"]["git"]["repositories"][0]["relative_path"],
        "."
    );
    assert_eq!(
        status["data"]["git"]["repositories"][0]["tracked_changes"],
        0
    );
}

#[test]
fn workspace_help_lists_the_query_commands() {
    let (code, stdout, stderr) = execute(&["thinws", "workspace", "--help"]);
    assert_eq!(code, 0);
    assert!(stderr.is_empty());
    let help = String::from_utf8(stdout).unwrap();
    assert!(help.contains("list"));
    assert!(help.contains("path"));
    assert!(help.contains("status"));
}

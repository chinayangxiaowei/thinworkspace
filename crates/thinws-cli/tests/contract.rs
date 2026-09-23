use std::ffi::OsString;

use serde_json::Value;
use thinws_application::{DoctorOutcome, InitOutcome, InitRequest, UseCaseError, UseCases};
use thinws_cli::{ApplicationCommands, Commands, DoctorView, ErrorView, InitView, run};

struct FakeCommands;

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
}

fn error(code: &str) -> ErrorView {
    ErrorView {
        code: code.to_owned(),
        message: "failure".to_owned(),
        context: Vec::new(),
        remediation: None,
    }
}

struct PanicUseCases;

impl UseCases for PanicUseCases {
    fn init(&self, _request: InitRequest) -> Result<InitOutcome, UseCaseError> {
        panic!("invalid CLI input must not reach Application orchestration")
    }

    fn doctor(&self) -> Result<DoctorOutcome, UseCaseError> {
        panic!("not used by this contract test")
    }
}

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
fn doctor_human_and_json_outputs_expose_current_p1_03_capabilities() {
    let (status, stdout, stderr) = execute(&["thinws", "doctor"]);
    assert_eq!(status, 0);
    assert!(stderr.is_empty());
    let human = String::from_utf8(stdout).unwrap();
    assert!(human.contains("ThinWorkspace doctor\n"));
    assert!(human.contains("Status:              ready\n"));
    assert!(human.contains("Incomplete workspaces: 2\n"));
    assert!(human.contains("Git check:           unavailable (not implemented)\n"));

    let (status, stdout, stderr) = execute(&["thinws", "--json", "doctor"]);
    assert_eq!(status, 0);
    assert!(stderr.is_empty());
    let json: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(json["data"]["command"], "doctor");
    assert_eq!(json["data"]["status"], "ready");
    assert_eq!(json["data"]["incomplete_workspaces"], 2);
    assert_eq!(json["data"]["git_check"]["available"], false);
    assert_eq!(json["data"]["git_check"]["reason"], "not-implemented");
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
    let commands = ApplicationCommands::new(PanicUseCases);
    let error = commands.init(b"relative/path".to_vec(), 1).unwrap_err();
    assert_eq!(error.code, "E_USAGE");
}

use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use rusqlite::{Connection, params};
use serde_json::Value;
use tempfile::{Builder, TempDir};
use thinws_cli::{LocalCommands, run};

const CHILD_BOOTSTRAP: &str = "THINWS_P1_10_CHILD_BOOTSTRAP";
const CHILD_EXPECTED_GIT: &str = "THINWS_P1_10_CHILD_EXPECTED_GIT";

fn fixture_git(cwd: &Path, home: &Path, args: &[&str]) {
    let output = Command::new("/usr/bin/git")
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("fixture Git command");
    assert!(
        output.status.success(),
        "fixture Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn initialize_git_repo(path: &Path, home: &Path) {
    fixture_git(path, home, &["init", "--quiet"]);
    fixture_git(path, home, &["config", "user.name", "Fixture"]);
    fixture_git(
        path,
        home,
        &["config", "user.email", "fixture@example.invalid"],
    );
    fixture_git(path, home, &["add", "tracked.txt"]);
    fixture_git(path, home, &["commit", "--quiet", "-m", "baseline"]);
}

fn isolated_status(bootstrap: &Path, home: &Path, expected: &str) {
    let output = Command::new(std::env::current_exe().expect("integration test executable"))
        .args(["--exact", "isolated_status_child", "--nocapture"])
        .env_clear()
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env(CHILD_BOOTSTRAP, bootstrap)
        .env(CHILD_EXPECTED_GIT, expected)
        .output()
        .expect("isolated status child");
    assert!(
        output.status.success(),
        "status child failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn fixture() -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-10-cli-tests");
    fs::create_dir_all(&controlled).expect("controlled test parent");
    let temp = Builder::new()
        .prefix("cli-query-")
        .tempdir_in(fs::canonicalize(controlled).expect("canonical test parent"))
        .expect("private APFS fixture");
    let bootstrap = temp.path().join("bootstrap");
    let data_root = temp.path().join("data-root");
    let source = temp.path().join("source");
    fs::create_dir(&source).expect("source directory");
    fs::write(source.join("tracked.txt"), b"fixture\n").expect("source content");
    (temp, bootstrap, data_root, source)
}

fn execute(bootstrap: &Path, args: Vec<OsString>) -> (i32, Vec<u8>, Vec<u8>) {
    let commands = LocalCommands::new(Some(bootstrap.to_path_buf()))
        .with_timeouts(Duration::from_secs(1), Duration::from_secs(1));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = run(args, &commands, 1_700_000_000_000, &mut stdout, &mut stderr);
    (code, stdout, stderr)
}

fn json(bootstrap: &Path, args: Vec<OsString>) -> (i32, Value) {
    let (code, stdout, stderr) = execute(bootstrap, args);
    assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
    (
        code,
        serde_json::from_slice(&stdout).expect("one JSON envelope"),
    )
}

fn initialize(bootstrap: &Path, data_root: &Path) {
    let (code, result) = json(
        bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "init".into(),
            "--data-root".into(),
            data_root.as_os_str().to_owned(),
        ],
    );
    assert_eq!(code, 0, "{result}");
}

fn create(bootstrap: &Path, source: &Path, name: &str) -> Value {
    let (code, result) = json(
        bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "create".into(),
            "--source".into(),
            source.as_os_str().to_owned(),
            "--name".into(),
            name.into(),
        ],
    );
    assert_eq!(code, 0, "{result}");
    result
}

#[test]
fn list_returns_active_workspaces_in_name_order_without_git_results() {
    let (_temp, bootstrap, data_root, source) = fixture();
    initialize(&bootstrap, &data_root);
    let (code, empty) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "list".into(),
        ],
    );
    assert_eq!(code, 0, "{empty}");
    assert!(empty["data"]["workspaces"].as_array().unwrap().is_empty());
    let zeta = create(&bootstrap, &source, "zeta");
    let alpha = create(&bootstrap, &source, "alpha");

    let (code, result) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "list".into(),
        ],
    );
    assert_eq!(code, 0, "{result}");
    let workspaces = result["data"]["workspaces"]
        .as_array()
        .expect("workspace list array");
    assert_eq!(workspaces.len(), 2);
    assert_eq!(workspaces[0]["name"], "alpha");
    assert_eq!(workspaces[1]["name"], "zeta");
    assert_eq!(workspaces[0]["workspace_id"], alpha["data"]["workspace_id"]);
    assert_eq!(workspaces[1]["workspace_id"], zeta["data"]["workspace_id"]);
    assert_eq!(workspaces[0]["state"], "ready");
    assert_eq!(workspaces[0]["source"], alpha["data"]["source"]);
    assert_eq!(workspaces[0]["materialization"]["actual_mode"], "cow-clone");
    assert!(workspaces[0].get("git").is_none());
}

#[test]
fn path_is_a_single_ordinary_path_line_and_rejects_json() {
    let (_temp, bootstrap, data_root, source) = fixture();
    initialize(&bootstrap, &data_root);
    let created = create(&bootstrap, &source, "plain");
    let expected = created["data"]["path"].as_str().expect("created path");

    let (code, stdout, stderr) = execute(
        &bootstrap,
        vec![
            "thinws".into(),
            "workspace".into(),
            "path".into(),
            "plain".into(),
        ],
    );
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert!(stderr.is_empty());
    assert_eq!(stdout, format!("{expected}\n").as_bytes());

    let (code, error) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "path".into(),
            "plain".into(),
        ],
    );
    assert_eq!(code, 2, "{error}");
    assert_eq!(error["error"]["code"], "E_USAGE");

    let (code, status) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "plain".into(),
        ],
    );
    assert_eq!(code, 0, "{status}");
    assert_eq!(status["data"]["git"]["state"], "not-applicable");
    assert_eq!(status["data"]["git"]["scan_complete"], true);
    assert_eq!(
        status["data"]["git"]["repositories"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn status_reports_real_tracked_changes_without_counting_untracked_files() {
    let (temp, bootstrap, data_root, source) = fixture();
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("isolated Git home");
    initialize_git_repo(&source, &home);
    initialize(&bootstrap, &data_root);
    let created = create(&bootstrap, &source, "git-copy");
    let copy = PathBuf::from(created["data"]["path"].as_str().expect("copy path"));

    isolated_status(&bootstrap, &home, "clean");
    fs::write(copy.join("untracked.txt"), b"untracked\n").expect("untracked fixture");
    isolated_status(&bootstrap, &home, "clean");
    fs::write(copy.join("tracked.txt"), b"changed\n").expect("tracked change");
    isolated_status(&bootstrap, &home, "dirty");

    let config = copy.join(".git/config");
    let mut contents = fs::read(&config).expect("read copied Git config");
    contents.extend_from_slice(b"\n[core]\n\thooksPath = /tmp\n");
    fs::write(&config, &contents).expect("unsupported Git configuration");
    isolated_status(&bootstrap, &home, "unknown");
    assert_eq!(fs::read(&config).unwrap(), contents);
}

#[test]
fn status_discovers_nested_repositories_without_treating_them_as_dirty() {
    let (temp, bootstrap, data_root, source) = fixture();
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("isolated Git home");
    initialize_git_repo(&source, &home);
    let nested = source.join("nested");
    fs::create_dir(&nested).expect("nested source");
    fs::write(nested.join("tracked.txt"), b"child\n").expect("nested tracked file");
    initialize_git_repo(&nested, &home);
    initialize(&bootstrap, &data_root);
    create(&bootstrap, &source, "git-copy");

    isolated_status(&bootstrap, &home, "nested");
}

#[test]
fn isolated_status_child() {
    let Some(bootstrap) = std::env::var_os(CHILD_BOOTSTRAP) else {
        return;
    };
    let expected = std::env::var(CHILD_EXPECTED_GIT).expect("expected Git state");
    let (code, result) = json(
        Path::new(&bootstrap),
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "git-copy".into(),
        ],
    );
    assert_eq!(code, 0, "{result}");
    assert_eq!(result["data"]["state"], "ready");
    assert_eq!(result["data"]["git"]["scan_complete"], true);
    assert_eq!(
        result["data"]["git"]["state"],
        if expected == "nested" {
            "clean"
        } else {
            expected.as_str()
        }
    );
    assert_eq!(
        result["data"]["git"]["repositories"][0]["relative_path"],
        "."
    );
    let repository = &result["data"]["git"]["repositories"][0];
    match expected.as_str() {
        "nested" => {
            assert_eq!(repository["state"], "clean");
            assert_eq!(repository["tracked_changes"], 0);
            assert_eq!(
                result["data"]["git"]["repositories"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert_eq!(
                result["data"]["git"]["repositories"][1]["relative_path"],
                "nested"
            );
            assert_eq!(
                result["data"]["git"]["repositories"][1]["tracked_changes"],
                0
            );
        }
        "clean" => {
            assert_eq!(repository["state"], "clean");
            assert_eq!(repository["tracked_changes"], 0);
        }
        "dirty" => {
            assert_eq!(repository["state"], "dirty");
            assert_eq!(repository["tracked_changes"], 1);
        }
        "unknown" => {
            assert_eq!(repository["state"], "unknown");
            assert_eq!(repository["tracked_changes"], Value::Null);
            assert_eq!(repository["issues"][0], "unsupported-configuration");
        }
        other => panic!("unexpected test expectation: {other}"),
    }
    if expected == "unknown" {
        let (code, stdout, stderr) = execute(
            Path::new(&bootstrap),
            vec![
                "thinws".into(),
                "workspace".into(),
                "status".into(),
                "git-copy".into(),
            ],
        );
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&stderr));
        assert!(stderr.is_empty());
        assert!(String::from_utf8_lossy(&stdout).contains("unsupported-configuration"));
    }
}

#[test]
fn path_and_status_refuse_replaced_ready_root_and_missing_names() {
    let (temp, bootstrap, data_root, source) = fixture();
    initialize(&bootstrap, &data_root);
    let created = create(&bootstrap, &source, "protected");
    let copy = PathBuf::from(created["data"]["path"].as_str().expect("copy path"));

    let (code, stdout, stderr) = execute(
        &bootstrap,
        vec![
            "thinws".into(),
            "workspace".into(),
            "path".into(),
            "absent".into(),
        ],
    );
    assert_eq!(code, 20);
    assert!(stdout.is_empty());
    assert!(String::from_utf8_lossy(&stderr).contains("E_WORKSPACE_NOT_FOUND"));
    let (code, missing) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "absent".into(),
        ],
    );
    assert_eq!(code, 20, "{missing}");
    assert_eq!(missing["error"]["code"], "E_WORKSPACE_NOT_FOUND");

    let moved = copy.with_file_name("root-moved");
    fs::rename(&copy, &moved).expect("move controlled root entry");
    symlink(temp.path(), &copy).expect("replace root with outside symlink");
    let (code, stdout, stderr) = execute(
        &bootstrap,
        vec![
            "thinws".into(),
            "workspace".into(),
            "path".into(),
            "protected".into(),
        ],
    );
    assert_eq!(code, 33, "{}", String::from_utf8_lossy(&stderr));
    assert!(stdout.is_empty());
    let (code, refused) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "protected".into(),
        ],
    );
    assert_eq!(code, 33, "{refused}");
    assert_eq!(refused["error"]["code"], "E_DATA_ROOT_LAYOUT");
}

#[test]
fn non_ready_status_is_diagnostic_and_path_is_rejected() {
    let (_temp, bootstrap, data_root, source) = fixture();
    initialize(&bootstrap, &data_root);
    let created = create(&bootstrap, &source, "failed-later");
    let workspace_id = created["data"]["workspace_id"]
        .as_str()
        .expect("workspace ID");
    let database = data_root.join("metadata/state.db");
    let connection = Connection::open(&database).expect("controlled test metadata");
    let updated = connection
        .execute(
            "UPDATE workspaces
             SET state='error', last_error_code='E_FILESYSTEM', updated_at_unix_ms=?2
             WHERE workspace_id=?1 AND state='ready'",
            params![workspace_id, 1_700_000_000_001_i64],
        )
        .expect("model post-Ready operation failure");
    assert_eq!(updated, 1);
    drop(connection);

    let (code, listing) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "list".into(),
        ],
    );
    assert_eq!(code, 0, "{listing}");
    assert_eq!(listing["data"]["workspaces"][0]["state"], "error");
    assert_eq!(
        listing["data"]["workspaces"][0]["materialization"]["actual_mode"],
        "cow-clone"
    );
    assert!(listing["data"]["workspaces"][0].get("path").is_none());

    let (code, status) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "failed-later".into(),
        ],
    );
    assert_eq!(code, 0, "{status}");
    assert_eq!(status["data"]["state"], "error");
    assert_eq!(status["data"]["git"]["state"], "unknown");
    assert_eq!(status["data"]["git"]["issues"][0], "workspace-not-ready");
    assert_eq!(
        status["data"]["git"]["repositories"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    let (code, stdout, stderr) = execute(
        &bootstrap,
        vec![
            "thinws".into(),
            "workspace".into(),
            "path".into(),
            "failed-later".into(),
        ],
    );
    assert_eq!(code, 21);
    assert!(stdout.is_empty());
    assert!(String::from_utf8_lossy(&stderr).contains("E_WORKSPACE_NOT_READY"));
}

#[test]
fn creating_without_receipt_stays_diagnostic_and_is_not_a_usable_path() {
    let (_temp, bootstrap, data_root, source) = fixture();
    initialize(&bootstrap, &data_root);
    let (code, doctor) = json(
        &bootstrap,
        vec!["thinws".into(), "--json".into(), "doctor".into()],
    );
    assert_eq!(code, 0, "{doctor}");
    let instance_id = doctor["data"]["instance_id"].as_str().unwrap();
    let volume_id = doctor["data"]["volume_id"].as_str().unwrap();
    let workspace_id = "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f40";
    let target = data_root.join("workspaces").join(workspace_id).join("root");
    let connection = Connection::open(data_root.join("metadata/state.db")).unwrap();
    connection
        .execute(
            "INSERT INTO workspaces (
                workspace_id, instance_id, name, source_path, target_path,
                source_volume_id, data_volume_id, allow_full_copy,
                state, last_error_code, created_at_unix_ms, updated_at_unix_ms
             ) VALUES (?1, ?2, 'unfinished', ?3, ?4, ?5, ?5, 0,
                       'creating', NULL, ?6, ?6)",
            params![
                workspace_id,
                instance_id,
                source.as_os_str().as_bytes(),
                target.as_os_str().as_bytes(),
                volume_id,
                1_700_000_000_000_i64,
            ],
        )
        .expect("controlled Creating fixture");
    drop(connection);

    let (code, listing) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "list".into(),
        ],
    );
    assert_eq!(code, 0, "{listing}");
    assert_eq!(listing["data"]["workspaces"][0]["state"], "creating");
    assert_eq!(
        listing["data"]["workspaces"][0]["materialization"],
        Value::Null
    );

    let (code, status) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "unfinished".into(),
        ],
    );
    assert_eq!(code, 0, "{status}");
    assert_eq!(status["data"]["git"]["issues"][0], "workspace-not-ready");
    assert_eq!(status["data"]["path"], Value::Null);

    let (code, stdout, stderr) = execute(
        &bootstrap,
        vec![
            "thinws".into(),
            "workspace".into(),
            "path".into(),
            "unfinished".into(),
        ],
    );
    assert_eq!(code, 21);
    assert!(stdout.is_empty());
    assert!(String::from_utf8_lossy(&stderr).contains("E_WORKSPACE_NOT_READY"));
}

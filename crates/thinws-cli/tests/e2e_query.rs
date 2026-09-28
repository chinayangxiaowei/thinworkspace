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
    let data_root = bootstrap.clone();
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
    assert_eq!(bootstrap, data_root);
    let (code, result) = json(
        bootstrap,
        vec!["thinws".into(), "--json".into(), "init".into()],
    );
    assert_eq!(code, 0, "{result}");
}

fn create(bootstrap: &Path, source: &Path, name: &str) -> Value {
    let target = bootstrap.parent().unwrap().join(format!("target-{name}"));
    let (code, result) = json(
        bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "create".into(),
            "--source".into(),
            source.as_os_str().to_owned(),
            "--target".into(),
            target.as_os_str().to_owned(),
            "--name".into(),
            name.into(),
        ],
    );
    assert_eq!(code, 0, "{result}");
    result
}

#[test]
fn status_measures_current_copy_space_without_reusing_creation_receipt() {
    let (_temp, bootstrap, data_root, source) = fixture();
    initialize(&bootstrap, &data_root);
    let created = create(&bootstrap, &source, "space-copy");
    let copy = PathBuf::from(created["data"]["path"].as_str().expect("copy path"));
    fs::write(copy.join("tracked.txt"), b"changed now").expect("change copied file");
    fs::write(copy.join("new.bin"), vec![0_u8; 4096]).expect("add a copied-side file");

    let (code, status) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "space-copy".into(),
        ],
    );
    assert_eq!(code, 0, "{status}");
    assert_eq!(status["data"]["space"]["state"], "complete");
    assert_eq!(status["data"]["space"]["logical_bytes"], 4107);
    assert!(status["data"]["space"]["allocated_bytes_estimate"].is_u64());
}

#[test]
fn status_space_does_not_follow_links_or_double_count_hardlink_allocations() {
    let (temp, bootstrap, data_root, source) = fixture();
    initialize(&bootstrap, &data_root);
    let created = create(&bootstrap, &source, "space-links");
    let copy = PathBuf::from(created["data"]["path"].as_str().expect("copy path"));
    let (code, before) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "space-links".into(),
        ],
    );
    assert_eq!(code, 0, "{before}");
    assert_eq!(before["data"]["space"]["logical_bytes"], 8);
    let allocated_before = before["data"]["space"]["allocated_bytes_estimate"]
        .as_u64()
        .expect("allocated estimate");

    fs::hard_link(copy.join("tracked.txt"), copy.join("hard-link"))
        .expect("same-inode copied-side link");
    let (code, hardlink_only) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "space-links".into(),
        ],
    );
    assert_eq!(code, 0, "{hardlink_only}");
    assert_eq!(hardlink_only["data"]["space"]["logical_bytes"], 16);
    assert_eq!(
        hardlink_only["data"]["space"]["allocated_bytes_estimate"],
        allocated_before
    );
    let outside = temp.path().join("outside-large-file");
    fs::write(&outside, vec![1_u8; 1_000_000]).expect("large outside target");
    symlink(&outside, copy.join("external-link")).expect("copied-side symlink");
    let (code, after) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "space-links".into(),
        ],
    );
    assert_eq!(code, 0, "{after}");
    assert_eq!(after["data"]["space"]["state"], "complete");
    assert_eq!(
        after["data"]["space"]["logical_bytes"],
        16 + outside.as_os_str().as_bytes().len()
    );
    assert!(
        after["data"]["space"]["allocated_bytes_estimate"]
            .as_u64()
            .expect("allocated estimate")
            >= allocated_before
    );
}

#[test]
fn status_space_reports_unknown_for_an_unscannable_special_entry() {
    let (_temp, bootstrap, data_root, source) = fixture();
    initialize(&bootstrap, &data_root);
    let created = create(&bootstrap, &source, "space-unknown");
    let copy = PathBuf::from(created["data"]["path"].as_str().expect("copy path"));
    let fifo = copy.join("unscannable.fifo");
    assert!(
        Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .status()
            .expect("controlled mkfifo")
            .success()
    );
    let (code, status) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "space-unknown".into(),
        ],
    );
    assert_eq!(code, 0, "{status}");
    assert_eq!(status["data"]["space"]["state"], "unknown");
    assert_eq!(status["data"]["space"]["logical_bytes"], Value::Null);
    assert_eq!(
        status["data"]["space"]["allocated_bytes_estimate"],
        Value::Null
    );
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
    let (temp, bootstrap, data_root, source) = fixture();
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

    let lock_file = data_root.join("lifecycle.lock");
    fs::rename(&lock_file, data_root.join("old-lifecycle.lock"))
        .expect("remove active lock entry from controlled fixture");
    symlink(temp.path(), &lock_file).expect("replace lock entry with an invalid symlink");
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
    assert_eq!(stdout, format!("{expected}\n").as_bytes());

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
fn path_and_status_refuse_symlink_replacement_and_missing_names() {
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
    assert_eq!(refused["error"]["code"], "E_TARGET_LAYOUT");
}

#[test]
fn path_and_status_report_uninitialized_when_control_root_is_absent() {
    let (temp, bootstrap, data_root, source) = fixture();
    initialize(&bootstrap, &data_root);
    create(&bootstrap, &source, "lost-root");
    fs::rename(&data_root, temp.path().join("control-root-moved"))
        .expect("move registered control root without replacing it");

    let (code, error) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "lost-root".into(),
        ],
    );
    assert_eq!(code, 10, "{error}");
    assert_eq!(error["error"]["code"], "E_NOT_INITIALIZED");
    let (code, stdout, stderr) = execute(
        &bootstrap,
        vec![
            "thinws".into(),
            "workspace".into(),
            "path".into(),
            "lost-root".into(),
        ],
    );
    assert_eq!(code, 10, "{}", String::from_utf8_lossy(&stderr));
    assert!(stdout.is_empty());
}

#[test]
fn path_and_status_classify_a_missing_metadata_directory_as_layout_failure() {
    let (_temp, bootstrap, data_root, source) = fixture();
    initialize(&bootstrap, &data_root);
    create(&bootstrap, &source, "lost-metadata");
    fs::rename(data_root.join("metadata"), data_root.join("metadata-moved"))
        .expect("move metadata directory without replacing it");

    let (code, status) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "lost-metadata".into(),
        ],
    );
    assert_eq!(code, 39, "{status}");
    assert_eq!(status["error"]["code"], "E_CONTROL_LAYOUT");
    let (code, stdout, stderr) = execute(
        &bootstrap,
        vec![
            "thinws".into(),
            "workspace".into(),
            "path".into(),
            "lost-metadata".into(),
        ],
    );
    assert_eq!(code, 39, "{}", String::from_utf8_lossy(&stderr));
    assert!(stdout.is_empty());
}

#[test]
fn non_ready_status_is_diagnostic_and_path_is_rejected() {
    let (temp, bootstrap, data_root, source) = fixture();
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
        listing["data"]["workspaces"][0]["target"],
        created["data"]["path"]
    );
    assert_eq!(
        listing["data"]["workspaces"][0]["target_hex"],
        created["data"]["path_hex"]
    );
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
    assert_eq!(status["data"]["target"], created["data"]["path"]);
    assert_eq!(status["data"]["target_hex"], created["data"]["path_hex"]);
    assert_eq!(status["data"]["path"], Value::Null);
    assert_eq!(status["data"]["git"]["state"], "unknown");
    assert_eq!(status["data"]["space"]["state"], "unknown");
    assert_eq!(status["data"]["space"]["logical_bytes"], Value::Null);
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
            "status".into(),
            "failed-later".into(),
        ],
    );
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert!(stderr.is_empty());
    let human = String::from_utf8(stdout).expect("human status is UTF-8 in this fixture");
    assert!(!human.contains("Path:"));
    assert!(human.contains("Space:         not measured (Workspace not Ready)"));

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

    let lock_file = data_root.join("lifecycle.lock");
    fs::rename(&lock_file, data_root.join("old-lifecycle.lock"))
        .expect("remove active lock entry from controlled fixture");
    symlink(temp.path(), &lock_file).expect("replace lock entry with an invalid symlink");
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
    assert_eq!(status["data"]["git"]["issues"][0], "workspace-not-ready");
    let (code, stdout, stderr) = execute(
        &bootstrap,
        vec![
            "thinws".into(),
            "workspace".into(),
            "path".into(),
            "failed-later".into(),
        ],
    );
    assert_eq!(code, 21, "{}", String::from_utf8_lossy(&stderr));
    assert!(stdout.is_empty());
}

#[test]
fn list_keeps_the_registered_target_when_the_directory_is_missing() {
    let (temp, bootstrap, data_root, source) = fixture();
    initialize(&bootstrap, &data_root);
    let created = create(&bootstrap, &source, "missing-target-list");
    let target = PathBuf::from(created["data"]["path"].as_str().unwrap());
    let moved = temp.path().join("target-moved-outside-registration");
    fs::rename(&target, &moved).unwrap();

    let (code, listed) = json(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "list".into(),
        ],
    );
    assert_eq!(code, 0, "{listed}");
    let workspace = &listed["data"]["workspaces"][0];
    assert_eq!(workspace["target"], created["data"]["path"]);
    assert_eq!(workspace["target_hex"], created["data"]["path_hex"]);
    assert!(workspace.get("path").is_none());
    assert!(!target.exists());
    assert!(moved.exists());
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
    let workspace_id = "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f40";
    let target = bootstrap.parent().unwrap().join("unfinished-target");
    let connection = Connection::open(data_root.join("metadata/state.db")).unwrap();
    let volume_id: String = connection
        .query_row("SELECT control_volume_id FROM installation", [], |row| {
            row.get(0)
        })
        .unwrap();
    connection
        .execute(
            "INSERT INTO workspaces (
                workspace_id, instance_id, name, source_path, target_path,
                source_volume_id, target_volume_id, allow_full_copy,
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

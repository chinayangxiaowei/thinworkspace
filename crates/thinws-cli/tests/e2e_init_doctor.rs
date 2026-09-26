use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use tempfile::{Builder, TempDir};
use thinws_cli::{LocalCommands, run};

fn apfs_tempdir(prefix: &str) -> TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-03-cli-tests");
    fs::create_dir_all(&root).expect("create controlled CLI test root");
    Builder::new()
        .prefix(prefix)
        .tempdir_in(fs::canonicalize(root).expect("canonicalize controlled CLI test root"))
        .expect("the repository test volume must satisfy the Phase 1 APFS requirement")
}

fn private_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn execute(bootstrap: &Path, arguments: Vec<OsString>) -> (i32, Vec<u8>, Vec<u8>) {
    let commands = LocalCommands::new(Some(bootstrap.to_path_buf()))
        .with_timeouts(Duration::from_secs(1), Duration::from_secs(1));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let status = run(
        arguments,
        &commands,
        1_700_000_000_000,
        &mut stdout,
        &mut stderr,
    );
    (status, stdout, stderr)
}

fn init_args(data_root: &Path) -> Vec<OsString> {
    vec![
        OsString::from("thinws"),
        OsString::from("--json"),
        OsString::from("init"),
        OsString::from("--data-root"),
        data_root.as_os_str().to_owned(),
    ]
}

fn doctor_args() -> Vec<OsString> {
    vec![
        OsString::from("thinws"),
        OsString::from("--json"),
        OsString::from("doctor"),
    ]
}

#[test]
fn concrete_init_is_idempotent_and_doctor_reports_the_ready_apfs_installation() {
    let temp = apfs_tempdir("thinws-p1-03-e2e-");
    let bootstrap = temp.path().join("bootstrap");
    let data_root = temp.path().join("data-root");

    let (status, stdout, stderr) = execute(&bootstrap, init_args(&data_root));
    assert_eq!(status, 0, "{}", String::from_utf8_lossy(&stderr));
    assert!(stderr.is_empty());
    let initialized: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(initialized["data"]["result"], "initialized");
    assert_eq!(
        initialized["data"]["data_root_hex"],
        data_root
            .as_os_str()
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );

    let (status, stdout, stderr) = execute(&bootstrap, init_args(&data_root));
    assert_eq!(status, 0, "{}", String::from_utf8_lossy(&stderr));
    let repeated: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(repeated["data"]["result"], "already-initialized");
    assert_eq!(
        repeated["data"]["instance_id"],
        initialized["data"]["instance_id"]
    );

    let (status, stdout, stderr) = execute(&bootstrap, doctor_args());
    assert_eq!(status, 0, "{}", String::from_utf8_lossy(&stderr));
    let doctor: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(doctor["data"]["status"], "ready");
    assert_eq!(doctor["data"]["incomplete_workspaces"], 0);
    assert_eq!(doctor["data"]["git_check"]["available"], true);

    for directory in ["metadata", "logs", "workspaces", "staging", "trash"] {
        assert_eq!(
            fs::metadata(data_root.join(directory))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o700
        );
    }
    for file in [
        bootstrap.join("config.toml"),
        bootstrap.join("init.lock"),
        data_root.join(".thinws-root.toml"),
        data_root.join("metadata/state.db"),
    ] {
        assert_eq!(
            fs::metadata(file).unwrap().permissions().mode() & 0o7777,
            0o600
        );
    }
    assert!(!data_root.join("metadata/lifecycle.lock").exists());
    assert!(!data_root.join("logs/operations.jsonl").exists());
}

#[test]
fn doctor_before_initialization_and_conflicting_reinit_keep_their_public_boundaries() {
    let temp = apfs_tempdir("thinws-p1-03-boundaries-");
    let bootstrap = temp.path().join("bootstrap");

    let (status, stdout, stderr) = execute(&bootstrap, doctor_args());
    assert_eq!(status, 10);
    assert!(stderr.is_empty());
    let uninitialized: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(uninitialized["error"]["code"], "E_NOT_INITIALIZED");
    assert!(!bootstrap.exists());

    let registered = temp.path().join("registered");
    assert_eq!(execute(&bootstrap, init_args(&registered)).0, 0);

    let conflicting = temp.path().join("must-not-be-touched");
    let (status, stdout, stderr) = execute(&bootstrap, init_args(&conflicting));
    assert_eq!(status, 16);
    assert!(stderr.is_empty());
    let conflict: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(conflict["error"]["code"], "E_DATA_ROOT_CHANGE_UNSUPPORTED");
    assert!(!conflicting.exists());
}

#[test]
fn nonempty_unowned_root_and_missing_registered_root_use_distinct_public_errors() {
    let temp = apfs_tempdir("thinws-p1-03-errors-");
    let bootstrap = temp.path().join("bootstrap-nonempty");
    let nonempty = temp.path().join("nonempty");
    private_dir(&nonempty);
    fs::write(nonempty.join("user-file"), b"preserve").unwrap();

    let (status, stdout, _) = execute(&bootstrap, init_args(&nonempty));
    assert_eq!(status, 36);
    let error: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(error["error"]["code"], "E_DATA_ROOT_NOT_EMPTY");
    assert_eq!(fs::read(nonempty.join("user-file")).unwrap(), b"preserve");

    let bootstrap = temp.path().join("bootstrap-missing");
    let data_root = temp.path().join("registered");
    assert_eq!(execute(&bootstrap, init_args(&data_root)).0, 0);
    let displaced = temp.path().join("displaced-registered");
    fs::rename(&data_root, &displaced).unwrap();

    let (status, stdout, _) = execute(&bootstrap, doctor_args());
    assert_eq!(status, 32);
    let error: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(error["error"]["code"], "E_DATA_ROOT_UNAVAILABLE");
}

#[test]
fn doctor_reports_controlled_directory_type_and_symlink_changes_as_layout_errors() {
    for replacement in ["file", "symlink", "permissions"] {
        let temp = apfs_tempdir("thinws-p1-03-layout-errors-");
        let bootstrap = temp.path().join("bootstrap");
        let data_root = temp.path().join("registered");
        assert_eq!(execute(&bootstrap, init_args(&data_root)).0, 0);

        let logs = data_root.join("logs");
        fs::remove_dir(&logs).unwrap();
        if replacement == "file" {
            fs::write(&logs, b"not a controlled directory").unwrap();
        } else if replacement == "symlink" {
            let outside = temp.path().join("outside");
            private_dir(&outside);
            symlink(&outside, &logs).unwrap();
        } else {
            private_dir(&logs);
            fs::set_permissions(&logs, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let (status, stdout, stderr) = execute(&bootstrap, doctor_args());
        assert_eq!(status, 33, "replacement={replacement}");
        assert!(stderr.is_empty());
        let error: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(
            error["error"]["code"], "E_DATA_ROOT_LAYOUT",
            "replacement={replacement}"
        );
    }
}

#[test]
fn doctor_reports_bootstrap_and_root_marker_symlinks_as_layout_errors() {
    for document in ["config", "marker"] {
        let temp = apfs_tempdir("thinws-p1-03-document-links-");
        let bootstrap = temp.path().join("bootstrap");
        let data_root = temp.path().join("registered");
        assert_eq!(execute(&bootstrap, init_args(&data_root)).0, 0);

        let document_path = if document == "config" {
            bootstrap.join("config.toml")
        } else {
            data_root.join(".thinws-root.toml")
        };
        let displaced = document_path.with_extension("displaced");
        fs::rename(&document_path, &displaced).unwrap();
        symlink(&displaced, &document_path).unwrap();

        let (status, stdout, stderr) = execute(&bootstrap, doctor_args());
        assert_eq!(status, 33, "document={document}");
        assert!(stderr.is_empty());
        let error: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(
            error["error"]["code"], "E_DATA_ROOT_LAYOUT",
            "document={document}"
        );
    }
}

#[test]
fn compiled_binary_help_does_not_initialize_user_state() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_thinws"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: thinws"));
    assert!(stdout.contains("init"));
    assert!(stdout.contains("doctor"));
}

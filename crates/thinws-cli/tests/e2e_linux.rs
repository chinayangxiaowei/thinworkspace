#![cfg(target_os = "linux")]

use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;
use tempfile::Builder;
use thinws_cli::{LocalCommands, run};

fn execute(control: &Path, arguments: Vec<OsString>) -> (i32, Vec<u8>, Vec<u8>) {
    if let Some(binary) = env::var_os("THINWS_LINUX_E2E_BINARY") {
        let mut args = arguments.into_iter();
        assert_eq!(args.next(), Some(OsString::from("thinws")));
        let output = Command::new(binary)
            .env("HOME", control.parent().unwrap())
            .args(args)
            .output()
            .unwrap();
        return (
            output.status.code().unwrap_or(1),
            output.stdout,
            output.stderr,
        );
    }

    let commands = LocalCommands::new(Some(control.to_path_buf()))
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

fn execute_json(control: &Path, arguments: Vec<OsString>) -> (i32, Value) {
    let (status, stdout, stderr) = execute(control, arguments);
    assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
    (status, serde_json::from_slice(&stdout).unwrap())
}

#[test]
fn occupied_target_symlink_returns_target_exists_without_following_it() {
    let ext4 = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
        .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test root");
    let btrfs = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test root");
    let control_fixture = Builder::new()
        .prefix("thinws-cli-linux-symlink-control-")
        .tempdir_in(ext4)
        .unwrap();
    let data_fixture = Builder::new()
        .prefix("thinws-cli-linux-symlink-data-")
        .tempdir_in(btrfs)
        .unwrap();
    let control = control_fixture.path().join(".thinws");
    let source = data_fixture.path().join("source");
    let target = data_fixture.path().join("existing-link");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("note.txt"), b"keep source").unwrap();
    symlink("source", &target).unwrap();

    let (status, initialized) = execute_json(
        &control,
        vec!["thinws".into(), "--json".into(), "init".into()],
    );
    assert_eq!(status, 0, "{initialized}");

    for dry_run in [true, false] {
        let mut args = vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "create".into(),
            "--source".into(),
            source.as_os_str().to_owned(),
            "--target".into(),
            target.as_os_str().to_owned(),
            "--name".into(),
            "occupied-link".into(),
        ];
        if dry_run {
            args.push("--dry-run".into());
        }
        let (status, error) = execute_json(&control, args);
        assert_eq!(status, 43, "{error}");
        assert_eq!(error["error"]["code"], "E_TARGET_EXISTS");
        assert_eq!(fs::read_link(&target).unwrap(), Path::new("source"));
        assert_eq!(fs::read(source.join("note.txt")).unwrap(), b"keep source");
    }

    let (status, listed) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "list".into(),
        ],
    );
    assert_eq!(status, 0, "{listed}");
    assert!(listed["data"]["workspaces"].as_array().unwrap().is_empty());
}

#[test]
fn debian_cli_completes_an_ext4_control_and_btrfs_workspace_lifecycle() {
    let ext4 = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
        .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test root");
    let btrfs = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test root");
    let control_fixture = Builder::new()
        .prefix("thinws-cli-linux-control-")
        .tempdir_in(ext4)
        .unwrap();
    let data_fixture = Builder::new()
        .prefix("thinws-cli-linux-data-")
        .tempdir_in(btrfs)
        .unwrap();
    let control = control_fixture.path().join(".thinws");
    let source = data_fixture.path().join("source");
    let target = data_fixture.path().join("copy");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("note.txt"), b"source").unwrap();

    let (status, initialized) = execute_json(
        &control,
        vec!["thinws".into(), "--json".into(), "init".into()],
    );
    assert_eq!(status, 0, "{initialized}");
    assert_eq!(initialized["data"]["result"], "initialized");
    let (status, doctor) = execute_json(
        &control,
        vec!["thinws".into(), "--json".into(), "doctor".into()],
    );
    assert_eq!(status, 0, "{doctor}");
    assert_eq!(doctor["data"]["host"]["platform"], "linux");

    let create = || {
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
            "linux-copy".into(),
        ]
    };
    let mut preview_args = create();
    preview_args.push("--dry-run".into());
    let (status, preview) = execute_json(&control, preview_args);
    assert_eq!(status, 0, "{preview}");
    assert_eq!(
        preview["data"]["materialization"]["adapter"],
        "btrfs-reflink"
    );
    assert!(!target.exists());

    let (status, created) = execute_json(&control, create());
    assert_eq!(status, 0, "{created}");
    assert_eq!(
        created["data"]["materialization"]["adapter"],
        "btrfs-reflink"
    );
    let (status, repeated) = execute_json(&control, create());
    assert_eq!(status, 0, "{repeated}");
    assert_eq!(repeated["data"]["result"], "already-ready");
    assert_eq!(
        repeated["data"]["workspace_id"],
        created["data"]["workspace_id"]
    );
    assert_eq!(fs::read(target.join("note.txt")).unwrap(), b"source");
    fs::write(target.join("note.txt"), b"changed").unwrap();
    assert_eq!(fs::read(source.join("note.txt")).unwrap(), b"source");

    let (status, listed) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "list".into(),
        ],
    );
    assert_eq!(status, 0, "{listed}");
    assert_eq!(listed["data"]["workspaces"].as_array().unwrap().len(), 1);
    let (status, path, stderr) = execute(
        &control,
        vec![
            "thinws".into(),
            "workspace".into(),
            "path".into(),
            "linux-copy".into(),
        ],
    );
    assert_eq!(status, 0, "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(path, format!("{}\n", target.display()).as_bytes());
    let (status, workspace) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "linux-copy".into(),
        ],
    );
    assert_eq!(status, 0, "{workspace}");
    assert_eq!(workspace["data"]["state"], "ready");

    let (status, removed) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "remove".into(),
            "linux-copy".into(),
        ],
    );
    assert_eq!(status, 0, "{removed}");
    assert_eq!(removed["data"]["result"], "removed");
    assert!(!target.exists());
    assert!(control.join("logs/operations.jsonl").exists());
}

#[test]
fn source_subvolume_on_the_same_btrfs_mount_completes_the_cli_lifecycle() {
    let ext4 = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT").unwrap();
    let btrfs = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT").unwrap();
    let control_fixture = Builder::new()
        .prefix("thinws-cli-subvolume-control-")
        .tempdir_in(ext4)
        .unwrap();
    let data_fixture = Builder::new()
        .prefix("thinws-cli-subvolume-data-")
        .tempdir_in(btrfs)
        .unwrap();
    let control = control_fixture.path().join(".thinws");
    let source = data_fixture.path().join("source-subvolume");
    let target = data_fixture.path().join("copy");
    let created = Command::new("btrfs")
        .args(["subvolume", "create"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(created.status.success(), "{created:?}");
    fs::write(source.join("note.txt"), b"subvolume source").unwrap();
    assert_ne!(
        fs::metadata(&source).unwrap().dev(),
        fs::metadata(data_fixture.path()).unwrap().dev()
    );

    let (status, initialized) = execute_json(
        &control,
        vec!["thinws".into(), "--json".into(), "init".into()],
    );
    assert_eq!(status, 0, "{initialized}");
    let (status, cloned) = execute_json(
        &control,
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
            "subvolume-copy".into(),
        ],
    );
    assert_eq!(status, 0, "{cloned}");
    assert_eq!(
        cloned["data"]["materialization"]["adapter"],
        "btrfs-reflink"
    );
    assert_eq!(cloned["data"]["materialization"]["cow"], "confirmed");
    assert_eq!(
        fs::read(target.join("note.txt")).unwrap(),
        b"subvolume source"
    );

    let (status, removed) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "remove".into(),
            "subvolume-copy".into(),
        ],
    );
    assert_eq!(status, 0, "{removed}");
    assert!(!target.exists());
    fs::remove_file(source.join("note.txt")).unwrap();
    fs::remove_dir(&source).unwrap();
}

#[test]
fn non_btrfs_sources_never_fall_back_to_full_copy_on_linux() {
    let ext4 = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT").unwrap();
    let btrfs = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT").unwrap();
    let shared_file = env::var_os("THINWS_LINUX_OTHER_TEST_FILE").unwrap();
    let control_fixture = Builder::new()
        .prefix("thinws-cli-linux-control-")
        .tempdir_in(ext4)
        .unwrap();
    let data_fixture = Builder::new()
        .prefix("thinws-cli-linux-data-")
        .tempdir_in(btrfs)
        .unwrap();
    let control = control_fixture.path().join(".thinws");
    let target = data_fixture.path().join("copy");
    let source = Path::new(&shared_file).parent().unwrap();
    assert_eq!(
        execute_json(
            &control,
            vec!["thinws".into(), "--json".into(), "init".into()]
        )
        .0,
        0
    );
    let (status, rejected) = execute_json(
        &control,
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
            "crossfs".into(),
            "--allow-copy".into(),
        ],
    );
    assert_ne!(status, 0, "{rejected}");
    assert_eq!(rejected["error"]["code"], "E_TARGET_LAYOUT");
    assert!(!target.exists());
    let ext4_source = control_fixture.path().join("ext4-source");
    fs::create_dir(&ext4_source).unwrap();
    fs::write(ext4_source.join("note.txt"), b"source remains").unwrap();
    let ext4_target = data_fixture.path().join("ext4-copy");
    let (status, rejected) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "create".into(),
            "--source".into(),
            ext4_source.as_os_str().to_owned(),
            "--target".into(),
            ext4_target.as_os_str().to_owned(),
            "--name".into(),
            "ext4-crossfs".into(),
            "--allow-copy".into(),
        ],
    );
    assert_ne!(status, 0, "{rejected}");
    assert_eq!(rejected["error"]["code"], "E_TARGET_LAYOUT");
    assert!(!ext4_target.exists());
    assert_eq!(
        fs::read(ext4_source.join("note.txt")).unwrap(),
        b"source remains"
    );
    let (status, listed) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "list".into(),
        ],
    );
    assert_eq!(status, 0, "{listed}");
    assert!(listed["data"]["workspaces"].as_array().unwrap().is_empty());
}

#[test]
fn runtime_nocow_failures_keep_policy_and_cleanup_boundaries() {
    let ext4 = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT").unwrap();
    let btrfs = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT").unwrap();
    let control_fixture = Builder::new()
        .prefix("thinws-cli-linux-control-")
        .tempdir_in(ext4)
        .unwrap();
    let data_fixture = Builder::new()
        .prefix("thinws-cli-linux-nocow-")
        .tempdir_in(btrfs)
        .unwrap();
    let control = control_fixture.path().join(".thinws");
    let source = data_fixture.path().join("source");
    let target = data_fixture.path().join("copy");
    fs::create_dir(&source).unwrap();
    let chattr = Command::new("chattr")
        .arg("+C")
        .arg(&source)
        .status()
        .expect("chattr must be installed for the Btrfs NOCOW test");
    assert!(chattr.success(), "Btrfs test root must permit NOCOW");
    fs::write(source.join("file"), b"NOCOW source bytes").unwrap();
    assert_eq!(
        execute_json(
            &control,
            vec!["thinws".into(), "--json".into(), "init".into()]
        )
        .0,
        0
    );
    let (status, rejected) = execute_json(
        &control,
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
            "nocow-copy".into(),
            "--allow-copy".into(),
        ],
    );
    assert_eq!(status, 11, "{rejected}");
    assert_eq!(rejected["error"]["code"], "E_CAPABILITY_UNAVAILABLE");
    assert_eq!(
        rejected["error"]["message"],
        "Full Copy is unavailable for this path combination"
    );
    assert!(target.is_dir());
    assert!(fs::read_dir(&target).unwrap().next().is_none());
    assert_eq!(
        fs::read(source.join("file")).unwrap(),
        b"NOCOW source bytes"
    );

    let other_target = data_fixture.path().join("copy-without-fallback");
    let (status, denied) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "create".into(),
            "--source".into(),
            source.as_os_str().to_owned(),
            "--target".into(),
            other_target.as_os_str().to_owned(),
            "--name".into(),
            "nocow-without-copy".into(),
        ],
    );
    assert_eq!(status, 12, "{denied}");
    assert_eq!(denied["error"]["code"], "E_COW_UNAVAILABLE");
    assert!(other_target.is_dir());
    assert!(fs::read_dir(&other_target).unwrap().next().is_none());

    let (status, listed) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "list".into(),
        ],
    );
    assert_eq!(status, 0, "{listed}");
    assert_eq!(listed["data"]["workspaces"].as_array().unwrap().len(), 2);
    let (status, removed) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "remove".into(),
            "nocow-copy".into(),
            "--force".into(),
        ],
    );
    assert_eq!(status, 0, "{removed}");
    assert!(!target.exists());
    let (status, removed) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "remove".into(),
            "nocow-without-copy".into(),
            "--force".into(),
        ],
    );
    assert_eq!(status, 0, "{removed}");
    assert!(!other_target.exists());
}

#[test]
fn tracked_git_changes_require_explicit_force_and_keep_the_cleanup_log() {
    let ext4 = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT").unwrap();
    let btrfs = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT").unwrap();
    let control_fixture = Builder::new()
        .prefix("thinws-cli-linux-control-")
        .tempdir_in(ext4)
        .unwrap();
    let data_fixture = Builder::new()
        .prefix("thinws-cli-linux-data-")
        .tempdir_in(btrfs)
        .unwrap();
    let control = control_fixture.path().join(".thinws");
    let source = data_fixture.path().join("source");
    let target = data_fixture.path().join("copy");
    fs::create_dir(&source).unwrap();
    assert!(
        Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(&source)
            .status()
            .unwrap()
            .success()
    );
    fs::write(source.join("tracked.txt"), b"original").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&source)
            .arg("add")
            .arg("tracked.txt")
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&source)
            .arg("-c")
            .arg("user.name=ThinWorkspace Test")
            .arg("-c")
            .arg("user.email=thinws-test@example.invalid")
            .arg("commit")
            .arg("-qm")
            .arg("fixture")
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        execute_json(
            &control,
            vec!["thinws".into(), "--json".into(), "init".into()]
        )
        .0,
        0
    );
    let (status, created) = execute_json(
        &control,
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
            "git-copy".into(),
        ],
    );
    assert_eq!(status, 0, "{created}");
    assert!(target.join(".git").is_dir());
    let status_args = || {
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "status".into(),
            "git-copy".into(),
        ]
    };
    fs::write(target.join("untracked.txt"), b"new work").unwrap();
    let (status, clean) = execute_json(&control, status_args());
    assert_eq!(status, 0, "{clean}");
    assert_eq!(clean["data"]["git"]["scan_complete"], true);
    assert_eq!(clean["data"]["git"]["state"], "clean");
    assert_eq!(
        clean["data"]["git"]["repositories"][0]["tracked_changes"],
        0
    );
    assert_eq!(clean["data"]["space"]["state"], "complete");
    assert!(clean["data"]["space"]["logical_bytes"].as_u64().is_some());
    fs::write(target.join("tracked.txt"), b"modified").unwrap();
    let (status, dirty) = execute_json(&control, status_args());
    assert_eq!(status, 0, "{dirty}");
    assert_eq!(dirty["data"]["git"]["state"], "dirty");
    assert_eq!(
        dirty["data"]["git"]["repositories"][0]["tracked_changes"],
        1
    );
    let remove_args = || {
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "remove".into(),
            "git-copy".into(),
        ]
    };
    let (status, refused) = execute_json(&control, remove_args());
    assert_eq!(status, 22, "{refused}");
    assert_eq!(refused["error"]["code"], "E_WORKSPACE_DIRTY");
    assert_eq!(fs::read(target.join("tracked.txt")).unwrap(), b"modified");
    let mut force = remove_args();
    force.push("--force".into());
    let (status, removed) = execute_json(&control, force);
    assert_eq!(status, 0, "{removed}");
    assert_eq!(removed["data"]["forced"], true);
    assert!(!target.exists());
    let log = fs::read_to_string(control.join("logs/operations.jsonl")).unwrap();
    assert!(log.contains("\"event\":\"refused\""));
    assert!(log.contains("\"event\":\"started\""));
    assert!(log.contains("\"event\":\"completed\""));

    let untracked_target = data_fixture.path().join("untracked-copy");
    let (status, created) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "create".into(),
            "--source".into(),
            source.as_os_str().to_owned(),
            "--target".into(),
            untracked_target.as_os_str().to_owned(),
            "--name".into(),
            "untracked-copy".into(),
        ],
    );
    assert_eq!(status, 0, "{created}");
    fs::write(untracked_target.join("untracked.txt"), b"discardable").unwrap();
    let (status, removed) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "remove".into(),
            "untracked-copy".into(),
        ],
    );
    assert_eq!(status, 0, "{removed}");
    assert_eq!(removed["data"]["forced"], false);
    assert!(!untracked_target.exists());
}

#[test]
fn installed_binary_covers_all_public_linux_commands_with_an_ordinary_target_path() {
    let ext4 = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT").unwrap();
    let btrfs = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT").unwrap();
    let home = Builder::new()
        .prefix("thinws-cli-linux-home-")
        .tempdir_in(ext4)
        .unwrap();
    let data = Builder::new()
        .prefix("thinws-cli-linux-binary-")
        .tempdir_in(btrfs)
        .unwrap();
    let source = data.path().join("source");
    let target = data.path().join("ordinary target");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("example.txt"), b"contents").unwrap();

    let binary = env::var_os("THINWS_LINUX_E2E_BINARY")
        .unwrap_or_else(|| OsString::from(env!("CARGO_BIN_EXE_thinws")));
    let invoke = |args: &[&std::ffi::OsStr]| {
        Command::new(&binary)
            .env("HOME", home.path())
            .args(args)
            .output()
            .unwrap()
    };
    let invoke_json = |args: &[&std::ffi::OsStr]| {
        let output = invoke(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let init = invoke(&["init".as_ref()]);
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert!(home.path().join(".thinws").is_dir());
    let doctor = invoke_json(&["--json".as_ref(), "doctor".as_ref()]);
    assert_eq!(doctor["data"]["status"], "ready");
    assert_eq!(doctor["data"]["host"]["platform"], "linux");

    let preview = invoke_json(&[
        "--json".as_ref(),
        "workspace".as_ref(),
        "create".as_ref(),
        "--source".as_ref(),
        source.as_os_str(),
        "--target".as_ref(),
        target.as_os_str(),
        "--name".as_ref(),
        "binary-copy".as_ref(),
        "--dry-run".as_ref(),
    ]);
    assert_eq!(preview["data"]["dry_run"], true);
    assert!(preview["data"]["workspace_id"].is_null());
    assert!(!target.exists());

    let created = invoke(&[
        "workspace".as_ref(),
        "create".as_ref(),
        "--source".as_ref(),
        source.as_os_str(),
        "--target".as_ref(),
        target.as_os_str(),
        "--name".as_ref(),
        "binary-copy".as_ref(),
    ]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    assert_eq!(fs::read(target.join("example.txt")).unwrap(), b"contents");

    let listed = invoke_json(&["--json".as_ref(), "workspace".as_ref(), "list".as_ref()]);
    let workspaces = listed["data"]["workspaces"].as_array().unwrap();
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0]["name"], "binary-copy");
    let status = invoke_json(&[
        "--json".as_ref(),
        "workspace".as_ref(),
        "status".as_ref(),
        "binary-copy".as_ref(),
    ]);
    assert_eq!(status["data"]["state"], "ready");
    assert_eq!(status["data"]["git"]["state"], "not-applicable");
    assert_eq!(status["data"]["space"]["state"], "complete");

    let nested_target = target.join("nested target");
    for dry_run in [true, false] {
        let mut args = vec![
            "--json".as_ref(),
            "workspace".as_ref(),
            "create".as_ref(),
            "--source".as_ref(),
            source.as_os_str(),
            "--target".as_ref(),
            nested_target.as_os_str(),
            "--name".as_ref(),
            "nested-copy".as_ref(),
        ];
        if dry_run {
            args.push("--dry-run".as_ref());
        }
        let rejected = invoke(&args);
        assert_eq!(rejected.status.code(), Some(42));
        let response: Value = serde_json::from_slice(&rejected.stdout).unwrap();
        assert_eq!(response["error"]["code"], "E_TARGET_CONFLICT");
        assert!(!nested_target.exists());
    }
    let path = invoke(&[
        "workspace".as_ref(),
        "path".as_ref(),
        "binary-copy".as_ref(),
    ]);
    assert!(path.status.success());
    assert_eq!(path.stdout, format!("{}\n", target.display()).as_bytes());
    let removed = invoke(&[
        "workspace".as_ref(),
        "remove".as_ref(),
        "binary-copy".as_ref(),
    ]);
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(!target.exists());
    let listed_after = invoke_json(&["--json".as_ref(), "workspace".as_ref(), "list".as_ref()]);
    assert!(
        listed_after["data"]["workspaces"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn missing_registered_target_stays_registered_even_with_force() {
    let ext4 = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT").unwrap();
    let btrfs = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT").unwrap();
    let control_fixture = Builder::new()
        .prefix("thinws-cli-linux-control-")
        .tempdir_in(ext4)
        .unwrap();
    let data_fixture = Builder::new()
        .prefix("thinws-cli-linux-data-")
        .tempdir_in(btrfs)
        .unwrap();
    let control = control_fixture.path().join(".thinws");
    let source = data_fixture.path().join("source");
    let target = data_fixture.path().join("copy");
    let moved = data_fixture.path().join("moved-outside-thinws");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("keep.txt"), b"preserve").unwrap();
    assert_eq!(
        execute_json(
            &control,
            vec!["thinws".into(), "--json".into(), "init".into()]
        )
        .0,
        0
    );
    let (status, created) = execute_json(
        &control,
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
            "missing-copy".into(),
        ],
    );
    assert_eq!(status, 0, "{created}");
    fs::rename(&target, &moved).unwrap();
    let (status, refused) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "remove".into(),
            "missing-copy".into(),
            "--force".into(),
        ],
    );
    assert_eq!(status, 37, "{refused}");
    assert_eq!(refused["error"]["code"], "E_TARGET_MISSING");
    assert_eq!(fs::read(moved.join("keep.txt")).unwrap(), b"preserve");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("foreign.txt"), b"not a workspace").unwrap();
    let (status, refused) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "remove".into(),
            "missing-copy".into(),
            "--force".into(),
        ],
    );
    assert_eq!(status, 38, "{refused}");
    assert_eq!(refused["error"]["code"], "E_TARGET_IDENTITY");
    assert_eq!(
        fs::read(target.join("foreign.txt")).unwrap(),
        b"not a workspace"
    );
    assert_eq!(fs::read(moved.join("keep.txt")).unwrap(), b"preserve");
    let (status, listed) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "list".into(),
        ],
    );
    assert_eq!(status, 0, "{listed}");
    assert_eq!(listed["data"]["workspaces"].as_array().unwrap().len(), 1);
}

#[test]
fn confirmed_external_process_use_blocks_even_forced_removal() {
    let ext4 = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT").unwrap();
    let btrfs = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT").unwrap();
    let control_fixture = Builder::new()
        .prefix("thinws-cli-linux-control-")
        .tempdir_in(ext4)
        .unwrap();
    let data_fixture = Builder::new()
        .prefix("thinws-cli-linux-data-")
        .tempdir_in(btrfs)
        .unwrap();
    let control = control_fixture.path().join(".thinws");
    let source = data_fixture.path().join("source");
    let target = data_fixture.path().join("copy");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("keep.txt"), b"preserve").unwrap();
    assert_eq!(
        execute_json(
            &control,
            vec!["thinws".into(), "--json".into(), "init".into()]
        )
        .0,
        0
    );
    let (status, created) = execute_json(
        &control,
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
            "occupied-copy".into(),
        ],
    );
    assert_eq!(status, 0, "{created}");

    let mut occupant = Command::new("sleep")
        .arg("10")
        .current_dir(&target)
        .spawn()
        .unwrap();
    let (status, refused) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "remove".into(),
            "occupied-copy".into(),
            "--force".into(),
        ],
    );
    let _ = occupant.kill();
    let _ = occupant.wait();
    assert_eq!(status, 23, "{refused}");
    assert_eq!(refused["error"]["code"], "E_WORKSPACE_BUSY");
    assert_eq!(fs::read(target.join("keep.txt")).unwrap(), b"preserve");
}

#[test]
fn target_inside_an_active_workspace_is_rejected_on_linux() {
    let ext4 = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT").unwrap();
    let btrfs = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT").unwrap();
    let control_fixture = Builder::new()
        .prefix("thinws-cli-linux-control-")
        .tempdir_in(ext4)
        .unwrap();
    let data_fixture = Builder::new()
        .prefix("thinws-cli-linux-overlap-")
        .tempdir_in(btrfs)
        .unwrap();
    let control = control_fixture.path().join(".thinws");
    let source = data_fixture.path().join("source");
    let parent_target = data_fixture.path().join("parent-target");
    let child_target = parent_target.join("child-target");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("sentinel.txt"), b"source remains").unwrap();
    assert_eq!(
        execute_json(
            &control,
            vec!["thinws".into(), "--json".into(), "init".into()]
        )
        .0,
        0
    );
    let create = |name: &str, target: &Path, dry_run: bool| {
        let mut args = vec![
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
        ];
        if dry_run {
            args.push("--dry-run".into());
        }
        execute_json(&control, args)
    };
    let (status, created) = create("parent", &parent_target, false);
    assert_eq!(status, 0, "{created}");
    let id = created["data"]["workspace_id"].as_str().unwrap();

    for dry_run in [true, false] {
        let (status, rejected) = create("child", &child_target, dry_run);
        assert_eq!(status, 42, "{rejected}");
        assert_eq!(rejected["error"]["code"], "E_TARGET_CONFLICT");
        assert!(!child_target.exists());
    }
    for protected in [
        format!(".thinws-staging-{id}"),
        format!(".thinws-trash-{id}"),
        format!(".thinws-remove-{id}"),
    ] {
        let protected_target = data_fixture.path().join(protected);
        assert!(!protected_target.exists());
        for dry_run in [true, false] {
            let (status, rejected) = create("protected", &protected_target, dry_run);
            assert_eq!(status, 42, "{rejected}");
            assert_eq!(rejected["error"]["code"], "E_TARGET_CONFLICT");
            assert!(!protected_target.exists());
        }
    }
    let (status, listed) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "list".into(),
        ],
    );
    assert_eq!(status, 0, "{listed}");
    assert_eq!(listed["data"]["workspaces"].as_array().unwrap().len(), 1);
    assert_eq!(
        fs::read(parent_target.join("sentinel.txt")).unwrap(),
        b"source remains"
    );
}

#[test]
fn target_inside_an_active_removal_isolation_is_rejected_on_linux() {
    let ext4 = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT").unwrap();
    let btrfs = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT").unwrap();
    let control_fixture = Builder::new()
        .prefix("thinws-cli-linux-control-")
        .tempdir_in(ext4)
        .unwrap();
    let data_fixture = Builder::new()
        .prefix("thinws-cli-linux-isolation-")
        .tempdir_in(btrfs)
        .unwrap();
    let control = control_fixture.path().join(".thinws");
    let source = data_fixture.path().join("source");
    let target = data_fixture.path().join("parent-target");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("sentinel.txt"), b"source remains").unwrap();
    assert_eq!(
        execute_json(
            &control,
            vec!["thinws".into(), "--json".into(), "init".into()]
        )
        .0,
        0
    );
    let create = |name: &str, destination: &Path, dry_run: bool| {
        let mut args = vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "create".into(),
            "--source".into(),
            source.as_os_str().to_owned(),
            "--target".into(),
            destination.as_os_str().to_owned(),
            "--name".into(),
            name.into(),
        ];
        if dry_run {
            args.push("--dry-run".into());
        }
        execute_json(&control, args)
    };
    let (status, created) = create("parent", &target, false);
    assert_eq!(status, 0, "{created}");
    let id = created["data"]["workspace_id"].as_str().unwrap();
    let nested = target.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("file.txt"), b"keep in isolation").unwrap();
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o000)).unwrap();
    let (remove_status, failed) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "remove".into(),
            "parent".into(),
            "--force".into(),
        ],
    );
    let isolated = data_fixture.path().join(format!(".thinws-remove-{id}"));
    assert_ne!(remove_status, 0, "{failed}");
    assert!(isolated.is_dir(), "{failed}");
    assert!(!target.exists());

    let child = isolated.join("child-target");
    let attempts: Vec<_> = [true, false]
        .into_iter()
        .map(|dry_run| create("child", &child, dry_run))
        .collect();
    fs::set_permissions(isolated.join("nested"), fs::Permissions::from_mode(0o700)).unwrap();
    for (status, rejected) in attempts {
        assert_eq!(status, 42, "{rejected}");
        assert_eq!(rejected["error"]["code"], "E_TARGET_CONFLICT");
    }
    assert!(!child.exists());
    let (status, listed) = execute_json(
        &control,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "list".into(),
        ],
    );
    assert_eq!(status, 0, "{listed}");
    assert_eq!(listed["data"]["workspaces"].as_array().unwrap().len(), 1);
    assert_eq!(
        fs::read(isolated.join("nested/file.txt")).unwrap(),
        b"keep in isolation"
    );
}

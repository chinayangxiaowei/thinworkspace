#![cfg(target_os = "linux")]

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;
use tempfile::Builder;
use thinws_cli::{LocalCommands, run};

fn execute(control: &Path, arguments: Vec<OsString>) -> (i32, Vec<u8>, Vec<u8>) {
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
fn shared_source_never_falls_back_to_full_copy_on_linux() {
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
    fs::write(target.join("tracked.txt"), b"modified").unwrap();
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
}

#[test]
fn installed_binary_uses_an_isolated_home_and_accepts_an_ordinary_target_path() {
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

    let binary = env!("CARGO_BIN_EXE_thinws");
    let invoke = |args: &[&std::ffi::OsStr]| {
        Command::new(binary)
            .env("HOME", home.path())
            .args(args)
            .output()
            .unwrap()
    };
    let init = invoke(&["init".as_ref()]);
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert!(home.path().join(".thinws").is_dir());
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

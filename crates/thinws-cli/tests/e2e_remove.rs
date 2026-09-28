use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use rusqlite::Connection;
use serde_json::Value;
use tempfile::Builder;
use thinws_cli::{LocalCommands, run};

fn execute(bootstrap: &Path, args: Vec<OsString>) -> (i32, Value) {
    let commands = LocalCommands::new(Some(bootstrap.to_path_buf()))
        .with_timeouts(Duration::from_secs(1), Duration::from_secs(1));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = run(args, &commands, 1_700_000_000_000, &mut stdout, &mut stderr);
    assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
    (code, serde_json::from_slice(&stdout).unwrap())
}

#[test]
fn real_cli_remove_plain_copy_preserves_source_and_keeps_log_outside_copy() {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-12-cli-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("cli-remove-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let bootstrap = temp.path().join("bootstrap");
    let data_root = bootstrap.clone();
    let source = temp.path().join("source");
    let target = temp.path().join("ordinary-target");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("note.txt"), b"source stays").unwrap();
    let (code, init) = execute(
        &bootstrap,
        vec!["thinws".into(), "--json".into(), "init".into()],
    );
    assert_eq!(code, 0, "{init}");
    let (code, created) = execute(
        &bootstrap,
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
            "ordinary".into(),
        ],
    );
    assert_eq!(code, 0, "{created}");
    let copy = PathBuf::from(created["data"]["path"].as_str().unwrap());
    assert_eq!(copy, target);
    assert_eq!(fs::read(copy.join("note.txt")).unwrap(), b"source stays");
    assert!(!copy.join("metadata/lifecycle.lock").exists());
    let unowned_staging = temp.path().join("unowned-staging");
    let unowned_trash = temp.path().join("unowned-trash");
    fs::write(&unowned_staging, b"unowned staging").unwrap();
    fs::write(&unowned_trash, b"unowned trash").unwrap();

    let (code, removed) = execute(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "remove".into(),
            "ordinary".into(),
        ],
    );
    assert_eq!(code, 0, "{removed}");
    assert_eq!(removed["data"]["result"], "removed");
    assert_eq!(removed["data"]["forced"], false);
    assert!(!copy.exists());
    assert_eq!(fs::read(source.join("note.txt")).unwrap(), b"source stays");
    assert_eq!(fs::read(unowned_staging).unwrap(), b"unowned staging");
    assert_eq!(fs::read(unowned_trash).unwrap(), b"unowned trash");
    let log = data_root.join("logs/operations.jsonl");
    assert!(log.is_file());
    assert_eq!(removed["data"]["log"], log.to_str().unwrap());
    assert_eq!(fs::read_to_string(log).unwrap().lines().count(), 2);
}

fn fixture_git(path: &Path, args: &[&str]) {
    let output = Command::new("/usr/bin/git")
        .args(args)
        .current_dir(path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn real_cli_ignores_untracked_but_refuses_tracked_changes_until_explicit_force() {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-12-cli-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("cli-remove-git-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let bootstrap = temp.path().join("bootstrap");
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("tracked.txt"), b"baseline").unwrap();
    fixture_git(&source, &["init", "--quiet"]);
    fixture_git(&source, &["add", "tracked.txt"]);
    fixture_git(
        &source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "baseline",
        ],
    );
    let (code, init) = execute(
        &bootstrap,
        vec!["thinws".into(), "--json".into(), "init".into()],
    );
    assert_eq!(code, 0, "{init}");
    let create = |name: &str| {
        let target = temp.path().join(format!("target-{name}"));
        execute(
            &bootstrap,
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
        )
    };
    let remove = |name: &str, force: bool| {
        let mut args: Vec<OsString> = vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "remove".into(),
            name.into(),
        ];
        if force {
            args.push("--force".into());
        }
        execute(&bootstrap, args)
    };

    let (code, untracked) = create("untracked-only");
    assert_eq!(code, 0, "{untracked}");
    let untracked_copy = PathBuf::from(untracked["data"]["path"].as_str().unwrap());
    fs::write(untracked_copy.join("new.txt"), b"untracked").unwrap();
    let (code, removed) = remove("untracked-only", false);
    assert_eq!(code, 0, "{removed}");
    assert_eq!(removed["data"]["result"], "removed");
    assert!(!untracked_copy.exists());

    let (code, dirty) = create("tracked-dirty");
    assert_eq!(code, 0, "{dirty}");
    let dirty_copy = PathBuf::from(dirty["data"]["path"].as_str().unwrap());
    fs::write(dirty_copy.join("tracked.txt"), b"changed").unwrap();
    let (code, refused) = remove("tracked-dirty", false);
    assert_eq!(refused["error"]["code"], "E_WORKSPACE_DIRTY");
    assert_eq!(refused["error"]["message"], "Workspace has tracked changes");
    assert_eq!(refused["error"]["context"]["issues"], serde_json::json!([]));
    assert_ne!(code, 0);
    assert_eq!(
        refused["error"]["context"]["repositories"][0]["relative_path"],
        "."
    );
    assert_eq!(
        refused["error"]["context"]["repositories"][0]["relative_path_hex"],
        "2e"
    );
    assert_eq!(
        refused["error"]["context"]["repositories"][0]["tracked_changes"],
        1
    );
    assert_eq!(
        refused["error"]["context"]["repositories"][0]["issues"],
        serde_json::json!([])
    );
    assert_eq!(
        refused["error"]["remediation"],
        "Preserve and commit required work, or use workspace remove <name-or-id> --force to discard the copy."
    );
    assert!(dirty_copy.is_dir());
    let commands = LocalCommands::new(Some(bootstrap.clone()))
        .with_timeouts(Duration::from_secs(1), Duration::from_secs(1));
    let mut human_stdout = Vec::new();
    let mut human_stderr = Vec::new();
    let human_code = run(
        ["thinws", "workspace", "remove", "tracked-dirty"]
            .into_iter()
            .map(OsString::from),
        &commands,
        1_700_000_000_000,
        &mut human_stdout,
        &mut human_stderr,
    );
    assert_ne!(human_code, 0);
    assert!(human_stdout.is_empty());
    let hint = String::from_utf8(human_stderr).unwrap();
    assert!(hint.contains("Repository: .\n"));
    assert!(hint.contains("Tracked changes: 1\n"));
    assert!(hint.contains("--force"));
    assert!(hint.contains("No files were removed.\n"));
    let (code, forced) = remove("tracked-dirty", true);
    assert_eq!(code, 0, "{forced}");
    assert_eq!(forced["data"]["forced"], true);
    assert!(!dirty_copy.exists());
}

#[test]
fn real_cli_confirmed_cwd_process_blocks_force_until_the_process_exits() {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-12-cli-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("cli-remove-busy-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let bootstrap = temp.path().join("bootstrap");
    let source = temp.path().join("source");
    let target = temp.path().join("busy-target");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("note.txt"), b"source").unwrap();
    let (code, init) = execute(
        &bootstrap,
        vec!["thinws".into(), "--json".into(), "init".into()],
    );
    assert_eq!(code, 0, "{init}");
    let (code, created) = execute(
        &bootstrap,
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
            "busy".into(),
        ],
    );
    assert_eq!(code, 0, "{created}");
    let copy = PathBuf::from(created["data"]["path"].as_str().unwrap());
    let mut child = Command::new("/bin/sleep")
        .arg("10")
        .current_dir(&copy)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(150));
    let remove_force = || {
        execute(
            &bootstrap,
            vec![
                "thinws".into(),
                "--json".into(),
                "workspace".into(),
                "remove".into(),
                "busy".into(),
                "--force".into(),
            ],
        )
    };
    let (code, refused) = remove_force();
    assert_ne!(code, 0);
    assert_eq!(refused["error"]["code"], "E_WORKSPACE_BUSY");
    assert!(copy.join("note.txt").is_file());
    child.kill().unwrap();
    child.wait().unwrap();
    let (code, removed) = remove_force();
    assert_eq!(code, 0, "{removed}");
    assert!(!copy.exists());
}

#[test]
fn real_cli_git_incomplete_refusal_exposes_the_specific_issue() {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-12-cli-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("cli-remove-unknown-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let bootstrap = temp.path().join("bootstrap");
    let source = temp.path().join("source");
    let target = temp.path().join("unknown-git-target");
    let external_git = temp.path().join("external-git");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&external_git).unwrap();
    fs::write(source.join("note.txt"), b"source").unwrap();
    let (code, init) = execute(
        &bootstrap,
        vec!["thinws".into(), "--json".into(), "init".into()],
    );
    assert_eq!(code, 0, "{init}");
    let (code, created) = execute(
        &bootstrap,
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
            "unknown-git".into(),
        ],
    );
    assert_eq!(code, 0, "{created}");
    let copy = PathBuf::from(created["data"]["path"].as_str().unwrap());
    fs::write(
        copy.join(".git"),
        format!("gitdir: {}\n", external_git.display()),
    )
    .unwrap();
    let (code, refused) = execute(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "remove".into(),
            "unknown-git".into(),
        ],
    );
    assert_eq!(code, 25, "{refused}");
    assert_eq!(refused["error"]["code"], "E_GIT_CHECK_INCOMPLETE");
    assert_eq!(
        refused["error"]["context"]["repositories"][0]["issues"][0],
        "external-repository-metadata"
    );
    assert!(copy.join("note.txt").is_file());
    let commands = LocalCommands::new(Some(bootstrap.clone()))
        .with_timeouts(Duration::from_secs(1), Duration::from_secs(1));
    let mut human_stdout = Vec::new();
    let mut human_stderr = Vec::new();
    let human_code = run(
        ["thinws", "workspace", "remove", "unknown-git"]
            .into_iter()
            .map(OsString::from),
        &commands,
        1_700_000_000_000,
        &mut human_stdout,
        &mut human_stderr,
    );
    assert_eq!(human_code, 25);
    assert!(human_stdout.is_empty());
    assert!(
        String::from_utf8(human_stderr)
            .unwrap()
            .contains("Repo issue: external-repository-metadata\n")
    );
}

#[test]
fn real_cli_force_never_releases_a_missing_or_replaced_target() {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-12-cli-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("cli-remove-target-identity-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let control = temp.path().join(".thinws");
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("note.txt"), b"original").unwrap();
    assert_eq!(
        execute(
            &control,
            vec!["thinws".into(), "--json".into(), "init".into()]
        )
        .0,
        0
    );

    for (name, replace) in [("missing", false), ("replaced", true)] {
        let target = temp.path().join(format!("target-{name}"));
        let (code, created) = execute(
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
                name.into(),
            ],
        );
        assert_eq!(code, 0, "{created}");
        let id = created["data"]["workspace_id"].as_str().unwrap();
        let displaced = temp.path().join(format!("displaced-{name}"));
        fs::rename(&target, &displaced).unwrap();
        if replace {
            fs::create_dir(&target).unwrap();
            fs::write(target.join("foreign.txt"), b"keep foreign").unwrap();
        }

        for force in [false, true] {
            let mut args: Vec<OsString> = vec![
                "thinws".into(),
                "--json".into(),
                "workspace".into(),
                "remove".into(),
                name.into(),
            ];
            if force {
                args.push("--force".into());
            }
            let (code, refused) = execute(&control, args);
            assert_eq!(code, if replace { 38 } else { 37 }, "{refused}");
            assert_eq!(
                refused["error"]["code"],
                if replace {
                    "E_TARGET_IDENTITY"
                } else {
                    "E_TARGET_MISSING"
                }
            );
            assert_eq!(fs::read(displaced.join("note.txt")).unwrap(), b"original");
            if replace {
                assert_eq!(
                    fs::read(target.join("foreign.txt")).unwrap(),
                    b"keep foreign"
                );
            } else {
                assert!(!target.exists());
            }
            let database = Connection::open(control.join("metadata/state.db")).unwrap();
            let active: i64 = database
                .query_row(
                    "SELECT count(*) FROM workspaces WHERE workspace_id=?1",
                    [id],
                    |row| row.get(0),
                )
                .unwrap();
            let tombstones: i64 = database
                .query_row(
                    "SELECT count(*) FROM deletion_tombstones WHERE workspace_id=?1",
                    [id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!((active, tombstones), (1, 0));
            assert!(control.join("logs/operations.jsonl").is_file());
        }
    }
}

#[test]
fn real_cli_keeps_registration_while_the_target_parent_is_unavailable() {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-12-cli-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("cli-target-parent-unavailable-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let control = temp.path().join(".thinws");
    let source = temp.path().join("source");
    let parent = temp.path().join("removable-parent");
    let displaced = temp.path().join("parent-offline");
    let target = parent.join("copy");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("note.txt"), b"preserve while unavailable").unwrap();
    fs::create_dir(&parent).unwrap();
    assert_eq!(
        execute(
            &control,
            vec!["thinws".into(), "--json".into(), "init".into()]
        )
        .0,
        0
    );
    let (code, created) = execute(
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
            "offline".into(),
        ],
    );
    assert_eq!(code, 0, "{created}");
    let id = created["data"]["workspace_id"].as_str().unwrap();

    fs::rename(&parent, &displaced).unwrap();
    let remove_args = vec![
        "thinws".into(),
        "--json".into(),
        "workspace".into(),
        "remove".into(),
        "offline".into(),
        "--force".into(),
    ];
    let (code, refused) = execute(&control, remove_args.clone());
    assert_eq!(code, 37, "{refused}");
    assert_eq!(refused["error"]["code"], "E_TARGET_MISSING");
    assert_eq!(
        fs::read(displaced.join("copy/note.txt")).unwrap(),
        b"preserve while unavailable"
    );
    let database = Connection::open(control.join("metadata/state.db")).unwrap();
    let active: i64 = database
        .query_row(
            "SELECT count(*) FROM workspaces WHERE workspace_id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    let tombstones: i64 = database
        .query_row(
            "SELECT count(*) FROM deletion_tombstones WHERE workspace_id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!((active, tombstones), (1, 0));

    fs::rename(&displaced, &parent).unwrap();
    let (code, removed) = execute(&control, remove_args);
    assert_eq!(code, 0, "{removed}");
    assert_eq!(removed["data"]["result"], "removed");
    assert!(!target.exists());
}

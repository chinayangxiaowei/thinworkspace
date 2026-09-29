#![cfg(target_os = "macos")]

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use tempfile::Builder;
use thinws_cli::{LocalCommands, run};

fn execute(bootstrap: &Path, arguments: Vec<OsString>) -> (i32, Value) {
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
    assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
    (status, serde_json::from_slice(&stdout).unwrap())
}

fn init(bootstrap: &Path, data_root: &Path) {
    assert_eq!(bootstrap, data_root);
    let (status, json) = execute(
        bootstrap,
        vec!["thinws".into(), "--json".into(), "init".into()],
    );
    assert_eq!(status, 0, "{json}");
}

fn create_args(source: &Path, target: &Path, name: &str, more: &[&str]) -> Vec<OsString> {
    let mut arguments = vec![
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
    arguments.extend(more.iter().map(OsString::from));
    arguments
}

#[test]
fn real_cli_preview_create_and_idempotence_keep_the_source_untouched() {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-09-cli-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("cli-create-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let bootstrap = temp.path().join("bootstrap");
    let data_root = bootstrap.clone();
    let source = temp.path().join("source");
    let target = temp.path().join("plain-target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(source.join(".git")).unwrap();
    fs::write(source.join(".git/HEAD"), b"ordinary metadata\n").unwrap();
    fs::write(source.join("ignored.bin"), b"ignored content").unwrap();
    fs::write(source.join("untracked.txt"), b"untracked content").unwrap();
    symlink("untracked.txt", source.join("link.txt")).unwrap();
    init(&bootstrap, &data_root);

    let (status, preview) = execute(
        &bootstrap,
        create_args(&source, &target, "plain", &["--dry-run"]),
    );
    assert_eq!(status, 0, "{preview}");
    assert_eq!(preview["data"]["workspace_id"], Value::Null);
    assert_eq!(preview["data"]["same_volume"], true);
    assert_eq!(
        preview["data"]["materialization"]["effective_planned_mode"],
        "cow-clone"
    );
    assert!(!target.exists());
    assert!(!data_root.join("workspaces").exists());
    assert!(data_root.join("lifecycle.lock").exists());

    let (status, created) = execute(&bootstrap, create_args(&source, &target, "plain", &[]));
    assert_eq!(status, 0, "{created}");
    assert_eq!(created["data"]["result"], "created");
    assert_eq!(
        created["data"]["materialization"]["actual_mode"],
        "cow-clone"
    );
    assert_eq!(created["data"]["materialization"]["cow"], "confirmed");
    let path = PathBuf::from(created["data"]["path"].as_str().unwrap());
    assert_eq!(path, target);
    assert_eq!(
        fs::read(path.join(".git/HEAD")).unwrap(),
        b"ordinary metadata\n"
    );
    assert_eq!(
        fs::read(path.join("ignored.bin")).unwrap(),
        b"ignored content"
    );
    assert_eq!(
        fs::read(path.join("untracked.txt")).unwrap(),
        b"untracked content"
    );
    assert_eq!(
        fs::read_link(path.join("link.txt")).unwrap(),
        PathBuf::from("untracked.txt")
    );
    fs::write(path.join("untracked.txt"), b"changed only in workspace").unwrap();
    assert_eq!(
        fs::read(source.join("untracked.txt")).unwrap(),
        b"untracked content"
    );

    fs::rename(&source, temp.path().join("source-moved")).unwrap();
    let (status, repeated) = execute(&bootstrap, create_args(&source, &target, "plain", &[]));
    assert_eq!(status, 0, "{repeated}");
    assert_eq!(repeated["data"]["result"], "already-ready");
    assert_eq!(
        repeated["data"]["workspace_id"],
        created["data"]["workspace_id"]
    );

    let (status, conflict) = execute(
        &bootstrap,
        create_args(&temp.path().join("source-moved"), &target, "plain", &[]),
    );
    assert_eq!(status, 15);
    assert_eq!(conflict["error"]["code"], "E_NAME_CONFLICT");
}

#[test]
fn real_cli_rejects_a_target_inside_an_active_workspace() {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-09-cli-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("cli-nested-target-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let bootstrap = temp.path().join("bootstrap");
    let source = temp.path().join("source");
    let parent_target = temp.path().join("parent-target");
    let child_target = parent_target.join("child-target");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("sentinel.txt"), b"source remains").unwrap();
    init(&bootstrap, &bootstrap);

    let (code, parent) = execute(
        &bootstrap,
        create_args(&source, &parent_target, "parent", &[]),
    );
    assert_eq!(code, 0, "{parent}");

    for extra in [&["--dry-run"][..], &[][..]] {
        let (code, conflict) = execute(
            &bootstrap,
            create_args(&source, &child_target, "child", extra),
        );
        assert_eq!(code, 42, "{conflict}");
        assert_eq!(conflict["error"]["code"], "E_TARGET_CONFLICT");
        assert!(!child_target.exists());
    }
    let aliased_parent = temp.path().join("PARENT-TARGET");
    if aliased_parent.is_dir() {
        // Case-insensitive APFS resolves this alternate spelling to the same
        // registered target. The identity guard must still reject the child.
        for extra in [&["--dry-run"][..], &[][..]] {
            let (code, conflict) = execute(
                &bootstrap,
                create_args(&source, &aliased_parent, "alias-equal", extra),
            );
            assert_eq!(code, 42, "{conflict}");
            assert_eq!(conflict["error"]["code"], "E_TARGET_CONFLICT");
        }
        let alias_child = aliased_parent.join("alias-child");
        for extra in [&["--dry-run"][..], &[][..]] {
            let (code, conflict) = execute(
                &bootstrap,
                create_args(&source, &alias_child, "alias-child", extra),
            );
            assert_eq!(code, 42, "{conflict}");
            assert_eq!(conflict["error"]["code"], "E_TARGET_CONFLICT");
            assert!(!alias_child.exists());
        }
    }
    let (code, listed) = execute(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "list".into(),
        ],
    );
    assert_eq!(code, 0, "{listed}");
    assert_eq!(listed["data"]["workspaces"].as_array().unwrap().len(), 1);
    assert_eq!(
        fs::read(parent_target.join("sentinel.txt")).unwrap(),
        b"source remains"
    );
}

#[test]
fn real_cli_rejects_a_target_inside_an_active_isolation() {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-09-cli-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("cli-isolated-target-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let bootstrap = temp.path().join("bootstrap");
    let source = temp.path().join("source");
    let target = temp.path().join("parent-target");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("sentinel.txt"), b"source remains").unwrap();
    init(&bootstrap, &bootstrap);

    let (code, created) = execute(&bootstrap, create_args(&source, &target, "parent", &[]));
    assert_eq!(code, 0, "{created}");
    let id = created["data"]["workspace_id"].as_str().unwrap();
    let nested = target.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("file.txt"), b"keep in isolation").unwrap();
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o000)).unwrap();
    let (remove_code, failed) = execute(
        &bootstrap,
        vec![
            "thinws".into(),
            "--json".into(),
            "workspace".into(),
            "remove".into(),
            "parent".into(),
            "--force".into(),
        ],
    );
    let isolated = temp.path().join(format!(".thinws-remove-{id}"));
    assert_ne!(remove_code, 0, "{failed}");
    assert!(isolated.is_dir(), "{failed}");
    assert!(!target.exists());

    let child = isolated.join("child-target");
    let attempts: Vec<_> = [&["--dry-run"][..], &[][..]]
        .into_iter()
        .map(|extra| execute(&bootstrap, create_args(&source, &child, "child", extra)))
        .collect();
    let aliased_isolated = temp.path().join(format!(".THINWS-REMOVE-{id}"));
    let alias_child = aliased_isolated.join("alias-child");
    let alias_attempts: Vec<_> = if aliased_isolated.is_dir() {
        [&["--dry-run"][..], &[][..]]
            .into_iter()
            .map(|extra| {
                execute(
                    &bootstrap,
                    create_args(&source, &alias_child, "alias", extra),
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    fs::set_permissions(isolated.join("nested"), fs::Permissions::from_mode(0o700)).unwrap();
    for (code, result) in attempts.into_iter().chain(alias_attempts) {
        assert_eq!(code, 42, "{result}");
        assert_eq!(result["error"]["code"], "E_TARGET_CONFLICT");
    }
    assert!(!child.exists());
    assert!(!alias_child.exists());
    assert_eq!(
        fs::read(isolated.join("nested/file.txt")).unwrap(),
        b"keep in isolation"
    );
}

#[test]
#[ignore = "requires THINWS_P1_CROSS_VOLUME_ROOT on an APFS volume distinct from system temp"]
fn real_cli_rejects_cross_volume_even_with_allow_copy() {
    let cross_root = std::env::var_os("THINWS_P1_CROSS_VOLUME_ROOT")
        .expect("THINWS_P1_CROSS_VOLUME_ROOT must name the prepared APFS mount");
    let temp = Builder::new()
        .prefix("cli-cross-volume-")
        .tempdir()
        .unwrap();
    let external = Builder::new()
        .prefix("cli-cross-volume-source-")
        .tempdir_in(cross_root)
        .unwrap();
    let system_root = fs::canonicalize(temp.path()).unwrap();
    let bootstrap = system_root.join("bootstrap");
    let data_root = bootstrap.clone();
    assert_ne!(
        fs::metadata(temp.path()).unwrap().dev(),
        fs::metadata(external.path()).unwrap().dev(),
        "the CLI cross-volume fixture must use distinct mounted volumes"
    );
    init(&bootstrap, &data_root);
    let target = system_root.join("cross-volume-target");
    let (status, error) = execute(
        &bootstrap,
        create_args(external.path(), &target, "cross-volume", &["--allow-copy"]),
    );
    assert_eq!(status, 33, "{error}");
    assert_eq!(error["error"]["code"], "E_TARGET_LAYOUT");
    assert!(!target.exists());
}

#[test]
#[ignore = "requires THINWS_P1_CROSS_VOLUME_ROOT on an APFS volume distinct from system temp"]
fn real_cli_uses_same_volume_targets_independently_of_the_control_volume() {
    let external_root = std::env::var_os("THINWS_P1_CROSS_VOLUME_ROOT")
        .expect("THINWS_P1_CROSS_VOLUME_ROOT must name the prepared APFS mount");
    let system = Builder::new()
        .prefix("cli-system-volume-")
        .tempdir()
        .unwrap();
    let external = Builder::new()
        .prefix("cli-external-volume-")
        .tempdir_in(external_root)
        .unwrap();
    let system_root = fs::canonicalize(system.path()).unwrap();
    let external_root = fs::canonicalize(external.path()).unwrap();
    assert_ne!(
        fs::metadata(&system_root).unwrap().dev(),
        fs::metadata(&external_root).unwrap().dev(),
        "fixture must use two mounted APFS volumes"
    );

    for (control_parent, workspace_parent, name) in [
        (&system_root, &external_root, "external-copy"),
        (&external_root, &system_root, "system-copy"),
    ] {
        let control = control_parent.join(format!("control-{name}"));
        let source = workspace_parent.join(format!("source-{name}"));
        let target = workspace_parent.join(format!("target-{name}"));
        fs::create_dir(&source).unwrap();
        fs::write(source.join("content.txt"), b"source").unwrap();
        init(&control, &control);

        let (status, created) = execute(&control, create_args(&source, &target, name, &[]));
        assert_eq!(status, 0, "{created}");
        assert_eq!(created["data"]["path"], target.to_str().unwrap());
        assert_eq!(
            created["data"]["materialization"]["actual_mode"],
            "cow-clone"
        );
        assert_eq!(created["data"]["materialization"]["cow"], "confirmed");
        assert_eq!(fs::read(target.join("content.txt")).unwrap(), b"source");
        assert!(!control.join("workspaces").exists());
        assert!(!target.join(".thinws-control.toml").exists());
    }
}

#[test]
fn real_cli_distinguishes_missing_source_from_absent_control_root() {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-09-cli-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("cli-missing-paths-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let bootstrap = temp.path().join("bootstrap");
    let data_root = bootstrap.clone();
    let source = temp.path().join("source");
    let target = temp.path().join("missing-target");
    init(&bootstrap, &data_root);
    for extra in [&[][..], &["--allow-copy"][..], &["--dry-run"][..]] {
        let (status, error) = execute(&bootstrap, create_args(&source, &target, "missing", extra));
        assert_eq!(status, 31, "{error}");
        assert_eq!(error["error"]["code"], "E_FILESYSTEM");
    }
    fs::create_dir(&source).unwrap();
    fs::rename(&data_root, temp.path().join("control-root-moved")).unwrap();
    let (status, error) = execute(
        &bootstrap,
        create_args(&source, &target, "missing-root", &[]),
    );
    assert_eq!(status, 10, "{error}");
    assert_eq!(error["error"]["code"], "E_NOT_INITIALIZED");
}

#[test]
fn real_cli_refuses_existing_unregistered_and_already_registered_targets() {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-09-cli-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("cli-target-conflicts-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let control = temp.path().join(".thinws");
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file"), b"source").unwrap();
    init(&control, &control);

    let existing = temp.path().join("existing-empty-directory");
    fs::create_dir(&existing).unwrap();
    for option in [&["--dry-run"][..], &[][..]] {
        let (status, error) = execute(
            &control,
            create_args(&source, &existing, "existing", option),
        );
        assert_eq!(status, 43, "{error}");
        assert_eq!(error["error"]["code"], "E_TARGET_EXISTS");
        assert_eq!(fs::read_dir(&existing).unwrap().count(), 0);
    }

    let existing_file = temp.path().join("existing-file");
    fs::write(&existing_file, b"keep this file").unwrap();
    let existing_link = temp.path().join("existing-link");
    symlink("source", &existing_link).unwrap();
    for (target, name) in [
        (&existing_file, "existing-file"),
        (&existing_link, "existing-link"),
    ] {
        for option in [&["--dry-run"][..], &[][..]] {
            let (status, error) = execute(&control, create_args(&source, target, name, option));
            assert_eq!(status, 43, "{error}");
            assert_eq!(error["error"]["code"], "E_TARGET_EXISTS");
        }
    }
    assert_eq!(fs::read(&existing_file).unwrap(), b"keep this file");
    assert_eq!(
        fs::read_link(&existing_link).unwrap(),
        PathBuf::from("source")
    );

    let target = temp.path().join("registered-target");
    let (status, created) = execute(&control, create_args(&source, &target, "first", &[]));
    assert_eq!(status, 0, "{created}");
    let (status, conflict) = execute(&control, create_args(&source, &target, "second", &[]));
    assert_eq!(status, 42, "{conflict}");
    assert_eq!(conflict["error"]["code"], "E_TARGET_CONFLICT");
    assert_eq!(fs::read(target.join("file")).unwrap(), b"source");

    let missing_parent = temp.path().join("absent-parent/child");
    let (status, error) = execute(
        &control,
        create_args(&source, &missing_parent, "missing-parent", &[]),
    );
    assert_eq!(status, 33, "{error}");
    assert_eq!(error["error"]["code"], "E_TARGET_LAYOUT");
    assert!(!missing_parent.exists());
}

#[test]
fn real_cli_does_not_treat_a_user_source_as_a_preview_temporary_root() {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-09-cli-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("cli-preview-name-collision-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let control = temp.path().join(".thinws");
    init(&control, &control);

    for (source_name, name) in [
        (".thinws-preview-staging", "preview-staging"),
        (".thinws-preview-trash", "preview-trash"),
    ] {
        let source = temp.path().join(source_name);
        let target = temp.path().join(format!("copy-{name}"));
        fs::create_dir(&source).unwrap();
        fs::write(source.join("payload"), b"source").unwrap();

        let (status, preview) = execute(
            &control,
            create_args(&source, &target, name, &["--dry-run"]),
        );
        assert_eq!(status, 0, "{preview}");
        assert_eq!(preview["data"]["target"], target.to_str().unwrap());
        assert!(!target.exists());

        let (status, created) = execute(&control, create_args(&source, &target, name, &[]));
        assert_eq!(status, 0, "{created}");
        assert_eq!(fs::read(target.join("payload")).unwrap(), b"source");
    }
}

#[test]
fn real_cli_rejects_a_case_alias_target_inside_the_control_root() {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-09-cli-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("cli-control-target-alias-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let control = temp.path().join(".thinws");
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    init(&control, &control);

    let alias = temp.path().join(".THINWS");
    let Ok(alias_metadata) = fs::metadata(&alias) else {
        eprintln!("skipping APFS case alias on a case-sensitive volume");
        return;
    };
    let control_metadata = fs::metadata(&control).unwrap();
    assert_eq!(alias_metadata.dev(), control_metadata.dev());
    assert_eq!(alias_metadata.ino(), control_metadata.ino());

    let target = alias.join("copy");
    for extra in [&["--dry-run"][..], &[][..]] {
        let (status, error) = execute(&control, create_args(&source, &target, "alias", extra));
        assert_eq!(status, 33, "{error}");
        assert_eq!(error["error"]["code"], "E_TARGET_LAYOUT");
        assert!(!target.exists());
    }
    let (status, doctor) = execute(
        &control,
        vec!["thinws".into(), "--json".into(), "doctor".into()],
    );
    assert_eq!(status, 0, "{doctor}");
    assert_eq!(doctor["data"]["incomplete_workspaces"], 0);
}

#[test]
fn ten_cow_workspaces_keep_ordinary_file_writes_independent() {
    let controlled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-15-cli-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("ten-cow-copies-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let bootstrap = temp.path().join("bootstrap");
    let data_root = bootstrap.clone();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("note.txt"), b"source content").unwrap();
    init(&bootstrap, &data_root);

    let mut copies = Vec::new();
    for index in 0..10 {
        let name = format!("copy-{index}");
        let target = temp.path().join(format!("target-{index}"));
        let (status, created) = execute(&bootstrap, create_args(&source, &target, &name, &[]));
        assert_eq!(status, 0, "{created}");
        assert_eq!(
            created["data"]["materialization"]["actual_mode"],
            "cow-clone"
        );
        assert_eq!(created["data"]["materialization"]["cow"], "confirmed");
        let path = PathBuf::from(created["data"]["path"].as_str().unwrap());
        assert_eq!(path, target);
        assert_eq!(fs::read(path.join("note.txt")).unwrap(), b"source content");
        copies.push(path);
    }
    assert!(!data_root.join("workspaces").exists());

    for (index, copy) in copies.iter().enumerate() {
        fs::write(copy.join("note.txt"), format!("copy {index}")).unwrap();
    }
    fs::write(source.join("note.txt"), b"source changed after cloning").unwrap();
    for (index, copy) in copies.iter().enumerate() {
        assert_eq!(
            fs::read(copy.join("note.txt")).unwrap(),
            format!("copy {index}").as_bytes()
        );
    }
    assert_eq!(
        fs::read(source.join("note.txt")).unwrap(),
        b"source changed after cloning"
    );
}

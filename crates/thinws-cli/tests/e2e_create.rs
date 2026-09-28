use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::MetadataExt;
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

fn create_args(source: &Path, name: &str, more: &[&str]) -> Vec<OsString> {
    let mut arguments = vec![
        "thinws".into(),
        "--json".into(),
        "workspace".into(),
        "create".into(),
        "--source".into(),
        source.as_os_str().to_owned(),
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
    fs::create_dir(&source).unwrap();
    fs::create_dir(source.join(".git")).unwrap();
    fs::write(source.join(".git/HEAD"), b"ordinary metadata\n").unwrap();
    fs::write(source.join("ignored.bin"), b"ignored content").unwrap();
    fs::write(source.join("untracked.txt"), b"untracked content").unwrap();
    symlink("untracked.txt", source.join("link.txt")).unwrap();
    init(&bootstrap, &data_root);

    let (status, preview) = execute(&bootstrap, create_args(&source, "plain", &["--dry-run"]));
    assert_eq!(status, 0, "{preview}");
    assert_eq!(preview["data"]["workspace_id"], Value::Null);
    assert_eq!(preview["data"]["same_volume"], true);
    assert_eq!(
        preview["data"]["materialization"]["effective_planned_mode"],
        "cow-clone"
    );
    assert_eq!(
        fs::read_dir(data_root.join("workspaces")).unwrap().count(),
        0
    );
    assert!(data_root.join("lifecycle.lock").exists());

    let (status, created) = execute(&bootstrap, create_args(&source, "plain", &[]));
    assert_eq!(status, 0, "{created}");
    assert_eq!(created["data"]["result"], "created");
    assert_eq!(
        created["data"]["materialization"]["actual_mode"],
        "cow-clone"
    );
    assert_eq!(created["data"]["materialization"]["cow"], "confirmed");
    let path = PathBuf::from(created["data"]["path"].as_str().unwrap());
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
    let (status, repeated) = execute(&bootstrap, create_args(&source, "plain", &[]));
    assert_eq!(status, 0, "{repeated}");
    assert_eq!(repeated["data"]["result"], "already-ready");
    assert_eq!(
        repeated["data"]["workspace_id"],
        created["data"]["workspace_id"]
    );

    let (status, conflict) = execute(
        &bootstrap,
        create_args(&temp.path().join("source-moved"), "plain", &[]),
    );
    assert_eq!(status, 15);
    assert_eq!(conflict["error"]["code"], "E_NAME_CONFLICT");
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
    let (status, error) = execute(
        &bootstrap,
        create_args(external.path(), "cross-volume", &["--allow-copy"]),
    );
    assert_eq!(status, 33, "{error}");
    assert_eq!(error["error"]["code"], "E_DATA_ROOT_LAYOUT");
    assert_eq!(
        fs::read_dir(data_root.join("workspaces")).unwrap().count(),
        0
    );
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
    init(&bootstrap, &data_root);
    for extra in [&[][..], &["--allow-copy"][..], &["--dry-run"][..]] {
        let (status, error) = execute(&bootstrap, create_args(&source, "missing", extra));
        assert_eq!(status, 31, "{error}");
        assert_eq!(error["error"]["code"], "E_FILESYSTEM");
    }
    fs::create_dir(&source).unwrap();
    fs::rename(&data_root, temp.path().join("control-root-moved")).unwrap();
    let (status, error) = execute(&bootstrap, create_args(&source, "missing-root", &[]));
    assert_eq!(status, 10, "{error}");
    assert_eq!(error["error"]["code"], "E_NOT_INITIALIZED");
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
        let (status, created) = execute(&bootstrap, create_args(&source, &name, &[]));
        assert_eq!(status, 0, "{created}");
        assert_eq!(
            created["data"]["materialization"]["actual_mode"],
            "cow-clone"
        );
        assert_eq!(created["data"]["materialization"]["cow"], "confirmed");
        let path = PathBuf::from(created["data"]["path"].as_str().unwrap());
        assert_eq!(fs::read(path.join("note.txt")).unwrap(), b"source content");
        copies.push(path);
    }
    assert_eq!(
        fs::read_dir(data_root.join("workspaces")).unwrap().count(),
        10
    );

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

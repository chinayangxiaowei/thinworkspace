use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use thinws_p0_git_base::{
    AttributeQueryContext, AttributeTypeEvidence, ExperimentError, inspect_attribute_types,
};

const TIMEOUT: Duration = Duration::from_secs(10);
const PAYLOAD: &[u8] = b"fixture-content\n";
const CANARY_PAYLOAD: &[u8] = b"canary-positive-control\n";
static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const ATTRIBUTES_A: &[u8] = br#"[attr]thinwsmacro filter=macro
boolean-set.bin filter
boolean-unset.bin -filter
string-custom.bin filter=custom
string-set.bin filter=set
string-unset.bin filter=unset
string-unspecified.bin filter=unspecified
macro.bin thinwsmacro
canary.bin filter=canary
nested/** filter=nested
source-switch.bin filter
"#;

const ATTRIBUTES_B: &[u8] = br#"[attr]thinwsmacro filter=macro
boolean-set.bin filter
boolean-unset.bin -filter
string-custom.bin filter=custom
string-set.bin filter=set
string-unset.bin filter=unset
string-unspecified.bin filter=unspecified
macro.bin thinwsmacro
canary.bin filter=canary
nested/** filter=nested
source-switch.bin -filter
"#;

const ATTRIBUTES_WORKTREE: &[u8] = br#"[attr]thinwsmacro filter=macro
boolean-set.bin filter
boolean-unset.bin -filter
string-custom.bin filter=custom
string-set.bin filter=set
string-unset.bin filter=unset
string-unspecified.bin filter=unspecified
macro.bin thinwsmacro
canary.bin filter=canary
nested/** filter=nested
source-switch.bin filter=worktree
"#;

const NESTED_ATTRIBUTES: &[u8] = br#"set.bin filter
unset.bin -filter
unspecified.bin !filter
macro.bin thinwsmacro
"#;

#[test]
fn apple_git_attr_pathspec_preserves_filter_types_and_controlled_sources() {
    let fixture = Fixture::create();
    println!(
        "P0-03 gitdir={}, empty-worktree={}, conflict-worktree={}, index-a={}, index-b={}, tree-a={}, tree-b={}",
        fixture.git_dir.display(),
        fixture.empty_work_tree.display(),
        fixture.conflict_work_tree.display(),
        fixture.index_a.display(),
        fixture.index_b.display(),
        fixture.tree_a,
        fixture.tree_b
    );

    assert!(!fixture.git_dir.join("info/attributes").exists());
    assert!(
        fs::read_dir(&fixture.empty_work_tree)
            .expect("read empty controlled worktree")
            .next()
            .is_none()
    );
    assert_ne!(fixture.tree_a, fixture.tree_b);

    fixture.run_canary_positive_control();
    let marker_before = FileSnapshot::capture(&fixture.canary_marker);
    assert_eq!(marker_before.bytes, CANARY_PAYLOAD);

    let index_a_before = FileSnapshot::capture(&fixture.index_a);
    let index_b_before = FileSnapshot::capture(&fixture.index_b);
    let fixture_before = snapshot_tree(&fixture.root);

    let index_a =
        inspect_attribute_types(&fixture.context(&fixture.index_a, &fixture.empty_work_tree))
            .expect("query index A through the empty worktree");
    println!(
        "P0-03 query git={}, gitdir={}, worktree={}, index={}",
        index_a.git_version,
        index_a.git_dir.display(),
        index_a.work_tree.display(),
        index_a.index_file.display()
    );
    assert_common_evidence(
        &fixture,
        &index_a,
        &fixture.index_a,
        &fixture.empty_work_tree,
    );
    assert_index_a_types(&index_a);

    let index_b =
        inspect_attribute_types(&fixture.context(&fixture.index_b, &fixture.empty_work_tree))
            .expect("query index B through the empty worktree");
    assert_common_evidence(
        &fixture,
        &index_b,
        &fixture.index_b,
        &fixture.empty_work_tree,
    );
    assert_index_b_types(&index_b);

    let worktree =
        inspect_attribute_types(&fixture.context(&fixture.index_a, &fixture.conflict_work_tree))
            .expect("query the conflicting controlled worktree over index A");
    assert_common_evidence(
        &fixture,
        &worktree,
        &fixture.index_a,
        &fixture.conflict_work_tree,
    );
    assert_worktree_override_types(&worktree);

    assert_eq!(FileSnapshot::capture(&fixture.index_a), index_a_before);
    assert_eq!(FileSnapshot::capture(&fixture.index_b), index_b_before);
    assert_eq!(FileSnapshot::capture(&fixture.canary_marker), marker_before);
    assert_eq!(snapshot_tree(&fixture.root), fixture_before);
    assert!(
        fs::read_dir(&fixture.empty_work_tree)
            .expect("re-read empty controlled worktree")
            .next()
            .is_none()
    );
}

#[test]
fn apple_git_warning_is_not_silently_accepted() {
    let fixture = Fixture::create();
    fs::write(
        fixture.conflict_work_tree.join(".gitattributes"),
        b"unused.bin bad/attr\n",
    )
    .expect("write invalid controlled attribute");

    let result =
        inspect_attribute_types(&fixture.context(&fixture.index_a, &fixture.conflict_work_tree));
    match result {
        Err(ExperimentError::GitFailure {
            status,
            stdout,
            stderr,
            ..
        }) => {
            assert!(status.success());
            assert!(!stdout.is_empty());
            assert!(!stderr.is_empty());
        }
        other => panic!("expected exit-zero Git warning evidence, got {other:?}"),
    }
}

#[test]
fn fixture_runner_rejects_exit_zero_git_stderr() {
    let fixture = Fixture::create();
    fs::write(
        fixture.conflict_work_tree.join(".gitattributes"),
        b"unused.bin bad/attr\n",
    )
    .expect("write invalid controlled attribute");

    let result = std::panic::catch_unwind(|| {
        run_git(
            &fixture.root,
            &fixture.git_dir,
            &fixture.conflict_work_tree,
            &fixture.index_a,
            ["ls-files", "--cached", "-z", "--", ":(attr:filter)"],
            &[],
        )
    });
    let panic = result.expect_err("fixture Git runner must reject exit-zero stderr");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .expect("fixture runner panic must preserve a string diagnostic");
    assert!(
        message.contains("controlled Git wrote to stderr despite successful exit"),
        "unexpected fixture runner diagnostic: {message}"
    );
}

#[test]
fn initialization_failure_preserves_the_reported_private_root() {
    let root = create_private_root();
    let blocker = root.join("empty-worktree");
    fs::write(&blocker, b"controlled initialization blocker\n")
        .expect("create controlled initialization blocker");

    let error = match Fixture::initialize(root.clone()) {
        Ok(_) => panic!("controlled initialization must fail"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(root.is_dir());
    assert_eq!(
        fs::read(&blocker).expect("read preserved initialization blocker"),
        b"controlled initialization blocker\n"
    );
}

struct Fixture {
    root: PathBuf,
    git_dir: PathBuf,
    empty_work_tree: PathBuf,
    conflict_work_tree: PathBuf,
    isolated_home: PathBuf,
    global_config: PathBuf,
    system_config: PathBuf,
    index_a: PathBuf,
    index_b: PathBuf,
    canary_marker: PathBuf,
    tree_a: String,
    tree_b: String,
}

impl Fixture {
    fn create() -> Self {
        let root = create_private_root();
        Self::initialize(root.clone()).unwrap_or_else(|error| {
            panic!(
                "initialize controlled fixture at {}: {error}",
                root.display()
            )
        })
    }

    fn initialize(root: PathBuf) -> Result<Self, std::io::Error> {
        let git_dir = root.join("repo.git");
        let empty_work_tree = root.join("empty-worktree");
        let conflict_work_tree = root.join("conflict-worktree");
        let isolated_home = root.join("home");
        let inputs = root.join("inputs");
        for directory in [
            &empty_work_tree,
            &conflict_work_tree,
            &isolated_home,
            &inputs,
        ] {
            fs::create_dir(directory)?;
        }
        fs::create_dir(isolated_home.join("xdg"))?;

        let canary_marker = root.join("filter-canary.log");
        let global_attributes = inputs.join("global.attributes");
        let system_attributes = inputs.join("system.attributes");
        let global_config = root.join("global.gitconfig");
        let system_config = root.join("system.gitconfig");
        let attributes_a_file = inputs.join("index-a.gitattributes");
        let attributes_b_file = inputs.join("index-b.gitattributes");
        let nested_attributes_file = inputs.join("nested.gitattributes");
        let payload_file = inputs.join("payload.bin");

        fs::write(&global_attributes, b"* filter=global\n")?;
        fs::write(&system_attributes, b"* filter=system\n")?;
        fs::write(&attributes_a_file, ATTRIBUTES_A)?;
        fs::write(&attributes_b_file, ATTRIBUTES_B)?;
        fs::write(&nested_attributes_file, NESTED_ATTRIBUTES)?;
        fs::write(&payload_file, PAYLOAD)?;
        fs::write(
            conflict_work_tree.join(".gitattributes"),
            ATTRIBUTES_WORKTREE,
        )?;

        let global_text = format!(
            "[core]\n\tattributesFile = {}\n\tfsmonitor = /usr/bin/false\n[filter \"canary\"]\n\tclean = /usr/bin/tee -a {}\n\trequired = true\n",
            path_text(&global_attributes),
            path_text(&canary_marker)
        );
        let system_text = format!(
            "[core]\n\tattributesFile = {}\n\tfsmonitor = /usr/bin/false\n",
            path_text(&system_attributes)
        );
        fs::write(&global_config, global_text.as_bytes())?;
        fs::write(&system_config, system_text.as_bytes())?;

        run(
            &root,
            None,
            [
                OsString::from("-c"),
                OsString::from("init.defaultBranch=main"),
                OsString::from("-c"),
                OsString::from("core.attributesFile=/dev/null"),
                OsString::from("-c"),
                OsString::from("core.fsmonitor=false"),
                OsString::from("init"),
                OsString::from("--bare"),
                git_dir.as_os_str().to_owned(),
            ],
            &[],
        );

        let attr_a_oid = hash_blob(&root, &git_dir, ATTRIBUTES_A);
        let attr_b_oid = hash_blob(&root, &git_dir, ATTRIBUTES_B);
        let nested_attr_oid = hash_blob(&root, &git_dir, NESTED_ATTRIBUTES);
        let payload_oid = hash_blob(&root, &git_dir, PAYLOAD);
        let index_a = root.join("index-a");
        let index_b = root.join("index-b");
        let entries = fixture_paths()
            .into_iter()
            .map(|path| (path, payload_oid.as_str()))
            .collect::<Vec<_>>();
        let tree_a = build_index(
            &root,
            &git_dir,
            &empty_work_tree,
            &index_a,
            &attr_a_oid,
            &nested_attr_oid,
            &entries,
        );
        let tree_b = build_index(
            &root,
            &git_dir,
            &empty_work_tree,
            &index_b,
            &attr_b_oid,
            &nested_attr_oid,
            &entries,
        );

        Ok(Self {
            root,
            git_dir,
            empty_work_tree,
            conflict_work_tree,
            isolated_home,
            global_config,
            system_config,
            index_a,
            index_b,
            canary_marker,
            tree_a,
            tree_b,
        })
    }

    fn context(&self, index_file: &Path, work_tree: &Path) -> AttributeQueryContext {
        AttributeQueryContext {
            git_dir: self.git_dir.clone(),
            work_tree: work_tree.to_owned(),
            index_file: index_file.to_owned(),
            isolated_home: self.isolated_home.clone(),
            global_config: self.global_config.clone(),
            system_config: self.system_config.clone(),
        }
    }

    fn run_canary_positive_control(&self) {
        assert!(!self.canary_marker.exists());
        let output = run_git(
            &self.root,
            &self.git_dir,
            &self.conflict_work_tree,
            &self.index_a,
            ["hash-object", "--stdin", "--path=canary.bin"],
            CANARY_PAYLOAD,
        );
        let object_id = String::from_utf8(output).expect("canary object id is ASCII");
        let object_id = object_id.trim();
        assert_eq!(object_id.len(), 40);
        assert!(object_id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            fs::read(&self.canary_marker).expect("read positive canary marker"),
            CANARY_PAYLOAD
        );
    }
}

fn build_index<'a>(
    root: &Path,
    git_dir: &Path,
    work_tree: &Path,
    index_file: &Path,
    root_attributes_oid: &str,
    nested_attributes_oid: &str,
    entries: &[(&'a str, &'a str)],
) -> String {
    run_git(
        root,
        git_dir,
        work_tree,
        index_file,
        ["read-tree", "--empty"],
        &[],
    );
    add_index_entry(
        root,
        git_dir,
        work_tree,
        index_file,
        ".gitattributes",
        root_attributes_oid,
    );
    add_index_entry(
        root,
        git_dir,
        work_tree,
        index_file,
        "nested/.gitattributes",
        nested_attributes_oid,
    );
    for (path, oid) in entries {
        add_index_entry(root, git_dir, work_tree, index_file, path, oid);
    }
    let output = run_git(root, git_dir, work_tree, index_file, ["write-tree"], &[]);
    let tree = String::from_utf8(output)
        .expect("tree object id is ASCII")
        .trim()
        .to_owned();
    assert_eq!(tree.len(), 40);
    assert!(tree.bytes().all(|byte| byte.is_ascii_hexdigit()));
    tree
}

fn add_index_entry(
    root: &Path,
    git_dir: &Path,
    work_tree: &Path,
    index_file: &Path,
    path: &str,
    oid: &str,
) {
    run_git(
        root,
        git_dir,
        work_tree,
        index_file,
        [
            OsString::from("update-index"),
            OsString::from("--add"),
            OsString::from("--cacheinfo"),
            OsString::from(format!("100644,{oid},{path}")),
        ],
        &[],
    );
}

fn hash_blob(root: &Path, git_dir: &Path, bytes: &[u8]) -> String {
    let output = run(
        root,
        None,
        [
            OsString::from("-c"),
            OsString::from("core.attributesFile=/dev/null"),
            OsString::from("-c"),
            OsString::from("core.fsmonitor=false"),
            OsString::from("--git-dir"),
            git_dir.as_os_str().to_owned(),
            OsString::from("hash-object"),
            OsString::from("-w"),
            OsString::from("--stdin"),
        ],
        bytes,
    );
    let oid = String::from_utf8(output)
        .expect("object id is ASCII")
        .trim()
        .to_owned();
    assert_eq!(oid.len(), 40);
    assert!(oid.bytes().all(|byte| byte.is_ascii_hexdigit()));
    oid
}

fn run_git<I, S>(
    root: &Path,
    git_dir: &Path,
    work_tree: &Path,
    index_file: &Path,
    arguments: I,
    stdin: &[u8],
) -> Vec<u8>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let prefix = [
        OsString::from("-c"),
        OsString::from("core.bare=false"),
        OsString::from("-c"),
        OsString::from("core.attributesFile=/dev/null"),
        OsString::from("-c"),
        OsString::from("core.fsmonitor=false"),
        OsString::from("-c"),
        OsString::from("core.hooksPath=/dev/null"),
        OsString::from("-c"),
        OsString::from("diff.external="),
        OsString::from("--git-dir"),
        git_dir.as_os_str().to_owned(),
        OsString::from("--work-tree"),
        work_tree.as_os_str().to_owned(),
    ];
    let arguments = prefix
        .into_iter()
        .chain(arguments.into_iter().map(|value| value.as_ref().to_owned()));
    run_at(root, work_tree, Some(index_file), arguments, stdin)
}

fn run<I, S>(root: &Path, index: Option<&Path>, arguments: I, stdin: &[u8]) -> Vec<u8>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_at(root, root, index, arguments, stdin)
}

fn run_at<I, S>(
    environment_root: &Path,
    current_dir: &Path,
    index: Option<&Path>,
    arguments: I,
    stdin: &[u8],
) -> Vec<u8>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("/usr/bin/git");
    command
        .env_clear()
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("HOME", environment_root.join("home"))
        .env("XDG_CONFIG_HOME", environment_root.join("home/xdg"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            environment_root.join("global.gitconfig"),
        )
        .env(
            "GIT_CONFIG_SYSTEM",
            environment_root.join("system.gitconfig"),
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_PAGER", "cat")
        .current_dir(current_dir)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    let mut child = command.spawn().expect("spawn controlled Git");
    if !stdin.is_empty()
        && let Err(error) = child.stdin.as_mut().expect("Git stdin").write_all(stdin)
    {
        drop(child.stdin.take());
        let termination = terminate_fixture_child(child);
        panic!("write controlled Git stdin failed: {error}; termination={termination}");
    }
    drop(child.stdin.take());

    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let termination = terminate_fixture_child(child);
                panic!("controlled Git timed out; termination={termination}");
            }
            Err(error) => {
                let termination = terminate_fixture_child(child);
                panic!("poll controlled Git failed: {error}; termination={termination}");
            }
        }
    };
    let output = child.wait_with_output().unwrap_or_else(|error| {
        panic!("collect controlled Git output after observed status {status}: {error}")
    });
    successful_output(output)
}

fn terminate_fixture_child(mut child: std::process::Child) -> String {
    match child.kill() {
        Err(error) => {
            format!("kill failed ({error}); child state unconfirmed; no blocking wait attempted")
        }
        Ok(()) => match child.wait_with_output() {
            Ok(output) => format!(
                "kill request succeeded; status={}; stdout={:?}; stderr={:?}",
                output.status, output.stdout, output.stderr
            ),
            Err(error) => {
                format!("kill request succeeded but reap failed ({error}); child state unconfirmed")
            }
        },
    }
}

fn successful_output(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "controlled Git failed: status={}; stdout={:?}; stderr={:?}",
        output.status,
        output.stdout,
        output.stderr
    );
    assert!(
        output.stderr.is_empty(),
        "controlled Git wrote to stderr despite successful exit: status={}; stdout={:?}; stderr={:?}",
        output.status,
        output.stdout,
        output.stderr
    );
    output.stdout
}

fn assert_common_evidence(
    fixture: &Fixture,
    evidence: &AttributeTypeEvidence,
    index: &Path,
    work_tree: &Path,
) {
    assert!(evidence.git_version.starts_with("git version "));
    assert_eq!(evidence.git_dir, fixture.git_dir);
    assert_eq!(evidence.work_tree, work_tree);
    assert_eq!(evidence.index_file, index);
    assert_eq!(evidence.all_paths, all_paths());
    assert!(!evidence.set_paths.is_empty());
    assert!(!evidence.unset_paths.is_empty());
    assert!(!evidence.unspecified_paths.is_empty());
    for value in ["custom", "macro", "nested", "set", "unset", "unspecified"] {
        assert!(
            !value_paths(evidence, value).is_empty(),
            "fixed value query {value} must have a positive fixture"
        );
    }
    assert!(value_paths(evidence, "global").is_empty());
    assert!(value_paths(evidence, "system").is_empty());
}

fn assert_index_a_types(evidence: &AttributeTypeEvidence) {
    assert_eq!(
        evidence.set_paths,
        paths(["boolean-set.bin", "nested/set.bin", "source-switch.bin"])
    );
    assert_eq!(
        evidence.unset_paths,
        paths(["boolean-unset.bin", "nested/unset.bin"])
    );
    assert_eq!(
        evidence.unspecified_paths,
        paths([".gitattributes", "nested/unspecified.bin", "plain.bin",])
    );
    assert_shared_value_types(evidence);
    assert!(value_paths(evidence, "worktree").is_empty());
}

fn assert_index_b_types(evidence: &AttributeTypeEvidence) {
    assert_eq!(
        evidence.set_paths,
        paths(["boolean-set.bin", "nested/set.bin"])
    );
    assert_eq!(
        evidence.unset_paths,
        paths(["boolean-unset.bin", "nested/unset.bin", "source-switch.bin",])
    );
    assert_eq!(
        evidence.unspecified_paths,
        paths([".gitattributes", "nested/unspecified.bin", "plain.bin",])
    );
    assert_shared_value_types(evidence);
    assert!(value_paths(evidence, "worktree").is_empty());
}

fn assert_worktree_override_types(evidence: &AttributeTypeEvidence) {
    assert_eq!(
        evidence.set_paths,
        paths(["boolean-set.bin", "nested/set.bin"])
    );
    assert_eq!(
        evidence.unset_paths,
        paths(["boolean-unset.bin", "nested/unset.bin"])
    );
    assert_eq!(
        evidence.unspecified_paths,
        paths([".gitattributes", "nested/unspecified.bin", "plain.bin",])
    );
    assert_shared_value_types(evidence);
    assert_eq!(
        value_paths(evidence, "worktree"),
        paths(["source-switch.bin"])
    );
}

fn assert_shared_value_types(evidence: &AttributeTypeEvidence) {
    assert_eq!(value_paths(evidence, "canary"), paths(["canary.bin"]));
    assert_eq!(
        value_paths(evidence, "custom"),
        paths(["string-custom.bin"])
    );
    assert_eq!(
        value_paths(evidence, "macro"),
        paths(["macro.bin", "nested/macro.bin"])
    );
    assert_eq!(
        value_paths(evidence, "nested"),
        paths(["nested/.gitattributes", "nested/root-value.bin"])
    );
    assert_eq!(value_paths(evidence, "set"), paths(["string-set.bin"]));
    assert_eq!(value_paths(evidence, "unset"), paths(["string-unset.bin"]));
    assert_eq!(
        value_paths(evidence, "unspecified"),
        paths(["string-unspecified.bin"])
    );
}

fn value_paths(evidence: &AttributeTypeEvidence, value: &str) -> Vec<Vec<u8>> {
    evidence
        .value_paths
        .get(value)
        .unwrap_or_else(|| panic!("fixed value {value} must be queried"))
        .clone()
}

fn fixture_paths() -> Vec<&'static str> {
    vec![
        "boolean-set.bin",
        "boolean-unset.bin",
        "canary.bin",
        "macro.bin",
        "nested/macro.bin",
        "nested/root-value.bin",
        "nested/set.bin",
        "nested/unset.bin",
        "nested/unspecified.bin",
        "plain.bin",
        "source-switch.bin",
        "string-custom.bin",
        "string-set.bin",
        "string-unset.bin",
        "string-unspecified.bin",
    ]
}

fn all_paths() -> Vec<Vec<u8>> {
    let mut values = fixture_paths();
    values.extend([".gitattributes", "nested/.gitattributes"]);
    paths(values)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSnapshot {
    device: u64,
    inode: u64,
    mode: u32,
    bytes: Vec<u8>,
}

impl FileSnapshot {
    fn capture(path: &Path) -> Self {
        let metadata = fs::symlink_metadata(path).expect("stat tracked fixture file");
        assert!(metadata.file_type().is_file());
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            bytes: fs::read(path).expect("read tracked fixture file"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeSnapshot {
    device: u64,
    inode: u64,
    mode: u32,
    bytes: Option<Vec<u8>>,
}

fn snapshot_tree(root: &Path) -> BTreeMap<Vec<u8>, NodeSnapshot> {
    let mut snapshot = BTreeMap::new();
    snapshot_directory(root, root, &mut snapshot);
    snapshot
}

fn snapshot_directory(
    root: &Path,
    directory: &Path,
    snapshot: &mut BTreeMap<Vec<u8>, NodeSnapshot>,
) {
    let mut entries = fs::read_dir(directory)
        .expect("read controlled fixture directory")
        .map(|entry| entry.expect("read controlled fixture entry"))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.file_name()
            .as_bytes()
            .cmp(right.file_name().as_bytes())
    });
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).expect("stat controlled fixture entry");
        let relative = path
            .strip_prefix(root)
            .expect("entry must remain below controlled root")
            .as_os_str()
            .as_bytes()
            .to_vec();
        let bytes = if metadata.file_type().is_file() {
            Some(fs::read(&path).expect("read controlled fixture entry"))
        } else {
            assert!(metadata.file_type().is_dir());
            None
        };
        snapshot.insert(
            relative,
            NodeSnapshot {
                device: metadata.dev(),
                inode: metadata.ino(),
                mode: metadata.mode(),
                bytes,
            },
        );
        if metadata.file_type().is_dir() {
            snapshot_directory(root, &path, snapshot);
        }
    }
}

fn create_private_root() -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/p0-git-base-tests");
    fs::create_dir_all(&base).expect("create controlled test parent");
    let base = fs::canonicalize(base).expect("canonicalize controlled test parent");
    for _ in 0..100 {
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root = base.join(format!(
            "attribute-types-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        match fs::create_dir(&root) {
            Ok(()) => {
                println!("P0-03 preserved private fixture: {}", root.display());
                return root;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("create controlled root: {error}"),
        }
    }
    panic!("could not allocate a unique controlled root")
}

fn path_text(path: &Path) -> &str {
    let text = path.to_str().expect("controlled path is UTF-8");
    assert!(
        text.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte)),
        "controlled path must not need shell quoting"
    );
    text
}

fn paths<I, S>(values: I) -> Vec<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut values = values
        .into_iter()
        .map(|value| value.as_ref().as_bytes().to_vec())
        .collect::<Vec<_>>();
    values.sort();
    values
}

#![forbid(unsafe_code)]

mod path_output;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const GIT: &str = "/usr/bin/git";
const ATTRIBUTE: &str = "filter";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const FIXED_VALUES: [&str; 10] = [
    "canary",
    "custom",
    "global",
    "macro",
    "nested",
    "set",
    "system",
    "unset",
    "unspecified",
    "worktree",
];

/// Isolated filesystem inputs for the bounded P0 Git attribute experiment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeQueryContext {
    pub git_dir: PathBuf,
    pub work_tree: PathBuf,
    pub index_file: PathBuf,
    pub isolated_home: PathBuf,
    pub global_config: PathBuf,
    pub system_config: PathBuf,
}

/// Effective paths reported for each attribute type in the experiment fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeTypeEvidence {
    pub git_version: String,
    pub git_dir: PathBuf,
    pub work_tree: PathBuf,
    pub index_file: PathBuf,
    pub all_paths: Vec<Vec<u8>>,
    pub set_paths: Vec<Vec<u8>>,
    pub unset_paths: Vec<Vec<u8>>,
    pub unspecified_paths: Vec<Vec<u8>>,
    pub value_paths: BTreeMap<String, Vec<Vec<u8>>>,
}

/// The bounded termination evidence obtained after a child stops responding or cannot be polled.
#[derive(Debug)]
pub enum ChildTermination {
    /// The kill request succeeded and an exit status was subsequently collected.
    /// The status, rather than the successful kill request, identifies how the child exited.
    KilledAndReaped {
        status: ExitStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    KillFailedStateUnconfirmed {
        source: std::io::Error,
    },
    KilledButReapFailedStateUnconfirmed {
        source: std::io::Error,
    },
}

impl ChildTermination {
    fn source(&self) -> Option<&std::io::Error> {
        match self {
            Self::KilledAndReaped { .. } => None,
            Self::KillFailedStateUnconfirmed { source }
            | Self::KilledButReapFailedStateUnconfirmed { source } => Some(source),
        }
    }
}

impl fmt::Display for ChildTermination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KilledAndReaped { status, .. } => {
                write!(
                    formatter,
                    "kill request succeeded and child exit status was collected: {status}"
                )
            }
            Self::KillFailedStateUnconfirmed { source } => {
                write!(
                    formatter,
                    "kill failed and child state is unconfirmed: {source}"
                )
            }
            Self::KilledButReapFailedStateUnconfirmed { source } => {
                write!(
                    formatter,
                    "kill succeeded but reap failed, so child state is unconfirmed: {source}"
                )
            }
        }
    }
}

/// A structured failure from the bounded P0 Git attribute experiment.
#[derive(Debug)]
pub enum ExperimentError {
    Spawn {
        operation: &'static str,
        source: std::io::Error,
    },
    Poll {
        operation: &'static str,
        source: std::io::Error,
        termination: ChildTermination,
    },
    CollectOutput {
        operation: &'static str,
        status: ExitStatus,
        source: std::io::Error,
    },
    Timeout {
        operation: &'static str,
        termination: ChildTermination,
    },
    GitFailure {
        operation: &'static str,
        status: ExitStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    MalformedOutput {
        operation: &'static str,
        detail: &'static str,
    },
}

impl fmt::Display for ExperimentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { operation, source } => {
                write!(formatter, "{operation}: failed to spawn Git: {source}")
            }
            Self::Poll {
                operation,
                source,
                termination,
            } => {
                write!(
                    formatter,
                    "{operation}: failed while polling Git: {source}; {termination}"
                )
            }
            Self::CollectOutput {
                operation,
                status,
                source,
            } => write!(
                formatter,
                "{operation}: Git exited with {status}, but output collection failed: {source}"
            ),
            Self::Timeout {
                operation,
                termination,
            } => write!(formatter, "{operation}: Git timed out; {termination}"),
            Self::GitFailure {
                operation, status, ..
            } => write!(
                formatter,
                "{operation}: Git did not complete cleanly: {status}"
            ),
            Self::MalformedOutput { operation, detail } => {
                write!(formatter, "{operation}: malformed Git output: {detail}")
            }
        }
    }
}

impl std::error::Error for ExperimentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. }
            | Self::Poll { source, .. }
            | Self::CollectOutput { source, .. } => Some(source),
            Self::Timeout { termination, .. } => termination
                .source()
                .map(|source| source as &(dyn std::error::Error + 'static)),
            Self::GitFailure { .. } | Self::MalformedOutput { .. } => None,
        }
    }
}

/// Runs the first P0-03 attribute-type query against one controlled index.
pub fn inspect_attribute_types(
    context: &AttributeQueryContext,
) -> Result<AttributeTypeEvidence, ExperimentError> {
    let version_output = run_git(context, "git-version", ["--version"])?;
    let git_version = String::from_utf8(version_output)
        .map_err(|_| ExperimentError::MalformedOutput {
            operation: "git-version",
            detail: "version was not UTF-8",
        })?
        .trim()
        .to_owned();

    let all_paths = parse_path_list(run_git(
        context,
        "list-index",
        ["ls-files", "--cached", "-z"],
    )?)?;
    if all_paths.is_empty() {
        return Err(ExperimentError::MalformedOutput {
            operation: "list-index",
            detail: "the controlled index was empty",
        });
    }

    let set_paths = matching_paths(context, "match-set", ":(attr:filter)")?;
    let unset_paths = matching_paths(context, "match-unset", ":(attr:-filter)")?;
    let unspecified_paths = matching_paths(context, "match-unspecified", ":(attr:!filter)")?;
    let mut value_paths = BTreeMap::<String, Vec<Vec<u8>>>::new();
    for value in FIXED_VALUES {
        let pathspec = format!(":(attr:{ATTRIBUTE}={value})");
        value_paths.insert(
            value.to_owned(),
            matching_paths(context, "match-value", &pathspec)?,
        );
    }

    Ok(AttributeTypeEvidence {
        git_version,
        git_dir: context.git_dir.clone(),
        work_tree: context.work_tree.clone(),
        index_file: context.index_file.clone(),
        all_paths,
        set_paths,
        unset_paths,
        unspecified_paths,
        value_paths,
    })
}

fn matching_paths(
    context: &AttributeQueryContext,
    operation: &'static str,
    pathspec: &str,
) -> Result<Vec<Vec<u8>>, ExperimentError> {
    parse_path_list(run_git(
        context,
        operation,
        ["ls-files", "--cached", "-z", "--", pathspec],
    )?)
}

fn run_git<I, S>(
    context: &AttributeQueryContext,
    operation: &'static str,
    arguments: I,
) -> Result<Vec<u8>, ExperimentError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(GIT);
    command
        .env_clear()
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("HOME", &context.isolated_home)
        .env("XDG_CONFIG_HOME", context.isolated_home.join("xdg"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", &context.system_config)
        .env("GIT_CONFIG_GLOBAL", &context.global_config)
        .env("GIT_INDEX_FILE", &context.index_file)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .current_dir(&context.work_tree)
        .arg("-c")
        .arg("core.bare=false")
        .arg("-c")
        .arg("core.attributesFile=/dev/null")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("diff.external=")
        .arg("--git-dir")
        .arg(&context.git_dir)
        .arg("--work-tree")
        .arg(&context.work_tree)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = command
        .spawn()
        .map_err(|source| ExperimentError::Spawn { operation, source })?;
    let output = collect_bounded_output(child, operation, COMMAND_TIMEOUT)?;
    accept_git_output(operation, output)
}

fn collect_bounded_output(
    mut child: Child,
    operation: &'static str,
    timeout: Duration,
) -> Result<Output, ExperimentError> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return child
                    .wait_with_output()
                    .map_err(|source| ExperimentError::CollectOutput {
                        operation,
                        status,
                        source,
                    });
            }
            Ok(None) if is_before_deadline(Instant::now(), deadline) => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                return Err(ExperimentError::Timeout {
                    operation,
                    termination: terminate_child(child),
                });
            }
            Err(source) => {
                return Err(ExperimentError::Poll {
                    operation,
                    source,
                    termination: terminate_child(child),
                });
            }
        }
    }
}

fn is_before_deadline(now: Instant, deadline: Instant) -> bool {
    now < deadline
}

fn terminate_child(mut child: Child) -> ChildTermination {
    if let Err(source) = child.kill() {
        return ChildTermination::KillFailedStateUnconfirmed { source };
    }
    match child.wait_with_output() {
        Ok(output) => ChildTermination::KilledAndReaped {
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        },
        Err(source) => ChildTermination::KilledButReapFailedStateUnconfirmed { source },
    }
}

fn accept_git_output(operation: &'static str, output: Output) -> Result<Vec<u8>, ExperimentError> {
    if !output.status.success() || !output.stderr.is_empty() {
        return Err(ExperimentError::GitFailure {
            operation,
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        });
    }
    Ok(output.stdout)
}

fn parse_path_list(output: Vec<u8>) -> Result<Vec<Vec<u8>>, ExperimentError> {
    path_output::parse_path_list(output).map_err(|detail| ExperimentError::MalformedOutput {
        operation: "list-index",
        detail,
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Error, ErrorKind};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, ExitStatus, Stdio};
    use std::time::{Duration, Instant};

    use super::{
        ChildTermination, ExperimentError, accept_git_output, collect_bounded_output,
        is_before_deadline, parse_path_list,
    };

    fn source_kind(error: &ExperimentError) -> Option<ErrorKind> {
        std::error::Error::source(error)
            .and_then(|source| source.downcast_ref::<Error>())
            .map(Error::kind)
    }

    #[test]
    fn nul_path_list_accepts_empty_and_sorts_complete_fields() {
        assert_eq!(
            parse_path_list(Vec::new()).expect("empty path set"),
            Vec::<Vec<u8>>::new()
        );
        assert_eq!(
            parse_path_list(b"z\0a\0".to_vec()).expect("complete NUL fields"),
            vec![b"a".to_vec(), b"z".to_vec()]
        );
    }

    #[test]
    fn nul_path_list_preserves_non_utf8_bytes_and_duplicates() {
        assert_eq!(
            parse_path_list(vec![0xff, 0, b'a', 0, 0xff, 0])
                .expect("complete non-UTF-8 path fields"),
            vec![b"a".to_vec(), vec![0xff], vec![0xff]]
        );
    }

    #[test]
    fn nul_path_list_rejects_empty_fields_and_missing_terminator() {
        assert!(matches!(
            parse_path_list(b"\0".to_vec()),
            Err(ExperimentError::MalformedOutput { .. })
        ));
        assert!(matches!(
            parse_path_list(b"a\0\0b\0".to_vec()),
            Err(ExperimentError::MalformedOutput { .. })
        ));
        assert!(matches!(
            parse_path_list(b"a\0b".to_vec()),
            Err(ExperimentError::MalformedOutput { .. })
        ));
    }

    #[test]
    fn bounded_timeout_preserves_killed_child_status_and_output() {
        let mut command = Command::new("/bin/sleep");
        command
            .arg("5")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = command.spawn().expect("spawn controlled sleeping child");

        let error = collect_bounded_output(child, "controlled-timeout", Duration::from_millis(10))
            .expect_err("sleeping child must time out");
        match error {
            ExperimentError::Timeout {
                operation,
                termination:
                    ChildTermination::KilledAndReaped {
                        status,
                        stdout,
                        stderr,
                    },
            } => {
                assert_eq!(operation, "controlled-timeout");
                assert!(status.signal().is_some());
                assert!(stdout.is_empty());
                assert!(stderr.is_empty());
            }
            other => panic!("unexpected timeout evidence: {other:?}"),
        }
    }

    #[test]
    fn signalled_child_is_preserved_as_a_real_exit_status() {
        let mut command = Command::new("/bin/sleep");
        command
            .arg("5")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("spawn controlled sleeping child");
        child.kill().expect("kill controlled sleeping child");
        let output = collect_bounded_output(child, "controlled-signal", Duration::from_secs(1))
            .expect("collect signalled child");

        let error = accept_git_output("controlled-signal", output)
            .expect_err("a signalled process cannot be a clean Git result");
        match error {
            ExperimentError::GitFailure {
                operation,
                status,
                stdout,
                stderr,
            } => {
                assert_eq!(operation, "controlled-signal");
                assert!(status.signal().is_some());
                assert!(stdout.is_empty());
                assert!(stderr.is_empty());
            }
            other => panic!("unexpected signalled-child evidence: {other:?}"),
        }
    }

    #[test]
    fn deadline_boundary_is_strict() {
        let deadline = Instant::now();
        assert!(is_before_deadline(
            deadline - Duration::from_nanos(1),
            deadline
        ));
        assert!(!is_before_deadline(deadline, deadline));
        assert!(!is_before_deadline(
            deadline + Duration::from_nanos(1),
            deadline
        ));
    }

    #[test]
    fn child_termination_sources_and_diagnostics_preserve_the_variant() {
        let status = ExitStatus::from_raw(0);
        let status_text = status.to_string();
        let killed_and_reaped = ChildTermination::KilledAndReaped {
            status,
            stdout: b"stdout".to_vec(),
            stderr: b"stderr".to_vec(),
        };
        assert!(killed_and_reaped.source().is_none());
        assert_eq!(
            killed_and_reaped.to_string(),
            format!("kill request succeeded and child exit status was collected: {status_text}")
        );

        let kill_failed = ChildTermination::KillFailedStateUnconfirmed {
            source: Error::new(ErrorKind::PermissionDenied, "kill-denied"),
        };
        assert_eq!(
            kill_failed.source().map(Error::kind),
            Some(ErrorKind::PermissionDenied)
        );
        assert_eq!(
            kill_failed.to_string(),
            "kill failed and child state is unconfirmed: kill-denied"
        );

        let reap_failed = ChildTermination::KilledButReapFailedStateUnconfirmed {
            source: Error::new(ErrorKind::BrokenPipe, "reap-broke"),
        };
        assert_eq!(
            reap_failed.source().map(Error::kind),
            Some(ErrorKind::BrokenPipe)
        );
        assert_eq!(
            reap_failed.to_string(),
            "kill succeeded but reap failed, so child state is unconfirmed: reap-broke"
        );
    }

    #[test]
    fn experiment_error_sources_and_diagnostics_preserve_context() {
        let spawn = ExperimentError::Spawn {
            operation: "spawn-op",
            source: Error::new(ErrorKind::NotFound, "git-missing"),
        };
        assert_eq!(source_kind(&spawn), Some(ErrorKind::NotFound));
        assert_eq!(
            spawn.to_string(),
            "spawn-op: failed to spawn Git: git-missing"
        );

        let status = ExitStatus::from_raw(0);
        let status_text = status.to_string();
        let poll = ExperimentError::Poll {
            operation: "poll-op",
            source: Error::new(ErrorKind::Interrupted, "poll-interrupted"),
            termination: ChildTermination::KilledAndReaped {
                status,
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
        };
        assert_eq!(source_kind(&poll), Some(ErrorKind::Interrupted));
        assert_eq!(
            poll.to_string(),
            format!(
                "poll-op: failed while polling Git: poll-interrupted; kill request succeeded and child exit status was collected: {status_text}"
            )
        );

        let status = ExitStatus::from_raw(0);
        let status_text = status.to_string();
        let collect = ExperimentError::CollectOutput {
            operation: "collect-op",
            status,
            source: Error::new(ErrorKind::UnexpectedEof, "output-ended"),
        };
        assert_eq!(source_kind(&collect), Some(ErrorKind::UnexpectedEof));
        assert_eq!(
            collect.to_string(),
            format!(
                "collect-op: Git exited with {status_text}, but output collection failed: output-ended"
            )
        );

        let timeout = ExperimentError::Timeout {
            operation: "timeout-op",
            termination: ChildTermination::KillFailedStateUnconfirmed {
                source: Error::new(ErrorKind::PermissionDenied, "kill-denied"),
            },
        };
        assert_eq!(source_kind(&timeout), Some(ErrorKind::PermissionDenied));
        assert_eq!(
            timeout.to_string(),
            "timeout-op: Git timed out; kill failed and child state is unconfirmed: kill-denied"
        );

        let status = ExitStatus::from_raw(1 << 8);
        let status_text = status.to_string();
        let git_failure = ExperimentError::GitFailure {
            operation: "git-op",
            status,
            stdout: b"stdout".to_vec(),
            stderr: b"stderr".to_vec(),
        };
        assert_eq!(source_kind(&git_failure), None);
        assert_eq!(
            git_failure.to_string(),
            format!("git-op: Git did not complete cleanly: {status_text}")
        );

        let malformed = ExperimentError::MalformedOutput {
            operation: "parse-op",
            detail: "bad-fields",
        };
        assert_eq!(source_kind(&malformed), None);
        assert_eq!(
            malformed.to_string(),
            "parse-op: malformed Git output: bad-fields"
        );
    }
}

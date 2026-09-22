//! Bounded collection for the fixed internal Git query shapes used by P0-07.
//!
//! A successful [`run`] means only that the direct child exited and both output
//! pipes closed within their bounds. Callers must inspect [`GitExit`] and must
//! not interpret successful collection as a semantically successful or complete
//! Git inspection. [`inspect`] is the sole entry that connects bounded no-follow
//! discovery, source and repository preflight, index-flag checks, leaf-first
//! status queries, evidence revalidation, and the existing aggregate model.
//! Neither entry is a Phase 1 Port or a sandbox for user commands.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::os::fd::AsFd;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rustix::fs::{FileType, OFlags};

mod inspect;

#[cfg(fuzzing)]
pub use inspect::exercise_pure_parsers;
pub use inspect::{GitInspection, InspectionIssue, RepositoryInspection, inspect};

const GIT_EXECUTABLE: &str = "/usr/bin/git";
const RUN_TIMEOUT: Duration = Duration::from_secs(5);
const CLEANUP_TIMEOUT: Duration = Duration::from_millis(500);
const POLL_INTERVAL: Duration = Duration::from_millis(5);
const STDOUT_LIMIT: usize = 8 * 1024 * 1024;
const STDERR_LIMIT: usize = 256 * 1024;
const MAX_DRAIN_READS: usize = 16;

/// One of the only Git command shapes accepted by this experiment.
pub enum GitQuery<'a> {
    /// Query the fixed system Git version.
    Version,
    /// Query the fixed system Git's runtime executable directory.
    ExecPath,
    /// Read exactly one caller-prevalidated config file without includes.
    Config { file: &'a Path },
    /// Read tracked status with command-scoped guards for prevalidated filters.
    TrackedStatus { filter_drivers: &'a [OsString] },
    /// Read stable index flags without changing them.
    IndexFlags,
}

/// The direct child's real termination status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GitExit {
    /// The exit code, or `None` if the process ended by signal.
    pub code: Option<i32>,
    /// The terminating Unix signal, or `None` for a normal exit.
    pub signal: Option<i32>,
}

impl GitExit {
    /// Whether the direct child reported a zero exit code.
    pub fn success(self) -> bool {
        self.code == Some(0)
    }
}

/// Fully collected, separately bounded process output.
pub struct GitQueryOutput {
    /// The direct child's real termination status, including nonzero exits.
    pub exit: GitExit,
    /// At most 8 MiB of stdout; the raw bytes are intentionally omitted from Debug.
    pub stdout: Vec<u8>,
    /// At most 256 KiB of stderr; the raw bytes are intentionally omitted from Debug.
    pub stderr: Vec<u8>,
}

impl fmt::Debug for GitQueryOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GitQueryOutput")
            .field("exit", &self.exit)
            .field("stdout_len", &self.stdout.len())
            .field("stderr_len", &self.stderr.len())
            .finish()
    }
}

/// Output stream whose byte budget was exceeded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// Step at which bounded process I/O failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoOperation {
    ConfigureStdout,
    ConfigureStderr,
    ReadStdout,
    ReadStderr,
    PollExit,
}

/// Public input field rejected before a child was started.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputField {
    Cwd,
    ConfigFile,
    ConfigIsolationDevice,
}

/// Structured reason why output collection did not complete.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitQueryFailureKind {
    InvalidInput {
        field: InputField,
    },
    Start {
        error_kind: io::ErrorKind,
        raw_os_error: Option<i32>,
    },
    Io {
        operation: IoOperation,
        error_kind: io::ErrorKind,
        raw_os_error: Option<i32>,
    },
    TimedOut,
    OutputLimitExceeded {
        stream: OutputStream,
        limit: usize,
    },
}

/// What was confirmed about the directly spawned child during cleanup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectChildExit {
    NotStarted,
    Confirmed(GitExit),
    Unconfirmed,
}

/// Cleanup operation whose first I/O error was retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupOperation {
    InitialPoll,
    Kill,
    ConfirmPoll,
}

/// First direct-child cleanup error, independent of exit confirmation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CleanupIoFailure {
    pub operation: CleanupOperation,
    pub error_kind: io::ErrorKind,
    pub raw_os_error: Option<i32>,
}

/// Bounded bytes and direct-child cleanup evidence for a failed collection.
pub struct GitQueryFailure {
    /// The collection stage and structured failure reason.
    pub kind: GitQueryFailureKind,
    /// The retained stdout prefix, never longer than its configured limit.
    pub stdout: Vec<u8>,
    /// The retained stderr prefix, never longer than its configured limit.
    pub stderr: Vec<u8>,
    /// Whether exit of the directly spawned child was actually confirmed.
    pub direct_child_exit: DirectChildExit,
    /// First cleanup I/O error, without replacing the primary failure reason.
    pub cleanup_io: Option<CleanupIoFailure>,
}

impl fmt::Debug for GitQueryFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GitQueryFailure")
            .field("kind", &self.kind)
            .field("stdout_len", &self.stdout.len())
            .field("stderr_len", &self.stderr.len())
            .field("direct_child_exit", &self.direct_child_exit)
            .field("cleanup_io", &self.cleanup_io)
            .finish()
    }
}

impl fmt::Display for GitQueryFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "bounded Git query failed: {:?}", self.kind)
    }
}

impl std::error::Error for GitQueryFailure {}

#[derive(Clone, Copy)]
struct Budget {
    run_timeout: Duration,
    cleanup_timeout: Duration,
    poll_interval: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
}

const PRODUCTION_BUDGET: Budget = Budget {
    run_timeout: RUN_TIMEOUT,
    cleanup_timeout: CLEANUP_TIMEOUT,
    poll_interval: POLL_INTERVAL,
    stdout_limit: STDOUT_LIMIT,
    stderr_limit: STDERR_LIMIT,
};

/// Run one fixed Git query with a caller-supplied, explicit working directory.
pub fn run(cwd: &Path, query: GitQuery<'_>) -> Result<GitQueryOutput, GitQueryFailure> {
    run_bootstrap(cwd, query, RUN_TIMEOUT)
}

pub(super) fn run_bootstrap(
    cwd: &Path,
    query: GitQuery<'_>,
    run_timeout: Duration,
) -> Result<GitQueryOutput, GitQueryFailure> {
    validate_inputs(cwd, &query)?;
    let command = git_command(cwd, query, QueryEnvironment::Isolated, None);
    collect_command(
        command,
        Budget {
            run_timeout,
            ..PRODUCTION_BUDGET
        },
    )
}

pub(super) struct PreservedGitEnvironment<'a> {
    pub home: Option<&'a OsStr>,
    pub xdg_config_home: Option<&'a OsStr>,
    pub git_config_nosystem: Option<&'a OsStr>,
    pub git_config_system: Option<&'a OsStr>,
    pub git_config_global: Option<&'a OsStr>,
    pub git_attr_nosystem: Option<&'a OsStr>,
}

pub(super) fn run_prevalidated_repository(
    cwd: &Path,
    git_dir: &Path,
    query: GitQuery<'_>,
    environment: PreservedGitEnvironment<'_>,
    run_timeout: Duration,
) -> Result<GitQueryOutput, GitQueryFailure> {
    validate_inputs(cwd, &query)?;
    let command = git_command(
        cwd,
        query,
        QueryEnvironment::Preserved(environment),
        Some(git_dir),
    );
    collect_command(
        command,
        Budget {
            run_timeout,
            ..PRODUCTION_BUDGET
        },
    )
}

fn validate_inputs(cwd: &Path, query: &GitQuery<'_>) -> Result<(), GitQueryFailure> {
    if !cwd.is_absolute() {
        return Err(input_failure(InputField::Cwd));
    }
    if let GitQuery::Config { file } = query
        && !file.is_absolute()
    {
        return Err(input_failure(InputField::ConfigFile));
    }
    if matches!(query, GitQuery::Config { .. }) {
        let null = rustix::fs::statat(
            rustix::fs::CWD,
            "/dev/null",
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(|_| input_failure(InputField::ConfigIsolationDevice))?;
        if !FileType::from_raw_mode(null.st_mode).is_char_device() {
            return Err(input_failure(InputField::ConfigIsolationDevice));
        }
    }
    Ok(())
}

enum QueryEnvironment<'a> {
    Isolated,
    Preserved(PreservedGitEnvironment<'a>),
}

fn git_command(
    cwd: &Path,
    query: GitQuery<'_>,
    environment: QueryEnvironment<'_>,
    prevalidated_git_dir: Option<&Path>,
) -> Command {
    const BASE_CONFIG_COUNT: usize = 4;

    let mut command = Command::new(GIT_EXECUTABLE);
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_ASKPASS", "/usr/bin/false")
        .env("SSH_ASKPASS", "/usr/bin/false")
        .env("GIT_EDITOR", "/usr/bin/false")
        .env("GIT_SEQUENCE_EDITOR", "/usr/bin/false")
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_EXTERNAL_DIFF", "/usr/bin/false")
        .env("GIT_CONFIG_KEY_0", "core.fsmonitor")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env("GIT_CONFIG_KEY_1", "core.hooksPath")
        .env("GIT_CONFIG_VALUE_1", "/dev/null")
        .env("GIT_CONFIG_KEY_2", "core.pager")
        .env("GIT_CONFIG_VALUE_2", "cat")
        .env("GIT_CONFIG_KEY_3", "color.ui")
        .env("GIT_CONFIG_VALUE_3", "false");

    let filter_drivers = match &query {
        GitQuery::TrackedStatus { filter_drivers } => *filter_drivers,
        _ => &[],
    };
    command.env(
        "GIT_CONFIG_COUNT",
        (BASE_CONFIG_COUNT + filter_drivers.len() * 3).to_string(),
    );
    for (driver_index, driver) in filter_drivers.iter().enumerate() {
        for (field_index, (field, value)) in [("clean", ""), ("process", ""), ("required", "true")]
            .into_iter()
            .enumerate()
        {
            let index = BASE_CONFIG_COUNT + driver_index * 3 + field_index;
            let mut key = OsString::from("filter.");
            key.push(driver);
            key.push(".");
            key.push(field);
            command
                .env(format!("GIT_CONFIG_KEY_{index}"), key)
                .env(format!("GIT_CONFIG_VALUE_{index}"), value);
        }
    }

    match environment {
        QueryEnvironment::Isolated => {
            command
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_ATTR_NOSYSTEM", "1");
        }
        QueryEnvironment::Preserved(environment) => {
            set_optional_env(&mut command, "HOME", environment.home);
            set_optional_env(&mut command, "XDG_CONFIG_HOME", environment.xdg_config_home);
            set_optional_env(
                &mut command,
                "GIT_CONFIG_NOSYSTEM",
                environment.git_config_nosystem,
            );
            set_optional_env(
                &mut command,
                "GIT_CONFIG_SYSTEM",
                environment.git_config_system,
            );
            set_optional_env(
                &mut command,
                "GIT_CONFIG_GLOBAL",
                environment.git_config_global,
            );
            set_optional_env(
                &mut command,
                "GIT_ATTR_NOSYSTEM",
                environment.git_attr_nosystem,
            );
        }
    }

    match query {
        GitQuery::Version => {
            command.arg("--version");
        }
        GitQuery::ExecPath => {
            command.arg("--exec-path");
        }
        GitQuery::Config { file } => {
            command.arg("--git-dir=/dev/null");
            command.args(["config", "--file"]);
            command.arg(file);
            command.args(["--no-includes", "--null", "--list"]);
        }
        GitQuery::TrackedStatus { .. } => {
            command.arg(git_dir_argument(cwd, prevalidated_git_dir));
            command.args([
                "--no-optional-locks",
                "status",
                "--porcelain=v2",
                "-z",
                "--untracked-files=no",
                "--ignore-submodules=untracked",
            ]);
        }
        GitQuery::IndexFlags => {
            command.arg(git_dir_argument(cwd, prevalidated_git_dir));
            command.args(["--no-optional-locks", "ls-files", "-v", "-z"]);
        }
    }

    command
}

fn set_optional_env(command: &mut Command, key: &str, value: Option<&OsStr>) {
    if let Some(value) = value {
        command.env(key, value);
    }
}

fn git_dir_argument(cwd: &Path, prevalidated_git_dir: Option<&Path>) -> OsString {
    let mut argument = OsString::from("--git-dir=");
    match prevalidated_git_dir {
        Some(git_dir) => argument.push(git_dir),
        None => argument.push(cwd.join(".git")),
    }
    argument
}

fn collect_command(
    mut command: Command,
    budget: Budget,
) -> Result<GitQueryOutput, GitQueryFailure> {
    let started = Instant::now();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(start_failure)?;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    if let Some(pipe) = stdout_pipe.as_ref()
        && let Err(error) = set_nonblocking(pipe)
    {
        return Err(fail_with_io(
            &mut child,
            None,
            IoOperation::ConfigureStdout,
            error,
            stdout,
            stderr,
            budget,
        ));
    }
    if let Some(pipe) = stderr_pipe.as_ref()
        && let Err(error) = set_nonblocking(pipe)
    {
        return Err(fail_with_io(
            &mut child,
            None,
            IoOperation::ConfigureStderr,
            error,
            stdout,
            stderr,
            budget,
        ));
    }

    let deadline = started + budget.run_timeout;
    let mut exit_status = None;
    loop {
        match drain_pipe(&mut stdout_pipe, &mut stdout, budget.stdout_limit) {
            Ok(DrainResult::Open | DrainResult::Closed) => {}
            Ok(DrainResult::LimitExceeded) => {
                return Err(fail_after_child(
                    &mut child,
                    exit_status,
                    GitQueryFailureKind::OutputLimitExceeded {
                        stream: OutputStream::Stdout,
                        limit: budget.stdout_limit,
                    },
                    stdout,
                    stderr,
                    budget,
                ));
            }
            Err(error) => {
                return Err(fail_with_io(
                    &mut child,
                    exit_status,
                    IoOperation::ReadStdout,
                    error,
                    stdout,
                    stderr,
                    budget,
                ));
            }
        }
        match drain_pipe(&mut stderr_pipe, &mut stderr, budget.stderr_limit) {
            Ok(DrainResult::Open | DrainResult::Closed) => {}
            Ok(DrainResult::LimitExceeded) => {
                return Err(fail_after_child(
                    &mut child,
                    exit_status,
                    GitQueryFailureKind::OutputLimitExceeded {
                        stream: OutputStream::Stderr,
                        limit: budget.stderr_limit,
                    },
                    stdout,
                    stderr,
                    budget,
                ));
            }
            Err(error) => {
                return Err(fail_with_io(
                    &mut child,
                    exit_status,
                    IoOperation::ReadStderr,
                    error,
                    stdout,
                    stderr,
                    budget,
                ));
            }
        }

        if exit_status.is_none() {
            match child.try_wait() {
                Ok(status) => exit_status = status,
                Err(error) => {
                    return Err(fail_with_io(
                        &mut child,
                        None,
                        IoOperation::PollExit,
                        error,
                        stdout,
                        stderr,
                        budget,
                    ));
                }
            }
        }

        let now = Instant::now();
        if now >= deadline {
            return Err(fail_after_child(
                &mut child,
                exit_status,
                GitQueryFailureKind::TimedOut,
                stdout,
                stderr,
                budget,
            ));
        }
        if stdout_pipe.is_none()
            && stderr_pipe.is_none()
            && let Some(status) = exit_status
        {
            return Ok(GitQueryOutput {
                exit: git_exit(status),
                stdout,
                stderr,
            });
        }
        thread::sleep(budget.poll_interval.min(deadline - now));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DrainResult {
    Open,
    Closed,
    LimitExceeded,
}

fn set_nonblocking(fd: &impl AsFd) -> io::Result<()> {
    let flags = rustix::fs::fcntl_getfl(fd).map_err(io::Error::from)?;
    rustix::fs::fcntl_setfl(fd, flags | OFlags::NONBLOCK).map_err(io::Error::from)
}

fn drain_pipe<Fd: AsFd>(
    pipe: &mut Option<Fd>,
    output: &mut Vec<u8>,
    limit: usize,
) -> io::Result<DrainResult> {
    let mut buffer = [0_u8; 8192];
    for _ in 0..MAX_DRAIN_READS {
        let remaining = limit.saturating_sub(output.len());
        let read_length = remaining.saturating_add(1).min(buffer.len());
        let result = {
            let Some(fd) = pipe.as_ref() else {
                return Ok(DrainResult::Closed);
            };
            rustix::io::read(fd, &mut buffer[..read_length])
        };

        match result {
            Ok(0) => {
                *pipe = None;
                return Ok(DrainResult::Closed);
            }
            Ok(read) => {
                let retained = read.min(remaining);
                output.extend_from_slice(&buffer[..retained]);
                if read > remaining {
                    return Ok(DrainResult::LimitExceeded);
                }
            }
            Err(rustix::io::Errno::INTR) => {}
            Err(rustix::io::Errno::AGAIN) => return Ok(DrainResult::Open),
            Err(error) => return Err(io::Error::from(error)),
        }
    }
    Ok(DrainResult::Open)
}

fn start_failure(error: io::Error) -> GitQueryFailure {
    GitQueryFailure {
        kind: GitQueryFailureKind::Start {
            error_kind: error.kind(),
            raw_os_error: error.raw_os_error(),
        },
        stdout: Vec::new(),
        stderr: Vec::new(),
        direct_child_exit: DirectChildExit::NotStarted,
        cleanup_io: None,
    }
}

fn input_failure(field: InputField) -> GitQueryFailure {
    GitQueryFailure {
        kind: GitQueryFailureKind::InvalidInput { field },
        stdout: Vec::new(),
        stderr: Vec::new(),
        direct_child_exit: DirectChildExit::NotStarted,
        cleanup_io: None,
    }
}

fn fail_with_io(
    child: &mut Child,
    status: Option<ExitStatus>,
    operation: IoOperation,
    error: io::Error,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    budget: Budget,
) -> GitQueryFailure {
    fail_after_child(
        child,
        status,
        GitQueryFailureKind::Io {
            operation,
            error_kind: error.kind(),
            raw_os_error: error.raw_os_error(),
        },
        stdout,
        stderr,
        budget,
    )
}

fn fail_after_child(
    child: &mut Child,
    status: Option<ExitStatus>,
    kind: GitQueryFailureKind,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    budget: Budget,
) -> GitQueryFailure {
    let cleanup = cleanup_direct_child(child, status, budget);
    GitQueryFailure {
        kind,
        stdout,
        stderr,
        direct_child_exit: cleanup.exit,
        cleanup_io: cleanup.io_failure,
    }
}

struct CleanupEvidence {
    exit: DirectChildExit,
    io_failure: Option<CleanupIoFailure>,
}

fn cleanup_direct_child(
    child: &mut Child,
    status: Option<ExitStatus>,
    budget: Budget,
) -> CleanupEvidence {
    if let Some(status) = status {
        return CleanupEvidence {
            exit: DirectChildExit::Confirmed(git_exit(status)),
            io_failure: None,
        };
    }

    let mut io_failure = None;
    match child.try_wait() {
        Ok(Some(status)) => {
            return CleanupEvidence {
                exit: DirectChildExit::Confirmed(git_exit(status)),
                io_failure,
            };
        }
        Ok(None) => {}
        Err(error) => {
            retain_cleanup_io_error(&mut io_failure, CleanupOperation::InitialPoll, error)
        }
    }

    // This targets only the directly spawned child. P0-07 does not create a
    // process group or claim that descendants were terminated.
    if let Err(error) = child.kill() {
        retain_cleanup_io_error(&mut io_failure, CleanupOperation::Kill, error);
    }
    let deadline = Instant::now() + budget.cleanup_timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return CleanupEvidence {
                    exit: DirectChildExit::Confirmed(git_exit(status)),
                    io_failure,
                };
            }
            Ok(None) => {}
            Err(error) => {
                retain_cleanup_io_error(&mut io_failure, CleanupOperation::ConfirmPoll, error)
            }
        }
        let now = Instant::now();
        if now >= deadline {
            return CleanupEvidence {
                exit: classify_direct_child_exit(None),
                io_failure,
            };
        }
        thread::sleep(budget.poll_interval.min(deadline - now));
    }
}

fn retain_cleanup_io_error(
    retained: &mut Option<CleanupIoFailure>,
    operation: CleanupOperation,
    error: io::Error,
) {
    if retained.is_none() {
        *retained = Some(CleanupIoFailure {
            operation,
            error_kind: error.kind(),
            raw_os_error: error.raw_os_error(),
        });
    }
}

fn classify_direct_child_exit(status: Option<ExitStatus>) -> DirectChildExit {
    status.map_or(DirectChildExit::Unconfirmed, |status| {
        DirectChildExit::Confirmed(git_exit(status))
    })
}

fn git_exit(status: ExitStatus) -> GitExit {
    GitExit {
        code: status.code(),
        #[cfg(unix)]
        signal: status.signal(),
        #[cfg(not(unix))]
        signal: None,
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::io::{self, Write};
    use std::net::Shutdown;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    #[cfg(unix)]
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use super::*;

    const HELPER_MODE: &str = "THINWS_P0_07_GIT_QUERY_HELPER";
    static NEXT_TIMEOUT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    fn test_budget(stdout_limit: usize, stderr_limit: usize, run_ms: u64) -> Budget {
        Budget {
            run_timeout: Duration::from_millis(run_ms),
            cleanup_timeout: Duration::from_millis(500),
            poll_interval: Duration::from_millis(5),
            stdout_limit,
            stderr_limit,
        }
    }

    fn helper_command(mode: &str) -> Command {
        let mut command = Command::new(env::current_exe().expect("current unit-test binary"));
        command
            .args(["--exact", "git_query::tests::command_helper", "--nocapture"])
            .env(HELPER_MODE, mode);
        command
    }

    fn retained_fifo() -> PathBuf {
        let unique = NEXT_TIMEOUT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("p0-07-git-query-timeout-fixtures")
            .join(format!("{}-{nanos}-{unique}", process::id()));
        fs::create_dir_all(&directory).expect("create retained timeout fixture");
        let fifo = directory.join("config.fifo");
        let output = Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .env_clear()
            .output()
            .expect("create controlled FIFO");
        assert!(output.status.success(), "mkfifo must succeed");
        fifo
    }

    #[test]
    fn fixed_git_version_query_collects_real_output() {
        let output = run(Path::new("/"), GitQuery::Version).expect("fixed Git version query");
        assert!(output.exit.success());
        assert!(output.stdout.starts_with(b"git version "));
        assert!(output.stderr.is_empty());
    }

    #[test]
    fn production_budget_has_the_frozen_independent_limits() {
        assert_eq!(PRODUCTION_BUDGET.run_timeout, Duration::from_secs(5));
        assert_eq!(
            PRODUCTION_BUDGET.cleanup_timeout,
            Duration::from_millis(500)
        );
        assert_eq!(PRODUCTION_BUDGET.poll_interval, Duration::from_millis(5));
        assert_eq!(PRODUCTION_BUDGET.stdout_limit, 8_388_608);
        assert_eq!(PRODUCTION_BUDGET.stderr_limit, 262_144);
    }

    #[test]
    fn wrapper_queries_propagate_the_supplied_run_timeout() {
        let (reader, _writer) = UnixStream::pair().expect("owned preflight socket pair");
        set_nonblocking(&reader).expect("configure preflight socket");
        assert!(
            rustix::fs::fcntl_getfl(&reader)
                .expect("read preflight socket flags")
                .contains(OFlags::NONBLOCK)
        );

        let fifo = retained_fifo();
        let cwd = fifo.parent().expect("fixture directory");
        let timeout = Duration::from_millis(40);

        let started = Instant::now();
        let bootstrap = run_bootstrap(cwd, GitQuery::Config { file: &fifo }, timeout)
            .expect_err("bootstrap config reader must honor the short timeout");
        assert_eq!(bootstrap.kind, GitQueryFailureKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));

        let started = Instant::now();
        let prevalidated = run_prevalidated_repository(
            cwd,
            &cwd.join(".git"),
            GitQuery::Config { file: &fifo },
            PreservedGitEnvironment {
                home: None,
                xdg_config_home: None,
                git_config_nosystem: None,
                git_config_system: None,
                git_config_global: None,
                git_attr_nosystem: None,
            },
            timeout,
        )
        .expect_err("prevalidated config reader must honor the short timeout");
        assert_eq!(prevalidated.kind, GitQueryFailureKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn success_rejects_nonzero_and_signal_termination() {
        assert!(
            GitExit {
                code: Some(0),
                signal: None
            }
            .success()
        );
        assert!(
            !GitExit {
                code: Some(23),
                signal: None
            }
            .success()
        );
        assert!(
            !GitExit {
                code: None,
                signal: Some(9)
            }
            .success()
        );
    }

    #[test]
    fn diagnostics_keep_structure_without_raw_output() {
        let stdout_canary = b"raw-stdout-canary".to_vec();
        let stderr_canary = b"raw-stderr-canary".to_vec();
        let stdout_byte_debug = format!("{stdout_canary:?}");
        let stderr_byte_debug = format!("{stderr_canary:?}");
        let output = GitQueryOutput {
            exit: GitExit {
                code: Some(23),
                signal: None,
            },
            stdout: stdout_canary.clone(),
            stderr: stderr_canary.clone(),
        };
        let output_debug = format!("{output:?}");
        assert!(output_debug.contains("GitQueryOutput"));
        assert!(output_debug.contains("code: Some(23)"));
        assert!(output_debug.contains(&format!("stdout_len: {}", stdout_canary.len())));
        assert!(output_debug.contains(&format!("stderr_len: {}", stderr_canary.len())));
        assert!(!output_debug.contains("raw-stdout-canary"));
        assert!(!output_debug.contains("raw-stderr-canary"));
        assert!(!output_debug.contains(&stdout_byte_debug));
        assert!(!output_debug.contains(&stderr_byte_debug));

        let failure = GitQueryFailure {
            kind: GitQueryFailureKind::TimedOut,
            stdout: stdout_canary,
            stderr: stderr_canary,
            direct_child_exit: DirectChildExit::Unconfirmed,
            cleanup_io: Some(CleanupIoFailure {
                operation: CleanupOperation::Kill,
                error_kind: io::ErrorKind::PermissionDenied,
                raw_os_error: Some(13),
            }),
        };
        let failure_debug = format!("{failure:?}");
        assert!(failure_debug.contains("GitQueryFailure"));
        assert!(failure_debug.contains("TimedOut"));
        assert!(failure_debug.contains("direct_child_exit: Unconfirmed"));
        assert!(failure_debug.contains("operation: Kill"));
        assert!(!failure_debug.contains("raw-stdout-canary"));
        assert!(!failure_debug.contains("raw-stderr-canary"));
        assert!(!failure_debug.contains(&stdout_byte_debug));
        assert!(!failure_debug.contains(&stderr_byte_debug));

        let failure_display = failure.to_string();
        assert!(failure_display.contains("bounded Git query failed"));
        assert!(failure_display.contains("TimedOut"));
        assert!(!failure_display.contains("raw-stdout-canary"));
        assert!(!failure_display.contains("raw-stderr-canary"));
        assert!(!failure_display.contains(&stdout_byte_debug));
        assert!(!failure_display.contains(&stderr_byte_debug));
    }

    #[cfg(unix)]
    #[test]
    fn setting_nonblocking_twice_preserves_the_enabled_flag() {
        let (reader, _writer) = UnixStream::pair().expect("owned socket pair");

        set_nonblocking(&reader).expect("first nonblocking configuration");
        let first = rustix::fs::fcntl_getfl(&reader).expect("first status flags");
        assert!(first.contains(OFlags::NONBLOCK));
        set_nonblocking(&reader).expect("repeated nonblocking configuration");
        let second = rustix::fs::fcntl_getfl(&reader).expect("repeated status flags");
        assert_eq!(second, first);
        assert!(second.contains(OFlags::NONBLOCK));
    }

    #[cfg(unix)]
    #[test]
    fn drain_accepts_exact_limit_and_rejects_only_the_next_byte() {
        const LIMIT: usize = 16;

        let (exact_reader, mut exact_writer) = UnixStream::pair().expect("exact socket pair");
        set_nonblocking(&exact_reader).expect("exact reader nonblocking");
        exact_writer
            .write_all(b"exactly-16-bytes")
            .expect("exact bytes");
        exact_writer
            .shutdown(Shutdown::Write)
            .expect("close exact writer");
        let mut exact_pipe = Some(exact_reader);
        let mut exact_output = Vec::new();
        assert_eq!(
            drain_pipe(&mut exact_pipe, &mut exact_output, LIMIT).expect("drain exact bytes"),
            DrainResult::Closed
        );
        assert_eq!(exact_output, b"exactly-16-bytes");

        let (over_reader, mut over_writer) = UnixStream::pair().expect("over-limit socket pair");
        set_nonblocking(&over_reader).expect("over-limit reader nonblocking");
        over_writer
            .write_all(b"exactly-16-bytes+")
            .expect("over-limit bytes");
        over_writer
            .shutdown(Shutdown::Write)
            .expect("close over-limit writer");
        let mut over_pipe = Some(over_reader);
        let mut over_output = Vec::new();
        assert_eq!(
            drain_pipe(&mut over_pipe, &mut over_output, LIMIT).expect("drain over-limit bytes"),
            DrainResult::LimitExceeded
        );
        assert_eq!(over_output, b"exactly-16-bytes");
    }

    #[test]
    fn fixed_query_shapes_include_explicit_git_dir_and_exact_arguments() {
        let cwd = Path::new("/tmp/thinws-p0-07-query-cwd");
        let config = Path::new("/tmp/thinws-p0-07-query-config");
        let cases = [
            (GitQuery::Version, vec![OsStr::new("--version")]),
            (GitQuery::ExecPath, vec![OsStr::new("--exec-path")]),
            (
                GitQuery::Config { file: config },
                vec![
                    OsStr::new("--git-dir=/dev/null"),
                    OsStr::new("config"),
                    OsStr::new("--file"),
                    config.as_os_str(),
                    OsStr::new("--no-includes"),
                    OsStr::new("--null"),
                    OsStr::new("--list"),
                ],
            ),
            (
                GitQuery::TrackedStatus {
                    filter_drivers: &[],
                },
                vec![
                    OsStr::new("--git-dir=/tmp/thinws-p0-07-query-cwd/.git"),
                    OsStr::new("--no-optional-locks"),
                    OsStr::new("status"),
                    OsStr::new("--porcelain=v2"),
                    OsStr::new("-z"),
                    OsStr::new("--untracked-files=no"),
                    OsStr::new("--ignore-submodules=untracked"),
                ],
            ),
            (
                GitQuery::IndexFlags,
                vec![
                    OsStr::new("--git-dir=/tmp/thinws-p0-07-query-cwd/.git"),
                    OsStr::new("--no-optional-locks"),
                    OsStr::new("ls-files"),
                    OsStr::new("-v"),
                    OsStr::new("-z"),
                ],
            ),
        ];

        for (query, expected) in cases {
            let command = git_command(cwd, query, QueryEnvironment::Isolated, None);
            assert_eq!(command.get_program(), OsStr::new(GIT_EXECUTABLE));
            assert_eq!(command.get_current_dir(), Some(cwd));
            assert_eq!(command.get_args().collect::<Vec<_>>(), expected);
            let environment = command.get_envs().collect::<Vec<_>>();
            assert!(
                environment.contains(&(OsStr::new("GIT_ATTR_NOSYSTEM"), Some(OsStr::new("1"))))
            );
            assert!(environment.contains(&(OsStr::new("PATH"), Some(OsStr::new("/usr/bin:/bin")))));
        }
    }

    #[test]
    fn preserved_query_environment_is_forwarded_exactly() {
        let cwd = Path::new("/tmp/thinws-p0-07-query-cwd");
        let command = git_command(
            cwd,
            GitQuery::Version,
            QueryEnvironment::Preserved(PreservedGitEnvironment {
                home: Some(OsStr::new("/controlled/home")),
                xdg_config_home: Some(OsStr::new("/controlled/xdg")),
                git_config_nosystem: Some(OsStr::new("true")),
                git_config_system: Some(OsStr::new("/controlled/system")),
                git_config_global: Some(OsStr::new("/controlled/global")),
                git_attr_nosystem: Some(OsStr::new("yes")),
            }),
            None,
        );
        let environment = command.get_envs().collect::<Vec<_>>();

        for (key, value) in [
            ("HOME", "/controlled/home"),
            ("XDG_CONFIG_HOME", "/controlled/xdg"),
            ("GIT_CONFIG_NOSYSTEM", "true"),
            ("GIT_CONFIG_SYSTEM", "/controlled/system"),
            ("GIT_CONFIG_GLOBAL", "/controlled/global"),
            ("GIT_ATTR_NOSYSTEM", "yes"),
        ] {
            assert!(
                environment.contains(&(OsStr::new(key), Some(OsStr::new(value)))),
                "preserved {key} must be forwarded to the Git child"
            );
        }
    }

    #[test]
    fn tracked_status_guards_preserve_exact_driver_bytes() {
        let cwd = Path::new("/tmp/thinws-p0-07-query-cwd");
        let drivers = [
            OsString::from_vec(b"Case.dot = space \xff".to_vec()),
            OsString::from("EndingZ"),
        ];
        let command = git_command(
            cwd,
            GitQuery::TrackedStatus {
                filter_drivers: &drivers,
            },
            QueryEnvironment::Isolated,
            None,
        );
        let environment = command.get_envs().collect::<Vec<_>>();

        assert!(environment.contains(&(OsStr::new("GIT_CONFIG_COUNT"), Some(OsStr::new("10")))));
        for (driver_index, driver) in drivers.iter().enumerate() {
            for (field_index, (field, value)) in
                [("clean", ""), ("process", ""), ("required", "true")]
                    .into_iter()
                    .enumerate()
            {
                let mut expected_key = OsString::from("filter.");
                expected_key.push(driver);
                expected_key.push(".");
                expected_key.push(field);
                let index = 4 + driver_index * 3 + field_index;
                let key_name = format!("GIT_CONFIG_KEY_{index}");
                let value_name = format!("GIT_CONFIG_VALUE_{index}");
                assert!(
                    environment.contains(&(OsStr::new(&key_name), Some(expected_key.as_os_str())))
                );
                assert!(environment.contains(&(OsStr::new(&value_name), Some(OsStr::new(value)))));
            }
        }
        assert!(!environment.iter().any(|(_, value)| {
            value.is_some_and(|value| value.as_bytes().ends_with(b".smudge"))
        }));
    }

    #[test]
    fn rejects_relative_paths_before_starting_a_child() {
        let cwd_failure = run(Path::new("relative"), GitQuery::Version)
            .expect_err("relative cwd must be rejected");
        assert_eq!(
            cwd_failure.kind,
            GitQueryFailureKind::InvalidInput {
                field: InputField::Cwd
            }
        );
        assert_eq!(cwd_failure.direct_child_exit, DirectChildExit::NotStarted);

        let config_failure = run(
            Path::new("/"),
            GitQuery::Config {
                file: Path::new("-"),
            },
        )
        .expect_err("non-absolute config path must be rejected");
        assert_eq!(
            config_failure.kind,
            GitQueryFailureKind::InvalidInput {
                field: InputField::ConfigFile
            }
        );
        assert_eq!(
            config_failure.direct_child_exit,
            DirectChildExit::NotStarted
        );
    }

    #[test]
    fn collects_stdout_and_stderr_on_zero_exit() {
        let output = collect_command(helper_command("success"), test_budget(1024, 1024, 500))
            .expect("complete zero-exit child collection");

        assert!(output.exit.success());
        assert!(
            output
                .stdout
                .windows(10)
                .any(|bytes| bytes == b"stdout-ok\n")
        );
        assert!(
            output
                .stderr
                .windows(10)
                .any(|bytes| bytes == b"stderr-ok\n")
        );
    }

    #[test]
    fn preserves_nonzero_exit() {
        let output = collect_command(helper_command("nonzero"), test_budget(1024, 1024, 500))
            .expect("complete nonzero child collection");

        assert_eq!(output.exit.code, Some(23));
        assert!(
            output
                .stdout
                .windows(10)
                .any(|bytes| bytes == b"stdout-ok\n")
        );
        assert!(
            output
                .stderr
                .windows(10)
                .any(|bytes| bytes == b"stderr-ok\n")
        );
    }

    #[test]
    fn rejects_stdout_beyond_its_own_limit() {
        let failure = collect_command(helper_command("stdout-limit"), test_budget(32, 1024, 500))
            .expect_err("stdout must be bounded");

        assert_eq!(
            failure.kind,
            GitQueryFailureKind::OutputLimitExceeded {
                stream: OutputStream::Stdout,
                limit: 32,
            }
        );
        assert_eq!(failure.stdout.len(), 32);
        assert!(failure.stderr.len() <= 1024);
    }

    #[test]
    fn rejects_stderr_beyond_its_own_limit() {
        let failure = collect_command(helper_command("stderr-limit"), test_budget(1024, 32, 500))
            .expect_err("stderr must be bounded");

        assert_eq!(
            failure.kind,
            GitQueryFailureKind::OutputLimitExceeded {
                stream: OutputStream::Stderr,
                limit: 32,
            }
        );
        assert_eq!(failure.stderr.len(), 32);
        assert!(failure.stdout.len() <= 1024);
    }

    #[test]
    fn times_out_and_confirms_direct_child_exit() {
        let failure = collect_command(helper_command("timeout"), test_budget(1024, 1024, 40))
            .expect_err("sleeping child must time out");

        assert_eq!(failure.kind, GitQueryFailureKind::TimedOut);
        assert!(matches!(
            failure.direct_child_exit,
            DirectChildExit::Confirmed(_)
        ));
    }

    #[test]
    fn exhausted_budget_cannot_return_success() {
        let failure = collect_command(helper_command("success"), test_budget(1024, 1024, 0))
            .expect_err("an exhausted total budget must win over completion");

        assert_eq!(failure.kind, GitQueryFailureKind::TimedOut);
    }

    #[test]
    fn exited_child_does_not_make_inherited_open_pipe_complete() {
        let failure = collect_command(
            helper_command("descendant-holds-pipes"),
            test_budget(4096, 4096, 80),
        )
        .expect_err("inherited open pipes must remain part of the run budget");

        assert_eq!(failure.kind, GitQueryFailureKind::TimedOut);
        assert!(matches!(
            failure.direct_child_exit,
            DirectChildExit::Confirmed(GitExit {
                code: Some(0),
                signal: None
            })
        ));
    }

    #[test]
    fn reports_start_failure_without_claiming_a_child() {
        let failure = collect_command(
            Command::new("/definitely/not/a/thinworkspace-test-executable"),
            test_budget(1024, 1024, 100),
        )
        .expect_err("missing executable must fail to start");

        assert!(matches!(
            failure.kind,
            GitQueryFailureKind::Start {
                error_kind: io::ErrorKind::NotFound,
                ..
            }
        ));
        assert_eq!(failure.direct_child_exit, DirectChildExit::NotStarted);
    }

    #[test]
    fn absent_final_status_is_reported_as_unconfirmed() {
        assert_eq!(
            classify_direct_child_exit(None),
            DirectChildExit::Unconfirmed
        );
    }

    #[test]
    fn cleanup_keeps_the_first_io_error() {
        let mut retained = None;
        retain_cleanup_io_error(
            &mut retained,
            CleanupOperation::InitialPoll,
            io::Error::from_raw_os_error(5),
        );
        retain_cleanup_io_error(
            &mut retained,
            CleanupOperation::Kill,
            io::Error::from_raw_os_error(13),
        );

        let retained = retained.expect("first cleanup error");
        assert_eq!(retained.operation, CleanupOperation::InitialPoll);
        assert_eq!(retained.raw_os_error, Some(5));
    }

    #[test]
    #[allow(clippy::zombie_processes)]
    fn command_helper() {
        let Some(mode) = env::var_os(HELPER_MODE) else {
            return;
        };

        match mode.to_str().expect("ASCII helper mode") {
            "success" | "nonzero" => {
                io::stdout().write_all(b"stdout-ok\n").expect("stdout");
                io::stderr().write_all(b"stderr-ok\n").expect("stderr");
                if mode == "nonzero" {
                    process::exit(23);
                }
            }
            "stdout-limit" => {
                io::stdout().write_all(&[b'o'; 256]).expect("stdout");
            }
            "stderr-limit" => {
                io::stderr().write_all(&[b'e'; 256]).expect("stderr");
            }
            "timeout" => thread::sleep(Duration::from_millis(800)),
            "descendant-holds-pipes" => {
                // Intentionally do not wait here: this is the regression shape
                // where the direct child exits while a descendant retains both
                // inherited pipe descriptors. The descendant self-exits after
                // 250 ms and is then reaped by the operating system.
                Command::new(env::current_exe().expect("current unit-test binary"))
                    .args(["--exact", "git_query::tests::command_helper", "--nocapture"])
                    .env(HELPER_MODE, "descendant")
                    .spawn()
                    .expect("bounded descendant");
            }
            "descendant" => thread::sleep(Duration::from_millis(250)),
            other => panic!("unexpected helper mode: {other}"),
        }
    }
}

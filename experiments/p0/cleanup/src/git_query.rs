//! Bounded collection for the four internal Git query shapes used by P0-07.
//!
//! A successful [`run`] means only that the direct child exited and both output
//! pipes closed within their bounds. Callers must inspect [`GitExit`] and must
//! not interpret successful collection as a semantically successful or complete
//! Git inspection. Repository/config/path safety validation belongs to the next
//! P0-07 step. The repository-sensitive shapes pass an explicit `cwd/.git` to
//! stop parent discovery, but that does not validate whether `.git` is a safe
//! directory or reference. Callers must not use this helper on real user
//! directories until the next step connects that validation.

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::os::fd::AsFd;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rustix::fs::OFlags;

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
    /// Read exactly one caller-prevalidated config file without includes.
    Config { file: &'a Path },
    /// Read tracked status using the command frozen in the technical design.
    TrackedStatus,
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
    validate_inputs(cwd, &query)?;
    let command = git_command(cwd, query);
    collect_command(command, PRODUCTION_BUDGET)
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
    Ok(())
}

fn git_command(cwd: &Path, query: GitQuery<'_>) -> Command {
    let mut command = Command::new(GIT_EXECUTABLE);
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
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
        .env("GIT_CONFIG_COUNT", "4")
        .env("GIT_CONFIG_KEY_0", "core.fsmonitor")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env("GIT_CONFIG_KEY_1", "core.hooksPath")
        .env("GIT_CONFIG_VALUE_1", "/dev/null")
        .env("GIT_CONFIG_KEY_2", "core.pager")
        .env("GIT_CONFIG_VALUE_2", "cat")
        .env("GIT_CONFIG_KEY_3", "color.ui")
        .env("GIT_CONFIG_VALUE_3", "false");

    match query {
        GitQuery::Version => {
            command.arg("--version");
        }
        GitQuery::Config { file } => {
            command.arg(git_dir_argument(cwd));
            command.args(["config", "--file"]);
            command.arg(file);
            command.args(["--no-includes", "--null", "--list"]);
        }
        GitQuery::TrackedStatus => {
            command.arg(git_dir_argument(cwd));
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
            command.arg(git_dir_argument(cwd));
            command.args(["--no-optional-locks", "ls-files", "-v", "-z"]);
        }
    }

    command
}

fn git_dir_argument(cwd: &Path) -> OsString {
    let mut argument = OsString::from("--git-dir=");
    argument.push(cwd.join(".git"));
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
    use std::ffi::OsStr;
    use std::io::{self, Write};
    use std::net::Shutdown;
    #[cfg(unix)]
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::process::{self, Command};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

    const HELPER_MODE: &str = "THINWS_P0_07_GIT_QUERY_HELPER";

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
            (
                GitQuery::Config { file: config },
                vec![
                    OsStr::new("--git-dir=/tmp/thinws-p0-07-query-cwd/.git"),
                    OsStr::new("config"),
                    OsStr::new("--file"),
                    config.as_os_str(),
                    OsStr::new("--no-includes"),
                    OsStr::new("--null"),
                    OsStr::new("--list"),
                ],
            ),
            (
                GitQuery::TrackedStatus,
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
            let command = git_command(cwd, query);
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
        let started = Instant::now();
        let failure = collect_command(helper_command("timeout"), test_budget(1024, 1024, 40))
            .expect_err("sleeping child must time out");

        assert_eq!(failure.kind, GitQueryFailureKind::TimedOut);
        assert!(matches!(
            failure.direct_child_exit,
            DirectChildExit::Confirmed(_)
        ));
        assert!(started.elapsed() < Duration::from_millis(500));
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

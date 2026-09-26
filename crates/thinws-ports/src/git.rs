//! Read-only, bounded Git inspection of an already validated workspace copy.

use std::io;
use std::path::PathBuf;

use thinws_core::{
    AbsolutePath, DiscoveryCompleteness, GitState, RepositoryState, aggregate_git_state,
};

/// Actual direct-child termination, not inferred from a closed output pipe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GitExit {
    /// Normal exit code; absent on signal termination.
    pub code: Option<i32>,
    /// Unix termination signal; absent on normal exit.
    pub signal: Option<i32>,
}

impl GitExit {
    /// Whether Git exited normally with code zero.
    #[must_use]
    pub fn success(self) -> bool {
        self.code == Some(0)
    }
}

/// Bounded stream that exceeded its byte limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// Process I/O operation that failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoOperation {
    /// Configure stdout nonblocking collection.
    ConfigureStdout,
    /// Configure stderr nonblocking collection.
    ConfigureStderr,
    /// Read standard output.
    ReadStdout,
    /// Read standard error.
    ReadStderr,
    /// Poll direct-child termination.
    PollExit,
}

/// Rejected fixed-query input field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputField {
    /// Explicit working directory.
    Cwd,
    /// Prevalidated configuration file.
    ConfigFile,
    /// Git configuration isolation device.
    ConfigIsolationDevice,
}

/// Structured reason why bounded Git output collection failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitQueryFailureKind {
    /// Input was invalid before spawning Git.
    InvalidInput {
        /// Rejected field.
        field: InputField,
    },
    /// The direct child could not start.
    Start {
        /// Portable I/O class.
        error_kind: io::ErrorKind,
        /// Original OS error number when available.
        raw_os_error: Option<i32>,
    },
    /// Process I/O failed after start.
    Io {
        /// Failed operation.
        operation: IoOperation,
        /// Portable I/O class.
        error_kind: io::ErrorKind,
        /// Original OS error number when available.
        raw_os_error: Option<i32>,
    },
    /// The command exceeded its time budget.
    TimedOut,
    /// One output stream exceeded its own byte budget.
    OutputLimitExceeded {
        /// Stream that exceeded its limit.
        stream: OutputStream,
        /// Enforced byte limit.
        limit: usize,
    },
}

/// Confirmation of directly spawned child cleanup after a failed query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectChildExit {
    /// No child was started.
    NotStarted,
    /// The direct child's actual exit was observed.
    Confirmed(GitExit),
    /// Its exit could not be confirmed within the cleanup budget.
    Unconfirmed,
}

/// Cleanup operation whose first I/O failure was retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupOperation {
    /// Initial child poll.
    InitialPoll,
    /// Kill request.
    Kill,
    /// Bounded confirmation poll.
    ConfirmPoll,
}

/// First direct-child cleanup I/O error, separate from the primary failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CleanupIoFailure {
    /// Failed cleanup operation.
    pub operation: CleanupOperation,
    /// Portable I/O class.
    pub error_kind: io::ErrorKind,
    /// Original OS error number when available.
    pub raw_os_error: Option<i32>,
}

/// Why repository discovery or a tracked-content query is not trustworthy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitInspectionIssue {
    /// The supplied copy root could not be opened as the expected directory.
    InvalidCopyRoot,
    /// The inherited environment can change Git's repository or config selection.
    EnvironmentUnsupported,
    /// The fixed Git executable or its runtime path could not be trusted.
    InvalidExecPath,
    /// A no-follow directory scan failed.
    ScanFailed,
    /// The directory scan exceeded its entry budget.
    ScanLimitReached,
    /// The directory scan exceeded its nesting budget.
    DepthLimitReached,
    /// More repositories were found than the inspection budget permits.
    RepositoryLimitReached,
    /// Local repository metadata was malformed or unsafe.
    UnsafeRepositoryMetadata,
    /// Git metadata points outside the controlled copy.
    ExternalRepositoryMetadata,
    /// A configuration value can hide changes or invoke an unapproved program.
    UnsupportedConfiguration,
    /// Attributes cannot be safely interpreted for a tracked-content query.
    UnsafeAttributes,
    /// Index flags may hide tracked changes.
    HiddenIndexFlags,
    /// Sparse checkout prevents a complete tracked-content check.
    SparseCheckout,
    /// A bounded Git query failed before a trustworthy result was collected.
    QueryFailed {
        /// Collection failure class without raw stdout/stderr bytes.
        kind: GitQueryFailureKind,
        /// Whether direct-child exit was confirmed.
        direct_child_exit: DirectChildExit,
        /// First additional cleanup failure, if any.
        cleanup_io: Option<CleanupIoFailure>,
    },
    /// Git completed but did not report successful execution.
    QueryNonZeroExit(GitExit),
    /// Git returned output outside the accepted grammar.
    InvalidGitOutput,
    /// Filesystem evidence changed during inspection.
    EvidenceChanged,
    /// An unsafe or unknown nested repository affects a parent query.
    UnsafeDescendant,
    /// The total inspection time budget expired.
    BudgetExpired,
}

/// One discovered repository, rooted at a path relative to the copy root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryInspection {
    /// Path relative to the copy root; empty denotes the root repository.
    pub relative_path: PathBuf,
    /// Current tracked-content state.
    pub state: RepositoryState,
    /// Reasons this repository could not be established clean or dirty.
    pub issues: Vec<GitInspectionIssue>,
}

impl RepositoryInspection {
    /// Records one repository; an empty relative path denotes the copy root.
    #[must_use]
    pub fn new(
        relative_path: PathBuf,
        state: RepositoryState,
        issues: Vec<GitInspectionIssue>,
    ) -> Self {
        let state = if issues.is_empty() {
            state
        } else {
            RepositoryState::Unknown
        };
        Self {
            relative_path,
            state,
            issues,
        }
    }

    /// Returns the repository's path relative to the copy root.
    #[must_use]
    pub fn relative_path(&self) -> &PathBuf {
        &self.relative_path
    }

    /// Returns the tracked-content state, including a known change count.
    #[must_use]
    pub const fn state(&self) -> RepositoryState {
        self.state
    }

    /// Returns structured reasons why this repository is unknown.
    #[must_use]
    pub fn issues(&self) -> &[GitInspectionIssue] {
        &self.issues
    }
}

/// Repository discovery, per-repository states, and a conservative aggregate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitInspection {
    /// Completeness of no-follow repository discovery.
    pub discovery: DiscoveryCompleteness,
    /// Per-repository results in stable relative-path order.
    pub repositories: Vec<RepositoryInspection>,
    /// Conservative aggregate; unknown outranks dirty.
    pub aggregate: GitState,
    /// Whole-scan issues separate from individual repository issues.
    pub issues: Vec<GitInspectionIssue>,
}

impl GitInspection {
    /// Computes an aggregate that cannot be clean when discovery or evidence is incomplete.
    #[must_use]
    pub fn new(
        discovery: DiscoveryCompleteness,
        repositories: Vec<RepositoryInspection>,
        issues: Vec<GitInspectionIssue>,
    ) -> Self {
        let states = repositories
            .iter()
            .map(RepositoryInspection::state)
            .collect::<Vec<_>>();
        let aggregate = if issues.is_empty() {
            aggregate_git_state(discovery, &states)
        } else {
            GitState::Unknown
        };
        Self {
            discovery,
            repositories,
            aggregate,
            issues,
        }
    }

    /// Reports whether the repository scan itself completed.
    #[must_use]
    pub const fn discovery(&self) -> DiscoveryCompleteness {
        self.discovery
    }

    /// Returns discovered repositories in stable relative-path order.
    #[must_use]
    pub fn repositories(&self) -> &[RepositoryInspection] {
        &self.repositories
    }

    /// Returns `not-applicable` only when no repository exists and discovery completed.
    #[must_use]
    pub const fn aggregate(&self) -> GitState {
        self.aggregate
    }

    /// Returns whole-scan issues, separate from per-repository issues.
    #[must_use]
    pub fn issues(&self) -> &[GitInspectionIssue] {
        &self.issues
    }
}

/// Inspect current tracked changes in a caller-validated workspace copy.
pub trait GitInspector {
    /// Runs bounded, read-only discovery and Git queries without changing product state.
    ///
    /// The caller has already established that `copy_root` identifies the controlled
    /// workspace copy. Invalid or changed roots produce an incomplete report.
    fn inspect(&self, copy_root: &AbsolutePath) -> GitInspection;
}

//! Bounded repository discovery and inspection inside one validated copy root.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, RawMode};

use super::{
    CleanupIoFailure, DirectChildExit, GitExit, GitQuery, GitQueryFailure, GitQueryFailureKind,
    GitQueryOutput, PreservedGitEnvironment, run_bootstrap, run_prevalidated_repository,
};
use crate::{
    DiscoveryCompleteness, GitState, RepositoryState, aggregate_git_state,
    parse_tracked_change_count,
};

const TOTAL_TIMEOUT: Duration = Duration::from_secs(60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_SCAN_ENTRIES: usize = 100_000;
const MAX_REPOSITORIES: usize = 256;
const MAX_DEPTH: usize = 128;
const MAX_TEXT_BYTES: usize = 1024 * 1024;

/// A structured reason that inspection evidence is incomplete or unknown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InspectionIssue {
    InvalidCopyRoot,
    EnvironmentUnsupported,
    InvalidExecPath,
    ScanFailed,
    ScanLimitReached,
    DepthLimitReached,
    RepositoryLimitReached,
    UnsafeRepositoryMetadata,
    ExternalRepositoryMetadata,
    UnsupportedConfiguration,
    UnsafeAttributes,
    HiddenIndexFlags,
    SparseCheckout,
    /// Bounded collection failed; retains only safe structured process evidence.
    QueryFailed {
        kind: GitQueryFailureKind,
        direct_child_exit: DirectChildExit,
        cleanup_io: Option<CleanupIoFailure>,
    },
    /// The direct Git child completed with a nonzero code or terminating signal.
    QueryNonZeroExit(GitExit),
    InvalidGitOutput,
    EvidenceChanged,
    UnsafeDescendant,
    BudgetExpired,
}

/// One discovered repository and its tracked-content evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryInspection {
    /// Path relative to the copy root; an empty path denotes the root itself.
    pub relative_path: PathBuf,
    pub state: RepositoryState,
    pub issues: Vec<InspectionIssue>,
}

/// Bounded inspection evidence for an entire copy root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitInspection {
    pub discovery: DiscoveryCompleteness,
    pub repositories: Vec<RepositoryInspection>,
    pub aggregate: GitState,
    pub issues: Vec<InspectionIssue>,
}

/// Discovers and inspects repositories rooted inside `copy_root`.
///
/// The caller must already have authorized `copy_root` as the controlled copy
/// being evaluated. This experiment revalidates no-follow filesystem evidence,
/// but does not claim hard real-time interruption of filesystem calls or
/// protection against replacement by a malicious process with the same UID.
pub fn inspect(copy_root: &Path) -> GitInspection {
    inspect_with_options(
        copy_root,
        SourceEnvironment::capture(),
        InspectionLimits::production(),
    )
}

#[derive(Clone, Default)]
struct SourceEnvironment {
    home: Option<OsString>,
    xdg_config_home: Option<OsString>,
    git_config_nosystem: Option<OsString>,
    git_config_system: Option<OsString>,
    git_config_global: Option<OsString>,
    git_attr_nosystem: Option<OsString>,
    unsupported: bool,
}

impl SourceEnvironment {
    fn capture() -> Self {
        Self::from_lookup(|name| env::var_os(name))
    }

    fn from_lookup(mut lookup: impl FnMut(&str) -> Option<OsString>) -> Self {
        const UNSUPPORTED: &[&str] = &[
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_ATTR_SOURCE",
            "GIT_EXEC_PATH",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_KEY_0",
            "GIT_CONFIG_VALUE_0",
            "GIT_CEILING_DIRECTORIES",
            "GIT_NAMESPACE",
            "GIT_REPLACE_REF_BASE",
            "GIT_GRAFT_FILE",
            "GIT_SHALLOW_FILE",
            "GIT_QUARANTINE_PATH",
        ];
        let unsupported = UNSUPPORTED.iter().any(|name| lookup(name).is_some());
        Self {
            home: lookup("HOME"),
            xdg_config_home: lookup("XDG_CONFIG_HOME"),
            git_config_nosystem: lookup("GIT_CONFIG_NOSYSTEM"),
            git_config_system: lookup("GIT_CONFIG_SYSTEM"),
            git_config_global: lookup("GIT_CONFIG_GLOBAL"),
            git_attr_nosystem: lookup("GIT_ATTR_NOSYSTEM"),
            unsupported,
        }
    }
}

#[derive(Clone, Copy)]
struct InspectionLimits {
    total_timeout: Duration,
    max_scan_entries: usize,
    max_repositories: usize,
    max_depth: usize,
    max_text_bytes: usize,
    #[cfg(test)]
    before_final_root_revalidation: fn(&Path),
}

#[cfg(test)]
fn keep_root_unchanged(_: &Path) {}

impl InspectionLimits {
    const fn production() -> Self {
        Self {
            total_timeout: TOTAL_TIMEOUT,
            max_scan_entries: MAX_SCAN_ENTRIES,
            max_repositories: MAX_REPOSITORIES,
            max_depth: MAX_DEPTH,
            max_text_bytes: MAX_TEXT_BYTES,
            #[cfg(test)]
            before_final_root_revalidation: keep_root_unchanged,
        }
    }
}

struct CooperativeBudget {
    deadline: Instant,
    limits: InspectionLimits,
    scanned_entries: usize,
}

impl CooperativeBudget {
    fn new(limits: InspectionLimits) -> Self {
        Self {
            deadline: Instant::now() + limits.total_timeout,
            limits,
            scanned_entries: 0,
        }
    }

    fn query_timeout(&self) -> Option<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .map(|remaining| remaining.min(QUERY_TIMEOUT))
            .filter(|remaining| !remaining.is_zero())
    }

    fn expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    fn ensure_remaining(&self) -> Result<(), InspectionIssue> {
        if self.expired() {
            Err(InspectionIssue::BudgetExpired)
        } else {
            Ok(())
        }
    }

    fn consume_scan_entry(&mut self) -> Result<(), InspectionIssue> {
        self.scanned_entries = self.scanned_entries.saturating_add(1);
        if self.scanned_entries > self.limits.max_scan_entries {
            Err(InspectionIssue::ScanLimitReached)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Identity {
    device: u64,
    inode: u64,
    mode: RawMode,
    size: i64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
}

impl Identity {
    fn from_stat(stat: &rustix::fs::Stat) -> Self {
        Self {
            device: stat.st_dev as u64,
            inode: stat.st_ino,
            mode: stat.st_mode,
            size: stat.st_size,
            modified_seconds: stat.st_mtime,
            modified_nanoseconds: stat.st_mtime_nsec,
        }
    }

    fn file_type(self) -> FileType {
        FileType::from_raw_mode(self.mode)
    }
}

struct Discovery {
    root_fd: OwnedFd,
    root_identity: Identity,
    candidates: Vec<PathBuf>,
    attribute_files: Vec<PathBuf>,
    completeness: DiscoveryCompleteness,
    issues: Vec<InspectionIssue>,
}

struct PendingDirectory {
    relative: PathBuf,
    fd: OwnedFd,
    depth: usize,
}

#[derive(Clone)]
struct FileEvidence {
    path: PathBuf,
    identity: Identity,
}

struct ReadEvidence {
    evidence: FileEvidence,
    contents: Vec<u8>,
}

#[derive(Clone)]
struct ConfigEntry {
    key: Vec<u8>,
    value: Vec<u8>,
}

struct GlobalContext {
    entries: Vec<ConfigEntry>,
    evidence: Vec<FileEvidence>,
}

struct PreparedRepository {
    relative_path: PathBuf,
    worktree_identity: Identity,
    git_dir: PathBuf,
    git_dir_identity: Identity,
    common_dir: PathBuf,
    common_dir_identity: Identity,
    index_identity: Option<Identity>,
    evidence: Vec<FileEvidence>,
    issues: Vec<InspectionIssue>,
}

#[cfg(test)]
fn inspect_with_environment(copy_root: &Path, environment: SourceEnvironment) -> GitInspection {
    inspect_with_options(copy_root, environment, InspectionLimits::production())
}

fn inspect_with_options(
    copy_root: &Path,
    environment: SourceEnvironment,
    limits: InspectionLimits,
) -> GitInspection {
    let mut budget = CooperativeBudget::new(limits);
    let mut discovery = match discover(copy_root, &mut budget) {
        Ok(discovery) => discovery,
        Err(issue) => return failed_inspection(issue),
    };
    if discovery.completeness == DiscoveryCompleteness::Incomplete {
        let repository_issue = discovery
            .issues
            .first()
            .copied()
            .unwrap_or(InspectionIssue::ScanFailed);
        let repositories = discovery
            .candidates
            .into_iter()
            .map(|relative_path| RepositoryInspection {
                relative_path,
                state: RepositoryState::Unknown,
                issues: vec![repository_issue],
            })
            .collect();
        return finish_inspection(discovery.completeness, repositories, discovery.issues);
    }
    if discovery.candidates.is_empty() {
        return finish_inspection(discovery.completeness, Vec::new(), discovery.issues);
    }
    let global = match preflight_sources(copy_root, &environment, &mut budget) {
        Ok(global) => global,
        Err(issue) => {
            let repositories = discovery
                .candidates
                .into_iter()
                .map(|relative_path| RepositoryInspection {
                    relative_path,
                    state: RepositoryState::Unknown,
                    issues: vec![issue],
                })
                .collect::<Vec<_>>();
            discovery.issues.push(issue);
            return finish_inspection(discovery.completeness, repositories, discovery.issues);
        }
    };

    let mut repositories = Vec::with_capacity(discovery.candidates.len());
    let mut prepared = Vec::new();
    for relative_path in &discovery.candidates {
        if budget.expired() {
            discovery.completeness = DiscoveryCompleteness::Incomplete;
            discovery.issues.push(InspectionIssue::BudgetExpired);
            repositories.push(RepositoryInspection {
                relative_path: relative_path.clone(),
                state: RepositoryState::Unknown,
                issues: vec![InspectionIssue::BudgetExpired],
            });
            continue;
        }
        match preflight_repository(
            copy_root,
            &discovery,
            relative_path,
            &global,
            &environment,
            &mut budget,
        ) {
            Ok(repository) => prepared.push(repository),
            Err(issue) => repositories.push(RepositoryInspection {
                relative_path: relative_path.clone(),
                state: RepositoryState::Unknown,
                issues: vec![issue],
            }),
        }
    }

    // Every repository must pass metadata/configuration preflight before any
    // repository-sensitive Git query can run.
    let mut indexed = Vec::new();
    for repository in prepared {
        match inspect_index_flags(copy_root, &repository, &environment, &budget) {
            Ok(()) => indexed.push(repository),
            Err(issue) => repositories.push(RepositoryInspection {
                relative_path: repository.relative_path,
                state: RepositoryState::Unknown,
                issues: vec![issue],
            }),
        }
    }

    indexed.sort_by_key(|repository| std::cmp::Reverse(path_depth(&repository.relative_path)));
    for repository in indexed {
        let unsafe_descendant = repositories.iter().any(|inspected| {
            inspected.state == RepositoryState::Unknown
                && is_strict_descendant(&inspected.relative_path, &repository.relative_path)
        });
        if unsafe_descendant {
            repositories.push(RepositoryInspection {
                relative_path: repository.relative_path,
                state: RepositoryState::Unknown,
                issues: vec![InspectionIssue::UnsafeDescendant],
            });
            continue;
        }
        repositories.push(inspect_status(copy_root, repository, &environment, &budget));
    }

    #[cfg(test)]
    (budget.limits.before_final_root_revalidation)(copy_root);
    match budget.ensure_remaining() {
        Ok(()) if !revalidate_identity(copy_root, discovery.root_identity) => {
            discovery.completeness = DiscoveryCompleteness::Incomplete;
            push_unique(&mut discovery.issues, InspectionIssue::EvidenceChanged);
        }
        Err(issue) => {
            discovery.completeness = DiscoveryCompleteness::Incomplete;
            push_unique(&mut discovery.issues, issue);
        }
        Ok(()) => {}
    }
    if let Err(issue) = revalidate_file_evidence(&global.evidence, &budget) {
        discovery.completeness = DiscoveryCompleteness::Incomplete;
        push_unique(&mut discovery.issues, issue);
    }
    repositories.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    finish_inspection(discovery.completeness, repositories, discovery.issues)
}

fn failed_inspection(issue: InspectionIssue) -> GitInspection {
    finish_inspection(DiscoveryCompleteness::Incomplete, Vec::new(), vec![issue])
}

fn finish_inspection(
    discovery: DiscoveryCompleteness,
    repositories: Vec<RepositoryInspection>,
    issues: Vec<InspectionIssue>,
) -> GitInspection {
    let states = repositories
        .iter()
        .map(|repository| repository.state)
        .collect::<Vec<_>>();
    GitInspection {
        discovery,
        aggregate: aggregate_git_state(discovery, &states),
        repositories,
        issues,
    }
}

fn discover(
    copy_root: &Path,
    budget: &mut CooperativeBudget,
) -> Result<Discovery, InspectionIssue> {
    let root_fd =
        open_absolute_directory(copy_root).map_err(|_| InspectionIssue::InvalidCopyRoot)?;
    let root_identity = identity_of(&root_fd).map_err(|_| InspectionIssue::InvalidCopyRoot)?;
    if !root_identity.file_type().is_dir() {
        return Err(InspectionIssue::InvalidCopyRoot);
    }
    let mut pending = vec![PendingDirectory {
        relative: PathBuf::new(),
        fd: rustix::io::dup(&root_fd).map_err(|_| InspectionIssue::ScanFailed)?,
        depth: 0,
    }];
    let mut candidates = Vec::new();
    let mut attribute_files = Vec::new();
    let mut completeness = DiscoveryCompleteness::Complete;
    let mut issues = Vec::new();
    while let Some(directory) = pending.pop() {
        if budget.expired() {
            completeness = DiscoveryCompleteness::Incomplete;
            push_unique(&mut issues, InspectionIssue::BudgetExpired);
            break;
        }
        let mut entries = match Dir::read_from(&directory.fd) {
            Ok(entries) => entries,
            Err(_) => {
                completeness = DiscoveryCompleteness::Incomplete;
                push_unique(&mut issues, InspectionIssue::ScanFailed);
                continue;
            }
        };
        for entry in &mut entries {
            if budget.expired() {
                completeness = DiscoveryCompleteness::Incomplete;
                push_unique(&mut issues, InspectionIssue::BudgetExpired);
                pending.clear();
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    completeness = DiscoveryCompleteness::Incomplete;
                    push_unique(&mut issues, InspectionIssue::ScanFailed);
                    continue;
                }
            };
            let name = entry.file_name();
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            if budget.consume_scan_entry().is_err() {
                completeness = DiscoveryCompleteness::Incomplete;
                push_unique(&mut issues, InspectionIssue::ScanLimitReached);
                pending.clear();
                break;
            }
            let stat = match rustix::fs::statat(&directory.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => stat,
                Err(_) => {
                    completeness = DiscoveryCompleteness::Incomplete;
                    push_unique(&mut issues, InspectionIssue::ScanFailed);
                    continue;
                }
            };
            let file_type = FileType::from_raw_mode(stat.st_mode);
            let os_name = OsStr::from_bytes(name.to_bytes());
            let relative = directory.relative.join(os_name);
            if name.to_bytes() == b".git" {
                if file_type.is_dir() || file_type.is_file() {
                    if candidates.len() == budget.limits.max_repositories {
                        completeness = DiscoveryCompleteness::Incomplete;
                        push_unique(&mut issues, InspectionIssue::RepositoryLimitReached);
                    } else {
                        candidates.push(directory.relative.clone());
                    }
                } else {
                    completeness = DiscoveryCompleteness::Incomplete;
                    push_unique(&mut issues, InspectionIssue::UnsafeRepositoryMetadata);
                }
                continue;
            }
            if name.to_bytes() == b".gitattributes" && file_type.is_file() {
                attribute_files.push(relative.clone());
            }
            if file_type.is_dir() {
                if directory.depth == budget.limits.max_depth {
                    completeness = DiscoveryCompleteness::Incomplete;
                    push_unique(&mut issues, InspectionIssue::DepthLimitReached);
                    continue;
                }
                match rustix::fs::openat(&directory.fd, name, directory_open_flags(), Mode::empty())
                {
                    Ok(fd) => pending.push(PendingDirectory {
                        relative,
                        fd,
                        depth: directory.depth + 1,
                    }),
                    Err(_) => {
                        completeness = DiscoveryCompleteness::Incomplete;
                        push_unique(&mut issues, InspectionIssue::ScanFailed);
                    }
                }
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    Ok(Discovery {
        root_fd,
        root_identity,
        candidates,
        attribute_files,
        completeness,
        issues,
    })
}

fn preflight_sources(
    copy_root: &Path,
    environment: &SourceEnvironment,
    budget: &mut CooperativeBudget,
) -> Result<GlobalContext, InspectionIssue> {
    if environment.unsupported {
        return Err(InspectionIssue::EnvironmentUnsupported);
    }
    validate_environment_directory(environment.home.as_deref())?;
    validate_environment_directory(environment.xdg_config_home.as_deref())?;
    let selected_system = environment
        .git_config_system
        .as_deref()
        .map(validate_selected_source)
        .transpose()?;
    let selected_global = environment
        .git_config_global
        .as_deref()
        .map(validate_selected_source)
        .transpose()?;
    let timeout = budget
        .query_timeout()
        .ok_or(InspectionIssue::BudgetExpired)?;
    let exec_path = map_query_result(run_bootstrap(copy_root, GitQuery::ExecPath, timeout))?;
    let runtime_prefix = parse_runtime_prefix(&exec_path.stdout)?;
    let mut source_paths = Vec::new();
    if !environment_disables(environment.git_config_nosystem.as_deref())? {
        source_paths.push(runtime_prefix.join("share/git-core/gitconfig"));
        if let Some(path) = selected_system {
            source_paths.push(path);
        } else {
            source_paths.push(resolve_system_path(Path::new("/etc/gitconfig"))?);
        }
    }
    if let Some(path) = selected_global {
        source_paths.push(path);
    } else {
        if let Some(xdg) = environment.xdg_config_home.as_deref() {
            source_paths.push(PathBuf::from(xdg).join("git/config"));
        } else if let Some(home) = environment.home.as_deref() {
            source_paths.push(PathBuf::from(home).join(".config/git/config"));
        }
        if let Some(home) = environment.home.as_deref() {
            source_paths.push(PathBuf::from(home).join(".gitconfig"));
        }
    }

    let mut entries = Vec::new();
    let mut evidence = Vec::new();
    for source in source_paths {
        budget.ensure_remaining()?;
        if source == Path::new("/dev/null") {
            continue;
        }
        if let Some(read) = read_absolute_file(&source, budget.limits.max_text_bytes, false)? {
            let parsed = parse_config_file(copy_root, &source, budget)?;
            entries.extend(parsed);
            evidence.push(read.evidence);
        }
    }

    if !environment_disables(environment.git_attr_nosystem.as_deref())? {
        let system_attributes = [
            runtime_prefix.join("share/git-core/gitattributes"),
            resolve_system_path(Path::new("/etc/gitattributes"))?,
        ];
        for source in system_attributes {
            budget.ensure_remaining()?;
            if let Some(read) = read_absolute_file(&source, budget.limits.max_text_bytes, false)? {
                validate_attributes(&read.contents)?;
                evidence.push(read.evidence);
            }
        }
    }
    budget.ensure_remaining()?;
    validate_config_entries(&entries)?;
    Ok(GlobalContext { entries, evidence })
}

fn validate_environment_directory(path: Option<&OsStr>) -> Result<(), InspectionIssue> {
    let Some(path) = path else {
        return Ok(());
    };
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err(InspectionIssue::EnvironmentUnsupported);
    }
    open_absolute_directory(path)
        .map(|_| ())
        .or_else(|error| {
            if error == rustix::io::Errno::NOENT {
                Ok(())
            } else {
                Err(error)
            }
        })
        .map_err(|_| InspectionIssue::EnvironmentUnsupported)
}

fn environment_disables(value: Option<&OsStr>) -> Result<bool, InspectionIssue> {
    let Some(value) = value else {
        return Ok(false);
    };
    let value = value.as_bytes();
    if value.is_empty() || is_false(value) {
        Ok(false)
    } else if value.eq_ignore_ascii_case(b"true")
        || value.eq_ignore_ascii_case(b"yes")
        || value.eq_ignore_ascii_case(b"on")
        || value == b"1"
    {
        Ok(true)
    } else {
        Err(InspectionIssue::EnvironmentUnsupported)
    }
}

fn validate_selected_source(path: &OsStr) -> Result<PathBuf, InspectionIssue> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(InspectionIssue::EnvironmentUnsupported);
    }
    if path == Path::new("/dev/null") {
        validate_null_device(&path)?;
    }
    Ok(path)
}

fn validate_null_device(path: &Path) -> Result<(), InspectionIssue> {
    let stat = rustix::fs::statat(rustix::fs::CWD, path, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| InspectionIssue::EnvironmentUnsupported)?;
    if FileType::from_raw_mode(stat.st_mode).is_char_device() {
        Ok(())
    } else {
        Err(InspectionIssue::EnvironmentUnsupported)
    }
}

fn parse_runtime_prefix(stdout: &[u8]) -> Result<PathBuf, InspectionIssue> {
    let path_bytes = stdout.strip_suffix(b"\n").unwrap_or(stdout);
    if path_bytes.is_empty() || path_bytes.contains(&b'\n') || path_bytes.contains(&0) {
        return Err(InspectionIssue::InvalidExecPath);
    }
    let exec_path = PathBuf::from(OsString::from_vec(path_bytes.to_vec()));
    if !exec_path.is_absolute()
        || exec_path.file_name() != Some(OsStr::new("git-core"))
        || exec_path.parent().and_then(Path::file_name) != Some(OsStr::new("libexec"))
    {
        return Err(InspectionIssue::InvalidExecPath);
    }
    let prefix = exec_path
        .parent()
        .and_then(Path::parent)
        .ok_or(InspectionIssue::InvalidExecPath)?
        .to_path_buf();
    open_absolute_directory(&exec_path).map_err(|_| InspectionIssue::InvalidExecPath)?;
    Ok(prefix)
}

fn resolve_system_path(path: &Path) -> Result<PathBuf, InspectionIssue> {
    let etc = Path::new("/etc");
    let suffix = path
        .strip_prefix(etc)
        .map_err(|_| InspectionIssue::EnvironmentUnsupported)?;
    let metadata =
        std::fs::symlink_metadata(etc).map_err(|_| InspectionIssue::EnvironmentUnsupported)?;
    if !metadata.file_type().is_symlink() {
        return Ok(path.to_path_buf());
    }
    if metadata.uid() != 0 {
        return Err(InspectionIssue::EnvironmentUnsupported);
    }
    let target = std::fs::read_link(etc).map_err(|_| InspectionIssue::EnvironmentUnsupported)?;
    let target = if target.is_absolute() {
        target
    } else {
        Path::new("/").join(target)
    };
    let directory =
        open_absolute_directory(&target).map_err(|_| InspectionIssue::EnvironmentUnsupported)?;
    let stat =
        rustix::fs::fstat(&directory).map_err(|_| InspectionIssue::EnvironmentUnsupported)?;
    validate_system_directory_metadata(stat.st_mode, stat.st_uid)?;
    Ok(target.join(suffix))
}

fn validate_system_directory_metadata(mode: RawMode, uid: u32) -> Result<(), InspectionIssue> {
    if !FileType::from_raw_mode(mode).is_dir() {
        return Err(InspectionIssue::EnvironmentUnsupported);
    }
    if uid != 0 {
        return Err(InspectionIssue::EnvironmentUnsupported);
    }
    if mode & 0o022 != 0 {
        return Err(InspectionIssue::EnvironmentUnsupported);
    }
    Ok(())
}

fn resolve_attributes_source(
    environment: &SourceEnvironment,
    value: &[u8],
) -> Result<PathBuf, InspectionIssue> {
    if value.first() == Some(&b'~') {
        if value.get(1) != Some(&b'/') {
            return Err(InspectionIssue::UnsafeAttributes);
        }
        let home = environment
            .home
            .as_deref()
            .ok_or(InspectionIssue::UnsafeAttributes)?;
        return Ok(PathBuf::from(home).join(OsStr::from_bytes(&value[2..])));
    }
    let path = PathBuf::from(OsString::from_vec(value.to_vec()));
    if !path.is_absolute() {
        return Err(InspectionIssue::UnsafeAttributes);
    }
    Ok(path)
}

fn default_user_attributes(environment: &SourceEnvironment) -> Option<PathBuf> {
    environment
        .xdg_config_home
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| {
            environment
                .home
                .as_deref()
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .map(|xdg| xdg.join("git/attributes"))
}

fn validate_effective_user_attributes(
    entries: &[ConfigEntry],
    environment: &SourceEnvironment,
    budget: &CooperativeBudget,
    evidence: &mut Vec<FileEvidence>,
) -> Result<(), InspectionIssue> {
    budget.ensure_remaining()?;
    if let Some(entry) = entries
        .iter()
        .rev()
        .find(|entry| key_eq(entry, b"core.attributesfile"))
    {
        let path = resolve_attributes_source(environment, &entry.value)?;
        let read = read_absolute_file(&path, budget.limits.max_text_bytes, true)?
            .ok_or(InspectionIssue::UnsafeAttributes)?;
        validate_attributes(&read.contents)?;
        evidence.push(read.evidence);
    } else if let Some(path) = default_user_attributes(environment)
        && let Some(read) = read_absolute_file(&path, budget.limits.max_text_bytes, false)?
    {
        validate_attributes(&read.contents)?;
        evidence.push(read.evidence);
    }
    Ok(())
}

fn preflight_repository(
    copy_root: &Path,
    discovery: &Discovery,
    relative_path: &Path,
    global: &GlobalContext,
    environment: &SourceEnvironment,
    budget: &mut CooperativeBudget,
) -> Result<PreparedRepository, InspectionIssue> {
    let root_fd = &discovery.root_fd;
    let worktree_fd = open_relative_directory(root_fd, relative_path)
        .map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;
    let worktree_identity =
        identity_of(&worktree_fd).map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;
    let marker_stat = rustix::fs::statat(&worktree_fd, ".git", AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;
    let marker_type = FileType::from_raw_mode(marker_stat.st_mode);
    let marker_relative = relative_path.join(".git");
    let mut marker_file_identity = None;
    let mut evidence = global.evidence.clone();
    let git_dir = if marker_type.is_dir() {
        marker_relative.clone()
    } else if marker_type.is_file() {
        let read = read_root_file(
            copy_root,
            root_fd,
            &marker_relative,
            budget.limits.max_text_bytes,
            true,
        )?
        .ok_or(InspectionIssue::UnsafeRepositoryMetadata)?;
        let reference = parse_single_reference(&read.contents, b"gitdir: ")?;
        marker_file_identity = Some(read.evidence.identity);
        evidence.push(read.evidence);
        resolve_internal_reference(copy_root, relative_path, reference)?
    } else {
        return Err(InspectionIssue::UnsafeRepositoryMetadata);
    };
    let git_dir_fd = open_relative_directory(root_fd, &git_dir)
        .map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;
    let git_dir_identity =
        identity_of(&git_dir_fd).map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;

    let commondir_path = git_dir.join("commondir");
    let common_dir = match read_root_file(
        copy_root,
        root_fd,
        &commondir_path,
        budget.limits.max_text_bytes,
        false,
    )? {
        Some(read) => {
            let reference = parse_single_reference(&read.contents, b"")?;
            evidence.push(read.evidence);
            resolve_internal_reference(copy_root, &git_dir, reference)?
        }
        None => git_dir.clone(),
    };
    let common_dir_fd = open_relative_directory(root_fd, &common_dir)
        .map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;
    let common_dir_identity =
        identity_of(&common_dir_fd).map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;

    scan_metadata_nofollow(root_fd, &common_dir, budget)?;
    if common_dir_identity != git_dir_identity {
        scan_metadata_nofollow(root_fd, &git_dir, budget)?;
    }
    reject_metadata_links(root_fd, &git_dir, &common_dir)?;
    let index_identity = stat_relative(root_fd, &git_dir.join("index"))
        .map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;
    collect_reference_evidence(
        copy_root,
        root_fd,
        &git_dir,
        &common_dir,
        budget,
        &mut evidence,
    )?;
    let mut entries = global.entries.clone();
    let common_config = common_dir.join("config");
    if let Some(read) = read_root_file(
        copy_root,
        root_fd,
        &common_config,
        budget.limits.max_text_bytes,
        false,
    )? {
        let common_entries = parse_config_file(copy_root, &read.evidence.path, budget)?;
        let worktree_config = effective_config_bool(&common_entries, b"extensions.worktreeconfig")?;
        entries.extend(common_entries);
        evidence.push(read.evidence);
        if worktree_config {
            let worktree_config = git_dir.join("config.worktree");
            if let Some(read) = read_root_file(
                copy_root,
                root_fd,
                &worktree_config,
                budget.limits.max_text_bytes,
                false,
            )? {
                entries.extend(parse_config_file(copy_root, &read.evidence.path, budget)?);
                evidence.push(read.evidence);
            }
        }
    }
    validate_config_entries(&entries)?;
    validate_effective_user_attributes(&entries, environment, budget, &mut evidence)?;
    let mut worktree_corresponds = marker_file_identity.is_none();
    for entry in &entries {
        if key_eq(entry, b"core.worktree") {
            let resolved = resolve_internal_reference(copy_root, &git_dir, &entry.value)?;
            let resolved_fd = open_relative_directory(root_fd, &resolved)
                .map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;
            if identity_of(&resolved_fd).ok() != Some(worktree_identity) {
                return Err(InspectionIssue::ExternalRepositoryMetadata);
            }
            worktree_corresponds = true;
        }
    }
    if let Some(marker_identity) = marker_file_identity
        && let Some(read) = read_root_file(
            copy_root,
            root_fd,
            &git_dir.join("gitdir"),
            budget.limits.max_text_bytes,
            false,
        )?
    {
        let reference = parse_single_reference(&read.contents, b"")?;
        let back_reference = resolve_internal_reference(copy_root, &git_dir, reference)?;
        if back_reference != marker_relative {
            return Err(InspectionIssue::ExternalRepositoryMetadata);
        }
        let back_reference_file = read_root_file(
            copy_root,
            root_fd,
            &back_reference,
            budget.limits.max_text_bytes,
            true,
        )?
        .ok_or(InspectionIssue::UnsafeRepositoryMetadata)?;
        if back_reference_file.evidence.identity != marker_identity {
            return Err(InspectionIssue::EvidenceChanged);
        }
        evidence.push(read.evidence);
        evidence.push(back_reference_file.evidence);
        worktree_corresponds = true;
    }
    if !worktree_corresponds {
        return Err(InspectionIssue::UnsafeRepositoryMetadata);
    }

    if let Some(read) = read_root_file(
        copy_root,
        root_fd,
        &relative_path.join(".gitmodules"),
        budget.limits.max_text_bytes,
        false,
    )? {
        let gitmodules = parse_config_file(copy_root, &read.evidence.path, budget)?;
        validate_config_entries(&gitmodules)?;
        validate_gitmodules(copy_root, relative_path, &gitmodules)?;
        evidence.push(read.evidence);
    }

    for attributes in &discovery.attribute_files {
        budget.ensure_remaining()?;
        let belongs_to_repository = attributes.starts_with(relative_path)
            && !discovery.candidates.iter().any(|candidate| {
                is_strict_descendant(candidate, relative_path) && attributes.starts_with(candidate)
            });
        if belongs_to_repository {
            let read = read_root_file(
                copy_root,
                root_fd,
                attributes,
                budget.limits.max_text_bytes,
                true,
            )?
            .ok_or(InspectionIssue::UnsafeAttributes)?;
            validate_attributes(&read.contents)?;
            evidence.push(read.evidence);
        }
    }
    for info_attributes in [
        common_dir.join("info/attributes"),
        git_dir.join("info/attributes"),
    ] {
        budget.ensure_remaining()?;
        if let Some(read) = read_root_file(
            copy_root,
            root_fd,
            &info_attributes,
            budget.limits.max_text_bytes,
            false,
        )? {
            validate_attributes(&read.contents)?;
            evidence.push(read.evidence);
        }
    }

    revalidate_file_evidence(&evidence, budget)?;
    Ok(PreparedRepository {
        relative_path: relative_path.to_path_buf(),
        worktree_identity,
        git_dir: copy_root.join(&git_dir),
        git_dir_identity,
        common_dir: copy_root.join(&common_dir),
        common_dir_identity,
        index_identity,
        evidence,
        issues: Vec::new(),
    })
}

fn validate_gitmodules(
    copy_root: &Path,
    worktree: &Path,
    entries: &[ConfigEntry],
) -> Result<(), InspectionIssue> {
    for entry in entries {
        if entry.key.starts_with(b"submodule.") && entry.key.ends_with(b".path") {
            let path = Path::new(OsStr::from_bytes(&entry.value));
            if path.is_absolute()
                || path
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
            {
                return Err(InspectionIssue::ExternalRepositoryMetadata);
            }
            let _ = normalize_internal_reference(copy_root, worktree, &entry.value)?;
        }
    }
    Ok(())
}

fn collect_reference_evidence(
    copy_root: &Path,
    root_fd: &OwnedFd,
    git_dir: &Path,
    common_dir: &Path,
    budget: &CooperativeBudget,
    evidence: &mut Vec<FileEvidence>,
) -> Result<(), InspectionIssue> {
    let mut reference_path = git_dir.join("HEAD");
    for depth in 0..=budget.limits.max_depth {
        let required = depth == 0;
        let Some(read) = read_root_file(
            copy_root,
            root_fd,
            &reference_path,
            budget.limits.max_text_bytes,
            required,
        )?
        else {
            break;
        };
        let value = parse_single_reference(&read.contents, b"")?;
        evidence.push(read.evidence);
        if let Some(symbolic) = value.strip_prefix(b"ref: ") {
            let symbolic = validate_symbolic_ref(symbolic)?;
            reference_path = common_dir.join(symbolic);
            if depth == budget.limits.max_depth {
                return Err(InspectionIssue::DepthLimitReached);
            }
        } else {
            if !matches!(value.len(), 40 | 64) || !value.iter().all(u8::is_ascii_hexdigit) {
                return Err(InspectionIssue::UnsafeRepositoryMetadata);
            }
            break;
        }
    }
    for relative in [common_dir.join("packed-refs"), common_dir.join("shallow")] {
        if let Some(read) = read_root_file(
            copy_root,
            root_fd,
            &relative,
            budget.limits.max_text_bytes,
            false,
        )? {
            evidence.push(read.evidence);
        }
    }
    Ok(())
}

fn validate_symbolic_ref(reference: &[u8]) -> Result<PathBuf, InspectionIssue> {
    let path = PathBuf::from(OsString::from_vec(reference.to_vec()));
    if !path.starts_with("refs")
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(InspectionIssue::UnsafeRepositoryMetadata);
    }
    Ok(path)
}

fn scan_metadata_nofollow(
    root_fd: &OwnedFd,
    metadata_root: &Path,
    budget: &mut CooperativeBudget,
) -> Result<(), InspectionIssue> {
    let initial_depth = path_depth(metadata_root);
    if initial_depth > budget.limits.max_depth {
        return Err(InspectionIssue::DepthLimitReached);
    }
    let fd = open_relative_directory(root_fd, metadata_root)
        .map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;
    let mut pending = vec![PendingDirectory {
        relative: metadata_root.to_path_buf(),
        fd,
        depth: initial_depth,
    }];
    while let Some(directory) = pending.pop() {
        if budget.expired() {
            return Err(InspectionIssue::BudgetExpired);
        }
        let mut entries =
            Dir::read_from(&directory.fd).map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;
        for entry in &mut entries {
            if budget.expired() {
                return Err(InspectionIssue::BudgetExpired);
            }
            let entry = entry.map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;
            let name = entry.file_name();
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            budget.consume_scan_entry()?;
            let stat = rustix::fs::statat(&directory.fd, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;
            let identity = Identity::from_stat(&stat);
            let relative = directory.relative.join(OsStr::from_bytes(name.to_bytes()));
            if identity.file_type().is_symlink() {
                return Err(InspectionIssue::ExternalRepositoryMetadata);
            }
            if identity.file_type().is_dir() {
                if directory.depth == budget.limits.max_depth {
                    return Err(InspectionIssue::DepthLimitReached);
                }
                let fd =
                    rustix::fs::openat(&directory.fd, name, directory_open_flags(), Mode::empty())
                        .map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;
                pending.push(PendingDirectory {
                    relative,
                    fd,
                    depth: directory.depth + 1,
                });
            } else if !identity.file_type().is_file()
                || is_bounded_metadata_text(&relative, metadata_root)
                    && (identity.size < 0
                        || identity.size as u64 > budget.limits.max_text_bytes as u64)
            {
                return Err(InspectionIssue::UnsafeRepositoryMetadata);
            }
        }
    }
    Ok(())
}

fn is_bounded_metadata_text(path: &Path, metadata_root: &Path) -> bool {
    let name = path.file_name().and_then(OsStr::to_str);
    matches!(
        name,
        Some(
            "HEAD"
                | "FETCH_HEAD"
                | "ORIG_HEAD"
                | "MERGE_HEAD"
                | "CHERRY_PICK_HEAD"
                | "REVERT_HEAD"
                | "packed-refs"
                | "commondir"
                | "gitdir"
                | "config"
                | "config.worktree"
        )
    ) || path
        .strip_prefix(metadata_root)
        .ok()
        .and_then(|relative| relative.components().next())
        .is_some_and(|component| component.as_os_str() == OsStr::new("refs"))
}

fn reject_metadata_links(
    root_fd: &OwnedFd,
    git_dir: &Path,
    common_dir: &Path,
) -> Result<(), InspectionIssue> {
    for relative in [
        common_dir.join("objects"),
        common_dir.join("refs"),
        git_dir.join("index"),
    ] {
        match stat_relative(root_fd, &relative) {
            Ok(Some(identity)) if identity.file_type().is_symlink() => {
                return Err(InspectionIssue::ExternalRepositoryMetadata);
            }
            Ok(_) => {}
            Err(_) => return Err(InspectionIssue::UnsafeRepositoryMetadata),
        }
    }
    let alternates = common_dir.join("objects/info/alternates");
    if stat_relative(root_fd, &alternates)
        .map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?
        .is_some()
    {
        return Err(InspectionIssue::UnsupportedConfiguration);
    }
    for sparse in [
        common_dir.join("info/sparse-checkout"),
        git_dir.join("info/sparse-checkout"),
    ] {
        if stat_relative(root_fd, &sparse)
            .map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?
            .is_some()
        {
            return Err(InspectionIssue::SparseCheckout);
        }
    }
    Ok(())
}

fn parse_config_file(
    cwd: &Path,
    path: &Path,
    budget: &CooperativeBudget,
) -> Result<Vec<ConfigEntry>, InspectionIssue> {
    let timeout = budget
        .query_timeout()
        .ok_or(InspectionIssue::BudgetExpired)?;
    let output = map_query_result(run_bootstrap(cwd, GitQuery::Config { file: path }, timeout))?;
    if output.stdout.len() > budget.limits.max_text_bytes {
        return Err(InspectionIssue::InvalidGitOutput);
    }
    parse_config_output(&output.stdout)
}

fn parse_config_output(output: &[u8]) -> Result<Vec<ConfigEntry>, InspectionIssue> {
    let mut entries = Vec::new();
    if output.is_empty() {
        return Ok(entries);
    }
    if !output.ends_with(&[0]) {
        return Err(InspectionIssue::InvalidGitOutput);
    }
    for record in output[..output.len() - 1].split(|byte| *byte == 0) {
        if record.is_empty() {
            return Err(InspectionIssue::InvalidGitOutput);
        }
        let Some(separator) = record.iter().position(|byte| *byte == b'\n') else {
            return Err(InspectionIssue::InvalidGitOutput);
        };
        if separator == 0 {
            return Err(InspectionIssue::InvalidGitOutput);
        }
        let mut key = record[..separator].to_vec();
        key.make_ascii_lowercase();
        entries.push(ConfigEntry {
            key,
            value: record[separator + 1..].to_vec(),
        });
    }
    Ok(entries)
}

fn validate_config_entries(entries: &[ConfigEntry]) -> Result<(), InspectionIssue> {
    for entry in entries {
        let key = entry.key.as_slice();
        if key == b"include.path"
            || key.starts_with(b"includeif.")
            || key.starts_with(b"filter.")
            || key == b"core.hookspath"
            || key == b"core.fsmonitor" && !is_false(&entry.value)
            || key == b"core.sparsecheckout" && !is_false(&entry.value)
            || key == b"core.sparsecheckoutcone" && !is_false(&entry.value)
            || key.ends_with(b".promisor") && !is_false(&entry.value)
            || key.ends_with(b".partialclonefilter") && !entry.value.is_empty()
            || key == b"core.bare" && !is_false(&entry.value)
            || key.starts_with(b"sparse.")
        {
            return Err(InspectionIssue::UnsupportedConfiguration);
        }
        if key.starts_with(b"core.") && !known_core_key(key)
            || key.starts_with(b"extensions.") && key != b"extensions.worktreeconfig"
            || key.starts_with(b"index.") && key != b"index.version"
            || key.starts_with(b"status.")
        {
            return Err(InspectionIssue::UnsupportedConfiguration);
        }
    }
    Ok(())
}

fn known_core_key(key: &[u8]) -> bool {
    matches!(
        key,
        b"core.attributesfile"
            | b"core.autocrlf"
            | b"core.bare"
            | b"core.checkstat"
            | b"core.eol"
            | b"core.filemode"
            | b"core.fsmonitor"
            | b"core.hookspath"
            | b"core.ignorecase"
            | b"core.logallrefupdates"
            | b"core.precomposeunicode"
            | b"core.protecthfs"
            | b"core.protectntfs"
            | b"core.quotepath"
            | b"core.repositoryformatversion"
            | b"core.safecrlf"
            | b"core.sharedrepository"
            | b"core.sparsecheckout"
            | b"core.sparsecheckoutcone"
            | b"core.splitindex"
            | b"core.symlinks"
            | b"core.trustctime"
            | b"core.untrackedcache"
            | b"core.worktree"
    )
}

fn validate_attributes(contents: &[u8]) -> Result<(), InspectionIssue> {
    for line in contents.split(|byte| *byte == b'\n') {
        let line = trim_ascii(line);
        if line.is_empty() || line.starts_with(b"#") {
            continue;
        }
        for attribute in line
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty())
            .skip(1)
        {
            let name = attribute
                .strip_prefix(b"-")
                .or_else(|| attribute.strip_prefix(b"!"))
                .unwrap_or(attribute)
                .split(|byte| *byte == b'=')
                .next()
                .unwrap_or_default();
            if name == b"filter" {
                return Err(InspectionIssue::UnsafeAttributes);
            }
        }
    }
    Ok(())
}

fn is_false(value: &[u8]) -> bool {
    value.eq_ignore_ascii_case(b"false")
        || value.eq_ignore_ascii_case(b"no")
        || value == b"0"
        || value.eq_ignore_ascii_case(b"off")
}

fn effective_config_bool(entries: &[ConfigEntry], key: &[u8]) -> Result<bool, InspectionIssue> {
    let Some(entry) = entries.iter().rev().find(|entry| entry.key == key) else {
        return Ok(false);
    };
    if entry.value.is_empty()
        || entry.value.eq_ignore_ascii_case(b"true")
        || entry.value.eq_ignore_ascii_case(b"yes")
        || entry.value.eq_ignore_ascii_case(b"on")
        || entry.value == b"1"
    {
        Ok(true)
    } else if is_false(&entry.value) {
        Ok(false)
    } else {
        Err(InspectionIssue::UnsupportedConfiguration)
    }
}

fn key_eq(entry: &ConfigEntry, key: &[u8]) -> bool {
    entry.key == key
}

fn map_query_result(
    result: Result<GitQueryOutput, GitQueryFailure>,
) -> Result<GitQueryOutput, InspectionIssue> {
    match result {
        Ok(output) if output.exit.success() => Ok(output),
        Ok(output) => Err(InspectionIssue::QueryNonZeroExit(output.exit)),
        Err(GitQueryFailure {
            kind,
            direct_child_exit,
            cleanup_io,
            ..
        }) => Err(InspectionIssue::QueryFailed {
            kind,
            direct_child_exit,
            cleanup_io,
        }),
    }
}

fn inspect_index_flags(
    copy_root: &Path,
    repository: &PreparedRepository,
    environment: &SourceEnvironment,
    budget: &CooperativeBudget,
) -> Result<(), InspectionIssue> {
    revalidate_repository(copy_root, repository, budget)?;
    let timeout = budget
        .query_timeout()
        .ok_or(InspectionIssue::BudgetExpired)?;
    let output = map_query_result(run_prevalidated_repository(
        &copy_root.join(&repository.relative_path),
        &repository.git_dir,
        GitQuery::IndexFlags,
        preserved_environment(environment),
        timeout,
    ))?;
    validate_index_flags_output(&output.stdout)?;
    revalidate_repository(copy_root, repository, budget)
}

fn validate_index_flags_output(output: &[u8]) -> Result<(), InspectionIssue> {
    if output.is_empty() {
        return Ok(());
    }
    let records = output
        .strip_suffix(&[0])
        .ok_or(InspectionIssue::InvalidGitOutput)?;
    for record in records.split(|byte| *byte == 0) {
        if record.is_empty() {
            return Err(InspectionIssue::InvalidGitOutput);
        }
        if record.len() < 3 || record[1] != b' ' {
            return Err(InspectionIssue::InvalidGitOutput);
        }
        if record[0].is_ascii_lowercase() || record[0] == b'S' {
            return Err(InspectionIssue::HiddenIndexFlags);
        }
    }
    Ok(())
}

fn inspect_status(
    copy_root: &Path,
    mut repository: PreparedRepository,
    environment: &SourceEnvironment,
    budget: &CooperativeBudget,
) -> RepositoryInspection {
    if let Err(issue) = revalidate_repository(copy_root, &repository, budget) {
        repository.issues.push(issue);
        return RepositoryInspection {
            relative_path: repository.relative_path,
            state: RepositoryState::Unknown,
            issues: repository.issues,
        };
    }
    let Some(timeout) = budget.query_timeout() else {
        repository.issues.push(InspectionIssue::BudgetExpired);
        return RepositoryInspection {
            relative_path: repository.relative_path,
            state: RepositoryState::Unknown,
            issues: repository.issues,
        };
    };
    let output = match map_query_result(run_prevalidated_repository(
        &copy_root.join(&repository.relative_path),
        &repository.git_dir,
        GitQuery::TrackedStatus,
        preserved_environment(environment),
        timeout,
    )) {
        Ok(output) => output,
        Err(issue) => {
            repository.issues.push(issue);
            return RepositoryInspection {
                relative_path: repository.relative_path,
                state: RepositoryState::Unknown,
                issues: repository.issues,
            };
        }
    };
    if let Err(issue) = revalidate_repository(copy_root, &repository, budget) {
        repository.issues.push(issue);
        return RepositoryInspection {
            relative_path: repository.relative_path,
            state: RepositoryState::Unknown,
            issues: repository.issues,
        };
    }
    match parse_tracked_change_count(&output.stdout) {
        Ok(count) => RepositoryInspection {
            relative_path: repository.relative_path,
            state: RepositoryState::from_tracked_change_count(count),
            issues: repository.issues,
        },
        Err(_) => {
            repository.issues.push(InspectionIssue::InvalidGitOutput);
            RepositoryInspection {
                relative_path: repository.relative_path,
                state: RepositoryState::Unknown,
                issues: repository.issues,
            }
        }
    }
}

fn preserved_environment(environment: &SourceEnvironment) -> PreservedGitEnvironment<'_> {
    PreservedGitEnvironment {
        home: environment.home.as_deref(),
        xdg_config_home: environment.xdg_config_home.as_deref(),
        git_config_nosystem: environment.git_config_nosystem.as_deref(),
        git_config_system: environment.git_config_system.as_deref(),
        git_config_global: environment.git_config_global.as_deref(),
        git_attr_nosystem: environment.git_attr_nosystem.as_deref(),
    }
}

fn revalidate_repository(
    copy_root: &Path,
    repository: &PreparedRepository,
    budget: &CooperativeBudget,
) -> Result<(), InspectionIssue> {
    budget.ensure_remaining()?;
    if !revalidate_identity(
        &copy_root.join(&repository.relative_path),
        repository.worktree_identity,
    ) || !revalidate_identity(&repository.git_dir, repository.git_dir_identity)
        || !revalidate_identity(&repository.common_dir, repository.common_dir_identity)
        || stat_absolute_nofollow(&repository.git_dir.join("index")).ok()
            != Some(repository.index_identity)
    {
        return Err(InspectionIssue::EvidenceChanged);
    }
    revalidate_file_evidence(&repository.evidence, budget)
}

fn revalidate_file_evidence(
    evidence: &[FileEvidence],
    budget: &CooperativeBudget,
) -> Result<(), InspectionIssue> {
    budget.ensure_remaining()?;
    for item in evidence {
        budget.ensure_remaining()?;
        if !item.revalidate() {
            return Err(InspectionIssue::EvidenceChanged);
        }
    }
    budget.ensure_remaining()
}

fn stat_absolute_nofollow(path: &Path) -> Result<Option<Identity>, InspectionIssue> {
    let (parent, name) = split_absolute_file(path)?;
    let parent_fd = match open_absolute_directory(parent) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(_) => return Err(InspectionIssue::UnsafeRepositoryMetadata),
    };
    match rustix::fs::statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(Some(Identity::from_stat(&stat))),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(_) => Err(InspectionIssue::UnsafeRepositoryMetadata),
    }
}

impl FileEvidence {
    fn revalidate(&self) -> bool {
        read_absolute_file(&self.path, MAX_TEXT_BYTES, true)
            .ok()
            .flatten()
            .is_some_and(|read| read.evidence.identity == self.identity)
    }
}

fn revalidate_identity(path: &Path, expected: Identity) -> bool {
    open_absolute_directory(path)
        .and_then(|fd| identity_of(&fd))
        .is_ok_and(|identity| identity == expected)
}

fn read_root_file(
    copy_root: &Path,
    root_fd: &OwnedFd,
    relative: &Path,
    limit: usize,
    required: bool,
) -> Result<Option<ReadEvidence>, InspectionIssue> {
    let (parent, name) = split_relative_file(relative)?;
    let parent_fd = match open_relative_directory(root_fd, parent) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) if !required => return Ok(None),
        Err(_) => return Err(InspectionIssue::UnsafeRepositoryMetadata),
    };
    let fd = match rustix::fs::openat(&parent_fd, name, file_open_flags(), Mode::empty()) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) if !required => return Ok(None),
        Err(_) => return Err(InspectionIssue::UnsafeRepositoryMetadata),
    };
    read_open_file(fd, copy_root.join(relative), limit)
        .map(Some)
        .map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)
}

fn read_absolute_file(
    path: &Path,
    limit: usize,
    required: bool,
) -> Result<Option<ReadEvidence>, InspectionIssue> {
    let (parent, name) = split_absolute_file(path)?;
    let parent_fd = match open_absolute_directory(parent) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) if !required => return Ok(None),
        Err(_) => return Err(InspectionIssue::EnvironmentUnsupported),
    };
    let fd = match rustix::fs::openat(&parent_fd, name, file_open_flags(), Mode::empty()) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) if !required => return Ok(None),
        Err(_) => return Err(InspectionIssue::EnvironmentUnsupported),
    };
    read_open_file(fd, path.to_path_buf(), limit)
        .map(Some)
        .map_err(|_| InspectionIssue::EnvironmentUnsupported)
}

fn read_open_file(
    fd: OwnedFd,
    path: PathBuf,
    limit: usize,
) -> Result<ReadEvidence, rustix::io::Errno> {
    let before = identity_of(&fd)?;
    if !before.file_type().is_file() {
        return Err(rustix::io::Errno::FBIG);
    }
    if before.size < 0 {
        return Err(rustix::io::Errno::FBIG);
    }
    if before.size as u64 > limit as u64 {
        return Err(rustix::io::Errno::FBIG);
    }
    let mut file = File::from(fd);
    let mut contents = Vec::new();
    file.by_ref()
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut contents)
        .map_err(|error| {
            error
                .raw_os_error()
                .map_or(rustix::io::Errno::IO, rustix::io::Errno::from_raw_os_error)
        })?;
    if contents.len() > limit {
        return Err(rustix::io::Errno::FBIG);
    }
    let after = identity_of(&file)?;
    if before != after {
        return Err(rustix::io::Errno::STALE);
    }
    Ok(ReadEvidence {
        evidence: FileEvidence {
            path,
            identity: before,
        },
        contents,
    })
}

fn open_absolute_directory(path: &Path) -> Result<OwnedFd, rustix::io::Errno> {
    let mut components = path.components();
    if components.next() != Some(Component::RootDir) {
        return Err(rustix::io::Errno::INVAL);
    }
    let mut current = rustix::fs::open("/", directory_open_flags(), Mode::empty())?;
    for component in components {
        let Component::Normal(name) = component else {
            return Err(rustix::io::Errno::INVAL);
        };
        current = rustix::fs::openat(&current, name, directory_open_flags(), Mode::empty())?;
    }
    Ok(current)
}

fn open_relative_directory(
    root_fd: &OwnedFd,
    relative: &Path,
) -> Result<OwnedFd, rustix::io::Errno> {
    let mut current = rustix::io::dup(root_fd)?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(rustix::io::Errno::INVAL);
        };
        current = rustix::fs::openat(&current, name, directory_open_flags(), Mode::empty())?;
    }
    Ok(current)
}

fn directory_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::DIRECTORY
}

fn file_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK
}

fn identity_of(fd: &impl AsFd) -> Result<Identity, rustix::io::Errno> {
    rustix::fs::fstat(fd).map(|stat| Identity::from_stat(&stat))
}

fn stat_relative(
    root_fd: &OwnedFd,
    relative: &Path,
) -> Result<Option<Identity>, rustix::io::Errno> {
    let (parent, name) = split_relative_file_raw(relative)?;
    let parent_fd = match open_relative_directory(root_fd, parent) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(error),
    };
    match rustix::fs::statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(Some(Identity::from_stat(&stat))),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(error),
    }
}

fn split_relative_file(relative: &Path) -> Result<(&Path, &OsStr), InspectionIssue> {
    split_relative_file_raw(relative).map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)
}

fn split_relative_file_raw(relative: &Path) -> Result<(&Path, &OsStr), rustix::io::Errno> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(rustix::io::Errno::INVAL);
    }
    let name = relative.file_name().ok_or(rustix::io::Errno::INVAL)?;
    Ok((relative.parent().unwrap_or_else(|| Path::new("")), name))
}

fn split_absolute_file(path: &Path) -> Result<(&Path, &OsStr), InspectionIssue> {
    if !is_normal_absolute(path) {
        return Err(InspectionIssue::EnvironmentUnsupported);
    }
    let name = path
        .file_name()
        .ok_or(InspectionIssue::EnvironmentUnsupported)?;
    let parent = path
        .parent()
        .ok_or(InspectionIssue::EnvironmentUnsupported)?;
    Ok((parent, name))
}

fn is_normal_absolute(path: &Path) -> bool {
    let mut components = path.components();
    components.next() == Some(Component::RootDir)
        && components.all(|component| matches!(component, Component::Normal(_)))
}

fn parse_single_reference<'a>(
    contents: &'a [u8],
    prefix: &[u8],
) -> Result<&'a [u8], InspectionIssue> {
    let contents = contents.strip_suffix(b"\n").unwrap_or(contents);
    let value = contents
        .strip_prefix(prefix)
        .ok_or(InspectionIssue::UnsafeRepositoryMetadata)?;
    if value.is_empty() || value.contains(&0) || value.contains(&b'\n') || value.contains(&b'\r') {
        return Err(InspectionIssue::UnsafeRepositoryMetadata);
    }
    Ok(value)
}

fn resolve_internal_reference(
    copy_root: &Path,
    base_relative: &Path,
    reference: &[u8],
) -> Result<PathBuf, InspectionIssue> {
    validate_reference_traversal(copy_root, base_relative, reference)?;
    normalize_internal_reference(copy_root, base_relative, reference)
}

fn validate_reference_traversal(
    copy_root: &Path,
    base_relative: &Path,
    reference: &[u8],
) -> Result<(), InspectionIssue> {
    let reference = PathBuf::from(OsString::from_vec(reference.to_vec()));
    let root_fd = open_absolute_directory(copy_root)
        .map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;
    let mut directories = vec![root_fd];
    let mut position = PathBuf::new();
    if !reference.is_absolute() {
        for component in base_relative.components() {
            let Component::Normal(name) = component else {
                return Err(InspectionIssue::UnsafeRepositoryMetadata);
            };
            let next = rustix::fs::openat(
                directories.last().expect("root directory retained"),
                name,
                directory_open_flags(),
                Mode::empty(),
            )
            .map_err(|_| InspectionIssue::UnsafeRepositoryMetadata)?;
            directories.push(next);
            position.push(name);
        }
    }
    let components = if reference.is_absolute() {
        reference
            .strip_prefix(copy_root)
            .map_err(|_| InspectionIssue::ExternalRepositoryMetadata)?
            .components()
            .collect::<Vec<_>>()
    } else {
        reference.components().collect::<Vec<_>>()
    };
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::Normal(name) => {
                position.push(*name);
                if index + 1 < components.len() {
                    let next = rustix::fs::openat(
                        directories.last().expect("current directory retained"),
                        *name,
                        directory_open_flags(),
                        Mode::empty(),
                    )
                    .map_err(|_| InspectionIssue::ExternalRepositoryMetadata)?;
                    directories.push(next);
                }
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !position.pop() {
                    return Err(InspectionIssue::ExternalRepositoryMetadata);
                }
                directories.pop();
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(InspectionIssue::ExternalRepositoryMetadata);
            }
        }
    }
    Ok(())
}

/// Pure lexical normalization used by the fuzz harness and syntactic
/// `.gitmodules` checks. Filesystem-bearing references must use
/// `resolve_internal_reference`, which first validates raw traversal.
fn normalize_internal_reference(
    copy_root: &Path,
    base_relative: &Path,
    reference: &[u8],
) -> Result<PathBuf, InspectionIssue> {
    let reference = PathBuf::from(OsString::from_vec(reference.to_vec()));
    let components = if reference.is_absolute() {
        reference
            .strip_prefix(copy_root)
            .map_err(|_| InspectionIssue::ExternalRepositoryMetadata)?
            .components()
            .collect::<Vec<_>>()
    } else {
        base_relative
            .components()
            .chain(reference.components())
            .collect::<Vec<_>>()
    };
    let mut normalized = PathBuf::new();
    for component in components {
        match component {
            Component::Normal(name) => normalized.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(InspectionIssue::ExternalRepositoryMetadata);
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(InspectionIssue::ExternalRepositoryMetadata);
            }
        }
    }
    Ok(normalized)
}

fn trim_ascii(value: &[u8]) -> &[u8] {
    value.trim_ascii()
}

fn path_depth(path: &Path) -> usize {
    path.components().count()
}

fn is_strict_descendant(candidate: &Path, parent: &Path) -> bool {
    candidate != parent && candidate.starts_with(parent)
}

fn push_unique(issues: &mut Vec<InspectionIssue>, issue: InspectionIssue) {
    if !issues.contains(&issue) {
        issues.push(issue);
    }
}

/// Exercises only bounded, in-memory parsers for the repository-inspection
/// fuzz target. This symbol does not exist in normal library or release builds.
#[cfg(fuzzing)]
pub fn exercise_pure_parsers(input: &[u8]) {
    exercise_pure_parser_inputs(input);
}

#[cfg(any(test, fuzzing))]
fn exercise_pure_parser_inputs(input: &[u8]) {
    if let Ok(entries) = parse_config_output(input) {
        assert!(input.is_empty() || input.ends_with(&[0]));
        assert!(entries.iter().all(|entry| !entry.key.is_empty()));
        let _ = validate_config_entries(&entries);
    }
    if let Ok(reference) = parse_single_reference(input, b"") {
        assert!(!reference.is_empty());
        assert!(!reference.contains(&0));
        assert!(!reference.contains(&b'\n'));
        assert!(!reference.contains(&b'\r'));
    }
    let _ = validate_attributes(input);
    let _ = validate_index_flags_output(input);
    let _ = environment_disables(Some(OsStr::from_bytes(input)));
    let bool_entry = ConfigEntry {
        key: b"extensions.worktreeconfig".to_vec(),
        value: input.to_vec(),
    };
    let _ = effective_config_bool(&[bool_entry], b"extensions.worktreeconfig");
    if let Ok(path) = normalize_internal_reference(
        Path::new("/synthetic-copy-root"),
        Path::new("repository/.git"),
        input,
    ) {
        assert!(!path.is_absolute());
        assert!(
            path.components()
                .all(|component| matches!(component, Component::Normal(_)))
        );
    }

    assert!(parse_config_output(b"core.bare\nfalse\0").is_ok());
    assert_eq!(
        parse_config_output(b"missing-terminator").map(|_| ()),
        Err(InspectionIssue::InvalidGitOutput)
    );
    assert!(parse_single_reference(b"gitdir: ../metadata\n", b"gitdir: ").is_ok());
    assert!(parse_single_reference(b"gitdir: bad\nsecond\n", b"gitdir: ").is_err());
    assert!(validate_index_flags_output(b"").is_ok());
    assert!(validate_index_flags_output(b"H tracked\0").is_ok());
    assert_eq!(
        validate_index_flags_output(b"h tracked\0"),
        Err(InspectionIssue::HiddenIndexFlags)
    );
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use std::{env, fs};

    use super::super::{Budget, collect_command, set_nonblocking};
    use super::*;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    const CAPTURE_HELPER: &str = "THINWS_P0_07_INSPECT_CAPTURE_HELPER";
    const FIFO_PROBE_MODE: &str = "THINWS_P0_07_INSPECT_FIFO_PROBE_MODE";
    const FIFO_PROBE_ROOT: &str = "THINWS_P0_07_INSPECT_FIFO_PROBE_ROOT";
    const FIFO_PROBE_TEST: &str = "git_query::inspect::tests::fifo_probe_child";
    const FIFO_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
    const FIFO_BLOCKING_ENTRY_MARKER: &[u8] = b"thinws-fifo-blocking-open-entered\n";

    struct Fixture {
        root: PathBuf,
        home: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let unique = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos();
            let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("p0-07-inspect-fixtures")
                .join(format!("{label}-{}-{nanos}-{unique}", std::process::id()));
            let root = base.join("copy-root");
            let home = base.join("home");
            fs::create_dir_all(&root).expect("create retained copy root");
            fs::create_dir_all(&home).expect("create retained fixture home");
            Self { root, home }
        }

        fn environment(&self) -> SourceEnvironment {
            SourceEnvironment {
                home: Some(self.home.clone().into_os_string()),
                git_config_nosystem: Some(OsString::from("1")),
                git_attr_nosystem: Some(OsString::from("1")),
                ..SourceEnvironment::default()
            }
        }

        fn git(&self, cwd: &Path, args: &[&str]) -> Output {
            let output = Command::new("/usr/bin/git")
                .current_dir(cwd)
                .args(args)
                .env_clear()
                .env("HOME", &self.home)
                .env("LC_ALL", "C")
                .env("LANG", "C")
                .env("PATH", "/usr/bin:/bin")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_ATTR_NOSYSTEM", "1")
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
                .expect("run controlled fixture Git");
            assert!(
                output.status.success(),
                "fixture Git failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            output
        }

        fn init_repo(&self, path: &Path) {
            fs::create_dir_all(path).expect("create repository directory");
            self.git(path, &["init", "--quiet"]);
            self.git(path, &["config", "user.name", "P0 Fixture"]);
            self.git(path, &["config", "user.email", "p0@example.invalid"]);
            fs::write(path.join("tracked.txt"), b"tracked\n").expect("write tracked file");
            self.git(path, &["add", "tracked.txt"]);
            self.git(path, &["commit", "--quiet", "-m", "fixture"]);
        }
    }

    fn run_fifo_probe(
        mode: &str,
        root: &Path,
        run_timeout: Duration,
    ) -> Result<super::super::GitQueryOutput, super::super::GitQueryFailure> {
        let (reader, _writer) = UnixStream::pair().expect("owned collector preflight socket pair");
        set_nonblocking(&reader).expect("configure collector preflight socket");
        assert!(
            rustix::fs::fcntl_getfl(&reader)
                .expect("read collector preflight socket flags")
                .contains(OFlags::NONBLOCK),
            "collector pipe configuration must enable O_NONBLOCK before spawning a FIFO probe"
        );

        let mut command = Command::new(env::current_exe().expect("current unit-test binary"));
        command
            .args([
                "--ignored",
                "--exact",
                FIFO_PROBE_TEST,
                "--nocapture",
                "--test-threads=1",
            ])
            .current_dir(root)
            .env_clear()
            .env(FIFO_PROBE_MODE, mode)
            .env(FIFO_PROBE_ROOT, root);
        collect_command(
            command,
            Budget {
                run_timeout,
                cleanup_timeout: Duration::from_secs(1),
                poll_interval: Duration::from_millis(5),
                stdout_limit: 64 * 1024,
                stderr_limit: 64 * 1024,
            },
        )
    }

    fn expect_fifo_probe_success(mode: &str, root: &Path) {
        let output = run_fifo_probe(mode, root, FIFO_PROBE_TIMEOUT)
            .unwrap_or_else(|failure| panic!("isolated FIFO probe failed: {failure:?}"));
        assert!(
            output.exit.success(),
            "isolated FIFO probe returned a nonzero status: {output:?}"
        );
    }

    #[test]
    #[ignore = "fixed internal child entry; parent tests provide a controlled retained FIFO fixture"]
    fn fifo_probe_child() {
        let Some(mode) = env::var_os(FIFO_PROBE_MODE) else {
            return;
        };
        let root = PathBuf::from(
            env::var_os(FIFO_PROBE_ROOT).expect("FIFO probe root accompanies probe mode"),
        );
        let fixture_parent = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("p0-07-inspect-fixtures");
        assert!(root.is_absolute(), "FIFO probe root must be absolute");
        assert!(
            root.starts_with(&fixture_parent),
            "FIFO probe root must be a retained test fixture"
        );

        match mode.to_str().expect("fixed FIFO probe mode is UTF-8") {
            "metadata-scan" => {
                let root_fd =
                    open_absolute_directory(&root).expect("open controlled metadata scan root");
                let mut budget = CooperativeBudget::new(InspectionLimits::production());
                assert_eq!(
                    scan_metadata_nofollow(&root_fd, Path::new("metadata"), &mut budget),
                    Err(InspectionIssue::UnsafeRepositoryMetadata)
                );
            }
            "absolute-read" => {
                let fifo = root.join("special-fifo");
                let fd = rustix::fs::open(&fifo, file_open_flags(), Mode::empty())
                    .expect("open controlled FIFO without blocking");
                assert_eq!(
                    read_open_file(fd, fifo.clone(), 4).map(|_| ()),
                    Err(rustix::io::Errno::FBIG)
                );
                assert_eq!(
                    read_absolute_file(&fifo, 4, true).map(|_| ()),
                    Err(InspectionIssue::EnvironmentUnsupported)
                );
            }
            "combined-read" => {
                let fifo = root.join("metadata.fifo");
                assert_eq!(
                    read_absolute_file(&fifo, MAX_TEXT_BYTES, true).map(|_| ()),
                    Err(InspectionIssue::EnvironmentUnsupported)
                );
                let root_fd = open_absolute_directory(&root).expect("open controlled read root");
                assert_eq!(
                    read_root_file(
                        &root,
                        &root_fd,
                        Path::new("metadata.fifo"),
                        MAX_TEXT_BYTES,
                        true,
                    )
                    .map(|_| ()),
                    Err(InspectionIssue::UnsafeRepositoryMetadata)
                );
            }
            "blocking-open" => {
                let mut stdout = io::stdout().lock();
                stdout
                    .write_all(FIFO_BLOCKING_ENTRY_MARKER)
                    .expect("write fixed blocking-entry marker");
                stdout.flush().expect("flush fixed blocking-entry marker");
                drop(stdout);
                let _blocked = fs::File::open(root.join("blocking.fifo"))
                    .expect("intentional blocking FIFO open unexpectedly returned an error");
                panic!("intentional blocking FIFO open unexpectedly completed");
            }
            other => panic!("unknown fixed FIFO probe mode: {other}"),
        }
    }

    #[test]
    fn fifo_probe_watchdog_kills_and_reaps_a_true_block() {
        let fixture = Fixture::new("fifo-probe-watchdog");
        let fifo = fixture.root.join("blocking.fifo");
        let output = Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .env_clear()
            .output()
            .expect("create controlled blocking FIFO");
        assert!(output.status.success(), "mkfifo must succeed");

        let failure = run_fifo_probe("blocking-open", &fixture.root, Duration::from_secs(2))
            .expect_err("a genuinely blocking FIFO open must hit its watchdog");
        let entered_marker_seen = failure
            .stdout
            .windows(FIFO_BLOCKING_ENTRY_MARKER.len())
            .any(|bytes| bytes == FIFO_BLOCKING_ENTRY_MARKER);
        assert!(
            entered_marker_seen,
            "child must flush the fixed marker before entering the blocking open: {failure:?}"
        );
        assert_eq!(failure.kind, super::super::GitQueryFailureKind::TimedOut);
        assert!(matches!(
            failure.direct_child_exit,
            super::super::DirectChildExit::Confirmed(_)
        ));
        assert_eq!(failure.cleanup_io, None);
        eprintln!(
            "blocking FIFO probe: entered_marker_seen={entered_marker_seen}, kind={:?}, direct_child_exit={:?}, cleanup_io={:?}",
            failure.kind, failure.direct_child_exit, failure.cleanup_io,
        );
    }

    fn repository<'a>(inspection: &'a GitInspection, relative: &str) -> &'a RepositoryInspection {
        inspection
            .repositories
            .iter()
            .find(|repository| repository.relative_path == Path::new(relative))
            .unwrap_or_else(|| panic!("missing repository {relative:?}: {inspection:?}"))
    }

    fn owned_directory_identity(path: &Path) -> Identity {
        identity_of(&open_absolute_directory(path).expect("open owned directory"))
            .expect("owned directory identity")
    }

    fn prepared_identity_fixture(fixture: &Fixture) -> PreparedRepository {
        let worktree = fixture.root.join("worktree");
        let git_dir = fixture.root.join("git-dir");
        let common_dir = fixture.root.join("common-dir");
        fs::create_dir_all(&worktree).expect("create identity worktree");
        fs::create_dir_all(&git_dir).expect("create identity git dir");
        fs::create_dir_all(&common_dir).expect("create identity common dir");
        PreparedRepository {
            relative_path: PathBuf::from("worktree"),
            worktree_identity: owned_directory_identity(&worktree),
            git_dir: git_dir.clone(),
            git_dir_identity: owned_directory_identity(&git_dir),
            common_dir: common_dir.clone(),
            common_dir_identity: owned_directory_identity(&common_dir),
            index_identity: None,
            evidence: Vec::new(),
            issues: Vec::new(),
        }
    }

    fn replace_owned_directory(path: &Path, retained_name: &str) {
        let retained = path.with_file_name(retained_name);
        fs::rename(path, retained).expect("retain original owned directory");
        fs::create_dir(path).expect("create replacement owned directory");
    }

    fn repository_metadata_snapshot(git_dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let head = fs::read(git_dir.join("HEAD")).expect("read repository HEAD");
        let reference = head
            .strip_prefix(b"ref: ")
            .and_then(|value| value.strip_suffix(b"\n"))
            .expect("fixture HEAD is symbolic");
        let reference_path = git_dir.join(OsStr::from_bytes(reference));
        [
            git_dir.join("index"),
            git_dir.join("config"),
            git_dir.join("HEAD"),
            reference_path,
        ]
        .into_iter()
        .map(|path| {
            let contents = fs::read(&path).expect("read repository metadata snapshot");
            (path, contents)
        })
        .collect()
    }

    fn replace_root_before_final_revalidation(copy_root: &Path) {
        let retained = copy_root.with_file_name("copy-root-before-final-revalidation");
        fs::rename(copy_root, &retained).expect("retain original copy root");
        fs::create_dir(copy_root).expect("create replacement copy root");
    }

    #[test]
    fn final_root_revalidation_reports_a_real_identity_replacement() {
        let fixture = Fixture::new("final-root-revalidation");
        fixture.init_repo(&fixture.root);
        let original =
            identity_of(&open_absolute_directory(&fixture.root).expect("open original copy root"))
                .expect("original copy-root identity");

        let inspection = inspect_with_options(
            &fixture.root,
            fixture.environment(),
            InspectionLimits {
                before_final_root_revalidation: replace_root_before_final_revalidation,
                ..InspectionLimits::production()
            },
        );

        let retained = fixture
            .root
            .with_file_name("copy-root-before-final-revalidation");
        assert!(retained.join(".git").is_dir());
        let replacement = identity_of(
            &open_absolute_directory(&fixture.root).expect("open replacement copy root"),
        )
        .expect("replacement copy-root identity");
        assert_ne!(replacement, original);
        assert_eq!(inspection.discovery, DiscoveryCompleteness::Incomplete);
        assert!(
            inspection
                .issues
                .contains(&InspectionIssue::EvidenceChanged)
        );
        assert_eq!(inspection.aggregate, GitState::Unknown);
    }

    #[test]
    fn discovers_root_ignored_nested_and_internal_submodule() {
        let fixture = Fixture::new("discovery");
        fixture.init_repo(&fixture.root);
        fs::write(fixture.root.join(".gitignore"), b"ignored/\n").expect("write ignore rule");
        fixture.git(&fixture.root, &["add", ".gitignore"]);
        fixture.git(&fixture.root, &["commit", "--quiet", "-m", "ignore"]);

        let nested = fixture.root.join("ignored/nested");
        fixture.init_repo(&nested);

        let source = fixture
            .root
            .parent()
            .expect("fixture base")
            .join("sub-source");
        fixture.init_repo(&source);
        fixture.git(
            &fixture.root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "--quiet",
                source.to_str().expect("UTF-8 fixture path"),
                "modules/sub",
            ],
        );
        fixture.git(&fixture.root, &["commit", "--quiet", "-am", "submodule"]);

        fs::write(nested.join("tracked.txt"), b"nested dirty\n").expect("dirty nested repo");
        fs::write(fixture.root.join("modules/sub/tracked.txt"), b"sub dirty\n")
            .expect("dirty submodule");

        let inspection = inspect_with_environment(&fixture.root, fixture.environment());
        assert_eq!(inspection.discovery, DiscoveryCompleteness::Complete);
        assert!(matches!(
            repository(&inspection, "").state,
            RepositoryState::Dirty { .. }
        ));
        assert!(matches!(
            repository(&inspection, "ignored/nested").state,
            RepositoryState::Dirty { .. }
        ));
        assert!(matches!(
            repository(&inspection, "modules/sub").state,
            RepositoryState::Dirty { .. }
        ));
        assert_eq!(inspection.aggregate, GitState::Dirty);
    }

    #[test]
    fn real_untracked_and_staged_changes_have_exact_counts_without_metadata_writes() {
        let fixture = Fixture::new("real-staged-counts");
        fixture.init_repo(&fixture.root);
        fs::write(fixture.root.join("deleted.txt"), b"delete me\n")
            .expect("write deletion baseline");
        fs::write(fixture.root.join("rename-source.txt"), b"rename me\n")
            .expect("write rename baseline");
        fixture.git(&fixture.root, &["add", "deleted.txt", "rename-source.txt"]);
        fixture.git(
            &fixture.root,
            &["commit", "--quiet", "-m", "more tracked files"],
        );

        fs::write(fixture.root.join("untracked.txt"), b"untracked\n")
            .expect("write untracked file");
        let before_untracked = repository_metadata_snapshot(&fixture.root.join(".git"));
        let untracked = inspect_with_environment(&fixture.root, fixture.environment());
        assert_eq!(
            repository(&untracked, "").state.tracked_change_count(),
            Some(0)
        );
        assert_eq!(
            repository_metadata_snapshot(&fixture.root.join(".git")),
            before_untracked
        );

        fs::write(fixture.root.join("added.txt"), b"staged addition\n")
            .expect("write staged addition");
        fixture.git(&fixture.root, &["add", "added.txt"]);
        fixture.git(&fixture.root, &["rm", "--quiet", "deleted.txt"]);
        fixture.git(
            &fixture.root,
            &["mv", "rename-source.txt", "rename-target.txt"],
        );
        let before_staged = repository_metadata_snapshot(&fixture.root.join(".git"));
        let staged = inspect_with_environment(&fixture.root, fixture.environment());
        assert_eq!(
            repository(&staged, "").state.tracked_change_count(),
            Some(3)
        );
        assert_eq!(staged.aggregate, GitState::Dirty);
        assert_eq!(
            repository_metadata_snapshot(&fixture.root.join(".git")),
            before_staged
        );
    }

    #[test]
    fn real_submodule_and_conflict_states_have_exact_counts_without_metadata_writes() {
        let submodule = Fixture::new("real-submodule-counts");
        submodule.init_repo(&submodule.root);
        let source = submodule
            .root
            .parent()
            .expect("fixture base")
            .join("submodule-source");
        submodule.init_repo(&source);
        submodule.git(
            &submodule.root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "--quiet",
                source.to_str().expect("UTF-8 fixture path"),
                "modules/sub",
            ],
        );
        submodule.git(
            &submodule.root,
            &["commit", "--quiet", "-am", "add submodule"],
        );
        let child = submodule.root.join("modules/sub");
        let child_git_dir = submodule.root.join(".git/modules/modules/sub");
        submodule.git(&child, &["config", "user.name", "P0 Fixture"]);
        submodule.git(&child, &["config", "user.email", "p0@example.invalid"]);
        submodule.git(&child, &["checkout", "--quiet", "-b", "fixture-child"]);
        fs::write(child.join("untracked.txt"), b"untracked\n")
            .expect("write submodule untracked file");
        let before_untracked_parent = repository_metadata_snapshot(&submodule.root.join(".git"));
        let before_untracked_child = repository_metadata_snapshot(&child_git_dir);
        let untracked = inspect_with_environment(&submodule.root, submodule.environment());
        assert_eq!(
            repository(&untracked, "").state.tracked_change_count(),
            Some(0)
        );
        assert_eq!(
            repository(&untracked, "modules/sub")
                .state
                .tracked_change_count(),
            Some(0)
        );
        assert_eq!(untracked.aggregate, GitState::Clean);
        assert_eq!(
            repository_metadata_snapshot(&submodule.root.join(".git")),
            before_untracked_parent
        );
        assert_eq!(
            repository_metadata_snapshot(&child_git_dir),
            before_untracked_child
        );

        fs::write(child.join("tracked.txt"), b"tracked submodule change\n")
            .expect("modify tracked submodule file");
        let before_tracked_parent = repository_metadata_snapshot(&submodule.root.join(".git"));
        let before_tracked_child = repository_metadata_snapshot(&child_git_dir);
        let tracked_submodule = inspect_with_environment(&submodule.root, submodule.environment());
        assert_eq!(
            repository(&tracked_submodule, "")
                .state
                .tracked_change_count(),
            Some(1)
        );
        assert_eq!(
            repository(&tracked_submodule, "modules/sub")
                .state
                .tracked_change_count(),
            Some(1)
        );
        assert_eq!(tracked_submodule.aggregate, GitState::Dirty);
        assert_eq!(
            repository_metadata_snapshot(&submodule.root.join(".git")),
            before_tracked_parent
        );
        assert_eq!(
            repository_metadata_snapshot(&child_git_dir),
            before_tracked_child
        );

        submodule.git(&child, &["add", "tracked.txt"]);
        submodule.git(&child, &["commit", "--quiet", "-m", "advance submodule"]);
        let before_gitlink_parent = repository_metadata_snapshot(&submodule.root.join(".git"));
        let before_gitlink_child = repository_metadata_snapshot(&child_git_dir);
        let gitlink = inspect_with_environment(&submodule.root, submodule.environment());
        assert_eq!(
            repository(&gitlink, "").state.tracked_change_count(),
            Some(1)
        );
        assert_eq!(
            repository(&gitlink, "modules/sub")
                .state
                .tracked_change_count(),
            Some(0)
        );
        assert_eq!(gitlink.aggregate, GitState::Dirty);
        assert_eq!(
            repository_metadata_snapshot(&submodule.root.join(".git")),
            before_gitlink_parent
        );
        assert_eq!(
            repository_metadata_snapshot(&child_git_dir),
            before_gitlink_child
        );

        let conflict = Fixture::new("real-conflict-count");
        conflict.init_repo(&conflict.root);
        conflict.git(&conflict.root, &["branch", "-M", "main"]);
        conflict.git(&conflict.root, &["checkout", "--quiet", "-b", "side"]);
        fs::write(conflict.root.join("tracked.txt"), b"side\n").expect("write side change");
        conflict.git(&conflict.root, &["commit", "--quiet", "-am", "side"]);
        conflict.git(&conflict.root, &["checkout", "--quiet", "main"]);
        fs::write(conflict.root.join("tracked.txt"), b"main\n").expect("write main change");
        conflict.git(&conflict.root, &["commit", "--quiet", "-am", "main"]);
        let merge = Command::new("/usr/bin/git")
            .current_dir(&conflict.root)
            .args(["merge", "--no-edit", "side"])
            .env_clear()
            .env("HOME", &conflict.home)
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("run controlled conflicting merge");
        assert!(!merge.status.success(), "fixture merge must conflict");
        let unmerged = conflict.git(&conflict.root, &["ls-files", "-u"]);
        assert!(
            !unmerged.stdout.is_empty(),
            "fixture must contain native unmerged index entries"
        );
        let before_conflict = repository_metadata_snapshot(&conflict.root.join(".git"));
        let conflicted = inspect_with_environment(&conflict.root, conflict.environment());
        assert_eq!(
            repository(&conflicted, "").state.tracked_change_count(),
            Some(1)
        );
        assert_eq!(conflicted.aggregate, GitState::Dirty);
        assert_eq!(
            repository_metadata_snapshot(&conflict.root.join(".git")),
            before_conflict
        );
    }

    #[test]
    fn preserves_validated_user_config_and_native_attributes_semantics() {
        let fixture = Fixture::new("config-attributes");
        let attributes = fixture.home.join(".config/git/attributes");
        fs::create_dir_all(attributes.parent().expect("attributes parent"))
            .expect("create controlled attributes directory");
        fs::write(&attributes, b"*.txt text\n").expect("write controlled external attributes");
        fs::write(
            fixture.home.join(".gitconfig"),
            "[core]\n\tautocrlf = true\n[user]\n\tname = P0 Fixture\n\temail = p0@example.invalid\n",
        )
        .expect("write controlled global config");
        fixture.init_repo(&fixture.root);
        fs::remove_file(fixture.root.join("tracked.txt")).expect("remove tracked fixture file");
        fixture.git(&fixture.root, &["checkout", "--", "tracked.txt"]);
        assert!(
            fs::read(fixture.root.join("tracked.txt"))
                .expect("read native checkout")
                .ends_with(b"\r\n"),
            "native Git applies the validated conversion sources"
        );
        let native = fixture.git(
            &fixture.root,
            &["status", "--porcelain=v2", "-z", "--untracked-files=no"],
        );
        assert!(native.stdout.is_empty(), "native Git treats CRLF as clean");

        let mut environment = fixture.environment();
        environment.git_config_global = Some(fixture.home.join(".gitconfig").into_os_string());
        let inspection = inspect_with_environment(&fixture.root, environment);
        assert_eq!(inspection.discovery, DiscoveryCompleteness::Complete);
        assert_eq!(repository(&inspection, "").state, RepositoryState::Clean);
        assert_eq!(inspection.aggregate, GitState::Clean);
    }

    #[test]
    fn validates_repository_core_attributes_file_and_info_attributes() {
        let fixture = Fixture::new("repository-attributes");
        fixture.init_repo(&fixture.root);
        let attributes = fixture.home.join("repository-attributes");
        fs::write(&attributes, b"*.txt text\n").expect("write external attributes");
        fixture.git(
            &fixture.root,
            &[
                "config",
                "core.attributesFile",
                attributes.to_str().expect("UTF-8 fixture path"),
            ],
        );
        fs::write(fixture.root.join(".git/info/attributes"), b"*.md text\n")
            .expect("write info attributes");

        let inspection = inspect_with_environment(&fixture.root, fixture.environment());
        assert_eq!(repository(&inspection, "").state, RepositoryState::Clean);

        fs::write(
            fixture.root.join(".git/info/attributes"),
            b"*.txt filter=tripwire\n",
        )
        .expect("write unsafe info attributes");
        let rejected = inspect_with_environment(&fixture.root, fixture.environment());
        assert_eq!(repository(&rejected, "").state, RepositoryState::Unknown);
        assert!(
            repository(&rejected, "")
                .issues
                .contains(&InspectionIssue::UnsafeAttributes)
        );
    }

    #[test]
    fn resolves_tilde_attributes_and_rejects_named_user_expansion() {
        let fixture = Fixture::new("tilde-attributes");
        let environment = fixture.environment();
        assert_eq!(
            resolve_attributes_source(&environment, b"~/attributes"),
            Ok(fixture.home.join("attributes"))
        );
        assert_eq!(
            resolve_attributes_source(&environment, b"~other/attributes"),
            Err(InspectionIssue::UnsafeAttributes)
        );
    }

    #[test]
    fn unsafe_default_xdg_attributes_reach_repository_preflight() {
        let fixture = Fixture::new("unsafe-default-xdg-attributes");
        fixture.init_repo(&fixture.root);
        let xdg = fixture.root.parent().expect("fixture base").join("xdg");
        fs::create_dir_all(xdg.join("git")).expect("create controlled XDG Git directory");
        fs::write(xdg.join("git/attributes"), b"*.txt filter=tripwire\n")
            .expect("write unsafe default attributes");
        let mut environment = fixture.environment();
        environment.xdg_config_home = Some(xdg.into_os_string());

        let inspection = inspect_with_environment(&fixture.root, environment);
        let root = repository(&inspection, "");
        assert_eq!(root.state, RepositoryState::Unknown);
        assert!(root.issues.contains(&InspectionIssue::UnsafeAttributes));
    }

    #[test]
    fn attributes_parser_ignores_blank_and_comment_lines_only() {
        assert_eq!(
            validate_attributes(b"\n \t\n# ignored filter=tripwire\n*.txt text\n"),
            Ok(())
        );
        assert_eq!(
            validate_attributes(b"*.txt filter=tripwire\n"),
            Err(InspectionIssue::UnsafeAttributes)
        );
    }

    #[test]
    fn rejects_hidden_index_flags_and_sparse_checkout() {
        let hidden = Fixture::new("hidden-index");
        hidden.init_repo(&hidden.root);
        hidden.git(
            &hidden.root,
            &["update-index", "--skip-worktree", "tracked.txt"],
        );
        let hidden_inspection = inspect_with_environment(&hidden.root, hidden.environment());
        assert_eq!(
            repository(&hidden_inspection, "").state,
            RepositoryState::Unknown
        );
        assert!(
            repository(&hidden_inspection, "")
                .issues
                .contains(&InspectionIssue::HiddenIndexFlags)
        );

        let sparse = Fixture::new("sparse-checkout");
        sparse.init_repo(&sparse.root);
        fs::write(
            sparse.root.join(".git/info/sparse-checkout"),
            b"tracked.txt\n",
        )
        .expect("write sparse checkout patterns");
        let sparse_inspection = inspect_with_environment(&sparse.root, sparse.environment());
        assert_eq!(
            repository(&sparse_inspection, "").state,
            RepositoryState::Unknown
        );
        assert!(
            repository(&sparse_inspection, "")
                .issues
                .contains(&InspectionIssue::SparseCheckout)
        );
    }

    #[test]
    fn rejects_includes_filters_promisors_and_unclassified_core_settings() {
        for (label, key, value) in [
            ("include", "include.path", "/dev/null"),
            ("filter", "filter.tripwire.clean", "/bin/cat"),
            ("promisor", "remote.origin.promisor", "true"),
            ("unclassified", "core.preloadindex", "true"),
        ] {
            let fixture = Fixture::new(label);
            fixture.init_repo(&fixture.root);
            fixture.git(&fixture.root, &["config", key, value]);
            let inspection = inspect_with_environment(&fixture.root, fixture.environment());
            assert_eq!(
                repository(&inspection, "").state,
                RepositoryState::Unknown,
                "configuration {key} must not be silently classified as safe"
            );
            assert!(
                repository(&inspection, "")
                    .issues
                    .contains(&InspectionIssue::UnsupportedConfiguration),
                "configuration {key} must retain a structured reason"
            );
        }
    }

    #[test]
    fn parses_nosystem_environment_with_git_boolean_semantics() {
        assert!(!environment_disables(Some(OsStr::new(""))).expect("empty is false"));
        assert!(environment_disables(Some(OsStr::new("true"))).expect("true is true"));
        assert!(!environment_disables(Some(OsStr::new("0"))).expect("zero is false"));
        assert!(!environment_disables(Some(OsStr::new("false"))).expect("false is false"));
        assert!(environment_disables(Some(OsStr::new("1"))).expect("one is true"));
        assert!(environment_disables(Some(OsStr::new("yes"))).expect("yes is true"));
        assert!(environment_disables(Some(OsStr::new("on"))).expect("on is true"));
        assert_eq!(
            environment_disables(Some(OsStr::new("not-a-git-boolean"))),
            Err(InspectionIssue::EnvironmentUnsupported)
        );
    }

    #[test]
    fn config_output_parser_rejects_malformed_records_and_accepts_implicit_bool() {
        exercise_pure_parser_inputs(b"core.bare\nfalse\0");
        exercise_pure_parser_inputs(b"arbitrary\0bytes\nwithout-record-shape");
        let parsed = parse_config_output(b"core.bare\n\0user.name\nFixture\0")
            .expect("parse value-less and valued records");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].key, b"core.bare");
        assert!(parsed[0].value.is_empty());
        assert_eq!(parsed[1].key, b"user.name");
        assert_eq!(parsed[1].value, b"Fixture");

        for malformed in [
            b"core.bare\n".as_slice(),
            b"core.bare\0".as_slice(),
            b"\nvalue\0".as_slice(),
            b"core.bare\n\0\0".as_slice(),
        ] {
            assert_eq!(
                parse_config_output(malformed).map(|_| ()),
                Err(InspectionIssue::InvalidGitOutput)
            );
        }
    }

    #[test]
    fn config_file_output_accepts_the_exact_text_limit_and_rejects_one_less() {
        const EXPECTED: &[u8] = b"fixture.value\nabc\0";

        let fixture = Fixture::new("config-output-limit");
        let config = fixture.root.join("bounded.gitconfig");
        fs::write(&config, b"[fixture]\n\tvalue = abc\n").expect("write bounded config");
        let exact = CooperativeBudget::new(InspectionLimits {
            max_text_bytes: EXPECTED.len(),
            ..InspectionLimits::production()
        });
        let parsed = parse_config_file(&fixture.root, &config, &exact)
            .expect("exact config output limit is accepted");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].key, b"fixture.value");
        assert_eq!(parsed[0].value, b"abc");

        let short = CooperativeBudget::new(InspectionLimits {
            max_text_bytes: EXPECTED.len() - 1,
            ..InspectionLimits::production()
        });
        assert_eq!(
            parse_config_file(&fixture.root, &config, &short).map(|_| ()),
            Err(InspectionIssue::InvalidGitOutput)
        );
    }

    #[test]
    fn configuration_validator_covers_each_independent_policy_boundary() {
        fn entry(key: &[u8], value: &[u8]) -> ConfigEntry {
            ConfigEntry {
                key: key.to_vec(),
                value: value.to_vec(),
            }
        }
        fn assert_valid(key: &[u8], value: &[u8]) {
            assert_eq!(
                validate_config_entries(&[entry(key, value)]),
                Ok(()),
                "key={:?}, value={:?} must be accepted",
                String::from_utf8_lossy(key),
                String::from_utf8_lossy(value)
            );
        }
        fn assert_rejected(key: &[u8], value: &[u8]) {
            assert_eq!(
                validate_config_entries(&[entry(key, value)]),
                Err(InspectionIssue::UnsupportedConfiguration),
                "key={:?}, value={:?} must be rejected",
                String::from_utf8_lossy(key),
                String::from_utf8_lossy(value)
            );
        }

        for key in [
            b"core.fsmonitor".as_slice(),
            b"core.sparsecheckout".as_slice(),
            b"core.sparsecheckoutcone".as_slice(),
            b"core.bare".as_slice(),
            b"remote.origin.promisor".as_slice(),
        ] {
            for value in [
                b"false".as_slice(),
                b"FALSE".as_slice(),
                b"no".as_slice(),
                b"0".as_slice(),
                b"off".as_slice(),
            ] {
                assert_valid(key, value);
            }
            for value in [
                b"".as_slice(),
                b"true".as_slice(),
                b"yes".as_slice(),
                b"1".as_slice(),
                b"on".as_slice(),
                b"garbage".as_slice(),
            ] {
                assert_rejected(key, value);
            }
        }

        assert_valid(b"remote.origin.partialclonefilter", b"");
        assert_rejected(b"remote.origin.partialclonefilter", b"blob:none");
        for key in [
            b"include.path".as_slice(),
            b"includeif.gitdir:controlled.path".as_slice(),
            b"filter.tripwire.clean".as_slice(),
            b"core.hookspath".as_slice(),
            b"sparse.expectfilesoutsideofsparsecone".as_slice(),
        ] {
            assert_rejected(key, b"");
        }

        assert_valid(b"extensions.worktreeconfig", b"true");
        assert_rejected(b"extensions.future", b"");
        assert_valid(b"index.version", b"4");
        assert_rejected(b"index.future", b"");
        assert_rejected(b"status.showuntrackedfiles", b"no");
        assert_valid(b"core.autocrlf", b"input");
        assert_rejected(b"core.preloadindex", b"true");
        assert_valid(b"user.name", b"Fixture");

        for value in [b"".as_slice(), b"false".as_slice(), b"true".as_slice()] {
            assert_rejected(b"extensions.partialclone", value);
            assert_rejected(b"index.sparse", value);
        }

        let normalized =
            parse_config_output(b"CoRe.FsMonitor\nfalse\0").expect("parse mixed-case config key");
        assert_eq!(normalized[0].key, b"core.fsmonitor");
        assert_eq!(validate_config_entries(&normalized), Ok(()));
        let normalized = parse_config_output(b"CoRe.FsMonitor\n\0")
            .expect("parse value-less mixed-case config key");
        assert_eq!(
            validate_config_entries(&normalized),
            Err(InspectionIssue::UnsupportedConfiguration)
        );
    }

    #[test]
    fn effective_boolean_uses_git_values_and_the_last_matching_entry() {
        fn entry(key: &[u8], value: &[u8]) -> ConfigEntry {
            ConfigEntry {
                key: key.to_vec(),
                value: value.to_vec(),
            }
        }
        let key = b"extensions.worktreeconfig";
        assert_eq!(effective_config_bool(&[], key), Ok(false));
        for value in [
            b"".as_slice(),
            b"true".as_slice(),
            b"TRUE".as_slice(),
            b"yes".as_slice(),
            b"on".as_slice(),
            b"1".as_slice(),
        ] {
            assert_eq!(effective_config_bool(&[entry(key, value)], key), Ok(true));
        }
        for value in [
            b"false".as_slice(),
            b"no".as_slice(),
            b"0".as_slice(),
            b"off".as_slice(),
        ] {
            assert_eq!(effective_config_bool(&[entry(key, value)], key), Ok(false));
        }
        assert_eq!(
            effective_config_bool(&[entry(key, b"maybe")], key),
            Err(InspectionIssue::UnsupportedConfiguration)
        );
        assert_eq!(
            effective_config_bool(&[entry(key, b"true"), entry(key, b"false")], key),
            Ok(false)
        );
        assert_eq!(
            effective_config_bool(&[entry(key, b"false"), entry(key, b"yes")], key),
            Ok(true)
        );
        assert_eq!(
            effective_config_bool(&[entry(b"other.key", b"true")], key),
            Ok(false)
        );
    }

    #[test]
    fn index_flag_output_parser_covers_empty_shape_and_hidden_boundaries() {
        for valid in [
            b"".as_slice(),
            b"H x\0".as_slice(),
            b"H first\0M second\0".as_slice(),
        ] {
            assert_eq!(validate_index_flags_output(valid), Ok(()));
        }
        for malformed in [
            b"H x".as_slice(),
            b"\0".as_slice(),
            b"H x\0\0".as_slice(),
            b"H \0".as_slice(),
            b"H:x\0".as_slice(),
        ] {
            assert_eq!(
                validate_index_flags_output(malformed),
                Err(InspectionIssue::InvalidGitOutput)
            );
        }
        for hidden in [b"h x\0".as_slice(), b"S x\0".as_slice()] {
            assert_eq!(
                validate_index_flags_output(hidden),
                Err(InspectionIssue::HiddenIndexFlags)
            );
        }
    }

    #[test]
    fn query_result_mapping_preserves_safe_structure_without_raw_output() {
        let stdout_canary = b"inspection-raw-stdout-canary".to_vec();
        let stderr_canary = b"inspection-raw-stderr-canary".to_vec();
        let stdout_byte_debug = format!("{stdout_canary:?}");
        let stderr_byte_debug = format!("{stderr_canary:?}");
        let cleanup_io = CleanupIoFailure {
            operation: super::super::CleanupOperation::ConfirmPoll,
            error_kind: io::ErrorKind::PermissionDenied,
            raw_os_error: Some(13),
        };
        let issue = map_query_result(Err(GitQueryFailure {
            kind: GitQueryFailureKind::TimedOut,
            stdout: stdout_canary.clone(),
            stderr: stderr_canary.clone(),
            direct_child_exit: DirectChildExit::Unconfirmed,
            cleanup_io: Some(cleanup_io),
        }))
        .expect_err("transport failure must remain structured");
        assert_eq!(
            issue,
            InspectionIssue::QueryFailed {
                kind: GitQueryFailureKind::TimedOut,
                direct_child_exit: DirectChildExit::Unconfirmed,
                cleanup_io: Some(cleanup_io),
            }
        );
        let diagnostic = format!("{issue:?}");
        assert!(diagnostic.contains("QueryFailed"));
        assert!(diagnostic.contains("TimedOut"));
        assert!(diagnostic.contains("Unconfirmed"));
        assert!(diagnostic.contains("ConfirmPoll"));
        assert!(diagnostic.contains("PermissionDenied"));
        assert!(diagnostic.contains("Some(13)"));
        assert!(!diagnostic.contains("inspection-raw-stdout-canary"));
        assert!(!diagnostic.contains("inspection-raw-stderr-canary"));
        assert!(!diagnostic.contains(&stdout_byte_debug));
        assert!(!diagnostic.contains(&stderr_byte_debug));

        for exit in [
            GitExit {
                code: Some(23),
                signal: None,
            },
            GitExit {
                code: None,
                signal: Some(9),
            },
        ] {
            assert_eq!(
                map_query_result(Ok(GitQueryOutput {
                    exit,
                    stdout: stdout_canary.clone(),
                    stderr: stderr_canary.clone(),
                }))
                .map(|_| ()),
                Err(InspectionIssue::QueryNonZeroExit(exit))
            );
        }

        let output = map_query_result(Ok(GitQueryOutput {
            exit: GitExit {
                code: Some(0),
                signal: None,
            },
            stdout: b"valid-output".to_vec(),
            stderr: b"valid-diagnostic".to_vec(),
        }))
        .expect("successful query output must remain available to its parser");
        assert_eq!(output.stdout, b"valid-output");
        assert_eq!(output.stderr, b"valid-diagnostic");
    }

    #[test]
    fn real_config_nonzero_exit_reaches_the_inspection_issue() {
        let fixture = Fixture::new("real-config-failure");
        let malformed = fixture.root.join("malformed.gitconfig");
        fs::write(&malformed, b"[unterminated\n").expect("write malformed config");
        let budget = CooperativeBudget::new(InspectionLimits::production());
        let issue = match parse_config_file(&fixture.root, &malformed, &budget) {
            Err(issue) => issue,
            Ok(_) => panic!("native Git must reject malformed config"),
        };
        assert!(matches!(
            issue,
            InspectionIssue::QueryNonZeroExit(GitExit {
                code: Some(code),
                signal: None,
            }) if code != 0
        ));
        let diagnostic = format!("{issue:?}");
        assert!(diagnostic.contains("QueryNonZeroExit"));
        assert!(!diagnostic.contains("unterminated"));
    }

    #[test]
    fn reference_parser_is_single_line_and_bounded_by_its_caller() {
        assert_eq!(
            parse_single_reference(b"gitdir: ../metadata\n", b"gitdir: ")
                .expect("parse valid gitfile"),
            b"../metadata"
        );
        for malformed in [
            b"gitdir: \n".as_slice(),
            b"../metadata\nsecond\n".as_slice(),
            b"gitdir: path\0suffix\n".as_slice(),
            b"wrong: path\n".as_slice(),
        ] {
            assert_eq!(
                parse_single_reference(malformed, b"gitdir: ").map(|_| ()),
                Err(InspectionIssue::UnsafeRepositoryMetadata)
            );
        }
    }

    #[test]
    fn reference_collection_requires_head_collects_it_and_rejects_nonhex_object_ids() {
        let valid = Fixture::new("reference-evidence-valid");
        valid.init_repo(&valid.root);
        let root_fd = open_absolute_directory(&valid.root).expect("open valid reference root");
        let budget = CooperativeBudget::new(InspectionLimits::production());
        let mut evidence = Vec::new();
        collect_reference_evidence(
            &valid.root,
            &root_fd,
            Path::new(".git"),
            Path::new(".git"),
            &budget,
            &mut evidence,
        )
        .expect("collect valid reference evidence");
        assert!(
            evidence
                .iter()
                .any(|item| item.path == valid.root.join(".git/HEAD"))
        );

        let missing = Fixture::new("reference-evidence-missing-head");
        fs::create_dir_all(missing.root.join("repository/.git"))
            .expect("create metadata without HEAD");
        let root_fd = open_absolute_directory(&missing.root).expect("open missing reference root");
        let mut evidence = Vec::new();
        assert_eq!(
            collect_reference_evidence(
                &missing.root,
                &root_fd,
                Path::new("repository/.git"),
                Path::new("repository/.git"),
                &budget,
                &mut evidence,
            ),
            Err(InspectionIssue::UnsafeRepositoryMetadata)
        );

        let invalid = Fixture::new("reference-evidence-invalid-object-id");
        fs::create_dir_all(invalid.root.join("repository/.git"))
            .expect("create invalid reference metadata");
        let mut object_id = vec![b'0'; 40];
        object_id[39] = b'g';
        fs::write(invalid.root.join("repository/.git/HEAD"), object_id)
            .expect("write nonhex direct HEAD");
        let root_fd = open_absolute_directory(&invalid.root).expect("open invalid reference root");
        let mut evidence = Vec::new();
        assert_eq!(
            collect_reference_evidence(
                &invalid.root,
                &root_fd,
                Path::new("repository/.git"),
                Path::new("repository/.git"),
                &budget,
                &mut evidence,
            ),
            Err(InspectionIssue::UnsafeRepositoryMetadata)
        );
    }

    #[test]
    fn symbolic_references_require_the_refs_namespace_and_normal_components() {
        assert_eq!(
            validate_symbolic_ref(b"refs/heads/main"),
            Ok(PathBuf::from("refs/heads/main"))
        );
        for reference in [b"heads/main".as_slice(), b"refs/heads/../main".as_slice()] {
            assert_eq!(
                validate_symbolic_ref(reference),
                Err(InspectionIssue::UnsafeRepositoryMetadata)
            );
        }
    }

    #[test]
    fn metadata_scan_enforces_initial_and_recursive_depth() {
        let fixture = Fixture::new("metadata-scan-depth");
        fs::create_dir_all(fixture.root.join("at/limit")).expect("create directory at limit");
        fs::create_dir_all(fixture.root.join("past/the/limit"))
            .expect("create directory past limit");
        fs::create_dir_all(fixture.root.join("metadata/one/two"))
            .expect("create recursively deep metadata");
        let root_fd = open_absolute_directory(&fixture.root).expect("open metadata depth root");

        let mut at_limit = CooperativeBudget::new(InspectionLimits {
            max_depth: 2,
            ..InspectionLimits::production()
        });
        assert_eq!(
            scan_metadata_nofollow(&root_fd, Path::new("at/limit"), &mut at_limit),
            Ok(())
        );
        let mut past_limit = CooperativeBudget::new(InspectionLimits {
            max_depth: 2,
            ..InspectionLimits::production()
        });
        assert_eq!(
            scan_metadata_nofollow(&root_fd, Path::new("past/the/limit"), &mut past_limit),
            Err(InspectionIssue::DepthLimitReached)
        );
        let mut recursive = CooperativeBudget::new(InspectionLimits {
            max_depth: 2,
            ..InspectionLimits::production()
        });
        assert_eq!(
            scan_metadata_nofollow(&root_fd, Path::new("metadata"), &mut recursive),
            Err(InspectionIssue::DepthLimitReached)
        );
    }

    #[test]
    fn metadata_scan_accepts_exact_text_limits_and_rejects_the_next_byte() {
        let fixture = Fixture::new("metadata-scan-size");
        for directory in ["empty", "exact", "over"] {
            fs::create_dir_all(fixture.root.join(directory)).expect("create metadata size root");
        }
        fs::write(fixture.root.join("empty/FETCH_HEAD"), b"").expect("write empty metadata");
        fs::write(fixture.root.join("exact/FETCH_HEAD"), b"1234").expect("write exact metadata");
        fs::write(fixture.root.join("over/FETCH_HEAD"), b"12345")
            .expect("write oversized metadata");
        let root_fd = open_absolute_directory(&fixture.root).expect("open metadata size root");

        let mut empty = CooperativeBudget::new(InspectionLimits {
            max_text_bytes: 0,
            ..InspectionLimits::production()
        });
        assert_eq!(
            scan_metadata_nofollow(&root_fd, Path::new("empty"), &mut empty),
            Ok(())
        );
        let mut exact = CooperativeBudget::new(InspectionLimits {
            max_text_bytes: 4,
            ..InspectionLimits::production()
        });
        assert_eq!(
            scan_metadata_nofollow(&root_fd, Path::new("exact"), &mut exact),
            Ok(())
        );
        let mut over = CooperativeBudget::new(InspectionLimits {
            max_text_bytes: 4,
            ..InspectionLimits::production()
        });
        assert_eq!(
            scan_metadata_nofollow(&root_fd, Path::new("over"), &mut over),
            Err(InspectionIssue::UnsafeRepositoryMetadata)
        );
    }

    #[test]
    fn metadata_scan_rejects_a_real_fifo_without_opening_it() {
        let fixture = Fixture::new("metadata-scan-fifo");
        fs::create_dir_all(fixture.root.join("metadata")).expect("create FIFO metadata root");
        let fifo = fixture.root.join("metadata/ordinary-fifo");
        let output = Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .env_clear()
            .output()
            .expect("create controlled metadata FIFO");
        assert!(output.status.success(), "mkfifo must succeed");
        expect_fifo_probe_success("metadata-scan", &fixture.root);
    }

    #[test]
    fn bounded_metadata_classifier_covers_fixed_names_refs_and_binary_files() {
        assert!(is_bounded_metadata_text(
            Path::new(".git/HEAD"),
            Path::new(".git")
        ));
        assert!(is_bounded_metadata_text(
            Path::new(".git/refs/heads/main"),
            Path::new(".git")
        ));
        assert!(!is_bounded_metadata_text(
            Path::new(".git/objects/aa/bb"),
            Path::new(".git")
        ));
    }

    #[test]
    fn metadata_link_rejection_checks_the_specific_index_path() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("metadata-link-rejection");
        fs::create_dir_all(fixture.root.join(".git/objects")).expect("create objects dir");
        fs::create_dir_all(fixture.root.join(".git/refs")).expect("create refs dir");
        fs::write(fixture.root.join("outside-index"), b"outside\n")
            .expect("write controlled index target");
        symlink("../outside-index", fixture.root.join(".git/index"))
            .expect("create controlled index symlink");
        let root_fd = open_absolute_directory(&fixture.root).expect("open metadata link root");
        assert_eq!(
            reject_metadata_links(&root_fd, Path::new(".git"), Path::new(".git")),
            Err(InspectionIssue::ExternalRepositoryMetadata)
        );
    }

    #[test]
    fn source_environment_capture_marks_repository_redirection_as_unsupported() {
        let environment = SourceEnvironment::from_lookup(|name| match name {
            "HOME" => Some(OsString::from("/controlled/home")),
            "XDG_CONFIG_HOME" => Some(OsString::from("/controlled/xdg")),
            "GIT_CONFIG_GLOBAL" => Some(OsString::from("/controlled/global")),
            "GIT_ATTR_SOURCE" => Some(OsString::from("refs/heads/other")),
            _ => None,
        });
        assert_eq!(
            environment.home.as_deref(),
            Some(OsStr::new("/controlled/home"))
        );
        assert_eq!(
            environment.xdg_config_home.as_deref(),
            Some(OsStr::new("/controlled/xdg"))
        );
        assert_eq!(
            environment.git_config_global.as_deref(),
            Some(OsStr::new("/controlled/global"))
        );
        assert!(environment.unsupported);
    }

    #[test]
    fn source_environment_capture_reads_the_child_process_environment() {
        if env::var_os(CAPTURE_HELPER).is_some() {
            let environment = SourceEnvironment::capture();
            assert_eq!(
                environment.home.as_deref(),
                Some(OsStr::new("/controlled/home"))
            );
            assert_eq!(
                environment.xdg_config_home.as_deref(),
                Some(OsStr::new("/controlled/xdg"))
            );
            assert_eq!(
                environment.git_config_nosystem.as_deref(),
                Some(OsStr::new("true"))
            );
            assert_eq!(
                environment.git_config_system.as_deref(),
                Some(OsStr::new("/controlled/system"))
            );
            assert_eq!(
                environment.git_config_global.as_deref(),
                Some(OsStr::new("/controlled/global"))
            );
            assert_eq!(
                environment.git_attr_nosystem.as_deref(),
                Some(OsStr::new("yes"))
            );
            assert!(environment.unsupported);
            return;
        }

        let output = Command::new(env::current_exe().expect("current unit-test binary"))
            .args([
                "--exact",
                "git_query::inspect::tests::source_environment_capture_reads_the_child_process_environment",
                "--nocapture",
            ])
            .env_clear()
            .env(CAPTURE_HELPER, "1")
            .env("HOME", "/controlled/home")
            .env("XDG_CONFIG_HOME", "/controlled/xdg")
            .env("GIT_CONFIG_NOSYSTEM", "true")
            .env("GIT_CONFIG_SYSTEM", "/controlled/system")
            .env("GIT_CONFIG_GLOBAL", "/controlled/global")
            .env("GIT_ATTR_NOSYSTEM", "yes")
            .env("GIT_DIR", "/controlled/repository")
            .output()
            .expect("run isolated capture helper");
        assert!(
            output.status.success(),
            "capture helper failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn production_inspection_limits_are_independently_frozen() {
        let limits = InspectionLimits::production();
        assert_eq!(limits.total_timeout, Duration::from_secs(60));
        assert_eq!(limits.max_scan_entries, 100_000);
        assert_eq!(limits.max_repositories, 256);
        assert_eq!(limits.max_depth, 128);
        assert_eq!(limits.max_text_bytes, 1_048_576);
        assert_eq!(QUERY_TIMEOUT, Duration::from_secs(5));
    }

    #[test]
    fn cooperative_budget_expires_and_rejects_every_entry_beyond_the_limit() {
        let expired = CooperativeBudget::new(InspectionLimits {
            total_timeout: Duration::ZERO,
            ..InspectionLimits::production()
        });
        assert!(expired.expired());
        assert_eq!(
            expired.ensure_remaining(),
            Err(InspectionIssue::BudgetExpired)
        );
        let identity = Identity {
            device: 0,
            inode: 0,
            mode: 0,
            size: 0,
            modified_seconds: 0,
            modified_nanoseconds: 0,
        };
        let evidence = FileEvidence {
            path: PathBuf::from("/must-not-be-read-after-budget-expiry"),
            identity,
        };
        assert_eq!(
            revalidate_file_evidence(std::slice::from_ref(&evidence), &expired),
            Err(InspectionIssue::BudgetExpired)
        );
        let repository = PreparedRepository {
            relative_path: PathBuf::from("must-not-be-opened"),
            worktree_identity: identity,
            git_dir: PathBuf::from("/must-not-be-opened/git-dir"),
            git_dir_identity: identity,
            common_dir: PathBuf::from("/must-not-be-opened/common-dir"),
            common_dir_identity: identity,
            index_identity: None,
            evidence: vec![evidence],
            issues: Vec::new(),
        };
        assert_eq!(
            revalidate_repository(Path::new("/must-not-be-opened"), &repository, &expired),
            Err(InspectionIssue::BudgetExpired)
        );

        let mut entries = CooperativeBudget::new(InspectionLimits {
            max_scan_entries: 1,
            ..InspectionLimits::production()
        });
        assert_eq!(entries.consume_scan_entry(), Ok(()));
        assert_eq!(
            entries.consume_scan_entry(),
            Err(InspectionIssue::ScanLimitReached)
        );
        assert_eq!(
            entries.consume_scan_entry(),
            Err(InspectionIssue::ScanLimitReached)
        );
    }

    #[test]
    fn repository_revalidation_detects_each_identity_index_and_file_evidence_change() {
        let budget = CooperativeBudget::new(InspectionLimits::production());

        let worktree = Fixture::new("revalidate-worktree");
        let prepared = prepared_identity_fixture(&worktree);
        assert_eq!(
            revalidate_repository(&worktree.root, &prepared, &budget),
            Ok(())
        );
        replace_owned_directory(
            &worktree.root.join("worktree"),
            "worktree-before-revalidation",
        );
        assert_eq!(
            revalidate_repository(&worktree.root, &prepared, &budget),
            Err(InspectionIssue::EvidenceChanged)
        );

        let git_dir = Fixture::new("revalidate-git-dir");
        let prepared = prepared_identity_fixture(&git_dir);
        assert_eq!(
            revalidate_repository(&git_dir.root, &prepared, &budget),
            Ok(())
        );
        replace_owned_directory(&git_dir.root.join("git-dir"), "git-dir-before-revalidation");
        assert_eq!(
            revalidate_repository(&git_dir.root, &prepared, &budget),
            Err(InspectionIssue::EvidenceChanged)
        );

        let common = Fixture::new("revalidate-common-dir");
        let prepared = prepared_identity_fixture(&common);
        assert_eq!(
            revalidate_repository(&common.root, &prepared, &budget),
            Ok(())
        );
        replace_owned_directory(
            &common.root.join("common-dir"),
            "common-dir-before-revalidation",
        );
        assert_eq!(
            revalidate_repository(&common.root, &prepared, &budget),
            Err(InspectionIssue::EvidenceChanged)
        );

        let index = Fixture::new("revalidate-index");
        let prepared = prepared_identity_fixture(&index);
        assert_eq!(
            revalidate_repository(&index.root, &prepared, &budget),
            Ok(())
        );
        fs::write(index.root.join("git-dir/index"), b"new index\n").expect("create changed index");
        assert_eq!(
            revalidate_repository(&index.root, &prepared, &budget),
            Err(InspectionIssue::EvidenceChanged)
        );

        let file = Fixture::new("revalidate-file-evidence");
        let mut prepared = prepared_identity_fixture(&file);
        let evidence_path = file.root.join("evidence");
        fs::write(&evidence_path, b"before\n").expect("write evidence file");
        prepared.evidence.push(
            read_absolute_file(&evidence_path, MAX_TEXT_BYTES, true)
                .expect("read evidence")
                .expect("required evidence")
                .evidence,
        );
        assert_eq!(
            revalidate_repository(&file.root, &prepared, &budget),
            Ok(())
        );
        fs::rename(
            &evidence_path,
            file.root.join("evidence-before-revalidation"),
        )
        .expect("retain original evidence");
        fs::write(&evidence_path, b"after\n").expect("write replacement evidence");
        assert_eq!(
            revalidate_repository(&file.root, &prepared, &budget),
            Err(InspectionIssue::EvidenceChanged)
        );
    }

    #[test]
    fn required_root_and_absolute_reads_distinguish_missing_parent_and_file() {
        let fixture = Fixture::new("required-file-reads");
        fs::create_dir_all(fixture.root.join("existing")).expect("create existing parent");
        let root_fd = open_absolute_directory(&fixture.root).expect("open required read root");

        for relative in [
            Path::new("missing-parent/file"),
            Path::new("existing/missing-file"),
        ] {
            assert!(
                read_root_file(&fixture.root, &root_fd, relative, MAX_TEXT_BYTES, false)
                    .expect("optional root file")
                    .is_none()
            );
            assert_eq!(
                read_root_file(&fixture.root, &root_fd, relative, MAX_TEXT_BYTES, true).map(|_| ()),
                Err(InspectionIssue::UnsafeRepositoryMetadata)
            );
        }

        for absolute in [
            fixture.root.join("missing-absolute-parent/file"),
            fixture.root.join("existing/missing-absolute-file"),
        ] {
            assert!(
                read_absolute_file(&absolute, MAX_TEXT_BYTES, false)
                    .expect("optional absolute file")
                    .is_none()
            );
            assert_eq!(
                read_absolute_file(&absolute, MAX_TEXT_BYTES, true).map(|_| ()),
                Err(InspectionIssue::EnvironmentUnsupported)
            );
        }
    }

    #[test]
    fn open_file_reads_accept_empty_and_exact_limits_and_reject_special_or_larger_files() {
        let fixture = Fixture::new("open-file-boundaries");
        let empty = fixture.root.join("empty");
        let exact = fixture.root.join("exact");
        let over = fixture.root.join("over");
        fs::write(&empty, b"").expect("write empty file");
        fs::write(&exact, b"1234").expect("write exact file");
        fs::write(&over, b"12345").expect("write oversized file");
        assert_eq!(
            read_absolute_file(&empty, 0, true)
                .expect("read empty file")
                .expect("required empty file")
                .contents,
            b""
        );
        assert_eq!(
            read_absolute_file(&exact, 4, true)
                .expect("read exact file")
                .expect("required exact file")
                .contents,
            b"1234"
        );
        assert_eq!(
            read_absolute_file(&over, 4, true).map(|_| ()),
            Err(InspectionIssue::EnvironmentUnsupported)
        );

        assert_eq!(
            file_open_flags(),
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK
        );
        let fifo = fixture.root.join("special-fifo");
        let output = Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .env_clear()
            .output()
            .expect("create controlled read FIFO");
        assert!(output.status.success(), "mkfifo must succeed");
        expect_fifo_probe_success("absolute-read", &fixture.root);
    }

    #[test]
    fn path_trim_depth_and_descendant_helpers_cover_independent_boundaries() {
        assert_eq!(
            split_relative_file_raw(Path::new("nested/file")),
            Ok((Path::new("nested"), OsStr::new("file")))
        );
        assert!(split_relative_file_raw(Path::new("nested/../file")).is_err());

        assert!(is_normal_absolute(Path::new("/normal/path")));
        assert!(!is_normal_absolute(Path::new("relative/path")));
        assert!(!is_normal_absolute(Path::new("/normal/../path")));

        assert_eq!(trim_ascii(b""), b"");
        assert_eq!(trim_ascii(b"\t value \r\n"), b"value");
        assert_eq!(trim_ascii(b"value"), b"value");
        assert_eq!(trim_ascii(b" \t\r\n"), b"");
        assert_eq!(trim_ascii(b" \x80 "), b"\x80");
        for byte in u8::MIN..=u8::MAX {
            let input = [byte, b'X', byte];
            if byte.is_ascii_whitespace() {
                assert_eq!(trim_ascii(&input), b"X", "ASCII byte {byte:#04x}");
            } else {
                assert_eq!(trim_ascii(&input), input, "non-whitespace byte {byte:#04x}");
            }
        }

        assert_eq!(path_depth(Path::new("")), 0);
        assert_eq!(path_depth(Path::new("one")), 1);
        assert_eq!(path_depth(Path::new("one/two")), 2);

        assert!(is_strict_descendant(Path::new("one/two"), Path::new("one")));
        assert!(!is_strict_descendant(Path::new("one"), Path::new("one")));
        assert!(!is_strict_descendant(Path::new("other"), Path::new("one")));
    }

    #[test]
    fn file_and_directory_open_flags_retain_every_required_bit() {
        assert_eq!(
            directory_open_flags(),
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::DIRECTORY
        );
        assert_eq!(
            file_open_flags(),
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK
        );
    }

    #[test]
    fn discovery_records_only_regular_attribute_files_and_enforces_depth() {
        let attributes = Fixture::new("discovery-attributes");
        fs::write(attributes.root.join("ordinary"), b"ordinary\n").expect("write ordinary file");
        fs::create_dir_all(attributes.root.join(".gitattributes"))
            .expect("create directory with attribute filename");
        fs::create_dir_all(attributes.root.join("nested")).expect("create nested directory");
        fs::write(
            attributes.root.join("nested/.gitattributes"),
            b"*.txt text\n",
        )
        .expect("write nested attributes");
        let mut budget = CooperativeBudget::new(InspectionLimits::production());
        let discovery = discover(&attributes.root, &mut budget).expect("discover attributes");
        assert_eq!(
            discovery.attribute_files,
            vec![PathBuf::from("nested/.gitattributes")]
        );

        let depth = Fixture::new("discovery-depth");
        fs::create_dir_all(depth.root.join("one/two/.git"))
            .expect("create repository beyond depth limit");
        let mut budget = CooperativeBudget::new(InspectionLimits {
            max_depth: 1,
            ..InspectionLimits::production()
        });
        let discovery = discover(&depth.root, &mut budget).expect("bounded discovery");
        assert_eq!(discovery.completeness, DiscoveryCompleteness::Incomplete);
        assert!(
            discovery
                .issues
                .contains(&InspectionIssue::DepthLimitReached)
        );
        assert!(discovery.candidates.is_empty());
    }

    #[test]
    fn source_preflight_honors_disabled_system_sources_and_dev_null_global() {
        let fixture = Fixture::new("source-selection");
        let selected_system = fixture.home.join("system.gitconfig");
        fs::write(
            &selected_system,
            "[filter \"tripwire\"]\n\tclean = /usr/bin/false\n",
        )
        .expect("write disabled unsafe system config");
        let environment = SourceEnvironment {
            home: Some(fixture.home.clone().into_os_string()),
            git_config_nosystem: Some(OsString::from("1")),
            git_config_system: Some(selected_system.into_os_string()),
            git_config_global: Some(OsString::from("/dev/null")),
            git_attr_nosystem: Some(OsString::from("1")),
            ..SourceEnvironment::default()
        };
        let mut budget = CooperativeBudget::new(InspectionLimits::production());
        let global = preflight_sources(&fixture.root, &environment, &mut budget)
            .expect("disabled system sources and /dev/null are safe");
        assert!(global.entries.is_empty());
        assert!(global.evidence.is_empty());
    }

    #[test]
    fn empty_nosystem_matches_native_git_and_preflights_the_selected_system_file() {
        let fixture = Fixture::new("empty-nosystem");
        let selected_system = fixture.home.join("system.gitconfig");
        fs::write(
            &selected_system,
            "[filter \"tripwire\"]\n\tclean = /usr/bin/false\n",
        )
        .expect("write controlled system config");

        let native = Command::new("/usr/bin/git")
            .current_dir(&fixture.root)
            .args(["--git-dir=/dev/null", "config", "--null", "--list"])
            .env_clear()
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "")
            .env("GIT_CONFIG_SYSTEM", &selected_system)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .expect("run controlled native config query");
        assert!(native.status.success());
        assert!(
            native
                .stdout
                .windows(b"filter.tripwire.clean\n/usr/bin/false\0".len())
                .any(|window| window == b"filter.tripwire.clean\n/usr/bin/false\0"),
            "native Git must read the selected system file when NOSYSTEM is empty"
        );

        let environment = SourceEnvironment {
            home: Some(fixture.home.clone().into_os_string()),
            git_config_nosystem: Some(OsString::new()),
            git_config_system: Some(selected_system.into_os_string()),
            git_config_global: Some(OsString::from("/dev/null")),
            git_attr_nosystem: Some(OsString::from("1")),
            ..SourceEnvironment::default()
        };
        let mut budget = CooperativeBudget::new(InspectionLimits::production());
        assert_eq!(
            preflight_sources(&fixture.root, &environment, &mut budget).map(|_| ()),
            Err(InspectionIssue::UnsupportedConfiguration)
        );
    }

    #[test]
    fn reference_resolution_rejects_raw_symlink_or_nondirectory_before_parent_steps() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("raw-reference-components");
        fs::create_dir_all(fixture.root.join(".git/modules/sub"))
            .expect("create internal metadata");
        fs::create_dir_all(fixture.root.join("modules/sub")).expect("create internal worktree");
        let external = fixture
            .root
            .parent()
            .expect("fixture base")
            .join("external-reference-target");
        fs::create_dir_all(&external).expect("create external directory");
        symlink(&external, fixture.root.join("jump")).expect("create escaping symlink");
        fs::write(fixture.root.join("plain"), b"not a directory\n")
            .expect("write non-directory component");

        assert_eq!(
            resolve_internal_reference(&fixture.root, Path::new(".git"), b"../jump/.."),
            Err(InspectionIssue::ExternalRepositoryMetadata)
        );
        assert_eq!(
            resolve_internal_reference(&fixture.root, Path::new(".git"), b"../plain/.."),
            Err(InspectionIssue::ExternalRepositoryMetadata)
        );
        assert_eq!(
            resolve_internal_reference(
                &fixture.root,
                Path::new("modules/sub"),
                b"../../.git/modules/sub",
            ),
            Ok(PathBuf::from(".git/modules/sub"))
        );
    }

    #[test]
    fn environment_source_paths_are_validated_without_following_null_lookalikes() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("environment-paths");
        let missing = fixture.home.join("missing");
        let regular = fixture.home.join("regular");
        fs::write(&regular, b"regular\n").expect("write regular file");
        let null_link = fixture.home.join("null-link");
        symlink("/dev/null", &null_link).expect("create null-device symlink");

        assert_eq!(validate_environment_directory(None), Ok(()));
        assert_eq!(
            validate_environment_directory(Some(fixture.home.as_os_str())),
            Ok(())
        );
        assert_eq!(
            validate_environment_directory(Some(missing.as_os_str())),
            Ok(())
        );
        assert_eq!(
            validate_environment_directory(Some(OsStr::new("relative"))),
            Err(InspectionIssue::EnvironmentUnsupported)
        );
        assert_eq!(
            validate_environment_directory(Some(regular.as_os_str())),
            Err(InspectionIssue::EnvironmentUnsupported)
        );

        assert_eq!(
            validate_selected_source(regular.as_os_str()),
            Ok(regular.clone())
        );
        assert_eq!(
            validate_selected_source(OsStr::new("relative")),
            Err(InspectionIssue::EnvironmentUnsupported)
        );
        assert_eq!(
            validate_selected_source(OsStr::new("/dev/null")),
            Ok(PathBuf::from("/dev/null"))
        );
        assert_eq!(validate_null_device(Path::new("/dev/null")), Ok(()));
        assert_eq!(
            validate_null_device(&regular),
            Err(InspectionIssue::EnvironmentUnsupported)
        );
        assert_eq!(
            validate_null_device(&fixture.home),
            Err(InspectionIssue::EnvironmentUnsupported)
        );
        assert_eq!(
            validate_null_device(&null_link),
            Err(InspectionIssue::EnvironmentUnsupported)
        );
    }

    #[test]
    fn metadata_file_opens_are_nonblocking_before_type_validation() {
        assert_eq!(
            file_open_flags(),
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK
        );

        let fixture = Fixture::new("nonblocking-metadata");
        let fifo = fixture.root.join("metadata.fifo");
        let output = Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .env_clear()
            .output()
            .expect("create controlled metadata FIFO");
        assert!(output.status.success(), "mkfifo must succeed");

        expect_fifo_probe_success("combined-read", &fixture.root);
    }

    #[test]
    fn runtime_prefix_requires_the_frozen_libexec_git_core_suffix() {
        assert_eq!(
            parse_runtime_prefix(b"/Developer/usr/libexec/git-core\n")
                .expect_err("nonexistent directory must not be trusted"),
            InspectionIssue::InvalidExecPath
        );
        assert_eq!(
            parse_runtime_prefix(b"/usr/bin\n"),
            Err(InspectionIssue::InvalidExecPath)
        );
        let real = super::run_bootstrap(Path::new("/"), GitQuery::ExecPath, Duration::from_secs(5))
            .expect("fixed real exec-path query");
        assert!(real.exit.success());
        assert!(parse_runtime_prefix(&real.stdout).is_ok());

        let newline = Fixture::new("exec-path-newline");
        let synthetic_exec_path = newline.root.join("prefix\nsegment/libexec/git-core");
        fs::create_dir_all(&synthetic_exec_path).expect("create newline exec path");
        assert_eq!(
            parse_runtime_prefix(synthetic_exec_path.as_os_str().as_bytes()),
            Err(InspectionIssue::InvalidExecPath)
        );

        let shape = Fixture::new("exec-path-shape");
        let wrong_leaf = shape.root.join("prefix/libexec/not-git-core");
        let wrong_parent = shape.root.join("prefix/not-libexec/git-core");
        fs::create_dir_all(&wrong_leaf).expect("create wrong exec-path leaf");
        fs::create_dir_all(&wrong_parent).expect("create wrong exec-path parent");
        assert_eq!(
            parse_runtime_prefix(wrong_leaf.as_os_str().as_bytes()),
            Err(InspectionIssue::InvalidExecPath)
        );
        assert_eq!(
            parse_runtime_prefix(wrong_parent.as_os_str().as_bytes()),
            Err(InspectionIssue::InvalidExecPath)
        );
    }

    #[test]
    fn system_directory_metadata_requires_directory_root_owner_and_safe_mode() {
        assert_eq!(validate_system_directory_metadata(0o040755, 0), Ok(()));
        for (mode, uid) in [(0o100644, 0), (0o040755, 1), (0o040775, 0), (0o040757, 0)] {
            assert_eq!(
                validate_system_directory_metadata(mode, uid),
                Err(InspectionIssue::EnvironmentUnsupported),
                "mode={mode:o}, uid={uid} must be rejected"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolves_the_real_macos_etc_symlink_without_modifying_it() {
        let link = fs::symlink_metadata("/etc").expect("stat macOS /etc");
        assert!(link.file_type().is_symlink());
        assert_eq!(link.uid(), 0);
        assert_eq!(
            fs::read_link("/etc").expect("read macOS /etc"),
            Path::new("private/etc")
        );
        assert_eq!(
            resolve_system_path(Path::new("/etc/gitconfig")),
            Ok(PathBuf::from("/private/etc/gitconfig"))
        );
    }

    #[test]
    fn does_not_follow_worktree_or_repository_metadata_symlinks() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("nofollow");
        let external = fixture
            .root
            .parent()
            .expect("fixture base")
            .join("external");
        fixture.init_repo(&external);
        symlink(&external, fixture.root.join("linked-directory"))
            .expect("create controlled worktree symlink");
        let no_repository = inspect_with_environment(&fixture.root, fixture.environment());
        assert_eq!(no_repository.discovery, DiscoveryCompleteness::Complete);
        assert!(no_repository.repositories.is_empty());
        assert_eq!(no_repository.aggregate, GitState::NotApplicable);

        fs::create_dir_all(fixture.root.join("unsafe-worktree"))
            .expect("create unsafe worktree candidate");
        symlink(
            external.join(".git"),
            fixture.root.join("unsafe-worktree/.git"),
        )
        .expect("create controlled metadata symlink");
        let rejected = inspect_with_environment(&fixture.root, fixture.environment());
        assert_eq!(rejected.discovery, DiscoveryCompleteness::Incomplete);
        assert_eq!(rejected.aggregate, GitState::Unknown);
        assert!(
            rejected
                .issues
                .contains(&InspectionIssue::UnsafeRepositoryMetadata)
        );
    }

    #[test]
    fn rejects_external_commondir_core_worktree_and_damaged_gitfile() {
        let core_worktree = Fixture::new("external-core-worktree");
        core_worktree.init_repo(&core_worktree.root);
        let outside = core_worktree
            .root
            .parent()
            .expect("fixture base")
            .join("outside-worktree");
        fs::create_dir_all(&outside).expect("create outside worktree");
        core_worktree.git(
            &core_worktree.root,
            &[
                "config",
                "core.worktree",
                outside.to_str().expect("UTF-8 fixture path"),
            ],
        );
        let rejected = inspect_with_environment(&core_worktree.root, core_worktree.environment());
        assert!(
            repository(&rejected, "")
                .issues
                .contains(&InspectionIssue::ExternalRepositoryMetadata)
        );

        let commondir = Fixture::new("external-commondir");
        fs::create_dir_all(commondir.root.join("candidate/.git"))
            .expect("create candidate metadata");
        let external_common = commondir
            .root
            .parent()
            .expect("fixture base")
            .join("common");
        fs::create_dir_all(&external_common).expect("create external common directory");
        fs::write(
            commondir.root.join("candidate/.git/commondir"),
            format!("{}\n", external_common.display()),
        )
        .expect("write external commondir");
        let rejected = inspect_with_environment(&commondir.root, commondir.environment());
        assert!(
            repository(&rejected, "candidate")
                .issues
                .contains(&InspectionIssue::ExternalRepositoryMetadata)
        );

        let damaged = Fixture::new("damaged-gitfile");
        fs::create_dir_all(damaged.root.join("candidate")).expect("create damaged candidate");
        fs::write(
            damaged.root.join("candidate/.git"),
            b"not a gitdir reference\n",
        )
        .expect("write damaged gitfile");
        let rejected = inspect_with_environment(&damaged.root, damaged.environment());
        assert!(
            repository(&rejected, "candidate")
                .issues
                .contains(&InspectionIssue::UnsafeRepositoryMetadata)
        );
    }

    #[test]
    fn never_discovers_a_parent_repository() {
        let fixture = Fixture::new("no-parent-discovery");
        let parent = fixture.root.parent().expect("fixture base");
        fixture.init_repo(parent);
        let inspection = inspect_with_environment(&fixture.root, fixture.environment());
        assert_eq!(inspection.discovery, DiscoveryCompleteness::Complete);
        assert!(inspection.repositories.is_empty());
        assert_eq!(inspection.aggregate, GitState::NotApplicable);
    }

    #[test]
    fn accepts_internal_linked_worktree_commondir_and_back_reference() {
        let fixture = Fixture::new("linked-worktree");
        fixture.init_repo(&fixture.root);
        let linked = fixture.root.join("linked");
        fixture.git(
            &fixture.root,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "fixture-linked",
                linked.to_str().expect("UTF-8 fixture path"),
            ],
        );

        let inspection = inspect_with_environment(&fixture.root, fixture.environment());
        assert_eq!(inspection.discovery, DiscoveryCompleteness::Complete);
        assert_eq!(repository(&inspection, "").state, RepositoryState::Clean);
        assert_eq!(
            repository(&inspection, "linked").state,
            RepositoryState::Clean
        );
        assert_eq!(inspection.aggregate, GitState::Clean);
    }

    #[test]
    fn separately_located_git_dir_is_scanned_in_addition_to_its_common_dir() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("separate-git-and-common-dir");
        fixture.init_repo(&fixture.root);
        let linked = fixture.root.join("linked");
        fixture.git(
            &fixture.root,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "fixture-separated-admin",
                linked.to_str().expect("UTF-8 fixture path"),
            ],
        );
        let original_admin = fixture.root.join(".git/worktrees/linked");
        let separate_admin = fixture.root.join("separate-linked-admin");
        fs::rename(&original_admin, &separate_admin).expect("retain relocated worktree admin dir");
        fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", separate_admin.display()),
        )
        .expect("point linked worktree at relocated admin dir");
        fs::write(separate_admin.join("commondir"), b"../.git\n")
            .expect("point relocated admin dir at internal common dir");

        let native = fixture.git(
            &linked,
            &["status", "--porcelain=v2", "-z", "--untracked-files=no"],
        );
        assert!(
            native.stdout.is_empty(),
            "relocated fixture must remain a clean native worktree"
        );

        let external = fixture
            .root
            .parent()
            .expect("fixture base")
            .join("outside-link-target");
        fs::write(&external, b"outside\n").expect("write controlled outside link target");
        symlink(&external, separate_admin.join("unclassified-link"))
            .expect("create link only in separate git dir");

        let inspection = inspect_with_environment(&fixture.root, fixture.environment());
        let linked = repository(&inspection, "linked");
        assert_eq!(linked.state, RepositoryState::Unknown);
        assert!(
            linked
                .issues
                .contains(&InspectionIssue::ExternalRepositoryMetadata)
        );
    }

    #[test]
    fn root_attributes_are_not_hidden_by_a_nested_repository_candidate() {
        let fixture = Fixture::new("root-attributes-with-nested-repository");
        fixture.init_repo(&fixture.root);
        fixture.init_repo(&fixture.root.join("nested"));
        fs::write(
            fixture.root.join(".gitattributes"),
            b"*.txt filter=tripwire\n",
        )
        .expect("write unsafe root attributes");

        let inspection = inspect_with_environment(&fixture.root, fixture.environment());
        let root = repository(&inspection, "");
        assert_eq!(root.state, RepositoryState::Unknown);
        assert!(root.issues.contains(&InspectionIssue::UnsafeAttributes));
        assert_eq!(
            repository(&inspection, "nested").state,
            RepositoryState::Clean
        );
    }

    #[test]
    fn complete_copy_without_repositories_is_not_applicable_without_source_preflight() {
        let fixture = Fixture::new("no-repository");
        fs::write(fixture.root.join("ordinary"), b"not a repository\n")
            .expect("write ordinary file");
        let inspection = inspect_with_environment(
            &fixture.root,
            SourceEnvironment {
                unsupported: true,
                ..SourceEnvironment::default()
            },
        );
        assert_eq!(inspection.discovery, DiscoveryCompleteness::Complete);
        assert!(inspection.repositories.is_empty());
        assert_eq!(inspection.aggregate, GitState::NotApplicable);
        assert!(inspection.issues.is_empty());
    }

    #[test]
    fn discovery_limit_blocks_all_repository_sensitive_queries() {
        let fixture = Fixture::new("discovery-limit");
        fixture.init_repo(&fixture.root);
        fixture.init_repo(&fixture.root.join("nested"));
        let sentinel = fixture.root.join("sentinel");
        fs::write(&sentinel, b"unchanged\n").expect("write sentinel");

        let inspection = inspect_with_options(
            &fixture.root,
            fixture.environment(),
            InspectionLimits {
                max_repositories: 1,
                ..InspectionLimits::production()
            },
        );
        assert_eq!(inspection.discovery, DiscoveryCompleteness::Incomplete);
        assert_eq!(inspection.aggregate, GitState::Unknown);
        assert!(
            inspection
                .issues
                .contains(&InspectionIssue::RepositoryLimitReached)
        );
        assert!(
            inspection
                .repositories
                .iter()
                .all(|repository| repository.state == RepositoryState::Unknown)
        );
        assert_eq!(fs::read(&sentinel).expect("read sentinel"), b"unchanged\n");
    }

    #[test]
    fn unsafe_child_configuration_blocks_parent_query_without_running_filter() {
        let fixture = Fixture::new("unsafe-child");
        fixture.init_repo(&fixture.root);
        let child = fixture.root.join("ignored/child");
        fixture.init_repo(&child);
        let sentinel = fixture.root.join("sentinel");
        fs::write(&sentinel, b"unchanged\n").expect("write sentinel");
        let filter = fixture.root.join("tripwire.sh");
        fs::write(
            &filter,
            format!(
                "#!/bin/sh\nprintf changed > '{}'\ncat\n",
                sentinel.display()
            ),
        )
        .expect("write filter tripwire");
        let mut permissions = fs::metadata(&filter)
            .expect("tripwire metadata")
            .permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(0o700);
        }
        fs::set_permissions(&filter, permissions).expect("make tripwire executable");
        fixture.git(
            &child,
            &[
                "config",
                "filter.tripwire.clean",
                filter.to_str().expect("UTF-8 fixture path"),
            ],
        );
        fs::write(child.join(".gitattributes"), b"*.txt filter=tripwire\n")
            .expect("write child attributes");

        let inspection = inspect_with_environment(&fixture.root, fixture.environment());
        assert_eq!(
            repository(&inspection, "ignored/child").state,
            RepositoryState::Unknown
        );
        assert_eq!(repository(&inspection, "").state, RepositoryState::Unknown);
        assert!(
            repository(&inspection, "")
                .issues
                .contains(&InspectionIssue::UnsafeDescendant)
        );
        assert_eq!(fs::read(&sentinel).expect("read sentinel"), b"unchanged\n");
    }

    #[test]
    fn rejects_submodule_path_that_can_escape_the_copy_root() {
        let fixture = Fixture::new("external-submodule-path");
        fixture.init_repo(&fixture.root);
        fs::write(
            fixture.root.join(".gitmodules"),
            b"[submodule \"outside\"]\n\tpath = ../outside\n\turl = ./source\n",
        )
        .expect("write escaping gitmodules path");

        let inspection = inspect_with_environment(&fixture.root, fixture.environment());
        assert_eq!(repository(&inspection, "").state, RepositoryState::Unknown);
        assert!(
            repository(&inspection, "")
                .issues
                .contains(&InspectionIssue::ExternalRepositoryMetadata)
        );
    }

    #[test]
    fn rejects_non_normal_submodule_path_even_when_it_normalizes_inside_root() {
        let fixture = Fixture::new("nonnormal-submodule-path");
        fixture.init_repo(&fixture.root);
        fs::write(
            fixture.root.join(".gitmodules"),
            b"[submodule \"inside\"]\n\tpath = nested/../sub\n\turl = ./source\n",
        )
        .expect("write non-normal gitmodules path");

        let inspection = inspect_with_environment(&fixture.root, fixture.environment());
        let root = repository(&inspection, "");
        assert_eq!(root.state, RepositoryState::Unknown);
        assert!(
            root.issues
                .contains(&InspectionIssue::ExternalRepositoryMetadata)
        );
    }

    #[test]
    fn rejects_external_gitfile_without_inspecting_outside_root() {
        let fixture = Fixture::new("external-gitfile");
        fixture.init_repo(&fixture.root);
        let external = fixture
            .root
            .parent()
            .expect("fixture base")
            .join("external.git");
        fixture.git(
            fixture.root.parent().expect("fixture base"),
            &[
                "init",
                "--quiet",
                "--bare",
                external.to_str().expect("UTF-8 path"),
            ],
        );
        let candidate = fixture.root.join("external-worktree");
        fs::create_dir_all(&candidate).expect("create external worktree candidate");
        fs::write(
            candidate.join(".git"),
            format!("gitdir: {}\n", external.display()),
        )
        .expect("write external gitfile");

        let inspection = inspect_with_environment(&fixture.root, fixture.environment());
        let rejected = repository(&inspection, "external-worktree");
        assert_eq!(rejected.state, RepositoryState::Unknown);
        assert!(
            rejected
                .issues
                .contains(&InspectionIssue::ExternalRepositoryMetadata)
        );
        assert_eq!(inspection.aggregate, GitState::Unknown);
    }
}

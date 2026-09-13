//! Minimal synchronous JSONL persistence experiment for P0-07 removal events.
//!
//! This module accepts only the approved low-sensitivity event fields. It is
//! not a general secret scrubber, audit service, or lifecycle state store.

use std::fmt;
use std::fs::File;
use std::io::{self, Write};

use rustix::fs::{OFlags, fcntl_getfl};
use serde::{Serialize, Serializer};

use crate::{DiscoveryCompleteness, ProcessUse, RemovalRefusal};

/// A restricted experiment identifier, not a Phase 1 UUID contract.
#[derive(Clone, Copy, Serialize)]
#[serde(transparent)]
pub struct SafeId<'a>(&'a str);

impl<'a> SafeId<'a> {
    /// Validates the bounded ASCII representation used by this experiment.
    pub fn new(value: &'a str) -> Result<Self, FieldError> {
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(FieldError::InvalidIdentifier);
        }
        Ok(Self(value))
    }
}

/// A lexically checked repository-relative byte path; empty denotes the root.
///
/// This type does not prove filesystem identity, ownership, or containment.
#[derive(Clone, Copy)]
pub struct RepositoryRelativePath<'a>(&'a [u8]);

impl<'a> RepositoryRelativePath<'a> {
    /// Rejects absolute, escaping, ambiguous, and NUL-containing path syntax.
    pub fn new(value: &'a [u8]) -> Result<Self, FieldError> {
        if value.starts_with(b"/")
            || value.contains(&0)
            || (!value.is_empty()
                && value
                    .split(|byte| *byte == b'/')
                    .any(|component| component.is_empty() || matches!(component, b"." | b"..")))
        {
            return Err(FieldError::InvalidRelativePath);
        }
        Ok(Self(value))
    }
}

impl Serialize for RepositoryRelativePath<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let escaped: String = self
            .0
            .iter()
            .flat_map(|byte| byte.escape_ascii())
            .map(char::from)
            .collect();
        serializer.serialize_str(&escaped)
    }
}

/// A rejected field supplied to a typed removal-log event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldError {
    /// An identifier was empty, too long, or outside the restricted alphabet.
    InvalidIdentifier,
    /// A repository path was absolute, ambiguous, escaping, or contained NUL.
    InvalidRelativePath,
}

/// Safe per-repository evidence included in an event.
#[derive(Clone, Copy, Serialize)]
pub struct RepositoryEvidence<'a> {
    /// Repository location relative to the copy root.
    pub relative_path: RepositoryRelativePath<'a>,
    /// Known tracked-change count, or `None` when unknown.
    pub tracked_changes: Option<usize>,
}

/// Whether repository discovery and checking completed.
#[derive(Clone, Copy)]
pub struct CheckCompleteness(DiscoveryCompleteness);

impl From<DiscoveryCompleteness> for CheckCompleteness {
    fn from(value: DiscoveryCompleteness) -> Self {
        Self(value)
    }
}

impl Serialize for CheckCompleteness {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(match self.0 {
            DiscoveryCompleteness::Complete => "complete",
            DiscoveryCompleteness::Incomplete => "incomplete",
        })
    }
}

/// Best-effort process evidence backed by the existing removal-policy enum.
#[derive(Clone, Copy)]
pub struct ProcessUseEvidence(ProcessUse);

impl From<ProcessUse> for ProcessUseEvidence {
    fn from(value: ProcessUse) -> Self {
        Self(value)
    }
}

impl Serialize for ProcessUseEvidence {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(match self.0 {
            ProcessUse::NoEvidence => "no_evidence",
            ProcessUse::ScanIncomplete => "scan_incomplete",
            ProcessUse::ConfirmedInUse => "confirmed_in_use",
        })
    }
}

/// A protection reason backed directly by the existing removal-policy enum.
#[derive(Clone, Copy)]
pub struct ProtectionReason(RemovalRefusal);

impl From<RemovalRefusal> for ProtectionReason {
    fn from(value: RemovalRefusal) -> Self {
        Self(value)
    }
}

impl Serialize for ProtectionReason {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(match self.0 {
            RemovalRefusal::UnsafePath => "unsafe_path",
            RemovalRefusal::VolumeMismatch => "volume_mismatch",
            RemovalRefusal::ConfirmedInUse => "confirmed_in_use",
            RemovalRefusal::GitCheckIncomplete => "git_check_incomplete",
            RemovalRefusal::TrackedChanges => "tracked_changes",
        })
    }
}

/// A successful final result.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovalResult {
    /// The controlled copy was removed.
    Removed,
}

/// A closed failure category suitable for the minimal persistent event.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovalFailure {
    /// A filesystem operation failed.
    Filesystem,
    /// The operation stopped before a completed result was established.
    Interrupted,
}

/// Event-specific fields; its shape prevents invalid result/event combinations.
#[derive(Clone, Copy, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventOutcome {
    /// Removal was refused before destructive work.
    Refused {
        /// Policy reason that prevented removal.
        protection_reason: ProtectionReason,
    },
    /// A removal intent was synchronously persisted before destructive work.
    Started,
    /// Removal reached a successful final result.
    Completed {
        /// The successful result.
        result: RemovalResult,
    },
    /// Removal reached a reported failure.
    Failed {
        /// The closed failure category.
        error: RemovalFailure,
    },
}

/// One typed event for the minimal P0-07 persistent log experiment.
#[derive(Serialize)]
pub struct RemovalLogEvent<'a> {
    /// Caller-supplied UTC Unix timestamp in milliseconds.
    pub utc_unix_ms: i64,
    /// Experiment operation identity.
    pub operation_id: SafeId<'a>,
    /// Experiment workspace identity.
    pub workspace_id: SafeId<'a>,
    /// Whether the caller received explicit force intent.
    pub force: bool,
    /// Completeness of the content check.
    pub check_completeness: CheckCompleteness,
    /// Best-effort process occupancy evidence observed before the decision.
    pub process_use: ProcessUseEvidence,
    /// Bounded repository summaries without filenames or source content.
    pub repositories: &'a [RepositoryEvidence<'a>],
    /// Event-specific protection or final result.
    #[serde(flatten)]
    pub outcome: EventOutcome,
}

/// A validation, serialization, or I/O failure while appending an event.
#[derive(Debug)]
pub enum AppendError {
    /// The supplied descriptor does not refer to a regular file.
    NotRegularFile,
    /// The supplied descriptor is not both append-opened and writable.
    NotAppendWritable,
    /// Typed event serialization failed before any write.
    Serialize(serde_json::Error),
    /// Descriptor inspection, writing, or synchronization failed.
    Io(io::Error),
}

impl fmt::Display for AppendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRegularFile => formatter.write_str("removal log is not a regular file"),
            Self::NotAppendWritable => {
                formatter.write_str("removal log is not append-opened and writable")
            }
            Self::Serialize(error) => write!(formatter, "could not serialize removal log: {error}"),
            Self::Io(error) => write!(formatter, "could not persist removal log: {error}"),
        }
    }
}

impl std::error::Error for AppendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Serialize(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::NotRegularFile | Self::NotAppendWritable => None,
        }
    }
}

/// Appends and synchronizes exactly one typed JSONL event.
///
/// The caller must supply the already validated log file at the approved
/// location and serialize calls under the lifecycle lock. An I/O error can
/// leave a partial final line; this ordinary log must not drive recovery.
pub fn append_event(file: &mut File, event: &RemovalLogEvent<'_>) -> Result<(), AppendError> {
    if !file.metadata().map_err(AppendError::Io)?.is_file() {
        return Err(AppendError::NotRegularFile);
    }

    let flags = fcntl_getfl(&*file).map_err(|error| AppendError::Io(error.into()))?;
    let access_mode = flags & OFlags::ACCMODE;
    if !flags.contains(OFlags::APPEND) || !matches!(access_mode, OFlags::WRONLY | OFlags::RDWR) {
        return Err(AppendError::NotAppendWritable);
    }

    let mut line = serde_json::to_vec(event).map_err(AppendError::Serialize)?;
    line.push(b'\n');
    file.write_all(&line).map_err(AppendError::Io)?;
    file.sync_all().map_err(AppendError::Io)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs::{self, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    fn retained_fixture() -> PathBuf {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("cleanup crate is nested below the repository root")
            .to_path_buf();
        let path = repository.join("target").join(format!(
            "p007-removal-log-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("create retained P0-07 fixture");
        let metadata = path.metadata().expect("read retained fixture identity");
        assert_eq!(metadata.mode() & 0o777, 0o700);
        eprintln!(
            "retained P0-07 removal-log fixture: {} (dev {}, ino {})",
            path.display(),
            metadata.dev(),
            metadata.ino()
        );
        path
    }

    fn started_event<'a>(repositories: &'a [RepositoryEvidence<'a>]) -> RemovalLogEvent<'a> {
        RemovalLogEvent {
            utc_unix_ms: 1_789_300_000_123,
            operation_id: SafeId::new("op_test_1").expect("valid operation ID"),
            workspace_id: SafeId::new("ws_test_1").expect("valid workspace ID"),
            force: true,
            check_completeness: DiscoveryCompleteness::Incomplete.into(),
            process_use: ProcessUse::NoEvidence.into(),
            repositories,
            outcome: EventOutcome::Started,
        }
    }

    fn open_append_log(path: &std::path::Path) -> File {
        OpenOptions::new()
            .create_new(true)
            .read(true)
            .append(true)
            .mode(0o600)
            .open(path)
            .expect("open retained append log")
    }

    #[test]
    fn writes_only_the_typed_schema_and_lossless_escaped_path() {
        let fixture = retained_fixture();
        let log_path = fixture.join("operations.jsonl");
        let mut file = open_append_log(&log_path);
        let repositories = [RepositoryEvidence {
            relative_path: RepositoryRelativePath::new(b"child/\xff\n\\tab")
                .expect("valid relative bytes"),
            tracked_changes: None,
        }];

        append_event(&mut file, &started_event(&repositories)).expect("append typed event");
        file.seek(SeekFrom::Start(0)).expect("rewind log");
        let mut text = String::new();
        file.read_to_string(&mut text).expect("read log");
        let value: serde_json::Value = serde_json::from_str(text.trim_end()).expect("valid JSONL");
        let keys: BTreeSet<_> = value
            .as_object()
            .expect("JSON object")
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            keys,
            [
                "check_completeness",
                "event",
                "force",
                "operation_id",
                "process_use",
                "repositories",
                "utc_unix_ms",
                "workspace_id",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        );
        assert_eq!(
            value["repositories"][0]["relative_path"],
            "child/\\xff\\n\\\\tab"
        );
        assert!(value["repositories"][0]["tracked_changes"].is_null());
        assert_eq!(value["utc_unix_ms"], 1_789_300_000_123_i64);
        assert_eq!(value["operation_id"], "op_test_1");
        assert_eq!(value["workspace_id"], "ws_test_1");
        assert_eq!(value["force"], true);
        assert_eq!(value["check_completeness"], "incomplete");
        assert_eq!(value["process_use"], "no_evidence");
        assert_eq!(value["event"], "started");
        // These canaries are deliberately outside the typed input. This locks
        // the field boundary; it does not claim general secret recognition.
        assert!(!text.contains("source-body-canary"));
        assert!(!text.contains("[115, 111, 117, 114, 99, 101]"));
    }

    #[test]
    fn append_flag_preserves_prefix_even_after_seek_and_allows_multiple_lines() {
        let fixture = retained_fixture();
        let mut file = open_append_log(&fixture.join("operations.jsonl"));
        file.write_all(b"existing\n")
            .expect("write existing prefix");
        file.seek(SeekFrom::Start(0)).expect("seek before append");
        let event = started_event(&[]);

        append_event(&mut file, &event).expect("append first event");
        append_event(&mut file, &event).expect("append second event");
        drop(file);

        let text = fs::read_to_string(fixture.join("operations.jsonl")).expect("reopen log");
        assert!(text.starts_with("existing\n"));
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(
            lines[1..]
                .iter()
                .all(|line| serde_json::from_str::<serde_json::Value>(line).is_ok())
        );
    }

    #[test]
    fn rejects_nonappend_and_read_only_regular_files_before_overwrite() {
        let fixture = retained_fixture();
        let path = fixture.join("operations.jsonl");
        fs::write(&path, b"preserve-me\n").expect("seed log");
        let event = started_event(&[]);

        let mut nonappend = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("open nonappend file");
        nonappend
            .seek(SeekFrom::Start(0))
            .expect("seek to overwrite position");
        assert!(matches!(
            append_event(&mut nonappend, &event),
            Err(AppendError::NotAppendWritable)
        ));
        assert_eq!(
            fs::read(&path).expect("read preserved log"),
            b"preserve-me\n"
        );

        let mut read_only = File::open(&path).expect("open read-only file");
        assert!(matches!(
            append_event(&mut read_only, &event),
            Err(AppendError::NotAppendWritable)
        ));
        assert_eq!(
            fs::read(&path).expect("read preserved log"),
            b"preserve-me\n"
        );
    }

    #[test]
    fn rejects_nonregular_file_before_writing() {
        let mut null = OpenOptions::new()
            .append(true)
            .open("/dev/null")
            .expect("open controlled nonregular target");
        assert!(matches!(
            append_event(&mut null, &started_event(&[])),
            Err(AppendError::NotRegularFile)
        ));
    }

    #[test]
    fn rejects_absolute_traversing_and_ambiguous_relative_paths() {
        assert!(matches!(
            RepositoryRelativePath::new(b"/absolute"),
            Err(FieldError::InvalidRelativePath)
        ));
        for path in [b"..".as_slice(), b"a/../b", b"./a", b"a//b", b"a/"] {
            assert!(matches!(
                RepositoryRelativePath::new(path),
                Err(FieldError::InvalidRelativePath)
            ));
        }
        assert!(matches!(
            RepositoryRelativePath::new(b"a\0b"),
            Err(FieldError::InvalidRelativePath)
        ));
        assert!(RepositoryRelativePath::new(b"").is_ok());
        assert!(RepositoryRelativePath::new(b"nested/repository").is_ok());
    }

    #[test]
    fn validates_identifier_character_and_length_boundaries() {
        assert!(matches!(
            SafeId::new(""),
            Err(FieldError::InvalidIdentifier)
        ));
        assert!(matches!(
            SafeId::new("bad!ascii"),
            Err(FieldError::InvalidIdentifier)
        ));
        assert!(matches!(
            SafeId::new("non_ascii_é"),
            Err(FieldError::InvalidIdentifier)
        ));
        let maximum = "a".repeat(128);
        let too_long = "a".repeat(129);
        assert!(SafeId::new(&maximum).is_ok());
        assert!(matches!(
            SafeId::new(&too_long),
            Err(FieldError::InvalidIdentifier)
        ));
    }

    #[test]
    fn maps_closed_event_values_and_existing_protection_reasons() {
        let cases = [
            (RemovalRefusal::UnsafePath, "unsafe_path"),
            (RemovalRefusal::VolumeMismatch, "volume_mismatch"),
            (RemovalRefusal::ConfirmedInUse, "confirmed_in_use"),
            (RemovalRefusal::GitCheckIncomplete, "git_check_incomplete"),
            (RemovalRefusal::TrackedChanges, "tracked_changes"),
        ];
        for (reason, expected) in cases {
            let value = serde_json::to_value(EventOutcome::Refused {
                protection_reason: reason.into(),
            })
            .expect("serialize closed refusal event");
            assert_eq!(value["event"], "refused");
            assert_eq!(value["protection_reason"], expected);
        }

        assert_eq!(
            serde_json::to_value(CheckCompleteness::from(DiscoveryCompleteness::Complete))
                .expect("serialize complete evidence"),
            "complete"
        );
        assert_eq!(
            serde_json::to_value(CheckCompleteness::from(DiscoveryCompleteness::Incomplete))
                .expect("serialize incomplete evidence"),
            "incomplete"
        );
        assert_eq!(
            serde_json::to_value(EventOutcome::Started).expect("serialize started event"),
            serde_json::json!({"event": "started"})
        );
        assert_eq!(
            serde_json::to_value(EventOutcome::Completed {
                result: RemovalResult::Removed,
            })
            .expect("serialize completed event"),
            serde_json::json!({"event": "completed", "result": "removed"})
        );
        for (error, expected) in [
            (RemovalFailure::Filesystem, "filesystem"),
            (RemovalFailure::Interrupted, "interrupted"),
        ] {
            assert_eq!(
                serde_json::to_value(EventOutcome::Failed { error })
                    .expect("serialize failed event"),
                serde_json::json!({"event": "failed", "error": expected})
            );
        }
    }

    #[test]
    fn serializes_each_existing_process_evidence_value() {
        for (process_use, expected) in [
            (ProcessUse::NoEvidence, "no_evidence"),
            (ProcessUse::ScanIncomplete, "scan_incomplete"),
            (ProcessUse::ConfirmedInUse, "confirmed_in_use"),
        ] {
            assert_eq!(
                serde_json::to_value(ProcessUseEvidence::from(process_use))
                    .expect("serialize existing process evidence"),
                expected
            );
        }
    }

    #[test]
    fn displays_typed_errors_and_preserves_library_sources() {
        assert_eq!(
            AppendError::NotRegularFile.to_string(),
            "removal log is not a regular file"
        );
        assert_eq!(
            AppendError::NotAppendWritable.to_string(),
            "removal log is not append-opened and writable"
        );

        let io_error = AppendError::Io(io::Error::other("write failed"));
        assert_eq!(
            io_error.to_string(),
            "could not persist removal log: write failed"
        );
        assert_eq!(
            std::error::Error::source(&io_error)
                .expect("I/O source")
                .to_string(),
            "write failed"
        );
        let serde_error = serde_json::from_str::<serde_json::Value>("{")
            .expect_err("construct serialization-library error");
        let serialize_error = AppendError::Serialize(serde_error);
        let source = std::error::Error::source(&serialize_error).expect("serialization source");
        assert!(!source.to_string().is_empty());
        assert_eq!(
            serialize_error.to_string(),
            format!("could not serialize removal log: {source}")
        );
    }
}

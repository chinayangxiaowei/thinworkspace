#![deny(unsafe_code)]

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::{FileExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rustix::fs::{AtFlags, FlockOperation, Mode, OFlags, Timespec, Timestamps};
use thinws_p0_materialize::{
    AttemptEvidence, AttemptOutcome, Backend, CowEvidence, CreatedIdentity, ManifestEntryKind,
    MaterializeRequest, materialize_once,
};
use thinws_p0_probe::{Evidence, inspect_path};

#[path = "../materialize/tests/support/mod.rs"]
mod support;

use support::{ControlledTree, TrackedIdentity, TrackedKind};

const MESSAGE_TIMEOUT: Duration = Duration::from_secs(5);
const PROCESS_TIMEOUT: Duration = Duration::from_secs(30);
const FAILURE_REAP_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;
const LSOF_PATH: &str = "/usr/sbin/lsof";

const CHILD_PATH: &str = "THINWS_P0_LOCK_CHILD_PATH";
const CHILD_API: &str = "THINWS_P0_LOCK_CHILD_API";

const ORIGINAL_BYTES: &[u8] = b"p0-05 original bytes\n";
const CLONE_BYTES: &[u8] = b"p0-05 clone-only bytes\n";
const COPY_BYTES: &[u8] = b"p0-05 copy-only bytes\n";
const SOURCE_BYTES: &[u8] = b"p0-05 source-only bytes\n";
const L3_SOURCE_BYTES: &[u8] = b"p0-05 L3 in-place source bytes\n";
const L3_CLONE_BYTES: &[u8] = b"p0-05 L3 clone control bytes\n";
const L6_COPY_BYTES: &[u8] = b"p0-05 L6 full-copy bytes\n";
const L4_OLD_BYTES: &[u8] = b"p0-05 L4 old object bytes\n";
const L4_NEW_BYTES: &[u8] = b"p0-05 L4 replacement bytes\n";
const L7_MARKER_BYTES: &[u8] = b"p0-05 L7 persistent application marker\n";
const L7_REWRITTEN_MARKER_BYTES: &[u8] = b"p0-05 L7 rewritten persistent marker\n";
const L7_VALUE_ZERO: &[u8] = b"0\n";
const L7_VALUE_ONE: &[u8] = b"1\n";
const MAX_FIXTURE_CONTENT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LockApi {
    Flock,
    PosixFcntl,
}

impl LockApi {
    fn label(self) -> &'static str {
        match self {
            Self::Flock => "flock",
            Self::PosixFcntl => "posix-fcntl",
        }
    }

    fn syscall(self) -> &'static str {
        match self {
            Self::Flock => "flock(LOCK_EX|LOCK_NB)",
            Self::PosixFcntl => "fcntl(F_SETLK,F_WRLCK,start=0,len=0)",
        }
    }

    fn parse(value: &str) -> io::Result<Self> {
        match value {
            "flock" => Ok(Self::Flock),
            "posix-fcntl" => Ok(Self::PosixFcntl),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown fixed lock API selector",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

impl From<TrackedIdentity> for FileIdentity {
    fn from(value: TrackedIdentity) -> Self {
        Self {
            device: value.device,
            inode: value.inode,
        }
    }
}

#[derive(Debug)]
enum LockAttempt {
    Acquired { identity: FileIdentity },
    Conflict { errno: i32, identity: FileIdentity },
}

#[derive(Debug, Eq, PartialEq)]
struct VolumeIdentity {
    fsid: [i32; 2],
    uuid: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TimestampValue {
    seconds: i64,
    nanoseconds: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileMetadataObservation {
    identity: FileIdentity,
    length: u64,
    mode: u32,
    link_count: u64,
    access_time: TimestampValue,
    modified_time: TimestampValue,
    change_time: TimestampValue,
    birth_time: TimestampValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileObservation {
    metadata: FileMetadataObservation,
    content_digest_hex: String,
}

#[derive(Debug)]
struct LsofCapture {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

#[derive(Debug)]
struct LsofFileObservation {
    pid: u32,
    fd: String,
    lock_field: Option<String>,
    identity: FileIdentity,
    name: String,
}

struct LockFixture {
    tree: ControlledTree,
    root: PathBuf,
    source: PathBuf,
    clone_target: PathBuf,
    copy_target: PathBuf,
    clone_staging: PathBuf,
    copy_staging: PathBuf,
    clone_trash: PathBuf,
    copy_trash: PathBuf,
    source_identity: FileIdentity,
}

impl LockFixture {
    fn create(api: LockApi) -> io::Result<Self> {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .ok_or_else(|| io::Error::other("materialize crate is not below repository root"))?
            .canonicalize()?;
        let parent = repository_root.join("target/p0-lock-isolation-tests");
        fs::create_dir_all(&parent)?;

        let mut tree = ControlledTree::create_in(&parent, api.label())?;
        let root = tree.root_path().to_path_buf();
        for directory in [
            "source",
            "clone-target",
            "copy-target",
            "clone-staging",
            "copy-staging",
            "clone-trash",
            "copy-trash",
        ] {
            tree.create_directory(directory, 0o700)?;
        }
        tree.create_file("source/payload.bin", ORIGINAL_BYTES, 0o600)?;
        let source_tracked = tree.identity_at(Path::new("source/payload.bin"))?;
        if source_tracked.kind != TrackedKind::RegularFile {
            return Err(io::Error::other("source payload is not a regular file"));
        }

        rustix::fs::linkat(
            tree.root_fd(),
            Path::new("source/payload.bin"),
            tree.root_fd(),
            Path::new("source/payload-hard-link.bin"),
            AtFlags::empty(),
        )
        .map_err(errno_error)?;
        tree.adopt_confirmed(
            Path::new("source/payload-hard-link.bin"),
            source_tracked.device,
            source_tracked.inode,
            TrackedKind::RegularFile,
        )?;
        tree.create_symlink("source/payload-symbolic-link.bin", b"payload.bin")?;

        let fixture = Self {
            source: root.join("source"),
            clone_target: root.join("clone-target"),
            copy_target: root.join("copy-target"),
            clone_staging: root.join("clone-staging"),
            copy_staging: root.join("copy-staging"),
            clone_trash: root.join("clone-trash"),
            copy_trash: root.join("copy-trash"),
            source_identity: source_tracked.into(),
            tree,
            root,
        };
        fixture.verify_aliases()?;
        Ok(fixture)
    }

    fn verify_aliases(&self) -> io::Result<()> {
        let hard_link = self
            .tree
            .identity_at(Path::new("source/payload-hard-link.bin"))?;
        if FileIdentity::from(hard_link) != self.source_identity
            || hard_link.kind != TrackedKind::RegularFile
        {
            return Err(io::Error::other(
                "hard-link control does not resolve to the source inode",
            ));
        }

        let symbolic_link = self
            .tree
            .identity_at(Path::new("source/payload-symbolic-link.bin"))?;
        if symbolic_link.kind != TrackedKind::SymbolicLink
            || fs::read_link(self.source.join("payload-symbolic-link.bin"))?
                != Path::new("payload.bin")
        {
            return Err(io::Error::other(
                "symbolic-link control is not the expected internal relative link",
            ));
        }
        let followed = metadata_identity(&self.source.join("payload-symbolic-link.bin"))?;
        if followed != self.source_identity {
            return Err(io::Error::other(
                "symbolic-link control does not resolve to the source inode",
            ));
        }
        Ok(())
    }

    fn request(&self, backend: Backend) -> MaterializeRequest<'_> {
        match backend {
            Backend::ApfsFileClone => MaterializeRequest {
                source: &self.source,
                target: &self.clone_target,
                staging: &self.clone_staging,
                trash: &self.clone_trash,
            },
            Backend::FullCopy => MaterializeRequest {
                source: &self.source,
                target: &self.copy_target,
                staging: &self.copy_staging,
                trash: &self.copy_trash,
            },
        }
    }

    fn target(&self, backend: Backend) -> &Path {
        match backend {
            Backend::ApfsFileClone => &self.clone_target,
            Backend::FullCopy => &self.copy_target,
        }
    }

    fn adopt_created(&mut self, backend: Backend, receipt: &AttemptEvidence) -> io::Result<()> {
        let target_relative = match backend {
            Backend::ApfsFileClone => Path::new("clone-target"),
            Backend::FullCopy => Path::new("copy-target"),
        };
        for entry in &receipt.created {
            let CreatedIdentity::Confirmed(identity) = entry.identity else {
                return Err(io::Error::other(
                    "materialization receipt contains an unconfirmed identity",
                ));
            };
            let kind = match entry.kind {
                ManifestEntryKind::Directory => TrackedKind::Directory,
                ManifestEntryKind::RegularFile => TrackedKind::RegularFile,
                ManifestEntryKind::SymbolicLink => TrackedKind::SymbolicLink,
            };
            self.tree.adopt_confirmed(
                &target_relative.join(&entry.path.display),
                identity.device,
                identity.inode,
                kind,
            )?;
        }
        Ok(())
    }
}

struct ManagedChild {
    child: Child,
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    started: Instant,
    diagnostic_bytes: usize,
}

impl ManagedChild {
    fn spawn(entry: &str, api: LockApi, path: &Path, cwd: &Path) -> io::Result<Self> {
        let (parent_channel, child_channel) = UnixStream::pair()?;
        parent_channel.set_read_timeout(Some(MESSAGE_TIMEOUT))?;
        parent_channel.set_write_timeout(Some(MESSAGE_TIMEOUT))?;
        child_channel.set_read_timeout(Some(MESSAGE_TIMEOUT))?;
        child_channel.set_write_timeout(Some(MESSAGE_TIMEOUT))?;
        let writer = parent_channel.try_clone()?;
        let child_input = child_channel.try_clone()?;
        let child_input: OwnedFd = child_input.into();
        let child_diagnostics: OwnedFd = child_channel.into();
        let started = Instant::now();
        let child = Command::new(env::current_exe()?)
            .arg("--ignored")
            .arg("--exact")
            .arg(entry)
            .arg("--nocapture")
            .arg("--test-threads=1")
            .env_clear()
            .env(CHILD_API, api.label())
            .env(CHILD_PATH, path)
            .current_dir(cwd)
            .stdin(Stdio::from(child_input))
            .stdout(Stdio::null())
            .stderr(Stdio::from(child_diagnostics))
            .spawn()?;
        Ok(Self {
            child,
            reader: BufReader::new(parent_channel),
            writer,
            started,
            diagnostic_bytes: 0,
        })
    }

    fn read_message(&mut self) -> io::Result<String> {
        let message_deadline = Instant::now() + MESSAGE_TIMEOUT;
        let mut message = Vec::new();
        loop {
            let remaining = self.remaining(message_deadline, "lock child message deadline")?;
            self.reader.get_ref().set_read_timeout(Some(remaining))?;
            let available = self.reader.fill_buf()?;
            if available.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "lock child closed its protocol channel",
                ));
            }
            let take = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |index| index + 1);
            if self.diagnostic_bytes + take > MAX_DIAGNOSTIC_BYTES {
                return Err(io::Error::other(
                    "lock child exceeded the 64 KiB diagnostic limit",
                ));
            }
            message.extend_from_slice(&available[..take]);
            self.reader.consume(take);
            self.diagnostic_bytes += take;
            if message.last() == Some(&b'\n') {
                break;
            }
        }
        let message = String::from_utf8(message)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 child message"))?;
        Ok(message.trim_end_matches(['\r', '\n']).to_owned())
    }

    fn write_message(&mut self, message: &str) -> io::Result<()> {
        let deadline = Instant::now() + MESSAGE_TIMEOUT;
        self.write_all_until(message.as_bytes(), deadline)?;
        self.write_all_until(b"\n", deadline)?;
        let remaining = self.remaining(deadline, "lock child message deadline")?;
        self.writer.set_write_timeout(Some(remaining))?;
        self.writer.flush()
    }

    fn wait_for_exit(&mut self) -> io::Result<ExitStatus> {
        let deadline = self.started + PROCESS_TIMEOUT;
        wait_until(&mut self.child, deadline)
    }

    fn terminate_and_confirm(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.child.try_wait()? {
            return Ok(status);
        }
        self.child.kill()?;
        wait_until(&mut self.child, Instant::now() + FAILURE_REAP_TIMEOUT)
    }

    fn write_all_until(&mut self, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
        while !bytes.is_empty() {
            let remaining = self.remaining(deadline, "lock child message deadline")?;
            self.writer.set_write_timeout(Some(remaining))?;
            let written = self.writer.write(bytes)?;
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "lock child protocol write returned zero",
                ));
            }
            bytes = &bytes[written..];
        }
        Ok(())
    }

    fn remaining(&self, local_deadline: Instant, label: &str) -> io::Result<Duration> {
        let deadline = local_deadline.min(self.started + PROCESS_TIMEOUT);
        deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, label))
    }
}

struct HolderProcess {
    process: ManagedChild,
    identity: FileIdentity,
}

impl HolderProcess {
    fn spawn(api: LockApi, path: &Path, cwd: &Path) -> io::Result<Self> {
        let mut process = ManagedChild::spawn("lock_holder_process", api, path, cwd)?;
        let ready = process.read_message();
        match ready.and_then(|message| parse_holder_ready(&message, api)) {
            Ok(identity) => Ok(Self { process, identity }),
            Err(error) => {
                let termination = process.terminate_and_confirm();
                Err(with_termination(error, termination))
            }
        }
    }

    fn release(mut self) -> io::Result<()> {
        let result = (|| {
            self.process.write_message("RELEASE")?;
            let released = self.process.read_message()?;
            if released != "RELEASED" {
                return Err(io::Error::other(format!(
                    "unexpected holder release response: {released}"
                )));
            }
            let status = self.process.wait_for_exit()?;
            if !status.success() {
                return Err(io::Error::other(format!(
                    "holder exited unsuccessfully: {status}"
                )));
            }
            Ok(())
        })();
        if let Err(error) = result {
            let termination = self.process.terminate_and_confirm();
            return Err(with_termination(error, termination));
        }
        Ok(())
    }

    fn pid(&self) -> u32 {
        self.process.child.id()
    }

    fn terminate_after_failure(mut self) -> io::Result<()> {
        self.process.terminate_and_confirm().map(|_| ())
    }
}

#[test]
#[ignore = "fixed internal holder entry; spawned only by the L1/L2 parent test"]
fn lock_holder_process() {
    if !child_invocation_requested() {
        return;
    }
    child_entry(run_holder_child());
}

#[test]
#[ignore = "fixed internal contender entry; spawned only by the L1/L2 parent test"]
fn lock_contender_process() {
    if !child_invocation_requested() {
        return;
    }
    child_entry(run_contender_child());
}

#[test]
fn l1_l2_clone_and_full_copy_are_lock_and_write_isolated() {
    for api in [LockApi::Flock, LockApi::PosixFcntl] {
        if let Err(error) = run_l1_l2(api) {
            panic!("{} L1/L2 failed: {error}", api.label());
        }
    }
}

#[test]
fn l3_l4_l6_in_place_rename_and_metadata_observations() {
    for api in [LockApi::Flock, LockApi::PosixFcntl] {
        if let Err(error) = run_l3_l4_l6(api) {
            panic!("{} L3/L4/L6 failed: {error}", api.label());
        }
    }
}

#[test]
fn l5_l7_scanner_marker_and_interleaved_copy_observations() {
    for api in [LockApi::Flock, LockApi::PosixFcntl] {
        if let Err(error) = run_l5_l7(api) {
            panic!("{} L5/L7 failed: {error}", api.label());
        }
    }
}

fn run_l5_l7(api: LockApi) -> io::Result<()> {
    let mut fixture = LockFixture::create(api)?;
    eprintln!(
        "P0-05 L5/L7 fixture={} api={} state=preserved",
        fixture.root.display(),
        api.label()
    );
    fixture
        .tree
        .create_file("source/application.lock", L7_MARKER_BYTES, 0o600)?;
    run_l5(api, &mut fixture)?;
    run_l7_marker(api, &mut fixture)?;
    run_l7_interleaved_copy(api, &mut fixture)
}

fn run_l5(api: LockApi, fixture: &mut LockFixture) -> io::Result<()> {
    record_lsof_version(api)?;
    let path = fixture.source.join("payload.bin");
    let first_holder = HolderProcess::spawn(api, &path, &fixture.root)?;
    let first_pid = first_holder.pid();
    let first_observation = (|| {
        ensure_equal(
            "L5 first holder identity",
            first_holder.identity,
            fixture.source_identity,
        )?;
        let errno = expect_conflict(api, &path, &fixture.root, fixture.source_identity)?;
        let observation = expect_lsof_visible(
            api,
            "held-before-release",
            first_pid,
            &path,
            fixture.source_identity,
        )?;
        print_lsof_visible(api, "held-before-release", errno, &observation);
        Ok(())
    })();
    match first_observation {
        Ok(()) => first_holder.release()?,
        Err(error) => {
            let termination = first_holder.terminate_after_failure();
            return Err(with_termination(error, termination));
        }
    }

    expect_acquired(api, &path, &fixture.root, fixture.source_identity)?;
    let no_match = run_lsof(std::process::id(), &path)?;
    if no_match.status.code() != Some(1)
        || !no_match.stdout.trim().is_empty()
        || !no_match.stderr.trim().is_empty()
    {
        return Err(io::Error::other(format!(
            "expected an exit-1 no-match snapshot for the live parent PID: {no_match:?}"
        )));
    }
    print_lsof_capture(
        api,
        "released-no-match",
        "-a,-FplfiDn,-p<PID>,--,<fixture-path>",
        &no_match,
    );
    eprintln!(
        "P0-05 L5 api={} phase=released-no-match selector_pid={} exit=1 records=0 classification=no-match-not-no-lock-proof",
        api.label(),
        std::process::id()
    );

    let second_holder = HolderProcess::spawn(api, &path, &fixture.root)?;
    let second_pid = second_holder.pid();
    let reacquired = (|| {
        ensure_equal(
            "L5 second holder identity",
            second_holder.identity,
            fixture.source_identity,
        )?;
        let errno = expect_conflict(api, &path, &fixture.root, fixture.source_identity)?;
        let observation = expect_lsof_visible(
            api,
            "reacquired",
            second_pid,
            &path,
            fixture.source_identity,
        )?;
        print_lsof_visible(api, "reacquired", errno, &observation);
        Ok(())
    })();
    match reacquired {
        Ok(()) => second_holder.release()?,
        Err(error) => {
            let termination = second_holder.terminate_after_failure();
            return Err(with_termination(error, termination));
        }
    }

    run_lsof_access_denial(api, fixture)
}

fn run_lsof_access_denial(api: LockApi, fixture: &mut LockFixture) -> io::Result<()> {
    fixture.tree.create_directory("lsof-denied", 0o700)?;
    fixture
        .tree
        .create_file("lsof-denied/locked.bin", ORIGINAL_BYTES, 0o600)?;
    let directory_identity = fixture.tree.identity_at(Path::new("lsof-denied"))?;
    let locked_identity = fixture
        .tree
        .identity_at(Path::new("lsof-denied/locked.bin"))?;
    let directory = rustix::fs::openat(
        fixture.tree.root_fd(),
        Path::new("lsof-denied"),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(errno_error)?;
    ensure_equal(
        "L5 denied directory identity",
        tracked_fd_identity(&directory)?,
        directory_identity,
    )?;
    let path = fixture.root.join("lsof-denied/locked.bin");
    let holder = HolderProcess::spawn(api, &path, &fixture.root)?;
    let scenario = (|| {
        ensure_equal(
            "L5 denied-path holder identity",
            holder.identity,
            locked_identity.into(),
        )?;
        let conflict_errno = expect_conflict(api, &path, &fixture.root, locked_identity.into())?;
        rustix::fs::fchmod(&directory, Mode::empty()).map_err(errno_error)?;
        let scan = run_lsof(holder.pid(), &path);
        let restoration =
            rustix::fs::fchmod(&directory, Mode::from_bits_retain(0o700)).map_err(errno_error);
        if let Err(error) = restoration {
            return Err(match scan {
                Ok(scan) => io::Error::other(format!(
                    "failed to restore lsof-denied directory through held FD: {error}; scan={scan:?}"
                )),
                Err(scan_error) => io::Error::other(format!(
                    "lsof scan failed: {scan_error}; additionally failed to restore lsof-denied directory through held FD: {error}"
                )),
            });
        }
        ensure_equal(
            "L5 denied directory identity after restore",
            tracked_fd_identity(&directory)?,
            directory_identity,
        )?;
        let restored_mode =
            u32::from(rustix::fs::fstat(&directory).map_err(errno_error)?.st_mode & 0o7777);
        ensure_equal("L5 denied directory restored mode", restored_mode, 0o700)?;
        let scan = scan?;
        print_lsof_capture(
            api,
            "permission-probe",
            "-a,-FplfiDn,-p<PID>,--,<fixture-path>",
            &scan,
        );
        if !scan.status.success() && has_permission_denied(&scan.stderr) {
            eprintln!(
                "P0-05 L5 api={} permission_probe=observed conflict_errno={} classification=scan-failure-not-no-lock",
                api.label(),
                conflict_errno
            );
        } else {
            eprintln!(
                "P0-05 L5 api={} permission_probe_executed=true denial=not-observed reason=mode-000-did-not-produce-permission-denied conflict_errno={} exit={:?} stdout_bytes={} stderr_bytes={} classification=not-a-false-no-lock",
                api.label(),
                conflict_errno,
                scan.status.code(),
                scan.stdout.len(),
                scan.stderr.len()
            );
        }
        Ok(())
    })();
    match scenario {
        Ok(()) => holder.release(),
        Err(error) => {
            let termination = holder.terminate_after_failure();
            Err(with_termination(error, termination))
        }
    }
}

fn run_l7_marker(api: LockApi, fixture: &mut LockFixture) -> io::Result<()> {
    let source_marker_relative = Path::new("source/application.lock");
    let source_marker_identity = fixture.tree.identity_at(source_marker_relative)?;
    for backend in [Backend::ApfsFileClone, Backend::FullCopy] {
        let receipt = materialize_once(&fixture.request(backend), backend).map_err(|failure| {
            io::Error::other(format!(
                "L7 {backend:?} materialization failed: {} (evidence={:?})",
                failure.error, failure.evidence
            ))
        })?;
        fixture.adopt_created(backend, &receipt)?;
        verify_receipt(&receipt, backend, fixture.source_identity, 3)?;
        let (target_directory, marker_relative) = match backend {
            Backend::ApfsFileClone => (
                Path::new("clone-target"),
                Path::new("clone-target/application.lock"),
            ),
            Backend::FullCopy => (
                Path::new("copy-target"),
                Path::new("copy-target/application.lock"),
            ),
        };
        let bytes = read_tracked_file(&fixture.tree, marker_relative)?;
        ensure_equal(
            "L7 materialized marker bytes",
            bytes.as_slice(),
            L7_MARKER_BYTES,
        )?;
        let target_identity = fixture.tree.identity_at(marker_relative)?;
        if target_identity.device != source_marker_identity.device
            || target_identity.inode == source_marker_identity.inode
        {
            return Err(io::Error::other(format!(
                "L7 {backend:?} marker is not an independent same-device object"
            )));
        }
        expect_acquired(
            api,
            &fixture.root.join(marker_relative),
            &fixture.root,
            target_identity.into(),
        )?;
        ensure_equal(
            "L7 marker after independent kernel-lock acquisition",
            read_tracked_file(&fixture.tree, marker_relative)?,
            L7_MARKER_BYTES.to_vec(),
        )?;
        write_tracked_file(&fixture.tree, marker_relative, L7_REWRITTEN_MARKER_BYTES)?;
        let exclusive_errno = expect_exclusive_create_exists(
            &fixture.tree,
            target_directory,
            Path::new("application.lock"),
        )?;
        ensure_equal(
            "L7 rewritten copy marker",
            read_tracked_file(&fixture.tree, marker_relative)?,
            L7_REWRITTEN_MARKER_BYTES.to_vec(),
        )?;
        ensure_equal(
            "L7 source marker unaffected by copy marker rewrite",
            read_tracked_file(&fixture.tree, source_marker_relative)?,
            L7_MARKER_BYTES.to_vec(),
        )?;
        eprintln!(
            "P0-05 L7 api={} backend={backend:?} marker_identity=({}, {}) marker_digest={} rewritten_digest={} kernel_lock=acquired-and-released exclusive_create_errno={} marker_after_rewrite=present source_marker=unchanged cow={:?} clone_calls={}",
            api.label(),
            target_identity.device,
            target_identity.inode,
            digest_hex(L7_MARKER_BYTES),
            digest_hex(L7_REWRITTEN_MARKER_BYTES),
            exclusive_errno,
            receipt.cow_evidence,
            receipt.clone_calls_succeeded
        );
    }

    let source_marker_path = fixture.root.join(source_marker_relative);
    let holder = HolderProcess::spawn(api, &source_marker_path, &fixture.root)?;
    let scenario = (|| {
        ensure_equal(
            "L7 source marker holder identity",
            holder.identity,
            source_marker_identity.into(),
        )?;
        let before_errno = expect_conflict(
            api,
            &source_marker_path,
            &fixture.root,
            source_marker_identity.into(),
        )?;
        write_tracked_file(
            &fixture.tree,
            source_marker_relative,
            L7_REWRITTEN_MARKER_BYTES,
        )?;
        let after_errno = expect_conflict(
            api,
            &source_marker_path,
            &fixture.root,
            source_marker_identity.into(),
        )?;
        let exclusive_errno = expect_exclusive_create_exists(
            &fixture.tree,
            Path::new("source"),
            Path::new("application.lock"),
        )?;
        eprintln!(
            "P0-05 L7 api={} source_marker=({}, {}) before_errno={} after_rewrite_errno={} exclusive_create_errno={} rewritten_digest={} result=marker-present-while-kernel-lock-held",
            api.label(),
            source_marker_identity.device,
            source_marker_identity.inode,
            before_errno,
            after_errno,
            exclusive_errno,
            digest_hex(L7_REWRITTEN_MARKER_BYTES)
        );
        Ok(())
    })();
    match scenario {
        Ok(()) => holder.release()?,
        Err(error) => {
            let termination = holder.terminate_after_failure();
            return Err(with_termination(error, termination));
        }
    }

    expect_acquired(
        api,
        &source_marker_path,
        &fixture.root,
        source_marker_identity.into(),
    )?;
    ensure_equal(
        "L7 source marker after kernel-lock release",
        read_tracked_file(&fixture.tree, source_marker_relative)?,
        L7_REWRITTEN_MARKER_BYTES.to_vec(),
    )?;
    for relative in [
        Path::new("clone-target/application.lock"),
        Path::new("copy-target/application.lock"),
    ] {
        ensure_equal(
            "L7 copied marker after source kernel-lock release",
            read_tracked_file(&fixture.tree, relative)?,
            L7_REWRITTEN_MARKER_BYTES.to_vec(),
        )?;
    }
    eprintln!(
        "P0-05 L7 api={} source_marker_after_release=present clone_marker_after_release=present full_copy_marker_after_release=present deletion_attempted=false",
        api.label()
    );
    Ok(())
}

fn run_l7_interleaved_copy(api: LockApi, fixture: &mut LockFixture) -> io::Result<()> {
    for directory in ["interleave", "interleave/source", "interleave/target"] {
        fixture.tree.create_directory(directory, 0o700)?;
    }
    for relative in ["interleave/source/a", "interleave/source/b"] {
        fixture.tree.create_file(relative, L7_VALUE_ZERO, 0o600)?;
    }
    for relative in ["interleave/target/a", "interleave/target/b"] {
        fixture.tree.create_file(relative, b"", 0o600)?;
    }

    let state_00 = read_pair(&fixture.tree, "interleave/source")?;
    ensure_pair("L7 initial source", &state_00, L7_VALUE_ZERO, L7_VALUE_ZERO)?;
    copy_tracked_file(
        &fixture.tree,
        Path::new("interleave/source/a"),
        Path::new("interleave/target/a"),
    )?;
    write_tracked_file(
        &fixture.tree,
        Path::new("interleave/source/a"),
        L7_VALUE_ONE,
    )?;
    let state_10 = read_pair(&fixture.tree, "interleave/source")?;
    ensure_pair(
        "L7 intermediate source",
        &state_10,
        L7_VALUE_ONE,
        L7_VALUE_ZERO,
    )?;
    write_tracked_file(
        &fixture.tree,
        Path::new("interleave/source/b"),
        L7_VALUE_ONE,
    )?;
    let state_11 = read_pair(&fixture.tree, "interleave/source")?;
    ensure_pair("L7 final source", &state_11, L7_VALUE_ONE, L7_VALUE_ONE)?;
    copy_tracked_file(
        &fixture.tree,
        Path::new("interleave/source/b"),
        Path::new("interleave/target/b"),
    )?;
    let target = read_pair(&fixture.tree, "interleave/target")?;
    ensure_pair(
        "L7 interleaved target",
        &target,
        L7_VALUE_ZERO,
        L7_VALUE_ONE,
    )?;
    for (label, source_state) in [
        ("initial", &state_00),
        ("intermediate", &state_10),
        ("final", &state_11),
    ] {
        if &target == source_state {
            return Err(io::Error::other(format!(
                "L7 interleaved target unexpectedly equals the {label} observed source state"
            )));
        }
    }
    eprintln!(
        "P0-05 L7 api={} copy_model=deterministic-per-file-io source_states=(0,0)->(1,0)->(1,1) target=(0,1) target_a_digest={} target_b_digest={} each_file_io=succeeded materialize_once_bad_publication_claim=false",
        api.label(),
        digest_hex(&target.0),
        digest_hex(&target.1)
    );
    Ok(())
}

fn run_l3_l4_l6(api: LockApi) -> io::Result<()> {
    let mut fixture = LockFixture::create(api)?;
    eprintln!(
        "P0-05 L3/L4/L6 fixture={} api={} state=preserved",
        fixture.root.display(),
        api.label()
    );
    for backend in [Backend::ApfsFileClone, Backend::FullCopy] {
        let receipt = materialize_once(&fixture.request(backend), backend).map_err(|failure| {
            io::Error::other(format!(
                "{backend:?} materialization failed: {} (evidence={:?})",
                failure.error, failure.evidence
            ))
        })?;
        fixture.adopt_created(backend, &receipt)?;
        verify_receipt(&receipt, backend, fixture.source_identity, 2)?;
        eprintln!(
            "P0-05 L3/L6 materialization api={} backend={backend:?} cow={:?} clone_calls={} outcome={:?}",
            api.label(),
            receipt.cow_evidence,
            receipt.clone_calls_succeeded,
            receipt.outcome
        );
    }

    let source_before = observe_tracked_file(&fixture.tree, Path::new("source/payload.bin"))?;
    let clone_before = observe_tracked_file(&fixture.tree, Path::new("clone-target/payload.bin"))?;
    let copy_before = observe_tracked_file(&fixture.tree, Path::new("copy-target/payload.bin"))?;
    print_observation(api, "L6-before", "source", &source_before);
    print_observation(api, "L6-before", "clone", &clone_before);
    print_observation(api, "L6-before", "full-copy", &copy_before);

    let source_payload = fixture.source.join("payload.bin");
    let source_holder = HolderProcess::spawn(api, &source_payload, &fixture.root)?;
    let l3_l6 = ensure_equal(
        "L3 holder/source identity",
        source_holder.identity,
        fixture.source_identity,
    )
    .and_then(|()| {
        run_l3_l6_while_locked(api, &fixture, &source_before, &clone_before, &copy_before)
    });
    match l3_l6 {
        Ok(()) => source_holder.release()?,
        Err(error) => {
            let termination = source_holder.terminate_after_failure();
            return Err(with_termination(error, termination));
        }
    }
    run_l4(api, &mut fixture)
}

fn run_l3_l6_while_locked(
    api: LockApi,
    fixture: &LockFixture,
    source_before: &FileObservation,
    clone_before: &FileObservation,
    copy_before: &FileObservation,
) -> io::Result<()> {
    let before_errno = expect_conflict(
        api,
        &fixture.source.join("payload.bin"),
        &fixture.root,
        fixture.source_identity,
    )?;

    write_tracked_file(
        &fixture.tree,
        Path::new("source/payload.bin"),
        L3_SOURCE_BYTES,
    )?;
    let source_after = observe_tracked_file(&fixture.tree, Path::new("source/payload.bin"))?;
    ensure_equal(
        "L3 source identity after in-place write",
        source_after.metadata.identity,
        source_before.metadata.identity,
    )?;
    ensure_equal(
        "L3 source digest after in-place write",
        &source_after.content_digest_hex,
        &digest_hex(L3_SOURCE_BYTES),
    )?;
    let after_source_errno = expect_conflict(
        api,
        &fixture.source.join("payload.bin"),
        &fixture.root,
        fixture.source_identity,
    )?;

    write_tracked_file(
        &fixture.tree,
        Path::new("clone-target/payload.bin"),
        L3_CLONE_BYTES,
    )?;
    let clone_after = observe_tracked_file(&fixture.tree, Path::new("clone-target/payload.bin"))?;
    ensure_equal(
        "L3 clone identity after in-place write",
        clone_after.metadata.identity,
        clone_before.metadata.identity,
    )?;
    ensure_equal(
        "L3 clone digest after in-place write",
        &clone_after.content_digest_hex,
        &digest_hex(L3_CLONE_BYTES),
    )?;
    ensure_bytes(&fixture.source.join("payload.bin"), L3_SOURCE_BYTES)?;
    ensure_bytes(&fixture.copy_target.join("payload.bin"), ORIGINAL_BYTES)?;

    write_tracked_file(
        &fixture.tree,
        Path::new("copy-target/payload.bin"),
        L6_COPY_BYTES,
    )?;
    let copy_after = observe_tracked_file(&fixture.tree, Path::new("copy-target/payload.bin"))?;
    ensure_equal(
        "L6 Full Copy identity after in-place write",
        copy_after.metadata.identity,
        copy_before.metadata.identity,
    )?;
    ensure_equal(
        "L6 Full Copy digest after in-place write",
        &copy_after.content_digest_hex,
        &digest_hex(L6_COPY_BYTES),
    )?;
    ensure_bytes(&fixture.source.join("payload.bin"), L3_SOURCE_BYTES)?;
    ensure_bytes(&fixture.clone_target.join("payload.bin"), L3_CLONE_BYTES)?;

    print_observation(api, "L6-after-write", "source", &source_after);
    print_observation(api, "L6-after-write", "clone", &clone_after);
    print_observation(api, "L6-after-write", "full-copy", &copy_after);

    let source_restored = restore_tracked_times(
        &fixture.tree,
        Path::new("source/payload.bin"),
        &source_before.metadata,
    )?;
    let clone_restored = restore_tracked_times(
        &fixture.tree,
        Path::new("clone-target/payload.bin"),
        &clone_before.metadata,
    )?;
    let copy_restored = restore_tracked_times(
        &fixture.tree,
        Path::new("copy-target/payload.bin"),
        &copy_before.metadata,
    )?;
    verify_restored_times("source", &source_restored, &source_before.metadata)?;
    verify_restored_times("clone", &clone_restored, &clone_before.metadata)?;
    verify_restored_times("full-copy", &copy_restored, &copy_before.metadata)?;
    print_metadata(api, "L6-restored-before-read", "source", &source_restored);
    print_metadata(api, "L6-restored-before-read", "clone", &clone_restored);
    print_metadata(api, "L6-restored-before-read", "full-copy", &copy_restored);

    let source_after_restore_read =
        observe_tracked_file(&fixture.tree, Path::new("source/payload.bin"))?;
    let clone_after_restore_read =
        observe_tracked_file(&fixture.tree, Path::new("clone-target/payload.bin"))?;
    let copy_after_restore_read =
        observe_tracked_file(&fixture.tree, Path::new("copy-target/payload.bin"))?;
    ensure_equal(
        "source content after timestamp restore",
        &source_after_restore_read.content_digest_hex,
        &source_after.content_digest_hex,
    )?;
    ensure_equal(
        "clone content after timestamp restore",
        &clone_after_restore_read.content_digest_hex,
        &clone_after.content_digest_hex,
    )?;
    ensure_equal(
        "Full Copy content after timestamp restore",
        &copy_after_restore_read.content_digest_hex,
        &copy_after.content_digest_hex,
    )?;
    print_observation(
        api,
        "L6-after-restore-read",
        "source",
        &source_after_restore_read,
    );
    print_observation(
        api,
        "L6-after-restore-read",
        "clone",
        &clone_after_restore_read,
    );
    print_observation(
        api,
        "L6-after-restore-read",
        "full-copy",
        &copy_after_restore_read,
    );

    let final_errno = expect_conflict(
        api,
        &fixture.source.join("payload.bin"),
        &fixture.root,
        fixture.source_identity,
    )?;
    eprintln!(
        "P0-05 L3 api={} source_identity=({}, {}) before_errno={} after_write_errno={} final_errno={} source_digest={} clone_digest={} result=in-place-identities-stable-and-source-lock-retained",
        api.label(),
        fixture.source_identity.device,
        fixture.source_identity.inode,
        before_errno,
        after_source_errno,
        final_errno,
        source_after.content_digest_hex,
        clone_after.content_digest_hex
    );
    Ok(())
}

fn run_l4(api: LockApi, fixture: &mut LockFixture) -> io::Result<()> {
    fixture.tree.create_directory("rename", 0o700)?;
    fixture
        .tree
        .create_file("rename/active.bin", L4_OLD_BYTES, 0o600)?;
    let old_identity = fixture.tree.identity_at(Path::new("rename/active.bin"))?;
    rustix::fs::linkat(
        fixture.tree.root_fd(),
        Path::new("rename/active.bin"),
        fixture.tree.root_fd(),
        Path::new("rename/old-object.bin"),
        AtFlags::empty(),
    )
    .map_err(errno_error)?;
    fixture.tree.adopt_confirmed(
        Path::new("rename/old-object.bin"),
        old_identity.device,
        old_identity.inode,
        TrackedKind::RegularFile,
    )?;
    fixture
        .tree
        .create_file("rename/replacement.bin", L4_NEW_BYTES, 0o600)?;
    let new_identity = fixture
        .tree
        .identity_at(Path::new("rename/replacement.bin"))?;
    if new_identity == old_identity {
        return Err(io::Error::other(
            "L4 replacement unexpectedly shares the old object identity",
        ));
    }

    let (old_fd, observed_old_identity) = open_tracked_regular_file(
        &fixture.tree,
        Path::new("rename/active.bin"),
        OFlags::RDONLY,
    )?;
    ensure_equal(
        "L4 parent old FD identity",
        observed_old_identity,
        old_identity,
    )?;
    let old_file = File::from(old_fd);
    let old_before = observe_open_file(&old_file)?;
    ensure_equal(
        "L4 old FD initial digest",
        &old_before.content_digest_hex,
        &digest_hex(L4_OLD_BYTES),
    )?;

    let active_path = fixture.root.join("rename/active.bin");
    let old_link_path = fixture.root.join("rename/old-object.bin");
    let holder = HolderProcess::spawn(api, &active_path, &fixture.root)?;
    let scenario = (|| {
        ensure_equal(
            "L4 holder/old object identity",
            holder.identity,
            old_identity.into(),
        )?;
        let before_errno = expect_conflict(api, &active_path, &fixture.root, old_identity.into())?;
        let rename_dir_expected = fixture.tree.identity_at(Path::new("rename"))?;
        let rename_dir = rustix::fs::openat(
            fixture.tree.root_fd(),
            Path::new("rename"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(errno_error)?;
        ensure_equal(
            "L4 rename directory identity",
            tracked_fd_identity(&rename_dir)?,
            rename_dir_expected,
        )?;
        ensure_equal(
            "L4 active identity immediately before rename",
            statat_identity(&rename_dir, Path::new("active.bin"))?,
            old_identity,
        )?;
        ensure_equal(
            "L4 replacement identity immediately before rename",
            statat_identity(&rename_dir, Path::new("replacement.bin"))?,
            new_identity,
        )?;

        rustix::fs::renameat(
            &rename_dir,
            Path::new("replacement.bin"),
            &rename_dir,
            Path::new("active.bin"),
        )
        .map_err(errno_error)?;
        // The atomic replacement changes both registered names in one syscall.
        // From here onward, this preserved fixture is observed through the held
        // directory/old file FDs and is not passed back to the cleanup ledger.
        ensure_equal(
            "L4 active identity after rename",
            statat_identity(&rename_dir, Path::new("active.bin"))?,
            new_identity,
        )?;
        ensure_equal(
            "L4 old hard-link identity after rename",
            statat_identity(&rename_dir, Path::new("old-object.bin"))?,
            old_identity,
        )?;
        assert_missing_at(&rename_dir, Path::new("replacement.bin"))?;

        let old_after = observe_open_file(&old_file)?;
        ensure_equal(
            "L4 retained old FD identity",
            old_after.metadata.identity,
            FileIdentity::from(old_identity),
        )?;
        ensure_equal(
            "L4 retained old FD content",
            &old_after.content_digest_hex,
            &digest_hex(L4_OLD_BYTES),
        )?;

        let new_file = open_known_file_at(&rename_dir, Path::new("active.bin"), new_identity)?;
        let new_after = observe_open_file(&new_file)?;
        ensure_equal(
            "L4 new path content",
            &new_after.content_digest_hex,
            &digest_hex(L4_NEW_BYTES),
        )?;
        let old_errno = expect_conflict(api, &old_link_path, &fixture.root, old_identity.into())?;
        expect_acquired(api, &active_path, &fixture.root, new_identity.into())?;
        eprintln!(
            "P0-05 L4 api={} published=false old_fd=({}, {}) old_link=({}, {}) new_path=({}, {}) before_errno={} old_errno={} old_digest={} new_digest={} result=old-locked-object-retained-new-path-lockable",
            api.label(),
            old_after.metadata.identity.device,
            old_after.metadata.identity.inode,
            old_identity.device,
            old_identity.inode,
            new_identity.device,
            new_identity.inode,
            before_errno,
            old_errno,
            old_after.content_digest_hex,
            new_after.content_digest_hex
        );
        Ok(())
    })();
    match scenario {
        Ok(()) => holder.release(),
        Err(error) => {
            let termination = holder.terminate_after_failure();
            Err(with_termination(error, termination))
        }
    }
}

fn run_l1_l2(api: LockApi) -> io::Result<()> {
    let mut fixture = LockFixture::create(api)?;
    eprintln!(
        "P0-05 fixture={} api={} state=preserved",
        fixture.root.display(),
        api.label()
    );
    let source_payload = fixture.source.join("payload.bin");
    let holder = HolderProcess::spawn(api, &source_payload, &fixture.root)?;
    let scenario = run_l1_l2_while_locked(api, &mut fixture, &holder);
    match scenario {
        Ok(()) => holder.release(),
        Err(error) => {
            let termination = holder.terminate_after_failure();
            Err(with_termination(error, termination))
        }
    }
}

fn run_l1_l2_while_locked(
    api: LockApi,
    fixture: &mut LockFixture,
    holder: &HolderProcess,
) -> io::Result<()> {
    ensure_equal(
        "holder/source identity",
        holder.identity,
        fixture.source_identity,
    )?;

    for (alias, path) in [
        ("same-path", fixture.source.join("payload.bin")),
        ("hard-link", fixture.source.join("payload-hard-link.bin")),
        (
            "symbolic-link",
            fixture.source.join("payload-symbolic-link.bin"),
        ),
    ] {
        let errno = expect_conflict(api, &path, &fixture.root, fixture.source_identity)?;
        eprintln!(
            "P0-05 L1 api={} syscall={} alias={} result=conflict errno={} dev={} ino={} digest={}",
            api.label(),
            api.syscall(),
            alias,
            errno,
            fixture.source_identity.device,
            fixture.source_identity.inode,
            digest_hex(ORIGINAL_BYTES)
        );
    }

    let source_volume = apfs_volume(&fixture.source)?;
    eprintln!(
        "P0-05 volume api={} type=apfs fsid={:?} uuid={}",
        api.label(),
        source_volume.fsid,
        source_volume.uuid
    );
    for backend in [Backend::ApfsFileClone, Backend::FullCopy] {
        let receipt = materialize_once(&fixture.request(backend), backend).map_err(|failure| {
            io::Error::other(format!(
                "{backend:?} materialization failed: {} (evidence={:?})",
                failure.error, failure.evidence
            ))
        })?;
        fixture.adopt_created(backend, &receipt)?;
        verify_receipt(&receipt, backend, fixture.source_identity, 2)?;

        let target_payload = fixture.target(backend).join("payload.bin");
        let target_identity = metadata_identity(&target_payload)?;
        if target_identity.device != fixture.source_identity.device
            || target_identity.inode == fixture.source_identity.inode
        {
            return Err(io::Error::other(format!(
                "{backend:?} target is not an independent same-device file: source={:?}, target={target_identity:?}",
                fixture.source_identity
            )));
        }
        ensure_equal(
            "source/target APFS volume",
            apfs_volume(fixture.target(backend))?,
            VolumeIdentity {
                fsid: source_volume.fsid,
                uuid: source_volume.uuid.clone(),
            },
        )?;
        expect_acquired(api, &target_payload, &fixture.root, target_identity)?;
        let source_errno = expect_conflict(
            api,
            &fixture.source.join("payload.bin"),
            &fixture.root,
            fixture.source_identity,
        )?;
        eprintln!(
            "P0-05 L2 api={} backend={backend:?} cow={:?} clone_calls={} source=({}, {}) target=({}, {}) target_lock=acquired source_lock=conflict source_errno={} digest={}",
            api.label(),
            receipt.cow_evidence,
            receipt.clone_calls_succeeded,
            fixture.source_identity.device,
            fixture.source_identity.inode,
            target_identity.device,
            target_identity.inode,
            source_errno,
            digest_hex(ORIGINAL_BYTES)
        );
    }

    let clone_identity = metadata_identity(&fixture.clone_target.join("payload.bin"))?;
    let copy_identity = metadata_identity(&fixture.copy_target.join("payload.bin"))?;
    if clone_identity == copy_identity {
        return Err(io::Error::other(format!(
            "clone and Full Copy targets unexpectedly share one identity: {clone_identity:?}"
        )));
    }

    write_tracked_file(
        &fixture.tree,
        Path::new("clone-target/payload.bin"),
        CLONE_BYTES,
    )?;
    ensure_bytes(&fixture.clone_target.join("payload.bin"), CLONE_BYTES)?;
    ensure_bytes(&fixture.copy_target.join("payload.bin"), ORIGINAL_BYTES)?;
    ensure_bytes(&fixture.source.join("payload.bin"), ORIGINAL_BYTES)?;

    write_tracked_file(&fixture.tree, Path::new("source/payload.bin"), SOURCE_BYTES)?;
    ensure_bytes(&fixture.source.join("payload.bin"), SOURCE_BYTES)?;
    ensure_bytes(&fixture.source.join("payload-hard-link.bin"), SOURCE_BYTES)?;
    ensure_bytes(
        &fixture.source.join("payload-symbolic-link.bin"),
        SOURCE_BYTES,
    )?;
    ensure_bytes(&fixture.clone_target.join("payload.bin"), CLONE_BYTES)?;
    ensure_bytes(&fixture.copy_target.join("payload.bin"), ORIGINAL_BYTES)?;

    write_tracked_file(
        &fixture.tree,
        Path::new("copy-target/payload.bin"),
        COPY_BYTES,
    )?;
    ensure_bytes(&fixture.copy_target.join("payload.bin"), COPY_BYTES)?;
    ensure_bytes(&fixture.clone_target.join("payload.bin"), CLONE_BYTES)?;
    ensure_bytes(&fixture.source.join("payload.bin"), SOURCE_BYTES)?;
    let source_errno = expect_conflict(
        api,
        &fixture.source.join("payload.bin"),
        &fixture.root,
        fixture.source_identity,
    )?;
    eprintln!(
        "P0-05 L2 api={} writes=source/clone/full-copy-independent source_lock_after_writes=conflict source_errno={} source_digest={} clone_digest={} copy_digest={}",
        api.label(),
        source_errno,
        digest_hex(SOURCE_BYTES),
        digest_hex(CLONE_BYTES),
        digest_hex(COPY_BYTES)
    );
    Ok(())
}

fn run_holder_child() -> io::Result<()> {
    let api = child_api()?;
    let path = child_path()?;
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    lock_file(&file, api)?;
    let identity = fd_identity(&file)?;
    child_message(&format!(
        "READY {} {} {}",
        api.label(),
        identity.device,
        identity.inode
    ))?;
    let mut command = String::new();
    BufReader::new(io::stdin().lock()).read_line(&mut command)?;
    if command.trim_end_matches(['\r', '\n']) != "RELEASE" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "holder received an unexpected command",
        ));
    }
    unlock_file(&file, api)?;
    child_message("RELEASED")
}

fn run_contender_child() -> io::Result<()> {
    let api = child_api()?;
    let path = child_path()?;
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    let identity = fd_identity(&file)?;
    match try_lock_file(&file, api) {
        Ok(()) => {
            child_message(&format!(
                "ACQUIRED {} {} {}",
                api.label(),
                identity.device,
                identity.inode
            ))?;
            unlock_file(&file, api)
        }
        Err(errno) if is_lock_conflict(errno) => child_message(&format!(
            "CONFLICT {} {} {} {}",
            api.label(),
            errno.raw_os_error(),
            identity.device,
            identity.inode
        )),
        Err(errno) => Err(errno_error(errno)),
    }
}

fn child_entry(result: io::Result<()>) {
    if let Err(error) = result {
        let _ = child_message(&format!(
            "ERROR {} {}",
            error.raw_os_error().unwrap_or(0),
            error
        ));
        panic!("lock child failed: {error}");
    }
}

fn child_message(message: &str) -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    stderr.write_all(message.as_bytes())?;
    stderr.write_all(b"\n")?;
    stderr.flush()
}

fn child_api() -> io::Result<LockApi> {
    let value = env::var(CHILD_API)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "missing child lock API"))?;
    LockApi::parse(&value)
}

fn child_path() -> io::Result<PathBuf> {
    env::var_os(CHILD_PATH)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing child lock path"))
}

fn child_invocation_requested() -> bool {
    env::var_os(CHILD_API).is_some() || env::var_os(CHILD_PATH).is_some()
}

fn lock_file(file: &File, api: LockApi) -> io::Result<()> {
    let result = match api {
        LockApi::Flock => rustix::fs::flock(file, FlockOperation::LockExclusive),
        LockApi::PosixFcntl => rustix::fs::fcntl_lock(file, FlockOperation::LockExclusive),
    };
    result.map_err(errno_error)
}

fn try_lock_file(file: &File, api: LockApi) -> Result<(), rustix::io::Errno> {
    match api {
        LockApi::Flock => rustix::fs::flock(file, FlockOperation::NonBlockingLockExclusive),
        LockApi::PosixFcntl => {
            rustix::fs::fcntl_lock(file, FlockOperation::NonBlockingLockExclusive)
        }
    }
}

fn unlock_file(file: &File, api: LockApi) -> io::Result<()> {
    let result = match api {
        LockApi::Flock => rustix::fs::flock(file, FlockOperation::Unlock),
        LockApi::PosixFcntl => rustix::fs::fcntl_lock(file, FlockOperation::Unlock),
    };
    result.map_err(errno_error)
}

fn is_lock_conflict(errno: rustix::io::Errno) -> bool {
    errno == rustix::io::Errno::WOULDBLOCK
        || errno == rustix::io::Errno::AGAIN
        || errno == rustix::io::Errno::ACCESS
}

fn observe_contender(api: LockApi, path: &Path, cwd: &Path) -> io::Result<LockAttempt> {
    let mut process = ManagedChild::spawn("lock_contender_process", api, path, cwd)?;
    let observation = (|| {
        let message = process.read_message()?;
        let attempt = parse_lock_attempt(&message, api)?;
        let status = process.wait_for_exit()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "contender exited unsuccessfully after {message:?}: {status}"
            )));
        }
        Ok(attempt)
    })();
    if let Err(error) = observation {
        let termination = process.terminate_and_confirm();
        return Err(with_termination(error, termination));
    }
    observation
}

fn expect_conflict(
    api: LockApi,
    path: &Path,
    cwd: &Path,
    expected_identity: FileIdentity,
) -> io::Result<i32> {
    match observe_contender(api, path, cwd)? {
        LockAttempt::Conflict { errno, identity } => {
            ensure_equal(
                "conflicting contender identity",
                identity,
                expected_identity,
            )?;
            if ![libc::EWOULDBLOCK, libc::EAGAIN, libc::EACCES].contains(&errno) {
                return Err(io::Error::other(format!(
                    "lock conflict returned unexpected errno {errno}"
                )));
            }
            Ok(errno)
        }
        LockAttempt::Acquired { identity } => Err(io::Error::other(format!(
            "positive control unexpectedly acquired {} on {path:?} (identity={identity:?})",
            api.syscall()
        ))),
    }
}

fn expect_acquired(
    api: LockApi,
    path: &Path,
    cwd: &Path,
    expected_identity: FileIdentity,
) -> io::Result<()> {
    match observe_contender(api, path, cwd)? {
        LockAttempt::Acquired { identity } => ensure_equal(
            "independent target contender identity",
            identity,
            expected_identity,
        ),
        LockAttempt::Conflict { errno, identity } => Err(io::Error::other(format!(
            "independent target conflicted for {}: errno={errno}, identity={identity:?}",
            api.syscall()
        ))),
    }
}

fn run_lsof(pid: u32, path: &Path) -> io::Result<LsofCapture> {
    capture_lsof(Some((pid, path)))
}

fn capture_lsof(selection: Option<(u32, &Path)>) -> io::Result<LsofCapture> {
    let (mut stdout_reader, stdout_writer) = UnixStream::pair()?;
    let (mut stderr_reader, stderr_writer) = UnixStream::pair()?;
    stdout_reader.set_nonblocking(true)?;
    stderr_reader.set_nonblocking(true)?;
    let stdout_writer: OwnedFd = stdout_writer.into();
    let stderr_writer: OwnedFd = stderr_writer.into();
    let started = Instant::now();
    let mut command = Command::new(LSOF_PATH);
    match selection {
        Some((pid, path)) => {
            command
                .arg("-a")
                .arg("-FplfiDn")
                .arg(format!("-p{pid}"))
                .arg("--")
                .arg(path);
        }
        None => {
            command.arg("-v");
        }
    }
    let mut child = command
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_writer))
        .stderr(Stdio::from(stderr_writer))
        .spawn()?;
    drop(command);
    let deadline = (started + MESSAGE_TIMEOUT).min(started + PROCESS_TIMEOUT);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut diagnostic_bytes = 0;
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    let capture = (|| {
        let status = loop {
            if !stdout_eof {
                stdout_eof = drain_bounded_stream(
                    &mut stdout_reader,
                    &mut stdout,
                    &mut diagnostic_bytes,
                    deadline,
                )?;
            }
            if !stderr_eof {
                stderr_eof = drain_bounded_stream(
                    &mut stderr_reader,
                    &mut stderr,
                    &mut diagnostic_bytes,
                    deadline,
                )?;
            }
            if let Some(status) = child.try_wait()?
                && stdout_eof
                && stderr_eof
            {
                break status;
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "lsof exceeded its 5-second capture deadline",
                ));
            }
            thread::sleep(Duration::from_millis(10).min(deadline.duration_since(now)));
        };
        let stdout = String::from_utf8(stdout)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 lsof stdout"))?;
        let stderr = String::from_utf8(stderr)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 lsof stderr"))?;
        Ok(LsofCapture {
            status,
            stdout,
            stderr,
        })
    })();
    match capture {
        Ok(capture) => Ok(capture),
        Err(error) => {
            let termination = terminate_child(&mut child);
            Err(with_termination(error, termination))
        }
    }
}

fn drain_bounded_stream(
    stream: &mut UnixStream,
    destination: &mut Vec<u8>,
    diagnostic_bytes: &mut usize,
    deadline: Instant,
) -> io::Result<bool> {
    let mut buffer = [0_u8; 4096];
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "lsof exceeded its 5-second capture deadline while reading output",
            ));
        }
        let remaining = MAX_DIAGNOSTIC_BYTES.saturating_sub(*diagnostic_bytes);
        let read_limit = (remaining + usize::from(remaining == 0)).min(buffer.len());
        match stream.read(&mut buffer[..read_limit]) {
            Ok(0) => return Ok(true),
            Ok(read) if read > remaining => {
                return Err(io::Error::other(
                    "lsof exceeded the combined 64 KiB stdout/stderr limit",
                ));
            }
            Ok(read) => {
                destination.extend_from_slice(&buffer[..read]);
                *diagnostic_bytes += read;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

fn terminate_child(child: &mut Child) -> io::Result<ExitStatus> {
    if let Some(status) = child.try_wait()? {
        return Ok(status);
    }
    child.kill()?;
    wait_until(child, Instant::now() + FAILURE_REAP_TIMEOUT)
}

fn expect_lsof_visible(
    api: LockApi,
    phase: &str,
    pid: u32,
    path: &Path,
    expected_identity: FileIdentity,
) -> io::Result<LsofFileObservation> {
    let capture = run_lsof(pid, path)?;
    print_lsof_capture(
        api,
        phase,
        "-a,-FplfiDn,-p<PID>,--,<fixture-path>",
        &capture,
    );
    if !capture.status.success() {
        return Err(io::Error::other(format!(
            "lsof did not report the known holder: {capture:?}"
        )));
    }
    let observation = parse_lsof_file(&capture.stdout)?;
    ensure_equal("lsof holder PID", observation.pid, pid)?;
    ensure_equal(
        "lsof file identity",
        observation.identity,
        expected_identity,
    )?;
    let expected_name = path
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "non-UTF-8 lsof path"))?;
    ensure_equal("lsof file name", observation.name.as_str(), expected_name)?;
    Ok(observation)
}

fn record_lsof_version(api: LockApi) -> io::Result<()> {
    let capture = capture_lsof(None)?;
    print_lsof_capture(api, "version-check", "-v", &capture);
    if !capture.status.success() {
        return Err(io::Error::other(format!(
            "{LSOF_PATH} -v exited unsuccessfully: {capture:?}"
        )));
    }
    let revision = capture
        .stderr
        .lines()
        .find_map(|line| line.trim().strip_prefix("revision: "))
        .filter(|revision| !revision.is_empty())
        .ok_or_else(|| io::Error::other(format!("{LSOF_PATH} -v omitted its revision")))?;
    eprintln!(
        "P0-05 L5 api={} tool={} observed_revision={} support_gate=false",
        api.label(),
        LSOF_PATH,
        revision
    );
    Ok(())
}

fn parse_lsof_file(stdout: &str) -> io::Result<LsofFileObservation> {
    let pid = parse_single_lsof_field(stdout, 'p')?
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid lsof PID field"))?;
    let fd = parse_single_lsof_field(stdout, 'f')?.to_owned();
    let inode = parse_single_lsof_field(stdout, 'i')?
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid lsof inode field"))?;
    let device = parse_lsof_device(parse_single_lsof_field(stdout, 'D')?)?;
    let name = parse_single_lsof_field(stdout, 'n')?.to_owned();
    let lock_field = optional_single_lsof_field(stdout, 'l')?.map(ToOwned::to_owned);
    Ok(LsofFileObservation {
        pid,
        fd,
        lock_field,
        identity: FileIdentity { device, inode },
        name,
    })
}

fn parse_single_lsof_field(stdout: &str, prefix: char) -> io::Result<&str> {
    optional_single_lsof_field(stdout, prefix)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("lsof omitted required {prefix} field"),
        )
    })
}

fn optional_single_lsof_field(stdout: &str, prefix: char) -> io::Result<Option<&str>> {
    let mut values = stdout.lines().filter_map(|line| line.strip_prefix(prefix));
    let value = values.next();
    if values.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("lsof reported multiple {prefix} fields for one selected file"),
        ));
    }
    Ok(value)
}

fn parse_lsof_device(value: &str) -> io::Result<u64> {
    if let Some(hexadecimal) = value.strip_prefix("0x") {
        u64::from_str_radix(hexadecimal, 16)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid lsof device field"))
    } else {
        value
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid lsof device field"))
    }
}

fn has_permission_denied(stderr: &str) -> bool {
    stderr.to_ascii_lowercase().contains("permission denied")
}

fn print_lsof_capture(api: LockApi, phase: &str, argv: &str, capture: &LsofCapture) {
    eprintln!(
        "P0-05 L5 api={} phase={} tool={} argv={} exit={:?} stdout={:?} stderr={:?}",
        api.label(),
        phase,
        LSOF_PATH,
        argv,
        capture.status.code(),
        capture.stdout,
        capture.stderr
    );
}

fn print_lsof_visible(
    api: LockApi,
    phase: &str,
    conflict_errno: i32,
    observation: &LsofFileObservation,
) {
    eprintln!(
        "P0-05 L5 api={} phase={} contender=conflict errno={} scanner=visible pid={} fd={:?} lock_field={:?} dev={} ino={} name={:?} lock_field_blank_or_missing=incomplete-not-unlocked snapshot_guarantee=none",
        api.label(),
        phase,
        conflict_errno,
        observation.pid,
        observation.fd,
        observation.lock_field,
        observation.identity.device,
        observation.identity.inode,
        observation.name
    );
}

fn parse_holder_ready(message: &str, api: LockApi) -> io::Result<FileIdentity> {
    let fields: Vec<_> = message.split_whitespace().collect();
    if fields.len() != 4 || fields[0] != "READY" || fields[1] != api.label() {
        return Err(io::Error::other(format!(
            "unexpected holder handshake: {message:?}"
        )));
    }
    Ok(FileIdentity {
        device: parse_u64(fields[2])?,
        inode: parse_u64(fields[3])?,
    })
}

fn parse_lock_attempt(message: &str, api: LockApi) -> io::Result<LockAttempt> {
    let fields: Vec<_> = message.split_whitespace().collect();
    match fields.as_slice() {
        ["ACQUIRED", observed_api, device, inode] if *observed_api == api.label() => {
            Ok(LockAttempt::Acquired {
                identity: FileIdentity {
                    device: parse_u64(device)?,
                    inode: parse_u64(inode)?,
                },
            })
        }
        ["CONFLICT", observed_api, errno, device, inode] if *observed_api == api.label() => {
            Ok(LockAttempt::Conflict {
                errno: errno.parse().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid child errno")
                })?,
                identity: FileIdentity {
                    device: parse_u64(device)?,
                    inode: parse_u64(inode)?,
                },
            })
        }
        _ => Err(io::Error::other(format!(
            "unexpected contender message: {message:?}"
        ))),
    }
}

fn parse_u64(value: &str) -> io::Result<u64> {
    value
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid child identity"))
}

fn fd_identity(file: &File) -> io::Result<FileIdentity> {
    let metadata = rustix::fs::fstat(file).map_err(errno_error)?;
    Ok(FileIdentity {
        device: metadata.st_dev as u64,
        inode: metadata.st_ino,
    })
}

fn metadata_identity(path: &Path) -> io::Result<FileIdentity> {
    let metadata = fs::metadata(path)?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

fn apfs_volume(path: &Path) -> io::Result<VolumeIdentity> {
    let report = inspect_path(path)
        .map_err(|error| io::Error::other(format!("APFS path probe failed: {error}")))?;
    if report.filesystem.type_name != "apfs" {
        return Err(io::Error::other(format!(
            "lock fixture is not on APFS: {}",
            report.filesystem.type_name
        )));
    }
    let uuid = match report.filesystem.volume_uuid {
        Evidence::Known { value, .. } => value,
        Evidence::Unknown { reason, errno } => {
            return Err(io::Error::other(format!(
                "APFS Volume UUID is unknown: reason={reason}, errno={errno:?}"
            )));
        }
    };
    Ok(VolumeIdentity {
        fsid: report.filesystem.fsid,
        uuid,
    })
}

fn verify_receipt(
    receipt: &AttemptEvidence,
    backend: Backend,
    source_identity: FileIdentity,
    expected_ordinary_files: usize,
) -> io::Result<()> {
    ensure_equal("receipt backend", receipt.backend, backend)?;
    ensure_equal(
        "receipt outcome",
        receipt.outcome,
        AttemptOutcome::Succeeded,
    )?;
    ensure_equal(
        "source/target manifests",
        &receipt.source_manifest,
        &receipt.target_manifest,
    )?;
    ensure_equal(
        "ordinary file count",
        receipt.ordinary_files_materialized,
        expected_ordinary_files,
    )?;
    match backend {
        Backend::ApfsFileClone => {
            ensure_equal(
                "clone CoW evidence",
                receipt.cow_evidence,
                CowEvidence::Confirmed,
            )?;
            ensure_equal(
                "real clone call count",
                receipt.clone_calls_succeeded,
                expected_ordinary_files,
            )?;
        }
        Backend::FullCopy => {
            ensure_equal(
                "copy CoW evidence",
                receipt.cow_evidence,
                CowEvidence::NotUsed,
            )?;
            ensure_equal("copy clone call count", receipt.clone_calls_succeeded, 0)?;
        }
    }
    let payload = receipt
        .ordinary_files
        .iter()
        .find(|file| file.path.display == "payload.bin")
        .ok_or_else(|| io::Error::other("receipt omitted payload.bin identity evidence"))?;
    ensure_equal(
        "receipt source identity",
        FileIdentity {
            device: payload.source_identity.device,
            inode: payload.source_identity.inode,
        },
        source_identity,
    )?;
    ensure_equal(
        "receipt clone syscall flag",
        payload.real_clone_call_succeeded,
        backend == Backend::ApfsFileClone,
    )
}

fn ensure_bytes(path: &Path, expected: &[u8]) -> io::Result<()> {
    let actual = fs::read(path)?;
    if actual != expected {
        return Err(io::Error::other(format!(
            "unexpected content at {path:?}: expected {} bytes, observed {} bytes",
            expected.len(),
            actual.len()
        )));
    }
    Ok(())
}

fn digest_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn read_tracked_file(tree: &ControlledTree, relative: &Path) -> io::Result<Vec<u8>> {
    let (fd, expected) = open_tracked_regular_file(tree, relative, OFlags::RDONLY)?;
    let file = File::from(fd);
    let before = file_metadata(&file)?;
    if before.length > MAX_FIXTURE_CONTENT_BYTES as u64 {
        return Err(io::Error::other(
            "fixture file exceeds the fixed 64 KiB read limit",
        ));
    }
    let mut bytes = vec![0_u8; before.length as usize];
    let mut offset = 0;
    while offset < bytes.len() {
        let read = file.read_at(&mut bytes[offset..], offset as u64)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "fixture file changed length during tracked read",
            ));
        }
        offset += read;
    }
    let after = file_metadata(&file)?;
    ensure_equal(
        "tracked read expected identity",
        after.identity,
        expected.into(),
    )?;
    ensure_equal(
        "tracked read stable identity",
        after.identity,
        before.identity,
    )?;
    ensure_equal("tracked read stable length", after.length, before.length)?;
    Ok(bytes)
}

fn copy_tracked_file(tree: &ControlledTree, source: &Path, target: &Path) -> io::Result<()> {
    let bytes = read_tracked_file(tree, source)?;
    write_tracked_file(tree, target, &bytes)?;
    ensure_equal(
        "tracked per-file copy content",
        read_tracked_file(tree, target)?,
        bytes,
    )
}

fn read_pair(tree: &ControlledTree, directory: &str) -> io::Result<(Vec<u8>, Vec<u8>)> {
    Ok((
        read_tracked_file(tree, &Path::new(directory).join("a"))?,
        read_tracked_file(tree, &Path::new(directory).join("b"))?,
    ))
}

fn ensure_pair(
    label: &str,
    actual: &(Vec<u8>, Vec<u8>),
    expected_a: &[u8],
    expected_b: &[u8],
) -> io::Result<()> {
    ensure_equal(&format!("{label} a"), actual.0.as_slice(), expected_a)?;
    ensure_equal(&format!("{label} b"), actual.1.as_slice(), expected_b)
}

fn expect_exclusive_create_exists(
    tree: &ControlledTree,
    directory_relative: &Path,
    name: &Path,
) -> io::Result<i32> {
    tree.verify_roots()?;
    if directory_relative.components().count() != 1
        || !matches!(
            directory_relative.components().next(),
            Some(Component::Normal(_))
        )
        || name.components().count() != 1
        || !matches!(name.components().next(), Some(Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "exclusive-create check requires direct registered names",
        ));
    }
    let expected_directory = tree
        .tracked()
        .get(directory_relative)
        .copied()
        .ok_or_else(|| io::Error::other("exclusive-create directory is not registered"))?;
    if expected_directory.kind != TrackedKind::Directory {
        return Err(io::Error::other(
            "exclusive-create parent is not a registered directory",
        ));
    }
    let directory = rustix::fs::openat(
        tree.root_fd(),
        directory_relative,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(errno_error)?;
    ensure_equal(
        "exclusive-create directory identity",
        tracked_fd_identity(&directory)?,
        expected_directory,
    )?;
    match rustix::fs::openat(
        &directory,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_bits_retain(0o600),
    ) {
        Err(rustix::io::Errno::EXIST) => Ok(libc::EEXIST),
        Ok(_) => Err(io::Error::other(
            "exclusive create unexpectedly replaced or bypassed the marker",
        )),
        Err(error) => Err(errno_error(error)),
    }
}

fn observe_tracked_file(tree: &ControlledTree, relative: &Path) -> io::Result<FileObservation> {
    let (fd, expected) = open_tracked_regular_file(tree, relative, OFlags::RDONLY)?;
    let file = File::from(fd);
    let observation = observe_open_file(&file)?;
    ensure_equal(
        "tracked observation identity",
        observation.metadata.identity,
        expected.into(),
    )?;
    Ok(observation)
}

fn observe_open_file(file: &File) -> io::Result<FileObservation> {
    let before = file_metadata(file)?;
    if before.length > MAX_FIXTURE_CONTENT_BYTES as u64 {
        return Err(io::Error::other(
            "fixture file exceeds the fixed 64 KiB observation limit",
        ));
    }
    let mut bytes = vec![0_u8; before.length as usize];
    let mut offset = 0;
    while offset < bytes.len() {
        let read = file.read_at(&mut bytes[offset..], offset as u64)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "fixture file changed length during content observation",
            ));
        }
        offset += read;
    }
    let metadata = file_metadata(file)?;
    ensure_equal(
        "file identity during content observation",
        metadata.identity,
        before.identity,
    )?;
    ensure_equal(
        "file length during content observation",
        metadata.length,
        before.length,
    )?;
    Ok(FileObservation {
        metadata,
        content_digest_hex: digest_hex(&bytes),
    })
}

fn file_metadata(fd: impl AsFd) -> io::Result<FileMetadataObservation> {
    let metadata = rustix::fs::fstat(fd).map_err(errno_error)?;
    Ok(FileMetadataObservation {
        identity: FileIdentity {
            device: metadata.st_dev as u64,
            inode: metadata.st_ino,
        },
        length: u64::try_from(metadata.st_size)
            .map_err(|_| io::Error::other("fixture file reported a negative length"))?,
        mode: u32::from(metadata.st_mode & 0o7777),
        link_count: metadata.st_nlink as u64,
        access_time: TimestampValue {
            seconds: metadata.st_atime,
            nanoseconds: metadata.st_atime_nsec,
        },
        modified_time: TimestampValue {
            seconds: metadata.st_mtime,
            nanoseconds: metadata.st_mtime_nsec,
        },
        change_time: TimestampValue {
            seconds: metadata.st_ctime,
            nanoseconds: metadata.st_ctime_nsec,
        },
        birth_time: TimestampValue {
            seconds: metadata.st_birthtime,
            nanoseconds: metadata.st_birthtime_nsec,
        },
    })
}

fn restore_tracked_times(
    tree: &ControlledTree,
    relative: &Path,
    desired: &FileMetadataObservation,
) -> io::Result<FileMetadataObservation> {
    let (file, expected) = open_tracked_regular_file(tree, relative, OFlags::RDONLY)?;
    ensure_equal(
        "timestamp restore target identity",
        FileIdentity::from(expected),
        desired.identity,
    )?;
    rustix::fs::futimens(
        &file,
        &Timestamps {
            last_access: Timespec {
                tv_sec: desired.access_time.seconds,
                tv_nsec: desired.access_time.nanoseconds,
            },
            last_modification: Timespec {
                tv_sec: desired.modified_time.seconds,
                tv_nsec: desired.modified_time.nanoseconds,
            },
        },
    )
    .map_err(errno_error)?;
    file_metadata(&file)
}

fn verify_restored_times(
    label: &str,
    restored: &FileMetadataObservation,
    desired: &FileMetadataObservation,
) -> io::Result<()> {
    ensure_equal(
        &format!("{label} identity after timestamp restore"),
        restored.identity,
        desired.identity,
    )?;
    ensure_equal(
        &format!("{label} atime after timestamp restore"),
        restored.access_time,
        desired.access_time,
    )?;
    ensure_equal(
        &format!("{label} mtime after timestamp restore"),
        restored.modified_time,
        desired.modified_time,
    )
}

fn print_observation(api: LockApi, phase: &str, object: &str, value: &FileObservation) {
    eprintln!(
        "P0-05 L6 api={} phase={} object={} order=fstat-length-then-read-content-then-fstat dev={} ino={} digest={} len={} mode={:o} nlink={} atime={}.{:09} mtime={}.{:09} ctime={}.{:09} birthtime={}.{:09}",
        api.label(),
        phase,
        object,
        value.metadata.identity.device,
        value.metadata.identity.inode,
        value.content_digest_hex,
        value.metadata.length,
        value.metadata.mode,
        value.metadata.link_count,
        value.metadata.access_time.seconds,
        value.metadata.access_time.nanoseconds,
        value.metadata.modified_time.seconds,
        value.metadata.modified_time.nanoseconds,
        value.metadata.change_time.seconds,
        value.metadata.change_time.nanoseconds,
        value.metadata.birth_time.seconds,
        value.metadata.birth_time.nanoseconds
    );
}

fn print_metadata(api: LockApi, phase: &str, object: &str, value: &FileMetadataObservation) {
    eprintln!(
        "P0-05 L6 api={} phase={} object={} order=futimens-then-fstat-without-content-read dev={} ino={} len={} mode={:o} nlink={} atime={}.{:09} mtime={}.{:09} ctime={}.{:09} birthtime={}.{:09} ctime_restore_attempted=false cache_inference=none",
        api.label(),
        phase,
        object,
        value.identity.device,
        value.identity.inode,
        value.length,
        value.mode,
        value.link_count,
        value.access_time.seconds,
        value.access_time.nanoseconds,
        value.modified_time.seconds,
        value.modified_time.nanoseconds,
        value.change_time.seconds,
        value.change_time.nanoseconds,
        value.birth_time.seconds,
        value.birth_time.nanoseconds
    );
}

fn write_tracked_file(tree: &ControlledTree, relative: &Path, mut bytes: &[u8]) -> io::Result<()> {
    let (file, expected) = open_tracked_regular_file(tree, relative, OFlags::WRONLY)?;
    rustix::fs::ftruncate(&file, 0).map_err(errno_error)?;
    while !bytes.is_empty() {
        let written = rustix::io::write(&file, bytes).map_err(errno_error)?;
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "registered file write returned zero",
            ));
        }
        bytes = &bytes[written..];
    }
    if tracked_fd_identity(&file)? != expected {
        return Err(io::Error::other(
            "write target identity changed while held open",
        ));
    }
    Ok(())
}

fn open_tracked_regular_file(
    tree: &ControlledTree,
    relative: &Path,
    access: OFlags,
) -> io::Result<(OwnedFd, TrackedIdentity)> {
    tree.verify_roots()?;
    let expected = tree
        .tracked()
        .get(relative)
        .copied()
        .ok_or_else(|| io::Error::other("file target is not registered to this fixture"))?;
    if expected.kind != TrackedKind::RegularFile {
        return Err(io::Error::other(
            "registered file target is not a regular file",
        ));
    }
    let mut components = relative.components().peekable();
    let mut current = tree.root_fd().try_clone()?;
    let mut prefix = PathBuf::new();
    let file = loop {
        let Some(component) = components.next() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "registered write target is empty",
            ));
        };
        let Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "registered write target contains a non-normal component",
            ));
        };
        prefix.push(name);
        if components.peek().is_none() {
            break rustix::fs::openat(
                &current,
                name,
                access | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(errno_error)?;
        }
        let ancestor = tree
            .tracked()
            .get(&prefix)
            .copied()
            .ok_or_else(|| io::Error::other("write target ancestor is not registered"))?;
        if ancestor.kind != TrackedKind::Directory {
            return Err(io::Error::other(
                "registered write target ancestor is not a directory",
            ));
        }
        let next = rustix::fs::openat(
            &current,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(errno_error)?;
        if tracked_fd_identity(&next)? != ancestor {
            return Err(io::Error::other(
                "write target ancestor identity changed during no-follow traversal",
            ));
        }
        current = next;
    };

    if tracked_fd_identity(&file)? != expected {
        return Err(io::Error::other(
            "registered file identity changed during no-follow open",
        ));
    }
    Ok((file, expected))
}

fn statat_identity(directory: &OwnedFd, name: &Path) -> io::Result<TrackedIdentity> {
    support::tracked_identity(
        rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW).map_err(errno_error)?,
    )
}

fn assert_missing_at(directory: &OwnedFd, name: &Path) -> io::Result<()> {
    match rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Ok(_) => Err(io::Error::other(
            "renamed replacement name unexpectedly remains present",
        )),
        Err(error) => Err(errno_error(error)),
    }
}

fn open_known_file_at(
    directory: &OwnedFd,
    name: &Path,
    expected: TrackedIdentity,
) -> io::Result<File> {
    let file = rustix::fs::openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(errno_error)?;
    ensure_equal(
        "known file identity after no-follow open",
        tracked_fd_identity(&file)?,
        expected,
    )?;
    Ok(File::from(file))
}

fn tracked_fd_identity(fd: &OwnedFd) -> io::Result<TrackedIdentity> {
    support::tracked_identity(rustix::fs::fstat(fd).map_err(errno_error)?)
}

fn ensure_equal<T>(label: &str, actual: T, expected: T) -> io::Result<()>
where
    T: std::fmt::Debug + PartialEq,
{
    if actual != expected {
        return Err(io::Error::other(format!(
            "{label} mismatch: actual={actual:?}, expected={expected:?}"
        )));
    }
    Ok(())
}

fn wait_until(child: &mut Child, deadline: Instant) -> io::Result<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "lock child exceeded its observation deadline",
            ));
        }
        thread::sleep(Duration::from_millis(10).min(deadline.duration_since(now)));
    }
}

fn with_termination<T>(error: io::Error, termination: io::Result<T>) -> io::Error {
    match termination {
        Ok(_) => error,
        Err(termination_error) => io::Error::other(format!(
            "{error}; additionally failed to confirm child exit within 1 second: {termination_error}"
        )),
    }
}

fn errno_error(error: rustix::io::Errno) -> io::Error {
    io::Error::from_raw_os_error(error.raw_os_error())
}

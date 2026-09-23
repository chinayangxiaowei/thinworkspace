#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Phase 1 command parsing and human/JSON rendering.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use clap::{Parser, Subcommand, error::ErrorKind};
use serde_json::{Map, Value, json};
use thinws_application::{InitRequest, UseCases};

/// Renderer-facing successful init data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitView {
    /// Stable result enum.
    pub result: String,
    /// UUIDv7 installation identifier.
    pub instance_id: String,
    /// Lossless canonical data-root bytes.
    pub data_root: Vec<u8>,
    /// APFS volume UUID.
    pub volume_id: String,
}

/// Renderer-facing successful doctor data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoctorView {
    /// UUIDv7 installation identifier.
    pub instance_id: String,
    /// Lossless canonical data-root bytes.
    pub data_root: Vec<u8>,
    /// APFS volume UUID.
    pub volume_id: String,
    /// Number of active non-Ready Workspace records.
    pub incomplete_workspaces: usize,
}

/// Renderer-facing stable failure data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorView {
    /// Stable symbolic code.
    pub code: String,
    /// Human-readable implementation message.
    pub message: String,
    /// Stable string context fields.
    pub context: Vec<(String, String)>,
    /// Optional remediation.
    pub remediation: Option<String>,
}

/// CLI seam that contains no renderer or business policy.
pub trait Commands {
    /// Executes init from lossless path bytes and an injected wall clock.
    fn init(&self, data_root: Vec<u8>, now_ms: i64) -> Result<InitView, ErrorView>;

    /// Executes product-state diagnosis.
    fn doctor(&self) -> Result<DoctorView, ErrorView>;
}

/// Converts Application use cases into renderer-facing data.
pub struct ApplicationCommands<U> {
    use_cases: U,
}

impl<U> ApplicationCommands<U> {
    /// Wraps one Application use-case implementation.
    #[must_use]
    pub const fn new(use_cases: U) -> Self {
        Self { use_cases }
    }
}

impl<U: UseCases> Commands for ApplicationCommands<U> {
    fn init(&self, data_root: Vec<u8>, now_ms: i64) -> Result<InitView, ErrorView> {
        let request = InitRequest::try_from_raw(data_root, now_ms).map_err(use_case_error_view)?;
        let outcome = self.use_cases.init(request).map_err(use_case_error_view)?;
        let identity = outcome.installation().identity();
        Ok(InitView {
            result: outcome.result().as_str().to_owned(),
            instance_id: identity.instance_id().to_string(),
            data_root: identity.data_root().as_bytes().to_vec(),
            volume_id: identity.volume_id().to_string(),
        })
    }

    fn doctor(&self) -> Result<DoctorView, ErrorView> {
        let outcome = self.use_cases.doctor().map_err(use_case_error_view)?;
        let identity = outcome.installation().identity();
        Ok(DoctorView {
            instance_id: identity.instance_id().to_string(),
            data_root: identity.data_root().as_bytes().to_vec(),
            volume_id: identity.volume_id().to_string(),
            incomplete_workspaces: outcome.incomplete_workspaces(),
        })
    }
}

/// Parses one argv vector, executes a command, renders output and returns its exit status.
pub fn run<I, C, O, E>(args: I, commands: &C, now_ms: i64, stdout: &mut O, stderr: &mut E) -> i32
where
    I: IntoIterator<Item = OsString>,
    C: Commands,
    O: Write,
    E: Write,
{
    let args: Vec<OsString> = args.into_iter().collect();
    let prescan = prescan(&args);
    if prescan.json && prescan.help_or_version {
        let error = usage_error("--json cannot be combined with --help or --version");
        return render_error(&error, true, stdout, stderr);
    }

    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            if prescan.json {
                return render_error(
                    &usage_error("invalid command or arguments"),
                    true,
                    stdout,
                    stderr,
                );
            }
            let success = matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            );
            let target: &mut dyn Write = if success { stdout } else { stderr };
            if target.write_all(error.to_string().as_bytes()).is_err() {
                return 31;
            }
            return if success { 0 } else { 2 };
        }
    };

    let result = match cli.command {
        Command::Init { data_root } => commands
            .init(data_root.as_os_str().as_bytes().to_vec(), now_ms)
            .map(Success::Init),
        Command::Doctor => commands.doctor().map(Success::Doctor),
    };
    match result {
        Ok(success) => {
            let rendered = if cli.json {
                render_success_json(&success, stdout)
            } else {
                render_success_human(&success, stdout)
            };
            if rendered.is_ok() { 0 } else { 31 }
        }
        Err(error) => render_error(&error, cli.json, stdout, stderr),
    }
}

#[derive(Parser)]
#[command(
    name = "thinws",
    version,
    about = "Space-efficient local development workspaces"
)]
struct Cli {
    #[arg(long, global = true, help = "Emit one versioned JSON document")]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initializes one private ThinWorkspace data root.
    Init {
        /// Canonical absolute APFS directory used for ThinWorkspace data.
        #[arg(long)]
        data_root: PathBuf,
    },
    /// Validates installation and metadata state without repairing it.
    Doctor,
}

enum Success {
    Init(InitView),
    Doctor(DoctorView),
}

#[derive(Clone, Copy, Default)]
struct Prescan {
    json: bool,
    help_or_version: bool,
}

fn prescan(args: &[OsString]) -> Prescan {
    let mut result = Prescan::default();
    for argument in args.iter().skip(1) {
        if argument == OsStr::new("--") {
            break;
        }
        if argument == OsStr::new("--json") {
            result.json = true;
        }
        if matches!(
            argument.as_os_str().as_bytes(),
            b"--help" | b"-h" | b"--version" | b"-V"
        ) {
            result.help_or_version = true;
        }
    }
    result
}

fn render_success_human(success: &Success, output: &mut dyn Write) -> io::Result<()> {
    match success {
        Success::Init(view) => {
            writeln!(output, "ThinWorkspace initialized")?;
            writeln!(output, "Result:      {}", view.result)?;
            output.write_all(b"Data root:   ")?;
            output.write_all(&view.data_root)?;
            output.write_all(b"\n")?;
            writeln!(output, "Volume ID:   {}", view.volume_id)
        }
        Success::Doctor(view) => {
            writeln!(output, "ThinWorkspace doctor")?;
            writeln!(output, "Status:              ready")?;
            writeln!(
                output,
                "Host:                {}/{}",
                platform_name(),
                std::env::consts::ARCH
            )?;
            output.write_all(b"Data root:           ")?;
            output.write_all(&view.data_root)?;
            output.write_all(b"\n")?;
            writeln!(
                output,
                "Incomplete workspaces: {}",
                view.incomplete_workspaces
            )?;
            writeln!(output, "Git check:           unavailable (not implemented)")
        }
    }
}

fn render_success_json(success: &Success, output: &mut dyn Write) -> io::Result<()> {
    let data = match success {
        Success::Init(view) => json!({
            "command": "init",
            "result": view.result,
            "instance_id": view.instance_id,
            "data_root": String::from_utf8_lossy(&view.data_root),
            "data_root_hex": hex(&view.data_root),
            "volume_id": view.volume_id,
        }),
        Success::Doctor(view) => json!({
            "command": "doctor",
            "status": "ready",
            "host": {
                "platform": platform_name(),
                "architecture": std::env::consts::ARCH,
            },
            "instance_id": view.instance_id,
            "data_root": String::from_utf8_lossy(&view.data_root),
            "data_root_hex": hex(&view.data_root),
            "volume_id": view.volume_id,
            "incomplete_workspaces": view.incomplete_workspaces,
            "git_check": {
                "available": false,
                "reason": "not-implemented",
            },
        }),
    };
    serde_json::to_writer(
        &mut *output,
        &json!({
            "schema_version": 1,
            "ok": true,
            "data": data,
        }),
    )?;
    output.write_all(b"\n")
}

fn render_error(
    error: &ErrorView,
    json_mode: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let rendered = if json_mode {
        let context: Map<String, Value> = error
            .context
            .iter()
            .map(|(key, value)| (key.clone(), Value::String(value.clone())))
            .collect();
        let document = json!({
            "schema_version": 1,
            "ok": false,
            "error": {
                "code": error.code,
                "message": error.message,
                "context": context,
                "remediation": error.remediation,
            },
        });
        serde_json::to_writer(&mut *stdout, &document)
            .map_err(io::Error::other)
            .and_then(|()| stdout.write_all(b"\n"))
    } else {
        writeln!(stderr, "Error: {}", error.message)
            .and_then(|()| writeln!(stderr, "Code: {}", error.code))
    };
    if rendered.is_err() {
        31
    } else {
        exit_status(&error.code)
    }
}

fn usage_error(message: &str) -> ErrorView {
    ErrorView {
        code: "E_USAGE".to_owned(),
        message: message.to_owned(),
        context: Vec::new(),
        remediation: None,
    }
}

fn exit_status(code: &str) -> i32 {
    BTreeMap::from([
        ("E_USAGE", 2),
        ("E_NOT_INITIALIZED", 10),
        ("E_CAPABILITY_UNAVAILABLE", 11),
        ("E_COW_UNAVAILABLE", 12),
        ("E_NAME_CONFLICT", 15),
        ("E_DATA_ROOT_CHANGE_UNSUPPORTED", 16),
        ("E_WORKSPACE_NOT_FOUND", 20),
        ("E_WORKSPACE_NOT_READY", 21),
        ("E_WORKSPACE_DIRTY", 22),
        ("E_WORKSPACE_BUSY", 23),
        ("E_GIT_CHECK_INCOMPLETE", 25),
        ("E_GIT", 30),
        ("E_FILESYSTEM", 31),
        ("E_DATA_ROOT_UNAVAILABLE", 32),
        ("E_DATA_ROOT_LAYOUT", 33),
        ("E_METADATA", 35),
        ("E_DATA_ROOT_NOT_EMPTY", 36),
        ("E_WORKSPACE_INCOMPLETE", 40),
        ("E_LOCK_TIMEOUT", 41),
    ])
    .get(code)
    .copied()
    .unwrap_or(31)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

const fn platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else {
        std::env::consts::OS
    }
}

fn use_case_error_view(error: thinws_application::UseCaseError) -> ErrorView {
    let diagnostic = error.diagnostic();
    ErrorView {
        code: diagnostic.code().to_string(),
        message: diagnostic.message().to_owned(),
        context: diagnostic
            .context()
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.user_value().to_owned()))
            .collect(),
        remediation: diagnostic.remediation().map(str::to_owned),
    }
}

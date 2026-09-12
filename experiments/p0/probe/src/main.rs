#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use serde::Serialize;
use thinws_p0_probe::{
    EXPERIMENT_NAME, MaterializationPathProbeRequest, MaterializerCandidate, ProbeError,
    inspect_host, inspect_materialization_paths, inspect_path,
};

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    experiment: &'a str,
    outcome: &'a str,
    error: CliError,
}

#[derive(Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
enum CliError {
    Usage { message: String },
    Probe { error: ProbeError },
}

fn main() -> ExitCode {
    match run(std::env::args_os().skip(1).collect()) {
        Ok(value) => match serde_json::to_writer_pretty(std::io::stdout().lock(), &value) {
            Ok(()) => match writeln!(std::io::stdout().lock()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    let _ = writeln!(
                        std::io::stderr().lock(),
                        "thinws-p0-probe failed to finish JSON output: {error}"
                    );
                    ExitCode::from(1)
                }
            },
            Err(error) => {
                let _ = writeln!(
                    std::io::stderr().lock(),
                    "thinws-p0-probe failed to serialize JSON: {error}"
                );
                ExitCode::from(1)
            }
        },
        Err(error) => {
            let envelope = ErrorEnvelope {
                experiment: EXPERIMENT_NAME,
                outcome: "error",
                error,
            };
            let mut stderr = std::io::stderr().lock();
            if serde_json::to_writer_pretty(&mut stderr, &envelope).is_ok() {
                let _ = writeln!(stderr);
            }
            ExitCode::from(2)
        }
    }
}

fn run(arguments: Vec<OsString>) -> Result<serde_json::Value, CliError> {
    let Some(command) = arguments.first().and_then(|argument| argument.to_str()) else {
        return Err(usage());
    };
    match (command, &arguments[1..]) {
        ("host", []) => serde_json::to_value(inspect_host().map_err(probe_error)?)
            .map_err(|error| usage_message(format!("failed to serialize host report: {error}"))),
        ("path", [path]) => {
            serde_json::to_value(inspect_path(&PathBuf::from(path)).map_err(probe_error)?)
                .map_err(|error| usage_message(format!("failed to serialize path report: {error}")))
        }
        ("materialization", [source, target, staging, trash]) => {
            let source = PathBuf::from(source);
            let target = PathBuf::from(target);
            let staging = PathBuf::from(staging);
            let trash = PathBuf::from(trash);
            let candidates = [
                MaterializerCandidate::ApfsFileClone,
                MaterializerCandidate::FullCopy,
            ];
            serde_json::to_value(
                inspect_materialization_paths(&MaterializationPathProbeRequest {
                    source: &source,
                    target_root: &target,
                    staging: &staging,
                    trash: &trash,
                    candidates: &candidates,
                })
                .map_err(probe_error)?,
            )
            .map_err(|error| {
                usage_message(format!(
                    "failed to serialize materialization report: {error}"
                ))
            })
        }
        _ => Err(usage()),
    }
}

fn probe_error(error: ProbeError) -> CliError {
    CliError::Probe { error }
}

fn usage() -> CliError {
    usage_message(
        "usage: thinws-p0-probe host | thinws-p0-probe path <absolute-directory> | thinws-p0-probe materialization <source> <target> <staging> <trash>",
    )
}

fn usage_message(message: impl Into<String>) -> CliError {
    CliError::Usage {
        message: message.into(),
    }
}

use std::time::{SystemTime, UNIX_EPOCH};

use directories::BaseDirs;
use thinws_cli::{LocalCommands, run};

fn main() {
    let commands =
        LocalCommands::new(BaseDirs::new().map(|base| base.data_dir().join("ThinWorkspace")));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(-1);
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let status = run(
        std::env::args_os(),
        &commands,
        now_ms,
        &mut stdout,
        &mut stderr,
    );
    std::process::exit(status);
}

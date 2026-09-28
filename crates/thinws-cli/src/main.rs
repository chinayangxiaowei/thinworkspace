use std::time::{SystemTime, UNIX_EPOCH};

use directories::BaseDirs;
use thinws_cli::{LocalCommands, run};

fn unix_millis_or_invalid(now: SystemTime) -> i64 {
    now.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(-1)
}

fn main() {
    let commands = LocalCommands::new(BaseDirs::new().map(|base| base.home_dir().join(".thinws")));
    let now_ms = unix_millis_or_invalid(SystemTime::now());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn entry_clock_uses_an_invalid_sentinel_before_the_unix_epoch() {
        assert_eq!(
            unix_millis_or_invalid(UNIX_EPOCH - Duration::from_millis(1)),
            -1
        );
        assert_eq!(unix_millis_or_invalid(UNIX_EPOCH), 0);
        assert_eq!(
            unix_millis_or_invalid(UNIX_EPOCH + Duration::from_millis(1234)),
            1234
        );
    }
}

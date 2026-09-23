use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use directories::BaseDirs;
use thinws_adapter_macos::MacOsHostAdapter;
use thinws_application::ThinWorkspaceService;
use thinws_cli::{ApplicationCommands, Commands, DoctorView, ErrorView, InitView, run};
use thinws_metadata_sqlite::SqliteMetadataStoreFactory;

struct ProductionCommands {
    bootstrap_dir: Option<PathBuf>,
}

impl ProductionCommands {
    fn bootstrap_adapter(&self) -> Result<MacOsHostAdapter, ErrorView> {
        let path = self.bootstrap_dir.as_ref().ok_or_else(|| ErrorView {
            code: "E_FILESYSTEM".to_owned(),
            message: "the current user data directory is unavailable".to_owned(),
            context: Vec::new(),
            remediation: None,
        })?;
        MacOsHostAdapter::new(path).map_err(|_| ErrorView {
            code: "E_FILESYSTEM".to_owned(),
            message: "the ThinWorkspace bootstrap path is invalid".to_owned(),
            context: Vec::new(),
            remediation: None,
        })
    }
}

impl Commands for ProductionCommands {
    fn init(&self, data_root: Vec<u8>, now_ms: i64) -> Result<InitView, ErrorView> {
        let adapter = self.bootstrap_adapter()?;
        let service = ThinWorkspaceService::new(
            adapter,
            SqliteMetadataStoreFactory,
            Duration::from_secs(5),
            Duration::from_secs(5),
        );
        ApplicationCommands::new(service).init(data_root, now_ms)
    }

    fn doctor(&self) -> Result<DoctorView, ErrorView> {
        let adapter = self.bootstrap_adapter()?;
        let service = ThinWorkspaceService::new(
            adapter,
            SqliteMetadataStoreFactory,
            Duration::from_secs(5),
            Duration::from_secs(5),
        );
        ApplicationCommands::new(service).doctor()
    }
}

fn main() {
    let commands = ProductionCommands {
        bootstrap_dir: BaseDirs::new().map(|base| base.data_dir().join("ThinWorkspace")),
    };
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

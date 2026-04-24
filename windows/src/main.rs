use clap::Parser;
use tracing::{info, warn};

/// Beebeeb desktop sync daemon for Windows.
///
/// Runs as a background process managed by the WinUI shell.
/// Syncs the local vault folder with the Beebeeb server,
/// encrypting everything end-to-end before upload.
#[derive(Parser, Debug)]
#[command(name = "beebeeb-desktop-windows", version)]
struct Args {
    /// Start minimized to the system tray (no window).
    #[arg(long, default_value_t = false)]
    start_minimized: bool,

    /// Path to the local vault folder to sync.
    /// Defaults to %USERPROFILE%\Beebeeb.
    #[arg(long, env = "BEEBEEB_SYNC_PATH")]
    sync_path: Option<String>,

    /// Server endpoint override (for development).
    #[arg(long, env = "BEEBEEB_SERVER_URL")]
    server_url: Option<String>,

    /// Log level filter (e.g. "info", "debug", "beebeeb_sync=trace").
    #[arg(long, default_value = "info", env = "BEEBEEB_LOG")]
    log_level: String,
}

fn default_sync_path() -> String {
    std::env::var("USERPROFILE")
        .map(|home| format!(r"{}\Beebeeb", home))
        .unwrap_or_else(|_| r"C:\Beebeeb".to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(&args.log_level)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let sync_path = args.sync_path.unwrap_or_else(default_sync_path);

    info!(
        sync_path = %sync_path,
        start_minimized = args.start_minimized,
        "Beebeeb sync daemon starting"
    );

    // Ensure the sync folder exists.
    let path = std::path::Path::new(&sync_path);
    if !path.exists() {
        std::fs::create_dir_all(path)?;
        info!("Created vault folder at {}", sync_path);
    }

    // Build sync configuration.
    // TODO: Replace with real beebeeb_sync::SyncConfig once the core crate
    //       is available via the git dependency.
    //
    // let config = beebeeb_sync::SyncConfig {
    //     sync_path: sync_path.clone().into(),
    //     server_url: args.server_url.unwrap_or_else(|| "https://api.beebeeb.io".into()),
    //     debounce_ms: 100,
    //     ignore_patterns: vec![
    //         "Thumbs.db".into(),
    //         "desktop.ini".into(),
    //         "*.tmp".into(),
    //         "~$*".into(),
    //     ],
    // };
    //
    // let engine = beebeeb_sync::SyncEngine::new(config).await?;
    // engine.start().await?;

    info!("Sync engine running. Watching {}", sync_path);

    if !args.start_minimized {
        info!("Ready. The WinUI shell will connect over the named pipe.");
    }

    // Wait for Ctrl+C (or service stop signal) to shut down gracefully.
    tokio::signal::ctrl_c().await?;

    warn!("Received shutdown signal, stopping sync engine...");

    // TODO: engine.stop().await?;

    info!("Beebeeb sync daemon stopped cleanly.");
    Ok(())
}

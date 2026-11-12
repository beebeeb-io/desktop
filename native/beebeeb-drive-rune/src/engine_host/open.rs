//! The start path of the real engine: one handle per process, for the identity this binary was built as.
//!
//! [`open`] is everything between "the app decided to be the app" and "the window can ask the engine": the
//! state folder under the host's local-data folder, the account slot of the settings file, the handle, the
//! stored session read back (no network). The identity is asked for ONCE, here
//! (`shell::identity::engine_identity`), and handed to the engine; nothing after this names an app.
//!
//! Must not: run before the single-instance claim (a second launch must not open a second handle), run in a
//! tooling run, read `BB_API_BASE` (the API base is the caller's argument or the engine's built-in default).

use std::path::Path;
use std::sync::{Arc, mpsc};

use beebeeb_engine::app::{CredentialStores, Engine, EngineConfig};
use beebeeb_engine::host::{ChannelHost, HostMessage};
use beebeeb_engine::identity::EngineIdentity;
use beebeeb_engine::signin::gate::GateState;

/// The handle, whose it is, and what the stored session said at start.
pub struct Opened<C: CredentialStores> {
    pub engine: Arc<Engine<C>>,
    pub identity: &'static EngineIdentity,
    /// The gate after the stored session was read (`Engine::restore`).
    pub gate: GateState,
}

/// The API base of this run: `--api-base` in a dev build, else the engine's built-in default (production; a
/// unit-test build's is a closed loopback port). Never the environment. A trailing slash is dropped.
pub fn api_base(flag: Option<&str>, dev_build: bool) -> String {
    match flag.map(str::trim).filter(|url| dev_build && !url.is_empty()) {
        Some(url) => url.trim_end_matches('/').to_string(),
        None => beebeeb_engine::runner::default_api_base(),
    }
}

/// Open the engine for this binary's identity, with its local state under `local_data` (the OS's per-user
/// app-data folder; the engine adds the identity's own folder name) and `stores` for its credentials. `wake`
/// runs on an engine thread after each report: post a wake-up to the UI loop there and nothing else.
///
/// Refused, with nothing opened: on a second call with another folder, on macOS for the test build, when the
/// settings file cannot be read or written.
pub fn open<C: CredentialStores>(
    local_data: &Path,
    api_base: String,
    stores: C,
    wake: impl Fn() + Send + Sync + 'static,
) -> Result<(Opened<C>, mpsc::Receiver<HostMessage>), String> {
    let identity = crate::shell::identity::engine_identity();
    beebeeb_engine::state_paths::init_from_app_local_data_dir(identity, local_data)?;
    let state_dir = beebeeb_engine::state_paths::beebeeb_state_dir_for(identity)?;
    let account_id = beebeeb_engine::config::DesktopConfig::ensure_account_id_for(identity)?;
    let (host, messages) = ChannelHost::new(wake);
    let config = EngineConfig {
        account_id,
        state_dir,
        api_base,
    };
    let engine = Arc::new(Engine::open(identity, config, host, stores)?);
    let gate = engine.restore();
    Ok((Opened { engine, identity, gate }, messages))
}

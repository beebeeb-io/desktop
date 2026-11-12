//! The start path of the real engine: one handle per process, for the identity this binary was built as.
//!
//! [`open`] is everything between "the app decided to be the app" and "the window can ask the engine": the
//! state folder under the host's local-data folder, the account slot of the settings file, the handle, the
//! stored session read back (no network). The identity is asked for ONCE, here
//! (`shell::identity::engine_identity`), and handed to the engine; nothing after this names an app.
//!
//! [`api_base`] is the server of this run (decision 2462): the engine's built-in default, or `--api-base` in a dev
//! build or the test build, checked first ([`Refused`]: the app then does not start). The real app in a build that
//! ships never reads the flag.
//!
//! Must not: run before the single-instance claim (a second launch must not open a second handle), run in a
//! tooling run, read `BB_API_BASE` (the API base is the caller's argument or the engine's built-in default), hand
//! the engine a `--api-base` value that was not checked (an [`ApiBase`] is made in this file and nowhere else).

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::sync::{Arc, mpsc};

use beebeeb_engine::app::{CredentialStores, Engine, EngineConfig};
use beebeeb_engine::host::{ChannelHost, HostMessage};
use beebeeb_engine::identity::EngineIdentity;
use beebeeb_engine::signin::gate::GateState;
use url::{Host, Url};

use crate::shell::instance::API_BASE_FLAG;

/// The handle, whose it is, and what the stored session said at start.
pub struct Opened<C: CredentialStores> {
    pub engine: Arc<Engine<C>>,
    pub identity: &'static EngineIdentity,
    /// The gate after the stored session was read (`Engine::restore`).
    pub gate: GateState,
}

/// Whether this build reads `--api-base` (decision 2462): a dev build (`shell::instance::DEV_BUILD`), or the test
/// build in any profile. The real app in a build that ships never does, so a release cannot be pointed at another
/// server by a shortcut's arguments. `identity` is this binary's (`shell::identity::engine_identity`): the answer is
/// fixed when the binary is built, never read from the environment or the command line.
pub const fn reads_api_base_flag(dev_build: bool, identity: &EngineIdentity) -> bool {
    dev_build || !identity.is_real()
}

/// The server this run talks to: an origin (`scheme://host[:port]`) and whether `--api-base` chose it. Made only
/// here, by [`api_base`] (a checked flag) and [`ApiBase::engine_default`]: [`open`] takes nothing else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiBase {
    origin: String,
    from_flag: bool,
}

impl ApiBase {
    /// The engine's built-in default: production, except in a unit-test build (a closed loopback port).
    pub fn engine_default() -> Self {
        Self {
            origin: beebeeb_engine::runner::default_api_base(),
            from_flag: false,
        }
    }

    /// `https://host[:port]`, or `http://` to a loopback host: no path, no query, no slash at the end.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Whether `--api-base` chose it; else it is the engine's default.
    pub fn from_flag(&self) -> bool {
        self.from_flag
    }

    /// The one line the app prints when it opens the engine (stdout): the origin and where it came from. Nothing
    /// else: no token, no account, no path.
    pub fn start_line(&self) -> String {
        let from = if self.from_flag() { API_BASE_FLAG } else { "default" };
        format!("api base: {} ({from})", self.origin)
    }
}

/// Why a `--api-base` value was refused. The app then does not start (`shell::run`: one line on stderr, exit 2) and
/// never talks to the default in its place: whoever passed the flag meant another server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Nothing (or another flag) after `--api-base`.
    Missing,
    /// Not a URL the parser reads.
    NotAUrl,
    /// Neither `https://` nor `http://`.
    Scheme,
    /// Plain `http://` to a host other than `127.0.0.1`, `[::1]` or `localhost`.
    PlainHttp,
    /// An `@`: a user name or password, which also hides the real host (`http://127.0.0.1@elsewhere` is elsewhere).
    UserInfo,
    /// No host (`https://`, `http://:9`).
    NoHost,
    /// A path other than `/`.
    Path,
    /// A query (`?`), even an empty one.
    Query,
    /// A fragment (`#`), even an empty one.
    Fragment,
}

impl fmt::Display for Refused {
    /// The reason, never the value: it may hold a password.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let why = match self {
            Self::Missing => "it needs a URL after it",
            Self::NotAUrl => "it is not a URL",
            Self::Scheme => "it must start with https:// (or http:// to 127.0.0.1, [::1] or localhost)",
            Self::PlainHttp => "plain http:// is allowed only to 127.0.0.1, [::1] or localhost",
            Self::UserInfo => "it holds a user name or password (an @)",
            Self::NoHost => "it names no host",
            Self::Path => "it has a path; give only the scheme, the host and the port",
            Self::Query => "it has a query (?)",
            Self::Fragment => "it has a fragment (#)",
        };
        write!(f, "--api-base was refused: {why}. The app did not start.")
    }
}

/// The API base of this run: `flag` when `reads_flag` ([`reads_api_base_flag`], the same answer the parse was given)
/// and it is an origin the app may talk to ([`Refused`] says why not), else the engine's built-in default. A flag the
/// build does not read is ignored whatever it holds; a flag it reads is never replaced by the default.
pub fn api_base(flag: Option<&str>, reads_flag: bool) -> Result<ApiBase, Refused> {
    match flag.filter(|_| reads_flag) {
        None => Ok(ApiBase::engine_default()),
        Some(text) => checked_origin(text).map(|origin| ApiBase {
            origin,
            from_flag: true,
        }),
    }
}

/// `text` as the origin the engine is handed, or why not. Read by the URL parser reqwest connects with, so what is
/// checked is what is used, and the engine gets the parser's own serialization of the origin (lower case, no
/// default port, no slash at the end), never `text` itself.
fn checked_origin(text: &str) -> Result<String, Refused> {
    if text.trim().is_empty() {
        return Err(Refused::Missing);
    }
    let url = Url::parse(text).map_err(|error| match error {
        url::ParseError::EmptyHost => Refused::NoHost,
        _ => Refused::NotAUrl,
    })?;
    let plain_http = match url.scheme() {
        "https" => false,
        "http" => true,
        _ => return Err(Refused::Scheme),
    };
    let Some(host) = url.host() else {
        return Err(Refused::NoHost);
    };
    if url.path() != "/" {
        return Err(Refused::Path);
    }
    if url.query().is_some() {
        return Err(Refused::Query);
    }
    if url.fragment().is_some() {
        return Err(Refused::Fragment);
    }
    // With no path, query or fragment left, an `@` can only be user info: every user name or password the parser reads
    // comes with one, and so does the empty `user@` it drops without a word (`http://@127.0.0.1`).
    if text.contains('@') {
        return Err(Refused::UserInfo);
    }
    if plain_http && !is_loopback(&host) {
        return Err(Refused::PlainHttp);
    }
    Ok(url.origin().ascii_serialization())
}

/// `127.0.0.1`, `::1` or `localhost`, as the parser reads the host, and nothing else: not the rest of `127.0.0.0/8`,
/// not `0.0.0.0`, not another name that resolves to this computer.
fn is_loopback(host: &Host<&str>) -> bool {
    match host {
        Host::Ipv4(ip) => *ip == Ipv4Addr::LOCALHOST,
        Host::Ipv6(ip) => *ip == Ipv6Addr::LOCALHOST,
        Host::Domain(name) => *name == "localhost",
    }
}

/// Open the engine for this binary's identity, with its local state under `local_data` (the OS's per-user
/// app-data folder; the engine adds the identity's own folder name), `api_base` as its server and `stores` for its
/// credentials. `wake` runs on an engine thread after each report: post a wake-up to the UI loop there and nothing
/// else.
///
/// Refused, with nothing opened: on a second call with another folder, on macOS for the test build, when the
/// settings file cannot be read or written.
pub fn open<C: CredentialStores>(
    local_data: &Path,
    api_base: &ApiBase,
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
        api_base: api_base.origin().to_owned(),
    };
    let engine = Arc::new(Engine::open(identity, config, host, stores)?);
    let gate = engine.restore();
    Ok((Opened { engine, identity, gate }, messages))
}

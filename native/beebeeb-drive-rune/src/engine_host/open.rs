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
//! [`bind_to_server`] keeps the test build's local data with the server it was made for (the lead's ruling inside
//! decision 2462): the first open records that server in [`SERVER_FILE`], in the identity's state folder, and a later
//! open for another server is refused ([`OtherServer`]) before the engine reads a stored session or `state.db`. Stored
//! credentials with no local data beside them are refused for every server ([`CredentialsWithoutData`]).
//!
//! Must not: run before the single-instance claim (a second launch must not open a second handle), run in a
//! tooling run, read `BB_API_BASE` (the API base is the caller's argument or the engine's built-in default), hand
//! the engine a `--api-base` value that was not checked (an [`ApiBase`] is made in this file and nowhere else), let
//! the engine read a test build's stored session or `state.db` for a server they were not made for.

use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::sync::{Arc, mpsc};

use beebeeb_engine::app::{CredentialStores, Engine, EngineConfig};
use beebeeb_engine::host::{ChannelHost, HostMessage};
use beebeeb_engine::identity::EngineIdentity;
use beebeeb_engine::keychain::{AuthSecretStore, AuthStoreError};
use beebeeb_engine::signin::gate::GateState;
use url::{Host, Url};

use crate::shell::instance::{API_BASE_FLAG, API_BASE_GLUED};

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

    /// Which server this is for the binding of local data ([`bind_to_server`]): the default when its origin is the
    /// engine's default, however it was given, else a custom one.
    pub fn server(&self) -> Server {
        Server::of(&self.origin)
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
    /// Written `--api-base=<url>`: the flag takes its URL as the next argument, and no other form is read, so the run
    /// never falls back to the default because of the spelling.
    Glued,
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
            Self::Glued => "write it as --api-base <url>, with a space, not with =",
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
    // The parse hands over a whole `--api-base=<url>` argument as it was written (`shell::instance::parse_args`).
    if text.starts_with(API_BASE_GLUED) {
        return Err(Refused::Glued);
    }
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

/// The file in the test build's state folder that names the server its local data was made for: `default`, or the
/// origin `--api-base` gave. The first open writes it, before the engine makes anything in that folder; it lives and
/// goes with `state.db`, beside it.
pub const SERVER_FILE: &str = "server-origin";
/// Where [`SERVER_FILE`] is written before it is renamed into place, so a crash never leaves half a record.
const SERVER_FILE_PARTIAL: &str = "server-origin.partial";

/// A server, as the binding of a test build's local data names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Server {
    /// The engine's built-in default. Recorded by that word, not by its origin, so the record keeps naming the
    /// default if the default's address ever changes.
    Default,
    /// Any other origin (an [`ApiBase::origin`]).
    Custom(String),
}

impl Server {
    fn of(origin: &str) -> Self {
        if origin == beebeeb_engine::runner::default_api_base() {
            Self::Default
        } else {
            Self::Custom(origin.to_owned())
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Default => "the default server",
            Self::Custom(_) => "a custom server (--api-base)",
        }
    }
}

/// The test build's local data on this computer was made for another server than this run's: nothing was opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OtherServer {
    /// The server the data was made for; `None` when its record cannot be read.
    pub made_for: Option<Server>,
    /// This run's server.
    pub asked: Server,
}

impl fmt::Display for OtherServer {
    /// The two servers' kinds and nothing else: never an origin, a token or an account.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let made = self
            .made_for
            .as_ref()
            .map_or("a server its record no longer names", Server::kind);
        let asked = match (&self.made_for, &self.asked) {
            (Some(Server::Custom(_)), Server::Custom(_)) => "a different custom server (--api-base)",
            (_, asked) => asked.kind(),
        };
        write!(
            f,
            "this test build's data on this computer was made for {made}, and this run asks for {asked}. The app did \
             not start."
        )
    }
}

/// The test build's account slot holds stored credentials, and its state folder holds no data and no record: the
/// record went with the folder and the credentials stayed (the folder removed by hand, or the uninstaller's "Delete the
/// application data", which keeps the credentials and the settings file). Nothing says which server they were made
/// for, so no server gets them, the default included: nothing was opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CredentialsWithoutData;

impl fmt::Display for CredentialsWithoutData {
    /// The state and nothing else: never a token, an account or an origin.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "this test build has stored credentials without local data on this computer: nothing says which server \
             they were made for, so no server gets them. The app did not start.",
        )
    }
}

/// Why [`open`] opened nothing.
#[derive(Debug)]
pub enum OpenError {
    /// The test build's local data belongs to another server (`shell::run`: stderr and the start-refusal box, exit 2).
    OtherServer(OtherServer),
    /// The test build's stored credentials have no local data beside them (`shell::run`: as [`Self::OtherServer`]).
    CredentialsWithoutData(CredentialsWithoutData),
    /// Anything else: the folders, the settings file, the engine (`shell::run`: the start-refusal box, exit 1).
    Failed(String),
}

impl From<String> for OpenError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

/// How many of the account's credential entries are stored (a session token, a vault key, an account email): what
/// the engine's restore reads, asked the engine's own way (`AuthSecretStore::holds_*`), counted and never read out. An
/// entry the store cannot answer for counts as stored: the rule of the engine's `keychain::vault::presence`, which a
/// Windows build does not compile, so it is written out here (fail closed).
pub fn stored_account_entries<S: AuthSecretStore>(store: &S) -> usize {
    [
        store.holds_session_token(),
        store.holds_wrapped_master_key(),
        store.holds_account_email(),
    ]
    .into_iter()
    .filter(|answer| match answer {
        Ok(present) => *present,
        Err(AuthStoreError::Unsupported(_) | AuthStoreError::NotFound) => false,
        Err(_) => true,
    })
    .count()
}

/// What [`SERVER_FILE`] says.
enum Record {
    Missing,
    Names(Server),
    Unreadable,
}

fn could_not(what: &str) -> OpenError {
    OpenError::Failed(format!("This test build could not {what}."))
}

fn read_record(state_dir: &Path) -> Result<Record, OpenError> {
    match fs::read_to_string(state_dir.join(SERVER_FILE)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Record::Missing),
        Err(_) => Err(could_not("read which server its data on this computer was made for")),
        Ok(text) => Ok(match text.trim() {
            "default" => Record::Names(Server::Default),
            origin if checked_origin(origin).as_deref() == Ok(origin) => Record::Names(Server::of(origin)),
            _ => Record::Unreadable,
        }),
    }
}

/// Whether the state folder holds anything besides the record: the engine's database, its lock, a device identity, a
/// kept copy. A folder that is not there holds nothing.
fn holds_local_state(state_dir: &Path) -> Result<bool, OpenError> {
    let entries = match fs::read_dir(state_dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(could_not("look into its data folder")),
        Ok(entries) => entries,
    };
    for entry in entries {
        let name = entry.map_err(|_| could_not("look into its data folder"))?.file_name();
        if name != SERVER_FILE && name != SERVER_FILE_PARTIAL {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Record `server` as the one this data is made for: written beside, synced, then renamed into place.
fn write_record(state_dir: &Path, server: &Server) -> Result<(), OpenError> {
    let text = match server {
        Server::Default => "default",
        Server::Custom(origin) => origin.as_str(),
    };
    let fail = |_: io::Error| could_not("record which server its data on this computer is for");
    fs::create_dir_all(state_dir).map_err(fail)?;
    let partial = state_dir.join(SERVER_FILE_PARTIAL);
    let mut file = fs::File::create(&partial).map_err(fail)?;
    writeln!(file, "{text}").map_err(fail)?;
    file.sync_all().map_err(fail)?;
    drop(file);
    fs::rename(&partial, state_dir.join(SERVER_FILE)).map_err(fail)
}

/// The test build's local data belongs to the server it was made for (the lead's rulings inside decision 2462).
/// `state_dir` is the identity's state folder, `account_entries` [`stored_account_entries`] for its account slot.
///
/// - A record that names this run's server opens.
/// - A record with nothing beside it (no other entry in the folder, no credential entry) is replaced by this run's
///   server: nothing was made for the server it names.
/// - Any other record that names another server, or that cannot be read, is refused ([`OtherServer`]).
/// - No record, and credential entries with nothing in the folder: refused for every server
///   ([`CredentialsWithoutData`]).
/// - No record, but files in the folder: data from before the record existed, the default's. A custom server is
///   refused, the default opens it.
/// - Nothing at all: a first open, for any server.
///
/// Whenever it opens on anything but a record of this run's server, this run's server is recorded first.
pub fn bind_to_server(state_dir: &Path, account_entries: usize, asked: &ApiBase) -> Result<(), OpenError> {
    let asked = asked.server();
    let record = read_record(state_dir)?;
    if matches!(&record, Record::Names(made_for) if *made_for == asked) {
        return Ok(());
    }
    let files = holds_local_state(state_dir)?;
    let credentials = account_entries > 0;
    match record {
        Record::Names(_) | Record::Unreadable if !files && !credentials => write_record(state_dir, &asked),
        Record::Names(made_for) => Err(OpenError::OtherServer(OtherServer {
            made_for: Some(made_for),
            asked,
        })),
        Record::Unreadable => Err(OpenError::OtherServer(OtherServer { made_for: None, asked })),
        Record::Missing if credentials && !files => Err(OpenError::CredentialsWithoutData(CredentialsWithoutData)),
        Record::Missing if files && asked != Server::Default => Err(OpenError::OtherServer(OtherServer {
            made_for: Some(Server::Default),
            asked,
        })),
        Record::Missing => write_record(state_dir, &asked),
    }
}

/// Open the engine for this binary's identity, with its local state under `local_data` (the OS's per-user
/// app-data folder; the engine adds the identity's own folder name), `api_base` as its server and `stores` for its
/// credentials. `wake` runs on an engine thread after each report: post a wake-up to the UI loop there and nothing
/// else.
///
/// Refused, with nothing opened: on a second call with another folder, on macOS for the test build, when the
/// settings file cannot be read or written, and for the test build when its local data was made for another server or
/// its stored credentials have no local data beside them ([`bind_to_server`]).
pub fn open<C: CredentialStores>(
    local_data: &Path,
    api_base: &ApiBase,
    stores: C,
    wake: impl Fn() + Send + Sync + 'static,
) -> Result<(Opened<C>, mpsc::Receiver<HostMessage>), OpenError> {
    let identity = crate::shell::identity::engine_identity();
    beebeeb_engine::state_paths::init_from_app_local_data_dir(identity, local_data)?;
    let state_dir = beebeeb_engine::state_paths::beebeeb_state_dir_for(identity)?;
    let account_id = beebeeb_engine::config::DesktopConfig::ensure_account_id_for(identity)?;
    // Before the engine reads a stored session or `state.db`. The real app never talks to another server, so its data
    // gets no record.
    if !identity.is_real() {
        let entries = stored_account_entries(&stores.for_account(identity, &account_id));
        bind_to_server(&state_dir, entries, api_base)?;
    }
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

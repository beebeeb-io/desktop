//! The real engine behind the native shell, in one test process: `cargo test` builds the TEST identity
//! (`build.rs`), the engine's `test-support` puts the settings file in a sandbox next to the test binary and
//! makes a closed loopback port the default API, and the credential stores are in memory. Nothing here touches
//! the OS credential store, the person's folders, a registration or the network. Every name is invented.

use std::path::PathBuf;
use std::sync::OnceLock;

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::{EngineIdentity, REAL, TEST};
use beebeeb_engine::keychain::{AuthSecretStore, SessionToken};
use beebeeb_engine::signin::gate::GateState as EngineGate;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::sync_root_owner::{self, Registration};
use beebeeb_engine::{signin::gate, state_paths};

use super::backups::may_back_up;
use super::open::{self, ApiBase, Opened, Refused};
use super::*;
use crate::shell::app::GateState as Shell;
use crate::shell::identity::engine_identity;
use crate::shell::instance;

pub(super) struct Started {
    pub(super) opened: Opened<MemoryCredentials>,
    live: LiveEngine<MemoryCredentials>,
    /// The "local app data" folder the host passed.
    pub(super) base: PathBuf,
}

/// The start path, once per process (the state folder of an identity is set once): a stored session token
/// for the settings file's account slot, beside a state folder a run before the server record left (one file in
/// it), then `open::open` as `shell::run` calls it, which records the default (`binding_tests`). Without the file
/// it would be stored credentials without local data, which no server gets.
pub(super) fn started() -> &'static Started {
    static STARTED: OnceLock<Started> = OnceLock::new();
    STARTED.get_or_init(|| {
        let base = tempfile::tempdir().expect("a temp home").keep();
        let state_dir = engine_identity().state_dir_in(&base);
        std::fs::create_dir_all(&state_dir).expect("the state folder");
        std::fs::write(state_dir.join("from-an-earlier-run"), b"").expect("a file of an earlier run");
        let stores = MemoryCredentials::default();
        let slot = DesktopConfig::ensure_account_id_for(engine_identity()).expect("the sandboxed settings file");
        stores
            .store_for(&slot)
            .save_session_token(&SessionToken::new("an-invented-token").unwrap())
            .unwrap();
        let (opened, messages) =
            open::open(&base, &ApiBase::engine_default(), stores, || {}).expect("the engine opens");
        let live = LiveEngine::new(&opened, messages);
        Started { opened, live, base }
    })
}

/// The process's state folder, which the start path makes once (`started`): every other engine handle a test
/// opens is on it (the engine refuses another). A test that opens one must not write in it.
pub(crate) fn process_state_dir() -> PathBuf {
    started();
    state_paths::beebeeb_state_dir_for(engine_identity()).expect("the process's state folder")
}

fn put(db: &StateDb, id: &str, path: &str, parent: Option<&str>, folder: bool) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status: FileStatus::CloudOnly,
        size_bytes: if folder { 0 } else { 1_200 },
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: parent.map(str::to_string),
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.parent_id = entry.parent_id.clone();
        contract.item_kind = entry.item_kind.clone();
    })
    .unwrap();
}

/// The native start path on the test build: the handle opens for THIS binary's identity under the host's
/// folder, the stored session is read (signed in, no key on this computer: the recovery phrase), the shell's
/// session says the same, and a folder of the local state is served through the engine host with its crumbs.
#[test]
fn the_start_path_opens_the_test_build_reads_the_session_and_serves_a_listing() {
    let started = started();
    let identity = started.opened.identity;
    assert!(std::ptr::eq(identity, &TEST), "`cargo test` is the test build");
    assert!(std::ptr::eq(identity, engine_identity()));

    // The state folder: the test build's own, under the folder the host passed.
    let state_dir = state_paths::beebeeb_state_dir_for(identity).unwrap();
    assert_eq!(state_dir, TEST.state_dir_in(&started.base));
    assert!(state_dir.starts_with(&started.base) && state_dir != started.base);

    // The stored session was read at start, without a network: signed in, the vault key is not here.
    assert_eq!(started.opened.gate, EngineGate::NeedsRecoveryPhrase);
    let session = started.live.session();
    assert_eq!(session.gate, Shell::NeedsRecoveryPhrase);
    assert!(
        session.account.is_some(),
        "signed in: the session names an account slot"
    );
    let signin = EngineSignInSource::new(started.opened.engine.clone());
    assert_eq!(signin.gate(), GateState::NeedsRecoveryPhrase);
    assert!(!started.opened.engine.is_syncing());
    assert_eq!(
        started.live.start_sync(),
        Err(actions::ActionError::NotAvailable("not signed in and unlocked")),
        "no engine is asked to start without keys"
    );

    // Before the first sync there is no local state: an error, never an empty folder.
    let drive = EngineDriveSource::new(started.opened.engine.clone());
    assert_eq!(drive.list(None).err(), Some(SourceError::Unreadable));

    // The local state the engine would have written, with the engine's own writers.
    std::fs::create_dir_all(&state_dir).unwrap(); // the engine's first start makes it
    let db = StateDb::open(&state_paths::state_db_path_from_state_dir(&state_dir)).unwrap();
    put(&db, "d-plans", "/Plans", None, true);
    put(&db, "d-2027", "/Plans/2027", Some("d-plans"), true);
    put(&db, "f-1", "/Plans/2027/budget.ods", Some("d-2027"), false);
    put(&db, "f-2", "/notes.txt", None, false);
    drop(db);

    // The session is signed in and NOT unlocked: the local state is not handed out (the account-bound handle,
    // task 2488), whatever the database holds. Rows without a recorded owner are nobody's to show.
    assert_eq!(
        started.opened.engine.local_state_refusal(),
        Some(beebeeb_engine::app::LocalStateRefused::Locked)
    );
    assert_eq!(drive.list(None).err(), Some(SourceError::Unreadable));
    assert_eq!(drive.list(Some("d-2027")).err(), Some(SourceError::Unreadable));

    // The window on this engine: Files reads the handle (nothing while locked), Backups shows no sample devices.
    // A signed-in, unlocked account's listing with its crumbs is `shell::host::tests::owned_state`'s.
    let state =
        crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), started.opened.engine.clone());
    assert!(state.listing.items.is_empty());
    assert_eq!(state.list_error, Some(SourceError::Unreadable));
    assert!(state.selected.is_none());
    assert!(
        !state.backups.has_source(),
        "Backups is 'not built yet' on a real engine"
    );
    // Shared, Photos: unwired on a real engine, never sample data. Trash reads the engine
    // (`engine_host::trash`): this handle has no session in memory, so it draws the signed-out state, no row.
    // This PC reads the engine (`engine_host::this_pc`) but is held off until its chrome is honest.
    assert!(
        !state.frames.shared.source.wired(),
        "Shared is unwired on a real engine"
    );
    assert_eq!(
        (
            state.frames.trash.source.items().len(),
            state.frames.trash.source.loaded()
        ),
        (0, crate::frames::trash::source::Loaded::SignedOut),
        "Trash on a real engine without a session: signed out, no row"
    );
    assert!(!state.photos.source.wired(), "Photos is unwired on a real engine");
    assert_eq!(
        state.this_pc.wired(),
        crate::engine_host::this_pc::TURNED_ON,
        "This PC: held off until its chrome is honest"
    );
    assert_eq!(state.this_pc.changes().len() + state.this_pc.kept_count(), 0);
    assert_eq!(started.live.drain(), 0, "nothing reported, nothing queued");
}

/// The test build stays out of the real app's places: its state folder, its settings, its sync folder.
#[test]
fn the_test_build_refuses_the_real_apps_state_folder_and_sync_folder() {
    let started = started();
    let identity = engine_identity();
    assert!(!identity.is_real());

    // The real app's state folder under the same host folder: no handle of this build opens on it, and this
    // process cannot even resolve it.
    let reals = REAL.state_dir_in(&started.base);
    assert_ne!(reals, state_paths::beebeeb_state_dir_for(identity).unwrap());
    let open_on = |state_dir: PathBuf| {
        let (host, _messages) = ChannelHost::new(|| {});
        let config = EngineConfig {
            account_id: "slot".into(),
            state_dir,
            api_base: ApiBase::engine_default().origin().to_owned(),
        };
        Engine::open(identity, config, host, MemoryCredentials::default()).map(|_| ())
    };
    let refused = open_on(reals).expect_err("the real app's state folder");
    assert!(refused.contains("state dir"), "{refused}");
    assert!(state_paths::beebeeb_state_dir_for(&REAL).is_err());
    assert_eq!(
        open_on(TEST.state_dir_in(&started.base)),
        Ok(()),
        "its own opens (the check can pass)"
    );
    // A second start with another host folder is refused: one state folder per process.
    let elsewhere = tempfile::tempdir().unwrap();
    assert!(
        open::open(
            elsewhere.path(),
            &ApiBase::engine_default(),
            MemoryCredentials::default(),
            || {}
        )
        .is_err()
    );

    // Where it syncs: its own folder, not the real app's; and a folder Windows has registered to the real
    // app's provider is refused by the engine's owner check, as is one whose registration cannot be read.
    let root = started.live.sync_root().unwrap();
    assert_eq!(root.file_name().unwrap(), TEST.sync_folder_name());
    assert_ne!(root, beebeeb_engine::config::default_sync_root_suggestion(&REAL));
    let reals_registration = Registration::Provider(REAL.cloud_files_provider_name().to_string());
    assert!(sync_root_owner::refusal(identity, &reals_registration, &root).is_some());
    assert!(sync_root_owner::refusal(identity, &Registration::Unreadable, &root).is_some());
    assert_eq!(sync_root_owner::refusal(identity, &Registration::Nobody, &root), None);

    // Its settings file is its own.
    let settings = DesktopConfig::path_for(identity).unwrap();
    assert_ne!(settings, DesktopConfig::path_for(&REAL).unwrap());
    assert!(
        settings
            .components()
            .any(|part| part.as_os_str() == TEST.config_dir_name())
    );
}

/// The sync folder is never a backup folder, and whose sync folder that is comes from the identity: the test
/// build refuses "Beebeeb Test" and may back up a folder that merely has the real app's folder name in its path
/// (review of #309, F: the fixture compares with the real name for both builds).
#[test]
fn the_sync_folder_is_not_a_backup_folder_for_the_identity_that_owns_it() {
    let p = |text: &str| PathBuf::from(text);
    for (identity, own, others) in [(&TEST, "Beebeeb Test", "Beebeeb"), (&REAL, "Beebeeb", "Beebeeb Test")] {
        assert_eq!(identity.sync_folder_name(), own);
        assert!(!may_back_up(identity, None, &p(&format!("/data/a/{own}"))), "{own}");
        assert!(!may_back_up(identity, None, &p(&format!("/data/a/{own}/Photos"))));
        assert!(!may_back_up(
            identity,
            None,
            &p(&format!("/data/a/{}", own.to_uppercase()))
        ));
        assert!(
            may_back_up(identity, None, &p(&format!("/data/a/{others}/Photos"))),
            "{others}"
        );
        assert!(may_back_up(identity, None, &p("/data/a/Documents")));
        // A sync folder the person chose elsewhere, under any name.
        let chosen = p("/data/Cloud");
        assert!(!may_back_up(identity, Some(&chosen), &p("/data/Cloud")));
        assert!(!may_back_up(identity, Some(&chosen), &p("/data/cloud/Work")));
        assert!(may_back_up(identity, Some(&chosen), &p("/data/Cloudy")));
        assert!(may_back_up(identity, Some(&chosen), &p("/data")));
    }
}

/// Source text without any whitespace: what the pins below compare, so a reflow by rustfmt is not a red.
pub(super) fn squeezed(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Decision 2462, the rule, from the command line to the server: the test build takes `--api-base` in any profile,
/// a dev build takes it, and the real app in a build that ships ignores it whatever it holds (not even checked).
#[test]
fn the_test_build_and_a_dev_build_take_api_base_and_the_real_app_ignores_it() {
    let line = ["--api-base", "http://127.0.0.1:3001/"];
    let run = |dev: bool, identity: &'static EngineIdentity| {
        let reads = open::reads_api_base_flag(dev, identity);
        let launch = instance::parse_args(line.map(String::from), dev, reads);
        open::api_base(launch.api_base.as_deref(), reads)
    };
    for (dev, identity, who) in [
        (false, &TEST, "the test build, shipped profile"),
        (true, &TEST, "the test build, dev profile"),
        (true, &REAL, "a dev build of the real app"),
    ] {
        let base = run(dev, identity).expect(who);
        assert_eq!(
            (base.origin(), base.from_flag()),
            ("http://127.0.0.1:3001", true),
            "{who}"
        );
        assert_eq!(
            base.start_line(),
            "api base: http://127.0.0.1:3001 (--api-base)",
            "{who}"
        );
    }
    // The real app in a build that ships: the default, from the parse on, whatever the flag holds.
    assert_eq!(run(false, &REAL), Ok(ApiBase::engine_default()));
    let real = open::reads_api_base_flag(false, &REAL);
    assert_eq!(
        open::api_base(Some("http://127.0.0.1:3001"), real),
        Ok(ApiBase::engine_default())
    );
    assert_eq!(
        open::api_base(Some("ftp://someone@elsewhere"), real),
        Ok(ApiBase::engine_default()),
        "not even checked"
    );
    // The rule itself, and this binary (`cargo test` is the test build).
    assert!(open::reads_api_base_flag(false, &TEST) && open::reads_api_base_flag(true, &TEST));
    assert!(open::reads_api_base_flag(true, &REAL) && !open::reads_api_base_flag(false, &REAL));
    assert!(open::reads_api_base_flag(false, engine_identity()));
}

/// No flag, any build: the engine's built-in default, said as "(default)". Outside a unit-test build that default is
/// production, so the owner's own test of the test build talks to it as it is, and no build moves to another server
/// unless `--api-base` says so.
#[test]
fn without_the_flag_every_build_talks_to_the_engines_default_which_is_production() {
    let default = beebeeb_engine::runner::default_api_base();
    assert_eq!(
        default,
        beebeeb_engine::runner::TEST_API_BASE,
        "a unit-test build's default is a closed port"
    );
    for (dev, identity) in [(false, &TEST), (true, &TEST), (true, &REAL), (false, &REAL)] {
        let base = open::api_base(None, open::reads_api_base_flag(dev, identity)).expect("no flag is never refused");
        assert_eq!(base, ApiBase::engine_default());
        assert_eq!((base.origin(), base.from_flag()), (default.as_str(), false));
        assert_eq!(base.start_line(), format!("api base: {default} (default)"));
    }
    // A unit-test build cannot reach the production default (the engine's `test-support` replaces it), so the source
    // says it: the engine's default outside a unit-test build is production, and the native default is the engine's,
    // for the test build too.
    let runner = squeezed(include_str!("../../../../crates/beebeeb-engine/src/runner.rs"));
    assert!(runner.contains(&squeezed(
        "const PRODUCTION_API_BASE: &str = \"https://api.beebeeb.io\";"
    )));
    assert!(runner.contains(&squeezed(
        "pub fn default_api_base() -> String { api_base_url_from(None) }"
    )));
    let from = &runner[runner.find(&squeezed("fn api_base_url_from(")).unwrap()..];
    let from = &from[..from.find("//").unwrap_or(from.len())];
    assert!(from.contains(&squeezed(
        "#[cfg(not(any(test, feature = \"test-support\")))] return PRODUCTION_API_BASE.to_string();"
    )));
    let open_rs = squeezed(include_str!("open.rs"));
    assert!(open_rs.contains(&squeezed("None => Ok(ApiBase::engine_default()),")));
    assert!(open_rs.contains(&squeezed("origin: beebeeb_engine::runner::default_api_base(),")));
}

/// Decision 2462, what `--api-base` may be: `https://` to any host, `http://` only to 127.0.0.1, ::1 or localhost,
/// and nothing else: another scheme, a user name or password, no host, a path, a query, a fragment. The engine gets
/// the parser's origin, never the text. A refusal is one line naming the reason, never the value.
#[test]
fn api_base_takes_an_https_origin_or_a_loopback_http_one_and_refuses_every_other_shape() {
    let reads = open::reads_api_base_flag(false, &TEST);
    let took = |text: &str| open::api_base(Some(text), reads).map(|base| base.origin().to_string());
    let accepted = [
        ("https://api.example.com", "https://api.example.com"),
        ("https://api.example.com/", "https://api.example.com"),
        ("https://API.Example.COM:443/", "https://api.example.com"),
        ("https://203.0.113.7:8443", "https://203.0.113.7:8443"),
        ("https://bücher.example", "https://xn--bcher-kva.example"),
        ("http://127.0.0.1:9", "http://127.0.0.1:9"),
        ("http://127.0.0.1:3001/", "http://127.0.0.1:3001"),
        ("http://[::1]:3001", "http://[::1]:3001"),
        ("http://localhost:3001", "http://localhost:3001"),
        ("HTTP://LOCALHOST", "http://localhost"),
        ("  http://localhost:3001  ", "http://localhost:3001"),
    ];
    let refused = [
        ("", Refused::Missing),
        ("   ", Refused::Missing),
        ("api.example.com", Refused::NotAUrl),
        ("127.0.0.1:9", Refused::NotAUrl),
        ("http://127.0.0.1:99999", Refused::NotAUrl),
        ("http://[]:9", Refused::NotAUrl),
        ("ftp://api.example.com", Refused::Scheme),
        ("file:///etc/hosts", Refused::Scheme),
        ("ws://127.0.0.1:9", Refused::Scheme),
        ("localhost:3001", Refused::Scheme),
        ("http://api.example.com", Refused::PlainHttp),
        ("http://127.0.0.2:9", Refused::PlainHttp),
        ("http://0.0.0.0:9", Refused::PlainHttp),
        ("http://[::ffff:127.0.0.1]:9", Refused::PlainHttp),
        ("http://localhost.example.com", Refused::PlainHttp),
        // Exactly `localhost` (#335 review P3-1): not a name that ends in it, not the name with a dot at the end.
        ("http://evil.localhost", Refused::PlainHttp),
        ("http://xlocalhost:3001", Refused::PlainHttp),
        ("http://localhost.", Refused::PlainHttp),
        ("http://127.0.0.1@evil.example", Refused::UserInfo),
        ("https://127.0.0.1@evil.example", Refused::UserInfo),
        ("http://someone:hunter2@127.0.0.1:9", Refused::UserInfo),
        ("https://someone@api.example.com", Refused::UserInfo),
        ("https://:hunter2@api.example.com", Refused::UserInfo),
        ("http://@127.0.0.1:9", Refused::UserInfo),
        ("https://", Refused::NoHost),
        ("http://:9", Refused::NoHost),
        ("https://api.example.com/v1", Refused::Path),
        ("http://127.0.0.1:9/api/v1/", Refused::Path),
        ("https://api.example.com/a@b", Refused::Path),
        ("https://api.example.com?x=1", Refused::Query),
        ("https://api.example.com/?", Refused::Query),
        ("https://api.example.com/?ref=a@b", Refused::Query),
        ("https://api.example.com#top", Refused::Fragment),
        ("http://127.0.0.1:9/#", Refused::Fragment),
        ("https://api.example.com/#me@x", Refused::Fragment),
        // The whole `--api-base=<url>` argument, as the parse hands it over (#335 review P3-2).
        ("--api-base=http://127.0.0.1:9", Refused::Glued),
        ("--api-base=", Refused::Glued),
    ];
    // Every shape is tried before the verdict, so a red names all that changed.
    let mut wrong = Vec::new();
    for (text, origin) in accepted {
        let got = took(text);
        if got != Ok(origin.to_string()) {
            wrong.push(format!("{text:?} -> {got:?}, not Ok({origin:?})"));
        }
    }
    for (text, reason) in refused {
        let got = took(text);
        if got != Err(reason) {
            wrong.push(format!("{text:?} -> {got:?}, not Err({reason:?})"));
        }
        let line = reason.to_string();
        assert!(
            line.starts_with("--api-base was refused: ")
                && line.ends_with(". The app did not start.")
                && !line.contains('\n'),
            "{line}"
        );
    }
    assert!(
        wrong.is_empty(),
        "{} of {} shapes are wrong:\n{}",
        wrong.len(),
        accepted.len() + refused.len(),
        wrong.join("\n")
    );
    // The reason, never the value: it may hold a password.
    let line = took("https://someone:hunter2@api.example.com").unwrap_err().to_string();
    assert_eq!(
        line,
        "--api-base was refused: it holds a user name or password (an @). The app did not start."
    );
    assert!(!line.contains("hunter2") && !line.contains("someone"));
}

/// `run` checks `--api-base` before anything exists (the gate export, the platform, the control channel, the engine):
/// a refused value ends the start with one line on stderr and exit 2. The engine is opened with the checked value,
/// which is said once on stdout, first.
#[test]
fn run_checks_api_base_before_anything_starts_and_says_the_server_once() {
    let shell = squeezed(include_str!("../shell/mod.rs"));
    let run = &shell[shell.find(&squeezed("pub fn run() {")).unwrap()..];
    let at = |step: &str| run.find(&squeezed(step)).unwrap_or_else(|| panic!("run has {step}"));
    let (rule, parse, check) = (
        at(
            "let reads_api_base = crate::engine_host::open::reads_api_base_flag(instance::DEV_BUILD, \
            self::identity::engine_identity());",
        ),
        at("instance::parse_args(std::env::args().skip(1), instance::DEV_BUILD, reads_api_base);"),
        at("crate::engine_host::open::api_base(launch.api_base.as_deref(), reads_api_base)"),
    );
    // The platform object comes first, so a refusal can show the start-refusal box; it starts nothing.
    let platform = at("let platform = platform::current();");
    assert!(rule < parse && parse < platform && platform < check);
    let later = [
        "crate::ui::gate::run_from_args(",
        "platform.serve_control(",
        "open_engine(",
    ];
    for step in later {
        assert!(check < at(step), "the check comes before {step}");
    }
    let refusal = &run[check..at(later[0])];
    assert!(refusal.contains(&squeezed(
        "Err(refused) => std::process::exit(refuse_start(&*platform, &launch, identity, &refused.to_string())),"
    )));
    assert_eq!(shell.matches(&squeezed("platform::current()")).count(), 1);
    assert_eq!(shell.matches("instance::parse_args(").count(), 1);
    // Said once, before the engine opens, from the value that was checked.
    let opener = &shell[shell.find(&squeezed("fn open_engine(")).unwrap()..];
    let say = opener
        .find(&squeezed(
            "let _ = writeln!(std::io::stdout(), \"{}\", api_base.start_line());",
        ))
        .unwrap();
    assert!(say < opener.find("crate::engine_host::open::open(").unwrap());
    assert_eq!(shell.matches(".start_line()").count(), 1);
}

/// `--api-base` reaches the engine as an argument; nothing on the start path reads the environment for it.
#[test]
fn the_api_base_is_an_argument_never_the_environment() {
    let production = |text: &str| text.split("#[cfg(test)]").next().unwrap().to_string();
    for (file, text) in [
        ("engine_host.rs", production(include_str!("../engine_host.rs"))),
        ("engine_host/open.rs", production(include_str!("open.rs"))),
        ("shell/mod.rs", production(include_str!("../shell/mod.rs"))),
    ] {
        assert!(
            !text.contains("api_base_url("),
            "{file} reads the API base from the environment"
        );
        assert!(!text.contains("env::var(\"BB_"), "{file} reads an environment variable");
    }
    // The engine handle hands its own API base to the engine it starts.
    let app = include_str!("../../../../crates/beebeeb-engine/src/app.rs");
    assert!(app.contains("self.config.api_base.clone(),\n                sync_root,"));
    let runner = include_str!("../../../../crates/beebeeb-engine/src/runner.rs");
    let run = &runner[runner.find("async fn run(").unwrap()..];
    assert!(run.contains("ApiClient::new(\n        api_base,") && !run.contains("api_base_url()"));
}

/// The engine handle is opened once the single-instance claim is settled, only for the instance that is the
/// app, never beside `--db`; a handle that cannot open ends the start instead of showing the fixture.
#[test]
fn run_opens_the_engine_after_the_claim_and_never_falls_back_to_the_fixture() {
    let shell = include_str!("../shell/mod.rs").replace("\r\n", "\n");
    let run = &shell[shell.find("pub fn run() {").unwrap()..];
    let (claim, plan, open) = (
        run.find("platform.serve_control(").unwrap(),
        run.find("instance::plan(&launch, claim, identity)").unwrap(),
        run.find("open_engine(identity, &api_base, wake_target.clone())")
            .unwrap(),
    );
    assert!(claim < plan && plan < open);
    assert!(run.contains("if matches!(plan, Plan::BeTheApp) && launch.db.is_none() {"));
    let failed = &run[open..];
    let failed = &failed[..failed.find("None => Arc::new(FixtureEngine::ready()),").unwrap()];
    // A handle that was not opened ends the start (`engine_not_opened`: exit 1, or 2 for the binding's refusals).
    assert!(
        squeezed(failed).contains(&squeezed(
            "Err(error) => std::process::exit(engine_not_opened(&*platform, &launch, identity, &error)),"
        )),
        "a failed open ends the start"
    );
    assert_eq!(
        shell.matches("engine_host::open::open(").count(),
        1,
        "one place opens the engine"
    );
    assert_eq!(shell.matches("PlatformCredentials,").count(), 1);
}

/// The engine's sentences map to the screen's kinds by the exact sentence: an answer nobody knows is `Other`,
/// and "the phrase is right, sync did not start" is not a wrong phrase.
#[test]
fn an_engine_refusal_becomes_the_screens_kind_and_an_unknown_one_is_other() {
    assert_eq!(two_factor_problem(gate::WRONG_2FA_CODE), TwoFactorProblem::WrongCode);
    assert_eq!(two_factor_problem(gate::TWO_FACTOR_EXPIRED), TwoFactorProblem::Expired);
    assert_eq!(
        two_factor_problem(gate::two_factor_error("offline")),
        TwoFactorProblem::Offline
    );
    assert_eq!(
        two_factor_problem(gate::two_factor_error("rate_limited:30")),
        TwoFactorProblem::RateLimited
    );
    assert_eq!(
        two_factor_problem(gate::two_factor_error("failed")),
        TwoFactorProblem::Other
    );
    assert_eq!(two_factor_problem("anything else"), TwoFactorProblem::Other);
    assert_eq!(
        phrase_problem(gate::WRONG_PHRASE),
        PhraseProblem::WrongPhrase { bad_word: None }
    );
    assert_eq!(phrase_problem(gate::TOO_MANY_PHRASE_TRIES), PhraseProblem::TooManyTries);
    assert_eq!(phrase_problem(gate::phrase_error("offline")), PhraseProblem::Offline);
    assert_eq!(phrase_problem(gate::UNLOCKED_SYNC_NOT_STARTED), PhraseProblem::Other);
    assert_eq!(phrase_problem(gate::phrase_error("failed")), PhraseProblem::Other);
}

// ---------------------------------------------------------------------------
// Per-group: a live engine shows 0 fixture rows, the snapshot path keeps them
// ---------------------------------------------------------------------------

fn engine_state() -> crate::shell::state::State {
    let started = started();
    crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), started.opened.engine.clone())
}

fn snap_state() -> crate::shell::state::State {
    crate::shell::state::init_with(crate::shell::host::Shell::fixture())
}

#[test]
fn shared_shows_0_fixture_rows_on_a_live_engine() {
    let s = engine_state();
    assert!(!s.frames.shared.source.wired());
    assert_eq!(s.frames.shared.source.with_me().len(), 0);
    assert_eq!(s.frames.shared.source.people_shares().len(), 0);
    assert_eq!(s.frames.shared.source.invitations().len(), 0);
}

#[test]
fn trash_shows_0_fixture_rows_on_a_live_engine() {
    let s = engine_state();
    // Trash reads the engine (it is wired); without a session it holds no row and no count, never the board's.
    assert_eq!(s.frames.trash.source.items().len(), 0);
    assert_eq!(s.frames.trash.source.totals().items, 0);
    assert_eq!(s.frames.trash.source.count(), None, "no sidebar count from the engine");
    assert_eq!(
        s.frames.trash.source.loaded(),
        crate::frames::trash::source::Loaded::SignedOut
    );
}

#[test]
fn photos_shows_0_fixture_days_on_a_live_engine() {
    let s = engine_state();
    assert!(!s.photos.source.wired());
    assert_eq!(s.photos.source.days(None).len(), 0);
    assert_eq!(s.photos.source.devices().len(), 0);
}

#[test]
fn this_pc_shows_0_fixture_changes_on_a_live_engine() {
    let s = engine_state();
    // Engine-backed (`engine_host::this_pc`), held off until its chrome is honest; no name shows either way.
    assert_eq!(s.this_pc.wired(), crate::engine_host::this_pc::TURNED_ON);
    assert_eq!(s.this_pc.changes().len(), 0);
    assert_eq!(s.this_pc.kept_count(), 0);
}

#[test]
fn backups_has_no_source_on_a_live_engine() {
    let s = engine_state();
    assert!(!s.backups.has_source());
}

#[test]
fn settings_is_unwired_on_a_live_engine() {
    use crate::frames::settings::Pane;
    let s = engine_state();
    // Task 2473: General and Storage location answer from the engine; so do Notifications and Recent
    // notifications (`engine_host::settings::notifications`); every other pane is the shell's frame.
    assert!(s.settings.source.wired(Pane::General) && s.settings.source.wired(Pane::StorageLocation));
    assert!(s.settings.source.wired(Pane::Notifications) && s.settings.source.wired(Pane::Recent));
    for pane in [Pane::About] {
        assert!(!s.settings.source.wired(pane), "{pane:?}");
    }
    // Drive's panes (`engine_host::drive_settings`): Storage insights answers; the rest is the shell's frame.
    assert!(s.settings.drive.wired(Pane::Insights));
    for pane in [
        Pane::Storage,
        Pane::Sync,
        Pane::Bandwidth,
        Pane::Previews,
        Pane::Diagnostics,
    ] {
        assert!(!s.settings.drive.wired(pane), "{pane:?}");
    }
    assert!(!s.settings.source.sample());
}

#[test]
fn account_is_the_engines_source_on_a_live_engine_and_unbound_panes_are_unwired() {
    use crate::frames::account::Pane as AccountPane;
    use crate::frames::account::source::PaneState;
    let s = engine_state();
    let source = s.account.source.as_deref().expect("the engine's account source");
    // Account and Devices and sessions answer (`engine_host::account`); Security does not.
    assert_ne!(source.pane_state(AccountPane::Account), PaneState::Unwired);
    assert_ne!(source.pane_state(AccountPane::Sessions), PaneState::Unwired);
    assert_eq!(source.pane_state(AccountPane::Security), PaneState::Unwired);
    assert!(source.identity().email.is_empty() && source.sessions().is_empty());
}

#[test]
fn pc_status_is_empty_on_a_live_engine() {
    let s = engine_state();
    assert!(s.frames.pc.sync_words().is_empty());
    assert!(s.frames.pc.summary().is_empty());
}

#[test]
fn sidebar_badges_are_0_on_a_live_engine() {
    let s = engine_state();
    use crate::ui::sidebar::{SidebarSection, badge_for};
    assert!(
        badge_for(&s, SidebarSection::Shared).is_none(),
        "shared badge on engine"
    );
    assert!(badge_for(&s, SidebarSection::Trash).is_none(), "trash badge on engine");
    assert_eq!(s.this_pc.kept_count(), 0, "available offline count on engine");
}

#[test]
fn storage_split_is_none_on_a_live_engine() {
    let s = engine_state();
    assert!(s.storage_split.is_none(), "no board-sample split beside a real account");
}

/// The snapshot path (`init_with`, `--snap`) keeps the board's fixtures for every group.
#[test]
fn the_snapshot_path_keeps_every_groups_fixtures() {
    let s = snap_state();
    assert!(s.frames.shared.source.wired());
    assert!(s.frames.shared.source.with_me().len() > 0);
    assert!(s.frames.trash.source.wired());
    assert!(s.frames.trash.source.items().len() > 0);
    assert!(s.photos.source.wired());
    assert!(s.photos.source.days(None).len() > 0);
    assert!(s.this_pc.wired());
    assert!(s.this_pc.changes().len() > 0);
    assert!(s.this_pc.kept_count() > 0);
    assert!(s.settings.source.wired(crate::frames::settings::Pane::About));
    assert!(s.settings.drive.wired(crate::frames::settings::Pane::Storage));
    assert!(s.account.source.is_some());
    assert!(s.backups.has_source());
    assert!(!s.frames.pc.sync_words().is_empty());
    assert!(!s.frames.pc.summary().is_empty());
    assert!(s.storage_split.is_some(), "snap: the board's split is present");
    use crate::ui::sidebar::{SidebarSection, badge_for};
    assert!(badge_for(&s, SidebarSection::Shared).is_some(), "snap: shared badge");
    assert!(badge_for(&s, SidebarSection::Trash).is_some(), "snap: trash badge");
    assert!(s.this_pc.kept_count() > 0, "snap: available offline count");
}

/// Where the local state lives is where the 0.8.x app kept it: the per-user LOCAL app data on Windows.
#[test]
fn the_local_state_is_where_the_08x_app_kept_it() {
    let dirs = rune::dirs::AppDirs::rooted(std::env::temp_dir().join("bb-dirs"), "io.example");
    assert_eq!(crate::shell::local_data_of(&dirs, "windows"), dirs.state());
    for os in ["macos", "linux"] {
        assert_eq!(crate::shell::local_data_of(&dirs, os), dirs.data(), "{os}");
    }
}

/// P1: file names are attacker-controlled. The open/reveal code must never route a path through
/// cmd.exe, PowerShell, bash or any shell. On Windows: `ShellExecuteW` (the wide-string API). On
/// macOS/Linux: the path is a single argv element to `open`/`xdg-open`, never a shell command.
#[test]
fn the_open_and_reveal_code_never_routes_through_a_shell() {
    let source = include_str!("../engine_host.rs");
    // Scan from the open_path function to the LiveEngine section.
    let open_section = &source[source.find("fn open_path").unwrap()..];
    let open_section = &open_section[..open_section.find("// LiveEngine").unwrap_or(open_section.len())];
    assert!(!open_section.contains("\"cmd\""), "open_path must not use cmd.exe");
    assert!(
        !open_section.contains("\"sh\"") && !open_section.contains("\"bash\""),
        "open_path must not use a Unix shell"
    );
    assert!(
        !open_section.contains("\"powershell\""),
        "open_path must not use PowerShell"
    );
    // Verify open_path on Windows uses ShellExecuteW.
    #[cfg(target_os = "windows")]
    assert!(
        open_section.contains("ShellExecuteW"),
        "on Windows, open_path must use ShellExecuteW"
    );
}

/// The dangerous-executable list matches the old Tauri shell's list EXACTLY. An `.lnk` or `.scr`
/// added to the shared folder by another user must be revealed, never opened.
#[test]
fn executable_denylist_matches_the_old_shell() {
    use beebeeb_engine::file_ops::is_dangerous_executable;
    use std::path::Path;
    // The old shell's list from src-tauri/src/file_open_commands.rs.
    for ext in [
        "exe", "bat", "cmd", "com", "scr", "lnk", "url", "hta", "ps1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "msi",
        "reg", "cpl", "pif", "msc", "jar",
    ] {
        assert!(
            is_dangerous_executable(Path::new(&format!("a&calc.{ext}"))),
            "{ext} must be blocked even with shell metacharacters in the name"
        );
    }
    // A file with shell metacharacters in its name: always blocked for its extension.
    assert!(is_dangerous_executable(Path::new("a&calc.exe")));
    assert!(is_dangerous_executable(Path::new("x^y%PATH%.lnk")));
    assert!(is_dangerous_executable(Path::new("file\".scr")));
    // Safe types.
    assert!(!is_dangerous_executable(Path::new("photo.jpg")));
    assert!(!is_dangerous_executable(Path::new("doc.pdf")));
}

// ---------------------------------------------------------------------------
// Sync-running guard: every folder-route op is refused while sync is not running
// ---------------------------------------------------------------------------

use crate::frames::files::source::{ActionError, FilesSource};

/// P2-2: the sync guard is BEHAVIOURAL. The handle has a valid sync_root on disk and
/// `is_syncing()` is false. Every folder-route op returns `Unavailable` and the filesystem has
/// 0 changes. The guard is the `is_syncing()` check in `local()` (proven by the source-scan test
/// `the_sync_guard_is_present_in_the_local_choke_point`).
///
/// Mutant "remove the is_syncing guard from local()": tested manually — RED:
///   `left: Ok(()) right: Err(Unavailable) -- new_folder refused by sync guard`
///   (new_folder succeeds when both state_db and sync_root are available and the guard is absent).
///   GREEN after restoring the guard: `ok. 1 passed; 0 failed`.
///
/// This test does NOT create the state_db at the process-global path (doing so in a parallel
/// test runner would race with the listing test's "no local state before first sync" assertion).
#[test]
fn folder_route_ops_are_refused_by_the_sync_guard_not_by_missing_state() {
    let started = started();
    assert!(!started.opened.engine.is_syncing(), "precondition: sync is not running");

    // Create the sync root on disk so it is valid.
    let sync_root = started.opened.engine.sync_root().unwrap();
    std::fs::create_dir_all(&sync_root).unwrap();
    let count_before = std::fs::read_dir(&sync_root).unwrap().count();

    let mut source = EngineFilesSource::new(started.opened.engine.clone());

    let item = crate::source::DriveItem {
        id: "f-2".into(),
        parent_id: None,
        name: "notes.txt".into(),
        is_folder: false,
        size: Some(100),
        child_count: None,
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    };

    // Every folder-route op returns Unavailable (the guard, not a downstream error).
    assert_eq!(
        source.new_folder(None, "ShouldNotExist"),
        Err(ActionError::Unavailable),
        "new_folder refused by sync guard"
    );
    assert_eq!(
        source.rename(&item, "new.txt"),
        Err(ActionError::Unavailable),
        "rename refused by sync guard"
    );
    assert_eq!(
        source.move_to(&[item.clone()], None),
        Err(ActionError::Unavailable),
        "move_to refused by sync guard"
    );
    assert_eq!(
        source.duplicate(&[item.clone()], &[]),
        Err(ActionError::Unavailable),
        "duplicate refused by sync guard"
    );
    assert_eq!(
        source.upload(None, &[std::env::temp_dir().join("fake.txt")]),
        Err(ActionError::Unavailable),
        "upload refused by sync guard"
    );
    assert_eq!(
        source.open(&[item.clone()]),
        Err(ActionError::Unavailable),
        "open refused by sync guard"
    );
    assert_eq!(
        source.show_in_file_explorer(&[item.clone()]),
        Err(ActionError::Unavailable),
        "show_in_file_explorer refused by sync guard"
    );
    assert_eq!(
        source.open_in_file_explorer(None),
        Err(ActionError::Unavailable),
        "open_in_file_explorer refused by sync guard"
    );
    assert!(!source.can_add(None), "can_add is false");

    // Filesystem unchanged: the sync root has no new entries.
    let count_after = std::fs::read_dir(&sync_root).unwrap().count();
    assert_eq!(
        count_before, count_after,
        "0 filesystem changes in the sync root (the guard blocked everything)"
    );

    // Clean up: remove the sync root so subsequent runs start clean.
    let _ = std::fs::remove_dir_all(&sync_root);
}

/// The sync guard is the `is_syncing()` check in `local()`. Dropping the guard (mutant: removing
/// the `if !self.engine.is_syncing()` check) would let the operations through despite sync not
/// running, hitting the state_db/sync_root errors instead of the clear `Unavailable`. The source
/// scan proves the guard is present.
#[test]
fn the_sync_guard_is_present_in_the_local_choke_point() {
    let source = include_str!("../engine_host.rs");
    let local_fn = &source[source.find("fn local<T>(&self").unwrap()..];
    let local_fn = &local_fn[..local_fn.find("\n    }").unwrap()];
    assert!(
        local_fn.contains("is_syncing()"),
        "the local() choke point checks is_syncing: dropping it would let folder-route ops \
         through when sync is not running"
    );
    // Task 2488: the choke point's database is the signed-in account's, from the engine's account-bound handle.
    assert!(
        local_fn.contains(".local_state_for_changes()") && local_fn.contains("state.with("),
        "the local() choke point reaches the state through the account-bound handle"
    );
}

/// Task 2488: outside its tests this crate has no way to a state database but the engine's account-bound handle
/// (`Engine::local_state`, `Engine::local_state_for_changes`). The engine's raw constructors are not `pub` for
/// this crate (`Cargo.toml`: `default-features = false`), which a shipped build proves by compiling; this pin
/// names the doors, so a test build (where `test-support` opens them for fixtures) cannot grow a reader either.
/// The one exception is named: the development build's `--db` viewer.
#[test]
fn no_source_file_opens_a_state_database_around_the_account_bound_handle() {
    fn sources(dir: &std::path::Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if name != "tests" {
                    sources(&path, found);
                }
            } else if name.ends_with(".rs") && !name.contains("tests") {
                found.push(path);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    assert!(
        files.len() > 100,
        "the scan read {} files (the wrong folder?)",
        files.len()
    );
    let doors = [
        ["StateDb::", "open("].concat(),
        ["StateDb::", "open_read_only("].concat(),
        [".state_", "db()"].concat(),
        ["state_db_", "for_config"].concat(),
        ["state_db_path", "_from_state_dir"].concat(),
        ["rusqlite", "::"].concat(),
        ["free_up_space", "_blocking"].concat(),
    ];
    let dev_viewer = ["open_for_", "dev_viewer"].concat();
    let (mut handle_readers, mut dev_viewers) = (0, Vec::new());
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let name = file.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        for door in &doors {
            assert!(
                !text.contains(door.as_str()),
                "{name} reaches the local state by `{door}`"
            );
        }
        if text.contains(&dev_viewer) {
            dev_viewers.push(name);
        }
        handle_readers += text.matches(".local_state()").count() + text.matches(".local_state_for_changes()").count();
    }
    assert_eq!(dev_viewers, ["source_engine.rs"], "the development viewer's one door");
    assert!(
        handle_readers >= 4,
        "the scan saw {handle_readers} readers on the handle (it reads nothing?)"
    );
}

/// Trash and restore (API calls) are NOT gated by the sync guard: they go directly to the server
/// and do not touch the sync folder. Neither calls `self.local(…)`.
#[test]
fn trash_and_restore_are_not_gated_by_the_sync_guard() {
    let source = include_str!("../engine_host.rs");
    let trash_fn = &source[source.find("fn trash(&mut self").unwrap()..];
    let trash_fn = &trash_fn[..trash_fn.find("\n    fn ").unwrap()];
    assert!(
        !trash_fn.contains("self.local("),
        "trash goes to the API, not through the sync folder"
    );
    let restore_fn = &source[source.find("fn restore(&mut self").unwrap()..];
    let restore_fn = &restore_fn[..restore_fn.find("\n    fn ").unwrap()];
    assert!(
        !restore_fn.contains("self.local("),
        "restore goes to the API, not through the sync folder"
    );
}

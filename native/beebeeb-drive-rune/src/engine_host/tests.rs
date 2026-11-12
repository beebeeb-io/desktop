//! The real engine behind the native shell, in one test process: `cargo test` builds the TEST identity
//! (`build.rs`), the engine's `test-support` puts the settings file in a sandbox next to the test binary and
//! makes a closed loopback port the default API, and the credential stores are in memory. Nothing here touches
//! the OS credential store, the person's folders, a registration or the network. Every name is invented.

use std::path::PathBuf;
use std::sync::OnceLock;

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::{REAL, TEST};
use beebeeb_engine::keychain::{AuthSecretStore, SessionToken};
use beebeeb_engine::signin::gate::GateState as EngineGate;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::sync_root_owner::{self, Registration};
use beebeeb_engine::{signin::gate, state_paths};

use super::backups::may_back_up;
use super::open::{self, Opened};
use super::*;
use crate::shell::app::GateState as Shell;
use crate::shell::identity::engine_identity;

struct Started {
    opened: Opened<MemoryCredentials>,
    live: LiveEngine<MemoryCredentials>,
    /// The "local app data" folder the host passed.
    base: PathBuf,
}

/// The start path, once per process (the state folder of an identity is set once): a stored session token
/// for the settings file's account slot, then `open::open` as `shell::run` calls it.
fn started() -> &'static Started {
    static STARTED: OnceLock<Started> = OnceLock::new();
    STARTED.get_or_init(|| {
        let base = tempfile::tempdir().expect("a temp home").keep();
        let stores = MemoryCredentials::default();
        let slot = DesktopConfig::ensure_account_id_for(engine_identity()).expect("the sandboxed settings file");
        stores
            .store_for(&slot)
            .save_session_token(&SessionToken::new("an-invented-token").unwrap())
            .unwrap();
        let (opened, messages) =
            open::open(&base, open::api_base(None, true), stores, || {}).expect("the engine opens");
        let live = LiveEngine::new(&opened, messages);
        Started { opened, live, base }
    })
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

    let names = |listing: &Listing| listing.items.iter().map(|item| item.name.clone()).collect::<Vec<_>>();
    let crumbs = |listing: &Listing| listing.path.iter().map(|crumb| crumb.name.clone()).collect::<Vec<_>>();
    let root = drive.list(None).expect("the root");
    assert_eq!(names(&root), ["Plans", "notes.txt"]);
    assert_eq!(crumbs(&root).len(), 1);
    let inner = drive.list(Some("d-2027")).expect("a folder two levels down");
    assert_eq!(names(&inner), ["budget.ods"]);
    assert_eq!(
        crumbs(&inner)[1..],
        ["Plans", "2027"],
        "the folder's own name, from the engine's crumbs"
    );
    assert_eq!(inner.folder.id.as_deref(), Some("d-2027"));
    assert_eq!(drive.list(Some("gone")).err(), Some(SourceError::NotFound));

    // The window on this engine: Files reads the handle, Backups shows no sample devices.
    let state =
        crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), started.opened.engine.clone());
    assert_eq!(names(&state.listing), ["Plans", "notes.txt"]);
    assert!(state.list_error.is_none() && state.selected.is_none());
    assert!(
        !state.backups.has_source(),
        "Backups is 'not built yet' on a real engine"
    );
    // Shared, Trash, Photos, This PC: all unwired on a real engine, never sample data.
    assert!(
        !state.frames.shared.source.wired(),
        "Shared is unwired on a real engine"
    );
    assert!(!state.frames.trash.source.wired(), "Trash is unwired on a real engine");
    assert!(!state.photos.source.wired(), "Photos is unwired on a real engine");
    assert!(!state.this_pc.wired(), "This PC is unwired on a real engine");
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
            api_base: open::api_base(None, true),
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
            open::api_base(None, true),
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
        assert!(!may_back_up(identity, None, &p(&format!("/home/a/{own}"))), "{own}");
        assert!(!may_back_up(identity, None, &p(&format!("/home/a/{own}/Photos"))));
        assert!(!may_back_up(
            identity,
            None,
            &p(&format!("/home/a/{}", own.to_uppercase()))
        ));
        assert!(
            may_back_up(identity, None, &p(&format!("/home/a/{others}/Photos"))),
            "{others}"
        );
        assert!(may_back_up(identity, None, &p("/home/a/Documents")));
        // A sync folder the person chose elsewhere, under any name.
        let chosen = p("/data/Cloud");
        assert!(!may_back_up(identity, Some(&chosen), &p("/data/Cloud")));
        assert!(!may_back_up(identity, Some(&chosen), &p("/data/cloud/Work")));
        assert!(may_back_up(identity, Some(&chosen), &p("/data/Cloudy")));
        assert!(may_back_up(identity, Some(&chosen), &p("/data")));
    }
}

/// `--api-base` reaches the engine as an argument, in a dev build only; nothing on the start path reads the
/// environment for it.
#[test]
fn the_api_base_is_an_argument_never_the_environment() {
    let default = beebeeb_engine::runner::default_api_base();
    assert_eq!(
        default,
        beebeeb_engine::runner::TEST_API_BASE,
        "a test build's default is a closed port"
    );
    assert_eq!(open::api_base(None, true), default);
    assert_eq!(
        open::api_base(Some("http://127.0.0.1:3001/"), true),
        "http://127.0.0.1:3001"
    );
    assert_eq!(open::api_base(Some("  "), true), default);
    assert_eq!(
        open::api_base(Some("http://127.0.0.1:3001"), false),
        default,
        "a shipped build ignores the flag"
    );
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
        run.find("open_engine(identity, &launch, wake_target.clone())").unwrap(),
    );
    assert!(claim < plan && plan < open);
    assert!(run.contains("if matches!(plan, Plan::BeTheApp) && launch.db.is_none() {"));
    let failed = &run[open..];
    let failed = &failed[..failed.find("None => Arc::new(FixtureEngine::ready()),").unwrap()];
    assert!(
        failed.contains("std::process::exit(1);"),
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

/// A live source selection with an engine yields the unwired state for every area the engine does
/// not yet back: Shared, Trash, Photos, This PC, Backups. Not one fixture row is shown beside a
/// real account's files. The snapshot path (`init_with`, `--snap`) keeps the board's fixtures.
#[test]
fn a_live_engine_shows_no_fixture_rows_and_the_snapshot_path_keeps_fixtures() {
    let started = started();
    let engine_state =
        crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), started.opened.engine.clone());

    // --- Shared: 0 fixture rows on the engine path ---
    assert!(!engine_state.frames.shared.source.wired());
    assert_eq!(
        engine_state.frames.shared.source.with_me().len(),
        0,
        "Shared with-me: 0 fixture rows on the engine"
    );
    assert_eq!(
        engine_state.frames.shared.source.people_shares().len(),
        0,
        "Shared people: 0 fixture rows on the engine"
    );
    assert_eq!(
        engine_state.frames.shared.source.invitations().len(),
        0,
        "Shared invitations: 0 fixture rows on the engine"
    );

    // --- Trash: 0 fixture rows on the engine path ---
    assert!(!engine_state.frames.trash.source.wired());
    assert_eq!(
        engine_state.frames.trash.source.items().len(),
        0,
        "Trash: 0 fixture rows on the engine"
    );
    assert_eq!(engine_state.frames.trash.source.totals().items, 0);

    // --- Photos: 0 fixture days on the engine path ---
    assert!(!engine_state.photos.source.wired());
    assert_eq!(
        engine_state.photos.source.days(None).len(),
        0,
        "Photos: 0 fixture days on the engine"
    );
    assert_eq!(
        engine_state.photos.source.devices().len(),
        0,
        "Photos: 0 fixture devices on the engine"
    );

    // --- This PC: 0 fixture changes on the engine path ---
    assert!(!engine_state.this_pc.wired());
    assert_eq!(
        engine_state.this_pc.changes().len(),
        0,
        "This PC history: 0 fixture changes on the engine"
    );
    assert_eq!(
        engine_state.this_pc.kept_count(),
        0,
        "This PC kept: 0 fixture items on the engine"
    );

    // --- Backups: no source at all on the engine path ---
    assert!(!engine_state.backups.has_source());

    // --- The snapshot path keeps the board's fixtures (not unwired) ---
    let snap_state = crate::shell::state::init_with(crate::shell::host::Shell::fixture());
    assert!(snap_state.frames.shared.source.wired(), "snap: shared is wired");
    assert!(
        snap_state.frames.shared.source.with_me().len() > 0,
        "snap: shared has fixture rows"
    );
    assert!(snap_state.frames.trash.source.wired(), "snap: trash is wired");
    assert!(
        snap_state.frames.trash.source.items().len() > 0,
        "snap: trash has fixture rows"
    );
    assert!(snap_state.photos.source.wired(), "snap: photos is wired");
    assert!(
        snap_state.photos.source.days(None).len() > 0,
        "snap: photos has fixture days"
    );
    assert!(snap_state.this_pc.wired(), "snap: this_pc is wired");
    assert!(
        snap_state.this_pc.changes().len() > 0,
        "snap: this_pc has fixture changes"
    );
    assert!(
        snap_state.this_pc.kept_count() > 0,
        "snap: this_pc has fixture kept items"
    );
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

//! Bench: the Files window on a 40,000-file account. Signs in to the engine's fake server seeded with
//! `LargeAccount::GUUS`, opens each view and prints one line per view: p50 and p95 of the time one frame takes to
//! build, and the process's memory (RSS).
//!
//! ```sh
//! cargo run --release --example bench_large                        # all eight views, one line each
//! cargo run --release --example bench_large -- --view photos       # one of: files list grid photos search shared trash this-pc
//! cargo run --release --example bench_large -- --frames 60         # frames measured per view (default 40)
//! ```
//!
//! `files` is the root; `list` and `grid` the biggest folder (Camera Roll, 5,000 photos) as a list and as a grid.
//! The account is invented, the fake listens on loopback and nothing leaves the machine. The app's local state is
//! written from the seeded rows (what a sync would leave), so the numbers are the window's, not a sync's.
//!
//! The example is one crate with the app's modules: they are declared here, at this crate's root, from `src/`, and the
//! names the app's files read as `crate::…` are re-exported below (what `src/main.rs` does).
#![allow(warnings)]

// Every module of `src/main.rs`, at this crate's root (the app's `pub(in crate::engine_host::…)` paths need them
// there). `engine_host` and `log_file` are `<name>/mod.rs`: a `#[path]` file reads its children from its own folder.
#[path = "../src/engine_host/mod.rs"]
mod engine_host;
#[cfg(feature = "feedback")]
#[path = "../src/feedback/mod.rs"]
mod feedback;
#[path = "../src/frames/mod.rs"]
mod frames;
#[path = "../src/log_file/mod.rs"]
mod log_file;
#[path = "../src/media/mod.rs"]
mod media;
#[path = "../src/shell/mod.rs"]
mod shell;
#[path = "../src/source.rs"]
mod source;
#[path = "../src/source_engine.rs"]
mod source_engine;
#[path = "../src/text_kit/mod.rs"]
mod text_kit;
#[path = "../src/theme/mod.rs"]
mod theme;
#[path = "../src/ui/mod.rs"]
mod ui;
#[path = "../src/windows_floor.rs"]
mod windows_floor;
pub use shell::registry::view;
pub use shell::state::{State, init, load};
pub use shell::{FAMILIES, FONTS, TITLEBAR_H};

use std::sync::Arc;
use std::time::{Duration, Instant};

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::large_account::{LargeAccount, camera_roll_id, local_rows, seed};
use rune::{Env, Runtime};
use zeroize::Zeroizing;

use shell::app::{Route, Section};
use shell::host::{Shell, dispatch};
use shell::state::init_on_engine;

const EMAIL: &str = "large@example.invalid";
const PASSWORD: &str = "an invented password for the bench";
const VIEWS: [&str; 8] = ["files", "list", "grid", "photos", "search", "shared", "trash", "this-pc"];

fn rss_mb() -> f64 {
    let pid = sysinfo::get_current_pid().ok();
    let mut system = sysinfo::System::new();
    let Some(pid) = pid else { return 0.0 };
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::nothing().with_memory(),
    );
    system.process(pid).map_or(0.0, |p| p.memory() as f64 / 1_048_576.0)
}

/// Frames until the element count has not changed for 15 frames in a row (late answers have landed), at most `cap`.
fn settle(rt: &mut Runtime<State>, cap: Duration) {
    let began = Instant::now();
    let mut last = usize::MAX;
    let mut same = 0;
    while began.elapsed() < cap && same < 15 {
        rt.frame(0.016);
        let now = rt.elements().len();
        same = if now == last { same + 1 } else { 0 };
        last = now;
        std::thread::sleep(Duration::from_millis(5));
    }
    rt.settle_all();
    rt.frame(0.016);
}

fn open(rt: &mut Runtime<State>, view: &str) {
    match view {
        "files" => {
            dispatch(&mut rt.state, shell::app::Event::Navigate(Route::section(Section::Files)));
        }
        "list" | "grid" => {
            let roll = camera_roll_id();
            dispatch(&mut rt.state, shell::app::Event::Navigate(Route::folder(Section::Files, roll)));
            rt.state.listing = {
                let (listing, _) = shell::state::load(rt.state.source.as_ref(), Some(&camera_roll_id()));
                listing
            };
            frames::global::grid::set(&mut rt.state, view == "grid");
        }
        "photos" => frames::go(&mut rt.state, Section::Photos),
        "search" => {
            dispatch(&mut rt.state, shell::app::Event::Navigate(Route::section(Section::Files)));
            rt.update(|s, tx| {
                frames::search::open(s, tx);
                frames::search::run(s, "a", tx);
            });
        }
        "shared" => frames::go(&mut rt.state, Section::Shared),
        "trash" => frames::go(&mut rt.state, Section::Trash),
        "this-pc" => frames::go(&mut rt.state, Section::SyncHistory),
        other => panic!("no view {other}"),
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut only, mut frames_each) = (None::<String>, 40usize);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--view" => only = args.next(),
            "--frames" => frames_each = args.next().and_then(|n| n.parse().ok()).expect("--frames takes a number"),
            _ => {
                eprintln!("usage: bench_large [--view {}] [--frames N]", VIEWS.join("|"));
                std::process::exit(2);
            }
        }
    }
    if let Some(view) = &only {
        assert!(VIEWS.contains(&view.as_str()), "no view {view}");
    }

    let t0 = Instant::now();
    // The seeded account and the fake server that serves it.
    let (account, _phrase) = FakeAccount::register(EMAIL, PASSWORD, TEST_PHRASE);
    eprintln!("bench_large: registered ({:.1}s)", t0.elapsed().as_secs_f64());
    seed(&account, &LargeAccount::GUUS);
    eprintln!("bench_large: seeded ({:.1}s)", t0.elapsed().as_secs_f64());
    let owner = Identity::new(Some(&account.user_id), Some(&account.email));
    let rows = local_rows(&account);
    eprintln!("bench_large: rows listed ({:.1}s)", t0.elapsed().as_secs_f64());
    let api = FakeApi::start(account_handler(Arc::new(account)));

    // The engine, signed in and unlocked; its local state is what a sync of that account leaves.
    let (host, _messages) = ChannelHost::new(|| {});
    // The engine's state folder is the process's: a folder of this run's own, never the person's.
    let base = tempfile::tempdir().expect("a state folder");
    beebeeb_engine::state_paths::init_from_app_local_data_dir(&TEST, base.path()).expect("the state folder");
    let state_dir = beebeeb_engine::state_paths::beebeeb_state_dir_for(&TEST).expect("the state folder");
    std::fs::create_dir_all(&state_dir).expect("the state folder");
    let config = EngineConfig {
        account_id: "bench-large-slot".into(),
        state_dir,
        api_base: api.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens"));
    engine.restore();
    let local = tempfile::tempdir().expect("a folder for the local state");
    let db_path = local.path().join("state.db");
    let db = StateDb::open(&db_path).expect("the local state opens");
    for (n, row) in rows.iter().enumerate() {
        if n % 5000 == 0 {
            eprintln!("bench_large: local state row {n} ({:.1}s)", t0.elapsed().as_secs_f64());
        }
        let kind = if row.is_folder { ItemKind::Folder } else { ItemKind::File };
        let entry = FileEntry {
            file_id: row.id.clone(),
            path: row.path.clone(),
            status: FileStatus::CloudOnly,
            size_bytes: row.size_bytes as i64,
            modified_at: row.updated_at,
            content_hash: None,
            remote_updated_at: row.updated_at,
            parent_id: row.parent_id.clone(),
            item_kind: kind.clone(),
        };
        db.upsert_file_with_contract(&entry, |c| {
            c.parent_id = entry.parent_id.clone();
            c.item_kind = kind.clone();
        })
        .expect("a row is recorded");
    }
    db.set_owner(&owner).expect("the local state is the account's");
    drop(db);
    engine.use_local_state_for_test(db_path);
    engine
        .sign_in_password(EMAIL, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(TEST_PHRASE.to_string()))
        .expect("the phrase unlocks");
    eprintln!("bench_large: {} rows, signed in to {}", rows.len(), api.base);

    let state = init_on_engine(Shell::fixture(), engine.clone());
    let mut rt = Runtime::new(
        state,
        view,
        (1280.0, 800.0),
        Env {
            system_dark: false,
            scale: 1.0,
        },
    );
    rt.poll = Some(shell::host::poll);
    rt.on_key = Some(shell::host::on_key);
    rt.frame(0.0);

    for name in VIEWS.iter().filter(|v| only.as_deref().is_none_or(|o| o == **v)) {
        open(&mut rt, name);
        settle(&mut rt, Duration::from_secs(20));
        let mut ms = Vec::with_capacity(frames_each);
        for _ in 0..frames_each {
            let began = Instant::now();
            rt.frame(0.016);
            ms.push(began.elapsed().as_secs_f64() * 1000.0);
        }
        ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "view={name} p50_ms={:.2} p95_ms={:.2} rss_mb={:.0}",
            percentile(&ms, 0.5),
            percentile(&ms, 0.95),
            rss_mb()
        );
    }
}

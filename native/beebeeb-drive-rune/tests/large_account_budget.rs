//! Budgets for the Files window on the 40,000-file account (`LargeAccount::REFERENCE`, task 025's fixture), one check per
//! view. Time is CPU time of the test thread (not the clock), so a loaded build machine does not flake it.
//!
//! * frame build, p95 over 40 frames: under 16 ms on the files root, the 5,000-photo folder as a list and as a grid,
//!   photos, search, shared, trash and this-pc;
//! * the folder-list SQL on the 5,000-child folder: under 50 ms;
//! * the photos library read (`Engine::photos_library`): under 300 ms.
//!
//! Run: cargo test --locked --release --config profile.release.debug-assertions=true --test large_account_budget
//! (the engine refuses test-support in a release build without debug assertions; that guard stays.)
//! One test: the account is seeded once and all views are measured on it. Budgets can be raised on a slow machine by
//! BB_BUDGET_SCALE (a number, default 1), never lowered by code. An unoptimised build (the plain `cargo test` of the
//! whole suite, `BEEBEEB_OPT_LEVEL` 0 from build.rs) measures and prints every number but holds none: the budgets
//! are an optimised build's (a debug build's grid frame is ~47 ms and its photos library ~330 ms on bb-build-linux).
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
use beebeeb_engine::state_db::{FileStatus, ItemKind, StateDb};
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

const FRAMES: usize = 40;
const FRAME_BUDGET_MS: f64 = 16.0;
const SQL_BUDGET_MS: f64 = 50.0;
const PHOTOS_BUDGET_MS: f64 = 300.0;

/// CPU time this thread has used so far. Unix: clock_gettime(CLOCK_THREAD_CPUTIME_ID); Windows: GetThreadTimes. Elsewhere the
/// clock (no better source without a new dependency).
fn thread_cpu() -> Duration {
    #[cfg(unix)]
    {
        #[repr(C)]
        struct Timespec {
            sec: i64,
            nsec: i64,
        }
        unsafe extern "C" {
            fn clock_gettime(clock: i32, out: *mut Timespec) -> i32;
        }
        #[cfg(target_os = "linux")]
        const CLOCK_THREAD_CPUTIME_ID: i32 = 3;
        #[cfg(target_os = "macos")]
        const CLOCK_THREAD_CPUTIME_ID: i32 = 16;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let mut t = Timespec { sec: 0, nsec: 0 };
            if unsafe { clock_gettime(CLOCK_THREAD_CPUTIME_ID, &mut t) } == 0 {
                return Duration::new(t.sec as u64, t.nsec as u32);
            }
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
        let ok = unsafe { GetThreadTimes(GetCurrentThread(), &mut c, &mut e, &mut k, &mut u) };
        if ok != 0 {
            let ticks = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64; // 100 ns
            return Duration::from_nanos((ticks(k) + ticks(u)) * 100);
        }
    }
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

fn cpu_ms<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let before = thread_cpu();
    let value = f();
    (value, (thread_cpu() - before).as_secs_f64() * 1000.0)
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

#[test]
fn large_account_stays_inside_its_budgets() {
    let scale: f64 = std::env::var("BB_BUDGET_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let mut over: Vec<String> = Vec::new();
        // The seeded account and the fake server that serves it.
    let (account, _phrase) = FakeAccount::register(EMAIL, PASSWORD, TEST_PHRASE);
    seed(&account, &LargeAccount::REFERENCE);
    let owner = Identity::new(Some(&account.user_id), Some(&account.email));
    let rows = local_rows(&account);
    let api = FakeApi::start(account_handler(Arc::new(account)));

    // The engine, signed in and unlocked; its local state is what a sync of that account leaves.
    let (host, _messages) = ChannelHost::new(|| {});
    // The engine's state folder is the process's: a folder of this run's own, never the person's.
    let base = tempfile::tempdir().expect("a state folder");
    // Another part of this process may have resolved it first (the engine's test support does): then that folder, also a
    // scratch one, is the folder; either way it is this run's, never the person's.
    let _ = beebeeb_engine::state_paths::init_from_app_local_data_dir(&TEST, base.path());
    let state_dir = beebeeb_engine::state_paths::beebeeb_state_dir_for(&TEST).expect("the state folder");
    std::fs::create_dir_all(&state_dir).expect("the state folder");
    let config = EngineConfig {
        account_id: "budget-large-slot".into(),
        state_dir,
        api_base: api.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens"));
    engine.restore();
    let local = tempfile::tempdir().expect("a folder for the local state");
    let db_path = local.path().join("state.db");
    // The schema from the engine's own opener, the rows in ONE transaction (the engine's per-row writer keeps a change
    // log and is far too slow for 40,000 rows).
    drop(StateDb::open(&db_path).expect("the local state opens"));
    {
        let mut conn = rusqlite::Connection::open(&db_path).expect("the local state opens");
        let tx = conn.transaction().expect("a transaction");
        {
            let mut insert = tx
                .prepare(
                    "INSERT INTO files (file_id, path, status, size_bytes, modified_at, remote_updated_at, parent_id, item_kind)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7)",
                )
                .expect("the insert");
            for row in &rows {
                let kind = if row.is_folder { ItemKind::Folder } else { ItemKind::File };
                insert
                    .execute(rusqlite::params![
                        row.id,
                        row.path,
                        FileStatus::CloudOnly.as_str(),
                        row.size_bytes as i64,
                        row.updated_at,
                        row.parent_id,
                        kind.as_str()
                    ])
                    .expect("a row is recorded");
            }
        }
        tx.commit().expect("the rows are kept");
    }
    let db = StateDb::open(&db_path).expect("the local state opens");
    db.set_owner(&owner).expect("the local state is the account's");
    drop(db);
    let db_path_for_sql = db_path.clone();
    engine.use_local_state_for_test(db_path);
    engine
        .sign_in_password(EMAIL, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(TEST_PHRASE.to_string()))
        .expect("the phrase unlocks");

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


    for name in VIEWS {
        open(&mut rt, name);
        settle(&mut rt, Duration::from_secs(20));
        let mut ms = Vec::with_capacity(FRAMES);
        for _ in 0..FRAMES {
            ms.push(cpu_ms(|| rt.frame(0.016)).1);
        }
        ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p95 = percentile(&ms, 0.95);
        println!("BUDGET view={name} frame p95 cpu {p95:.2} ms (budget {FRAME_BUDGET_MS})");
        if p95 > FRAME_BUDGET_MS * scale {
            over.push(format!("{name}: frame build p95 {p95:.2} ms > {FRAME_BUDGET_MS} ms"));
        }
    }

    // The folder-list SQL: the engine's statement (`StateDb::children_paged`) for the whole 5,000-child folder.
    let roll = camera_roll_id();
    let conn = rusqlite::Connection::open(&db_path_for_sql).expect("the local state opens");
    let mut best = f64::MAX;
    for _ in 0..5 {
        let (n, ms) = cpu_ms(|| {
            let mut stmt = conn
                .prepare(
                    "SELECT file_id, path, status, size_bytes, modified_at, item_kind FROM files
                     WHERE parent_id IS ?1 AND status != 'trashing'
                     ORDER BY item_kind ASC, path ASC LIMIT ?2 OFFSET ?3",
                )
                .unwrap();
            stmt.query_map(rusqlite::params![roll, 5_000, 0], |r| r.get::<_, String>(0))
                .unwrap()
                .count()
        });
        assert_eq!(n, LargeAccount::REFERENCE.biggest_folder, "the biggest folder lists whole");
        best = best.min(ms);
    }
    println!("BUDGET folder list SQL {best:.2} ms (budget {SQL_BUDGET_MS})");
    if best > SQL_BUDGET_MS * scale {
        over.push(format!("folder list SQL {best:.2} ms > {SQL_BUDGET_MS} ms"));
    }

    // The photos library, in one pass over the local state.
    let mut best = f64::MAX;
    let mut count = 0;
    for _ in 0..3 {
        let (library, ms) = cpu_ms(|| engine.photos_library().expect("the library reads"));
        count = library.photos.len();
        best = best.min(ms);
    }
    assert!(count > 0, "the seeded account has photos");
    println!("BUDGET photos library {best:.2} ms for {count} (budget {PHOTOS_BUDGET_MS})");
    if best > PHOTOS_BUDGET_MS * scale {
        over.push(format!("photos library {best:.2} ms > {PHOTOS_BUDGET_MS} ms"));
    }

    if env!("BEEBEEB_OPT_LEVEL") == "0" {
        if !over.is_empty() {
            println!("BUDGET not held in an unoptimised build (run it with --release):\n{}", over.join("\n"));
        }
        return;
    }
    assert!(over.is_empty(), "over budget:\n{}", over.join("\n"));
}

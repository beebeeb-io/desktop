//! RE-a: the first listing of a 42k-row account (a real one, 2026-11-24: 41,872 rows, 39 at the root) is whole within a
//! budget, the bar moves at least every second from the moment the list arrives, and the root is in the database by
//! the ingest's first report (the Files view reads it from there).

use crate::engine_bridge::EngineBridge;
use crate::engine_bridge::syncflow::{ListingProgress, ingest_snapshot};
use crate::state_db::StateDb;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const KEY: [u8; 32] = [0x5a; 32];
const ROWS: usize = 41_872;
const FOLDERS: usize = 2_800;
const AT_ROOT: usize = 39;
/// The whole listing, decrypt pass included, in a debug test build: 14.4 s measured on a laptop build host
/// (2026-11-24, next to another 41k test; decrypt 4.9 s, ingest 9.3 s), so twice that. A release build is several
/// times faster.
const BUDGET: Duration = Duration::from_secs(60); // debug build on a loaded shard: 14 s alone, 35 s under 8 shards
/// The longest the bar may stand still.
const MOST_BETWEEN_REPORTS: Duration = Duration::from_secs(1);

fn id(i: usize) -> String {
    format!("{:08x}-0000-4000-8000-{:012x}", 0x0dc2_ea00u32, i)
}

/// 39 folders at the root, the other folders under earlier ones eight wide, every file in a folder.
fn nodes() -> Vec<serde_json::Value> {
    let master = beebeeb_core::kdf::MasterKey::from_bytes(KEY);
    (0..ROWS)
        .map(|i| {
            let folder = i < FOLDERS;
            let parent = if folder {
                (i >= AT_ROOT).then(|| id((i - AT_ROOT) / 8))
            } else {
                Some(id(AT_ROOT + i % (FOLDERS - AT_ROOT)))
            };
            let item = id(i);
            #[allow(deprecated)]
            let name = beebeeb_core::encrypt::encrypt_name(&master, &item, &format!("n{i}"), None).unwrap();
            serde_json::json!({
                "id": item, "parent_id": parent, "name_encrypted": name, "size_bytes": 10,
                "is_folder": folder, "content_hash": null, "version_number": 1,
                "is_trashed": false, "is_starred": false, "updated_at": 1_000, "has_thumbnail": false,
            })
        })
        .collect()
}

fn count(db: &Path, sql: &str) -> i64 {
    rusqlite::Connection::open(db)
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

#[ignore = "timing: 60 s wall-clock budget; runs in the non-blocking Timing tests job (scripts/ci-timing-tests.txt)"]
#[tokio::test]
async fn first_listing_of_42k_rows_is_whole_in_budget_and_reports_every_second() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&sync_root).unwrap();
    let db_path = dir.path().join("state.db");
    let db = Arc::new(StateDb::open(&db_path).unwrap());
    let api = Arc::new(crate::api_client::ApiClient::new(
        "http://127.0.0.1:9".into(),
        "token".into(),
        KEY,
    ));
    let bridge = EngineBridge::new(&crate::identity::TEST, db, api);
    let nodes = nodes();

    // Every report rings the wake hook: when, the count, and the root rows the database holds then (another
    // connection, as the window reads it; a report never comes under the database lock).
    let rings: Arc<Mutex<Vec<(Instant, Option<(u64, u64)>, i64)>>> = Arc::default();
    let board = bridge.transfers().clone();
    {
        let (rings, board, db_path) = (rings.clone(), Arc::downgrade(&board), db_path.clone());
        bridge.transfers().set_wake(move || {
            let listing = board.upgrade().and_then(|board| board.first_listing());
            let roots = count(&db_path, "SELECT COUNT(*) FROM files WHERE parent_id IS NULL");
            rings.lock().unwrap().push((Instant::now(), listing, roots));
        });
    }

    let started = Instant::now();
    let snapshot = crate::api_client::SyncSnapshot { seq_id: 42, nodes };
    let mut progress = ListingProgress::start(board.clone(), ROWS as u64);
    ingest_snapshot(&bridge, &sync_root, snapshot, 2_000, &mut Vec::new(), &mut progress)
        .await
        .unwrap();
    let phases = progress.phases().to_vec();
    drop(progress);
    let took = started.elapsed();
    println!("RE-a first listing of {ROWS} rows: {took:?}, phases (ms) {phases:?}");

    assert_eq!(count(&db_path, "SELECT COUNT(*) FROM files"), ROWS as i64);
    assert_eq!(
        count(&db_path, "SELECT COUNT(*) FROM files WHERE parent_id IS NULL"),
        AT_ROOT as i64
    );
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(42));
    assert!(
        took <= BUDGET,
        "the first listing took {took:?}, over its budget of {BUDGET:?}"
    );

    let rings = rings.lock().unwrap().clone();
    assert!(rings.len() >= 3, "{} reports", rings.len());
    assert_eq!(rings.last().unwrap().1, None, "the listing's report ends with it");
    let mut last = started;
    let mut listed = 0;
    for (at, listing, _) in &rings {
        assert!(
            *at - last <= MOST_BETWEEN_REPORTS,
            "the bar stood still {:?}",
            *at - last
        );
        last = *at;
        if let Some((now, total)) = listing {
            assert_eq!(*total, ROWS as u64);
            assert!(*now >= listed, "the count went back from {listed} to {now}");
            listed = *now;
        }
    }
    // The ingest's first report after its first batch (past the decrypt pass's quarter): the root is there.
    let first_ingest = rings
        .iter()
        .find(|(_, listing, _)| listing.is_some_and(|(now, _)| now > ROWS as u64 / 4))
        .expect("a report during the ingest");
    assert_eq!(
        first_ingest.2, AT_ROOT as i64,
        "the root was not in the database at the ingest's first report"
    );
}

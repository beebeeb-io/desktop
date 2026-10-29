//! Tasks 1888 + 2226, plan Task 1 Step 7: the reconcile pass on a copy of a real queue, timed per transaction, to set
//! [`super::RECONCILE_UNITS_PER_TX`]. Ignored: it runs only by hand, with `BB_RECONCILE_DRY_RUN_DB` naming a copy made
//! read-only from the live database. It works on a second copy and prints counts and milliseconds, never a name or
//! a path.
use super::*;
use crate::held_queue::{Counts, Options};
use std::time::Instant;

fn printable(counts: &Counts) -> String {
    let dirs = counts.scaffold_dirs.len();
    format!("{:?} scaffold_dirs={dirs}", Counts { scaffold_dirs: Vec::new(), ..counts.clone() })
}

fn second_copy(src: &str, dir: &Path) -> StateDb {
    let copy = dir.join("state.db");
    std::fs::copy(src, &copy).expect("copy the copy");
    StateDb::open(copy).expect("open the second copy")
}

#[test]
#[ignore]
fn reconcile_on_a_copy() {
    let src = std::env::var("BB_RECONCILE_DRY_RUN_DB").expect("set BB_RECONCILE_DRY_RUN_DB to a copy of state.db");
    let opts = Options {
        device_segment: crate::runner::device_name().map(|n| crate::known_folder::sanitize_device_segment(&n)),
        case_insensitive: cfg!(windows),
    };
    // `BB_RECONCILE_SYNC_ROOT`: the real sync folder, only listed (never written), so D5 sees what is on disk. Without
    // it every directory counts as holding a file and no scaffolding is dropped.
    let sync_root = std::env::var("BB_RECONCILE_SYNC_ROOT").ok().map(std::path::PathBuf::from);
    let keep = |rel: &str| sync_root.as_ref().map_or(true, |root| tree_holds_a_file(&root.join(rel)));
    let now = chrono::Utc::now().timestamp();

    let runs: Vec<(usize, bool)> = match std::env::var("BB_RECONCILE_RUNS").as_deref() {
        Ok("index") => vec![(256, true), (128, true), (64, true)],
        Ok("dry") => vec![(RECONCILE_UNITS_PER_TX, false)],
        _ => vec![(usize::MAX, false), (512, false), (256, false), (128, false), (64, false)],
    };
    for (per_tx, with_index) in runs {
        let dir = tempfile::tempdir().unwrap();
        let db = second_copy(&src, dir.path());
        if with_index {
            db.0.lock()
                .unwrap()
                .execute_batch(
                    "CREATE INDEX IF NOT EXISTS idx_files_parent_id ON files(parent_id);
                     CREATE INDEX IF NOT EXISTS idx_operation_queue_parent_id ON operation_queue(parent_id);",
                )
                .unwrap();
        }
        let dry = db.reconcile_dry_run(&opts, &keep).unwrap();
        // Task 2271: with `BB_RECONCILE_HELD_OUT`, the ids of the ops the plan holds, one per line, go to that local
        // file for a script to compare by count. They are never printed.
        if let Ok(out) = std::env::var("BB_RECONCILE_HELD_OUT") {
            let conn = db.0.lock().unwrap();
            let (units, _) = reconcile_plan_conn(&conn, &opts, &keep).unwrap();
            let held: Vec<&str> = units
                .iter()
                .flat_map(|unit| match unit {
                    crate::held_queue::Unit::Hold { op_id } => vec![op_id.as_str()],
                    crate::held_queue::Unit::Remap { hold, .. } => hold.iter().map(String::as_str).collect(),
                    _ => Vec::new(),
                })
                .collect();
            std::fs::write(&out, held.join("\n")).expect("write the held ids");
            println!("held_written={}", held.len());
        }
        let mut last = Instant::now();
        let mut batch_ms = Vec::new();
        let started = Instant::now();
        let counts = db
            .reconcile_in_batches(&opts, now, per_tx, &keep, &mut |_, _| {
                batch_ms.push(last.elapsed().as_millis());
                last = Instant::now();
            })
            .unwrap();
        let total = started.elapsed().as_millis();
        let again = db.reconcile_dry_run(&opts, &keep).unwrap();
        let label = format!("{}{}", if per_tx == usize::MAX { "all".to_string() } else { per_tx.to_string() }, if with_index { "+index" } else { "" });
        println!("per_tx={label} batches={} total_ms={total} batch_ms={batch_ms:?}", batch_ms.len());
        println!("per_tx={label} dry_equals_run={}", dry == counts);
        println!("per_tx={label} counts {}", printable(&counts));
        println!("per_tx={label} second_pass {}", printable(&again));
    }
}

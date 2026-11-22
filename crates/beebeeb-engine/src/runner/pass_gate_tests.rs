//! RC2-6b: an idle tick lists no folder; anything that can have changed something runs both passes.

use super::*;
use std::cell::Cell;
use std::collections::{HashMap, VecDeque};

use crate::os_state_reconcile::{ReconcileCounts, plan_reconcile};
use crate::state_db::{FileEntry, FileStatus, ItemKind};

/// A feed that says what it is told, then `Nothing`.
struct Told(VecDeque<Changes>);

impl ChangeFeed for Told {
    fn take(&mut self) -> Changes {
        self.0.pop_front().unwrap_or(Changes::Nothing)
    }
}

fn row(path: &str) -> (FileEntry, bool) {
    (
        FileEntry {
            file_id: path.into(),
            path: path.into(),
            status: FileStatus::CloudOnly,
            size_bytes: 1,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        },
        false,
    )
}

/// Two folders of files and one at the top: three folders to list.
fn rows() -> Vec<(FileEntry, bool)> {
    ["a/1.bin", "a/2.bin", "b/1.bin", "top.bin"]
        .into_iter()
        .map(row)
        .collect()
}

/// One tick's passes through the gate, the way the runner runs them: the placeholder refresh lists each folder
/// (counted as the refresh's `dirs_listed`), the reconcile plans from listings (`ReconcileCounts::dirs_listed`).
/// Returns (refresh dirs listed, reconcile dirs listed, why).
fn tick(
    gate: &mut PassGate,
    feed: &mut Told,
    work: TickWork,
    now: Instant,
    wall: SystemTime,
) -> (usize, usize, Option<RunWhy>) {
    let refresh_listed = Cell::new(0usize);
    let reconcile = Cell::new(ReconcileCounts::default());
    let ran = gate.step(feed, work, now, wall, || {
        refresh_listed.set(3);
        let plan = plan_reconcile(rows(), 0, |_| Ok(HashMap::new()), |_| None);
        reconcile.set(plan.counts);
        PassOutcome::default()
    });
    (
        refresh_listed.get(),
        reconcile.get().dirs_listed,
        ran.map(|(why, _)| why),
    )
}

fn told(changes: impl IntoIterator<Item = Changes>) -> Told {
    Told(changes.into_iter().collect())
}

/// The red test of RC2-6: with a feed that names nothing and a tick that applied nothing, a tick lists no folder.
#[test]
fn an_idle_tick_lists_no_folder_dirs_listed_is_zero_for_both_passes() {
    let (t0, w0) = (Instant::now(), SystemTime::now());
    let mut gate = PassGate::default();
    let mut feed = told([]);
    // The first tick of a run does the whole pass.
    assert_eq!(
        tick(&mut gate, &mut feed, TickWork::default(), t0, w0),
        (3, 3, Some(RunWhy::Owed))
    );
    for i in 1..=10u64 {
        let at = Duration::from_secs(30 * i);
        assert_eq!(
            tick(&mut gate, &mut feed, TickWork::default(), t0 + at, w0 + at),
            (0, 0, None),
            "idle tick {i}"
        );
    }
}

#[test]
fn dirs_listed_again_after_a_change_and_once_more_after_it() {
    let (t0, w0) = (Instant::now(), SystemTime::now());
    let s = |n: u64| Duration::from_secs(30 * n);
    let mut gate = PassGate::default();
    let mut feed = told([Changes::Nothing, Changes::Paths(["x".into()].into())]);
    tick(&mut gate, &mut feed, TickWork::default(), t0, w0);
    // A path named by the journal (a pin in Explorer, a deleted placeholder) runs both.
    assert_eq!(
        tick(&mut gate, &mut feed, TickWork::default(), t0 + s(1), w0 + s(1)),
        (3, 3, Some(RunWhy::FeedNamed))
    );
    // The tick after reads once more (a listing that showed the change late), then the gate closes.
    assert_eq!(
        tick(&mut gate, &mut feed, TickWork::default(), t0 + s(2), w0 + s(2)),
        (3, 3, Some(RunWhy::Owed))
    );
    assert_eq!(
        tick(&mut gate, &mut feed, TickWork::default(), t0 + s(3), w0 + s(3)),
        (0, 0, None)
    );
}

#[test]
fn dirs_listed_when_the_tick_applied_items_or_finished_operations() {
    let (t0, w0) = (Instant::now(), SystemTime::now());
    let s = |n: u64| Duration::from_secs(30 * n);
    for work in [
        TickWork {
            applied_items: 1,
            finished_ops: 0,
        },
        TickWork {
            applied_items: 0,
            finished_ops: 2,
        },
    ] {
        let mut gate = PassGate::default();
        let mut feed = told([]);
        tick(&mut gate, &mut feed, TickWork::default(), t0, w0);
        assert_eq!(
            tick(&mut gate, &mut feed, work, t0 + s(1), w0 + s(1)),
            (3, 3, Some(RunWhy::TickWork))
        );
    }
}

#[test]
fn dirs_listed_when_the_feed_cannot_say_after_sleep_and_hourly() {
    let (t0, w0) = (Instant::now(), SystemTime::now());
    let s = |n: u64| Duration::from_secs(30 * n);

    let mut gate = PassGate::default();
    let mut feed = told([Changes::Nothing, Changes::Unknown]);
    tick(&mut gate, &mut feed, TickWork::default(), t0, w0);
    assert_eq!(
        tick(&mut gate, &mut feed, TickWork::default(), t0 + s(1), w0 + s(1)),
        (3, 3, Some(RunWhy::FeedUnknown))
    );
    // An Unknown run owes no follow-up: it was the whole pass.
    assert_eq!(
        tick(&mut gate, &mut feed, TickWork::default(), t0 + s(2), w0 + s(2)),
        (0, 0, None)
    );

    // A tick long after the last (the machine slept).
    let mut gate = PassGate::default();
    let mut feed = told([]);
    tick(&mut gate, &mut feed, TickWork::default(), t0, w0);
    assert_eq!(
        tick(
            &mut gate,
            &mut feed,
            TickWork::default(),
            t0 + s(1),
            w0 + Duration::from_secs(3 * 3600)
        ),
        (3, 3, Some(RunWhy::Slept))
    );

    // The hourly backstop, on an hour of idle ticks.
    let mut gate = PassGate::default();
    let mut feed = told([]);
    tick(&mut gate, &mut feed, TickWork::default(), t0, w0);
    let mut ran = Vec::new();
    for i in 1..=125u64 {
        let (r, c, why) = tick(&mut gate, &mut feed, TickWork::default(), t0 + s(i), w0 + s(i));
        if why.is_some() {
            assert_eq!((r, c, why), (3, 3, Some(RunWhy::Backstop)));
            ran.push(i);
        }
    }
    assert_eq!(ran, [120], "once an hour");
}

#[test]
fn dirs_listed_on_the_next_tick_when_a_pass_left_work_to_retry() {
    let (t0, w0) = (Instant::now(), SystemTime::now());
    let s = |n: u64| Duration::from_secs(30 * n);
    let mut gate = PassGate::default();
    let mut feed = told([]);
    let ran = gate.step(&mut feed, TickWork::default(), t0, w0, || PassOutcome { retry: true });
    assert_eq!(ran.map(|(why, _)| why), Some(RunWhy::Owed));
    assert_eq!(
        tick(&mut gate, &mut feed, TickWork::default(), t0 + s(1), w0 + s(1)),
        (3, 3, Some(RunWhy::Owed))
    );
    assert_eq!(
        tick(&mut gate, &mut feed, TickWork::default(), t0 + s(2), w0 + s(2)),
        (0, 0, None)
    );
}

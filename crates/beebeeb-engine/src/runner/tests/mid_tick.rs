//! w36: a nudge (a save in the viewer, a New file, Try again) is served while the loop is inside a tick. Before, the
//! loop took it only between ticks: a save was stored and not sent for as long as the tick ran (minutes with a backup
//! pass or a long queue), and the viewer's 20 s wait ran out. All on tokio's paused clock.

use super::*;

struct Rig {
    link: Arc<SendPhaseLink>,
    state: Arc<LoopState>,
    paused: Arc<AtomicBool>,
    sent: Arc<AtomicU32>,
    sender: JoinHandle<()>,
}

impl Rig {
    /// A runner whose loop is inside a tick that never ends, after its first reconcile pass.
    fn mid_tick() -> Self {
        let link = Arc::new(SendPhaseLink::default());
        let state = Arc::new(LoopState::default());
        state.set_busy(true);
        state.set_reconciled(true);
        let paused = Arc::new(AtomicBool::new(false));
        let sent = Arc::new(AtomicU32::new(0));
        let sender = tokio::spawn(send_nudges_mid_tick(link.clone(), state.clone(), paused.clone(), {
            let sent = sent.clone();
            move || {
                let sent = sent.clone();
                async move {
                    sent.fetch_add(1, Ordering::SeqCst);
                }
            }
        }));
        Self {
            link,
            state,
            paused,
            sent,
            sender,
        }
    }
    fn sent(&self) -> u32 {
        self.sent.load(Ordering::SeqCst)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.sender.abort();
    }
}

#[tokio::test(start_paused = true)]
async fn w36_a_save_nudged_during_a_running_tick_is_sent_without_waiting_for_the_tick() {
    let rig = Rig::mid_tick();
    rig.link.nudge();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(rig.sent(), 1, "sent within a second, while the tick still runs");
    assert!(rig.state.is_busy(), "the tick has not ended");
}

/// The loop's own permit is kept: once the tick ends, `next_wake` still answers the nudge with a whole queue step
/// (Try again on a backup upload is sent by that one; the mid-tick step leaves the backup's uploads to the tick).
#[tokio::test(start_paused = true)]
async fn w36_the_loop_still_takes_the_nudge_after_its_tick() {
    let rig = Rig::mid_tick();
    rig.link.nudge();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let (_keep_open, mut cancel) = oneshot::channel::<()>();
    let mut tick = tokio::time::interval(TICK_INTERVAL);
    let board = crate::transfer_progress::TransferBoard::new();
    assert_eq!(next_wake(&mut cancel, &mut tick, &rig.link, &board).await, Wake::Tick);
    assert_eq!(next_wake(&mut cancel, &mut tick, &rig.link, &board).await, Wake::Nudge);
}

/// Spec D11b stays: nothing is sent before this run's first reconcile pass. The nudge waits for that pass and is sent
/// right after it, inside the same tick.
#[tokio::test(start_paused = true)]
async fn w36_nothing_is_sent_before_the_first_reconcile_pass_and_the_nudge_is_sent_right_after_it() {
    let rig = Rig::mid_tick();
    rig.state.set_reconciled(false);
    rig.link.nudge();
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(rig.sent(), 0, "the first pass has not committed");
    rig.state.set_reconciled(true);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(rig.sent(), 1);
}

/// A tick that ends before its first pass committed (offline): the nudge is the loop's again, nothing is sent here.
#[tokio::test(start_paused = true)]
async fn w36_a_tick_that_ends_unreconciled_sends_nothing_mid_tick() {
    let rig = Rig::mid_tick();
    rig.state.set_reconciled(false);
    rig.link.nudge();
    tokio::time::sleep(Duration::from_secs(2)).await;
    rig.state.set_busy(false);
    tokio::time::sleep(Duration::from_secs(5)).await;
    rig.state.set_reconciled(true);
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(rig.sent(), 0);
}

#[tokio::test(start_paused = true)]
async fn w36_a_nudge_while_the_loop_waits_is_the_loops_own() {
    let rig = Rig::mid_tick();
    rig.state.set_busy(false);
    rig.link.nudge();
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(
        rig.sent(),
        0,
        "`Wake::Nudge` runs that step; a second one here would only repeat it"
    );
}

/// A nudge and a tick that start together: the loop took the tick first (`biased`), so the nudge is served here.
#[tokio::test(start_paused = true)]
async fn w36_a_tick_that_starts_with_the_nudge_does_not_hold_it_back() {
    let rig = Rig::mid_tick();
    rig.state.set_busy(false);
    rig.link.nudge();
    tokio::time::sleep(Duration::from_millis(50)).await;
    rig.state.set_busy(true);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(rig.sent(), 1);
}

#[tokio::test(start_paused = true)]
async fn w36_nothing_is_sent_mid_tick_while_sync_is_paused() {
    let rig = Rig::mid_tick();
    rig.paused.store(true, Ordering::SeqCst);
    rig.link.nudge();
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(rig.sent(), 0);
}

/// The tick's pull and reconcile pass read the rows a landing writes: the step waits for them, then runs.
#[tokio::test(start_paused = true)]
async fn w36_the_step_waits_for_the_ticks_pull_and_passes() {
    let rig = Rig::mid_tick();
    let pulling = rig.state.pull.lock().await;
    rig.link.nudge();
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(rig.sent(), 0, "the pull is not over");
    drop(pulling);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(rig.sent(), 1);
}

/// At most one step a second, and a second nudge inside that second is served when it is out, never dropped.
#[tokio::test(start_paused = true)]
async fn w36_two_nudges_in_a_second_are_two_steps_a_second_apart() {
    let rig = Rig::mid_tick();
    rig.link.nudge();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.sent(), 1);
    rig.link.nudge();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(rig.sent(), 1, "inside the second");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(rig.sent(), 2);
}

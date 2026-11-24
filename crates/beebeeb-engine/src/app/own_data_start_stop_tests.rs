//! Row fx-stop-timeout (plan40, after CP3), at the engine: a stop (a Lock, a sign-out) whose ownership proof is held in
//! a write that does not yield, as a slow database write is, past the stop's wait. The stop does not wait longer, the
//! step goes on and tells the person nothing new, and every start is refused until the proof's client is gone.

use super::*;

/// The stop's wait in these tests, in place of [`STOP_WAIT`].
const SHORT_STOP_WAIT: Duration = Duration::from_millis(100);

/// A listing that says it was asked, then holds its worker in a write that does not yield (no await, so a stop's
/// abort cannot drop the proof) until the test drops the returned sender, or [`WAIT`] passed. Once the sender is
/// dropped, every later ask answers at once.
fn held_in_a_write(asked: mpsc::Sender<()>) -> (TestListing, mpsc::Sender<()>) {
    let (release, released) = mpsc::channel::<()>();
    let asked = Mutex::new(asked);
    let released = Arc::new(Mutex::new(released));
    let listing: TestListing = Arc::new(move || -> Answer {
        let _ = asked.lock().unwrap().send(());
        let released = Arc::clone(&released);
        Box::pin(async move {
            let _ = released.lock().unwrap().recv_timeout(WAIT);
            Listing::Unavailable
        })
    });
    (listing, release)
}

/// A PC whose refused start owes a proof that is now held in its write, with the stop's wait shortened. Returns the
/// proof's client (to see when it is gone) and the sender that lets the write finish when dropped.
fn a_proof_held_in_a_write(pc: &Pc) -> (Weak<ApiClient>, mpsc::Sender<()>) {
    synced_rows(&pc.db());
    pc.sign_in(&owner());
    let (asked, listing_asked) = mpsc::channel();
    let (held, release) = held_in_a_write(asked);
    pc.seams(|seams| {
        seams.listing = Some(held);
        seams.stop_wait = Some(SHORT_STOP_WAIT);
    });
    pc.engine.serve_own_data_proofs();
    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));
    listing_asked.recv_timeout(WAIT).expect("the proof is in its write");
    let clients = pc.seams(|seams| seams.clients.clone());
    assert_eq!(clients.len(), 1);
    assert!(clients[0].strong_count() > 0, "the premise: the proof holds its client");
    (clients[0].clone(), release)
}

/// Row fx-stop-timeout (mutant ST-K: the stop forgets a proof that did not end within its wait, as before the fix).
/// A Lock whose proof is held in a write past the stop's wait returns after that wait and closes the vault. Every start
/// is then refused ("didn't confirm it stopped") while the proof's client is alive, and nothing is registered; once
/// the client is gone, a start goes on to the binding again.
#[test]
fn a_lock_whose_proof_outlives_the_wait_closes_the_vault_and_refuses_starts_until_its_client_is_gone() {
    let pc = Pc::new();
    let (client, release) = a_proof_held_in_a_write(&pc);
    open_lock(&pc.engine);
    let begun = Instant::now();

    let locked = pc.engine.lock_vault();

    assert!(
        begun.elapsed() < WAIT / 2,
        "the Lock is bounded by the stop's wait, not by the proof's write"
    );
    assert_eq!(
        pc.engine.lock_state(),
        beebeeb_api::flows::LockState::VaultClosed,
        "the vault is closed"
    );
    assert!(locked.survives_restart, "the closed mark is stored");
    assert!(
        client.strong_count() > 0,
        "the premise: the proof's client outlived the stop's wait"
    );
    pc.sign_in(&owner());
    assert_eq!(
        pc.start(),
        Err(SyncStartRefused::StopFinishing),
        "no start while the stopped proof's client is alive"
    );
    assert_eq!(pc.engine.view().start, Start::Refused(SyncStartRefused::StopFinishing));
    assert_eq!(pc.port.registers(), 0, "nothing registered");

    drop(release);
    // Its outcome is recorded only once its task is dropped, after the sender the start's refusal reads.
    pc.wait_for_outcomes(1);
    assert_eq!(client.strong_count(), 0, "the stopped proof's client is gone");

    assert_eq!(
        pc.start(),
        Err(SyncStartRefused::OtherAccountsData),
        "once the client is gone, a start goes on to the binding again"
    );
}

/// Row fx-stop-timeout (mutant ST-E: the sign-out refuses, still signed in, when its proof did not end within the
/// wait, as it refuses for the engine's unconfirmed stop). The sign-out goes on and tells the person nothing new; the
/// next account's start is refused while the proof's client is alive, and goes on to the binding once it is gone.
#[test]
fn a_sign_out_whose_proof_outlives_the_wait_still_signs_out_and_refuses_starts_until_its_client_is_gone() {
    let pc = Pc::new();
    let (client, release) = a_proof_held_in_a_write(&pc);

    let signed_out = pc.engine.sign_out_keeping_local_data();

    assert_eq!(
        signed_out,
        Ok(()),
        "the sign-out goes on and tells the person nothing new"
    );
    assert!(
        pc.engine.acct.session.lock().unwrap().is_none(),
        "the session left memory"
    );
    assert!(
        client.strong_count() > 0,
        "the premise: the proof's client outlived the stop's wait"
    );
    pc.sign_in(&other());
    assert_eq!(
        pc.start(),
        Err(SyncStartRefused::StopFinishing),
        "no start while the stopped proof's client is alive"
    );

    drop(release);
    // Its outcome is recorded only once its task is dropped, after the sender the start's refusal reads.
    pc.wait_for_outcomes(1);
    assert_eq!(client.strong_count(), 0, "the stopped proof's client is gone");

    assert_eq!(
        pc.start(),
        Err(SyncStartRefused::OtherAccountsData),
        "once the client is gone, a start goes on to the binding again"
    );
}

/// Row fx-stop-timeout (mutant ST-R: the stop reports `Ended` when its wait ran out, as its old unit answer implied).
/// A stop reports a proof held past its wait as unconfirmed, and so does a later stop while it is still alive; once it
/// let go, a stop reports it ended. A proof that drops at its next await is reported ended, its client gone.
#[test]
fn a_stop_reports_a_proof_that_outlived_its_wait_until_it_let_go() {
    let pc = Pc::new();
    let (client, release) = a_proof_held_in_a_write(&pc);
    let stop = || pc.engine.runtime.block_on(pc.engine.stop_proofs());

    assert_eq!(
        stop(),
        ProofStop::Unconfirmed,
        "the wait ran out with the proof in its write"
    );
    assert!(
        client.strong_count() > 0,
        "the premise: the proof is still in its write"
    );
    assert_eq!(stop(), ProofStop::Unconfirmed, "a later stop waits for it again");
    drop(release);
    // Its outcome is recorded only once its task is dropped, after the sender the start's refusal reads.
    pc.wait_for_outcomes(1);
    assert_eq!(client.strong_count(), 0, "the stopped proof's client is gone");
    assert_eq!(stop(), ProofStop::Ended, "a stop once it let go");

    let pc = Pc::new();
    synced_rows(&pc.db());
    pc.sign_in(&owner());
    let (asked, listing_asked) = mpsc::channel();
    // The stop's own wait here: a proof that drops at its next await is reported as soon as it ended.
    pc.seams(|seams| seams.listing = Some(hanging(asked)));
    pc.engine.serve_own_data_proofs();
    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));
    listing_asked.recv_timeout(WAIT).expect("the proof is listing");
    let clients = pc.seams(|seams| seams.clients.clone());

    assert_eq!(
        pc.engine.runtime.block_on(pc.engine.stop_proofs()),
        ProofStop::Ended,
        "a proof that drops at its next await ends within the wait"
    );
    assert_eq!(clients[0].strong_count(), 0, "its client is gone");
}

/// Row fx-stop-words (mutant SW-D: the finishing sentence tells a restart again, or the unconfirmed one stops
/// telling it): a stop whose proof outlived the wait refuses as `StopFinishing`, whose words say to try again; an
/// engine stop that was never confirmed (only a restart ends it) stays `StopUnconfirmed` with "quit and reopen".
#[test]
fn a_finishing_stop_says_try_again_and_an_unconfirmed_engine_stop_keeps_quit_and_reopen() {
    let finishing = SyncStartRefused::StopFinishing.to_string();
    for word in ["restart", "reopen", "Quit"] {
        assert!(!finishing.contains(word), "{finishing}");
    }
    assert!(finishing.contains("Try again"), "{finishing}");
    assert_eq!(SyncStartRefused::StopFinishing.kind(), "stop_finishing");

    let pc = Pc::new();
    pc.sign_in(&owner());
    pc.engine.acct.engine_stop_unconfirmed.store(true, Ordering::SeqCst);
    let refused = pc
        .start()
        .expect_err("an engine whose stop was never confirmed refuses every start");
    assert_eq!(refused, SyncStartRefused::StopUnconfirmed);
    assert!(refused.to_string().contains("Quit and reopen"), "{refused}");
    // Unlike the proof's refusal, nothing clears it: a second start refuses the same way.
    assert_eq!(pc.start(), Err(SyncStartRefused::StopUnconfirmed));
}

//! Row S4 race (plan40, after CP3), at the engine: a stop (a Lock, a sign-out, the close of a session the server ended,
//! the strong unlock's replacement) that comes at a moment of the ownership proof the stop used to miss. The service
//! thread waits at a test's pause there, so the moment comes on demand instead of under load.

use super::*;

/// A pause for the service thread ([`TestSeams::before_keys`], [`TestSeams::after_spawn`]): it says it arrived, then
/// waits until the test lets it go or the test ended (its `Sender` dropped). Never on a clock: the service thread is
/// woken inside `start_sync`, before the start's own publish (a plan request) returns, so the thread can arrive long
/// before the test has the start back in hand. A timed pause let the proof go on before the stop the test makes after
/// the start, under load (a 30 s plan request against a 20 s pause).
fn pause() -> (TestPause, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (arrived, arrival) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let (arrived, released) = (Mutex::new(arrived), Mutex::new(released));
    let pause: TestPause = Arc::new(move || {
        let _ = arrived.lock().unwrap().send(());
        let _ = released.lock().unwrap().recv();
    });
    (pause, arrival, release)
}

/// Row S4 race (mutant S4-R1: the proof is registered for a stop after its task was spawned, outside the seat lock,
/// as before the fix). The close of a session the server ended comes while the service thread is right after the
/// spawn of the proof's task: it still stops that proof, and the proof's client is released before the close
/// returns. Under load this moment came by itself (`strong_count` 2, expected 0).
#[test]
fn a_close_right_after_the_proof_was_spawned_stops_it_and_its_client_is_released() {
    let pc = Pc::new();
    synced_rows(&pc.db());
    pc.sign_in(&owner());
    let (asked, listing_asked) = mpsc::channel();
    let (pause, paused, release) = pause();
    pc.seams(|seams| {
        seams.listing = Some(hanging(asked));
        seams.after_spawn = Some(pause);
    });
    pc.engine.serve_own_data_proofs();
    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));
    paused.recv_timeout(WAIT).expect("the service thread spawned the proof");
    listing_asked.recv_timeout(WAIT).expect("the proof is listing");
    let clients = pc.seams(|seams| seams.clients.clone());
    assert_eq!(clients.len(), 1);
    assert!(clients[0].strong_count() > 0, "the premise: the proof holds its client");
    open_lock(&pc.engine);
    let turn = pc.engine.acct.session_generation();

    let closed = pc.engine.close_session_the_server_refused(Some(turn));

    assert!(closed.is_some(), "the premise: the server's word closed the vault");
    assert_eq!(
        clients[0].strong_count(),
        0,
        "the close released the client of the proof spawned right before it"
    );
    release.send(()).unwrap();
    pc.wait_for_outcomes(1);
    assert_eq!(pc.outcomes(), vec!["stopped"]);
    assert_eq!(pc.db().owner().unwrap(), None);
}

/// Row S4 race (mutant S4-R2: no stop count compared before the proof reads the session's keys). A stop that comes
/// after the service thread took up the proof and before it read the session's keys ends the proof there, while the
/// session is still the one it is owed for: no key is read, no client is made, the server is asked nothing.
#[test]
fn a_stop_before_the_proof_read_the_keys_ends_it_without_a_client() {
    let pc = Pc::new();
    synced_rows(&pc.db());
    pc.sign_in(&owner());
    let (pause, paused, release) = pause();
    pc.seams(|seams| {
        seams.listing = Some(answering(Listing::Unavailable));
        seams.before_keys = Some(pause);
    });
    pc.engine.serve_own_data_proofs();
    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));
    paused.recv_timeout(WAIT).expect("the service thread took up the proof");
    let turn = pc.engine.acct.session_generation();

    assert_eq!(
        pc.engine.runtime.block_on(pc.engine.stop_proofs()),
        ProofStop::Ended,
        "no proof was running yet"
    );
    release.send(()).unwrap();

    pc.wait_for_outcomes(1);
    assert_eq!(
        pc.engine.acct.session_generation(),
        turn,
        "the premise: the session is still the one the proof is owed for"
    );
    assert_eq!(
        pc.seams(|seams| seams.clients.len()),
        0,
        "no client was made after the stop"
    );
    assert_eq!(pc.outcomes(), vec!["stopped"]);
    assert_eq!(pc.db().owner().unwrap(), None);
}

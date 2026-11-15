//! Task 2488 (ledger L186): `Engine::set_kept` into a local state that is another account's is refused with the
//! engine's own words (`NOT_YOURS`), and nothing is written into that state. The first account signs in, unlocks and
//! syncs for real (7 items, a sync folder set); it signs out keeping its local data; a second account signs in and
//! unlocks on the same computer and asks to keep one of the first account's files.
//! The order is the point: the second account has no sync folder set, so the owner check must come before the
//! sync-folder read. Mutant (`kept_api.rs`, `set_kept_in`): the guard `&& refused.not_this_accounts()` gets a
//! `&& false` after it -> the owner refusal falls through to the sync-folder read and the refusal at the first
//! `set_kept` reads "no sync folder is set on this computer": red. (The order before the fix is the same red.)

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn set_kept_into_another_accounts_state_is_refused() {
    use std::sync::{Arc, Mutex};

    use beebeeb_engine::app::kept_api::{NOT_SIGNED_IN, NOT_YOURS};
    use beebeeb_engine::app::{LocalStateRefused, SyncStart};
    use beebeeb_engine::signin::gate::{GateState, NextAfterSignIn};
    use beebeeb_engine::state_db::{PinState, StateDb};
    use beebeeb_engine::test_api::fake_account::{FakeAccount, account_handler};
    use beebeeb_engine::test_api::{Handler, Request};
    use zeroize::Zeroizing;

    /// A second valid 12-word phrase (a BIP-39 test vector).
    const SECOND_PHRASE: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";

    let (second, _) = FakeAccount::register("second@example.eu", common::PASSWORD, SECOND_PHRASE);
    let second = Arc::new(second);
    // One fake API for both accounts: by the session's token, and a sign-in by the email it names.
    let s = common::setup_with({
        let second = second.clone();
        move |to_first, _, _| {
            let to_second = account_handler(second.clone());
            let signing_in_second = Mutex::new(false);
            let handler: Handler = Arc::new(move |request: &Request| {
                let second_asked = match request.bearer() {
                    Some(token) => token == second.session_token,
                    None => {
                        let mut signing_in = signing_in_second.lock().unwrap();
                        if let Some(email) = request.json().get("email").and_then(|email| email.as_str()) {
                            *signing_in = email == second.email;
                        }
                        *signing_in
                    }
                };
                if second_asked {
                    to_second(request)
                } else {
                    to_first(request)
                }
            });
            handler
        }
    });
    let db = || StateDb::open(s.state_db()).unwrap();
    let pin_of = |id: &str| {
        db().get_file_contract_state(id)
            .unwrap()
            .map(|contract| contract.effective_pin_state())
    };

    // The first account syncs for real: its state holds 7 items and its sync folder is set.
    s.sign_in();
    s.unlock();
    assert_eq!(s.engine.start_sync(s.home.join("Beebeeb Test")), Ok(SyncStart::Started));
    assert_eq!(
        common::wait_for_rows(&s.state_db(), 7),
        7,
        "the first account's 7 items"
    );
    s.engine.stop();
    let id = db()
        .list_files()
        .unwrap()
        .into_iter()
        .find(|entry| !entry.is_dir())
        .expect("a synced file")
        .file_id;
    let before = pin_of(&id);
    assert_ne!(before, Some(PinState::Pinned), "the control: it is not kept yet");
    s.engine.sign_out_keeping_local_data().expect("the sign-out");

    // The second account, signed in and unlocked on the same computer, is refused with the engine's own words.
    assert_eq!(
        s.engine
            .sign_in_password(&second.email, Zeroizing::new(common::PASSWORD.into())),
        Ok(NextAfterSignIn::RecoveryPhrase)
    );
    s.engine
        .unlock_with_phrase(Zeroizing::new(SECOND_PHRASE.into()))
        .expect("the second account's phrase");
    assert_eq!(s.engine.gate(), GateState::Ready);
    assert_eq!(
        s.engine.local_state_refusal(),
        Some(LocalStateRefused::OtherAccount),
        "the control: the local state is another account's"
    );
    assert_eq!(
        s.engine.set_kept(&id, true),
        Err(NOT_YOURS.to_string()),
        "kept into the first account's state: refused as not theirs"
    );
    assert_ne!(NOT_YOURS, NOT_SIGNED_IN, "the two refusals are told apart");
    assert_eq!(
        pin_of(&id),
        before,
        "nothing was written into the first account's state"
    );
    assert_eq!(
        s.engine.set_kept(&id, false),
        Err(NOT_YOURS.to_string()),
        "stopping to keep is refused the same way"
    );
    assert_eq!(pin_of(&id), before);
    assert_eq!(db().list_files().unwrap().len(), 7, "nothing was lost");
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}

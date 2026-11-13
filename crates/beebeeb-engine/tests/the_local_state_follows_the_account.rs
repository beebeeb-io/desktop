//! Task 2488, end to end on the fake API: the local state is handed out through the account-bound handle only, to
//! the account it is recorded as. The first account signs in, unlocks and syncs for real (7 items); it signs out
//! keeping its local data; a second account signs in and unlocks on the same computer and is refused at every door,
//! while nothing is reset or deleted; the first account comes back to its list as it was.
//! Mutant: the handle skips the owner comparison (the second account's doors open).

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn the_local_state_follows_the_account() {
    use std::sync::{Arc, Mutex};

    use beebeeb_engine::app::{LocalStateRefused, SyncStart, SyncStartRefused};
    use beebeeb_engine::drive_v2::model::Sort;
    use beebeeb_engine::signin::gate::{GateState, NextAfterSignIn};
    use beebeeb_engine::state_db::StateDb;
    use beebeeb_engine::sync_view::Start;
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
    let refusal = || s.engine.local_state_refusal();
    let names = || -> Result<Vec<String>, String> {
        s.engine
            .listing(None, Sort::default(), 0, 100)
            .map(|(items, _)| items.into_iter().map(|item| item.name).collect())
    };
    let rows_on_disk = || StateDb::open(s.state_db()).unwrap().list_files().unwrap().len();
    let owner_on_disk = || StateDb::open(s.state_db()).unwrap().owner().unwrap();

    // The first account: no state until its first sync, then its own.
    assert_eq!(refusal(), Some(LocalStateRefused::SignedOut));
    s.sign_in();
    assert_eq!(refusal(), Some(LocalStateRefused::Locked), "signed in, no keys yet");
    s.unlock();
    assert_eq!(refusal(), Some(LocalStateRefused::NothingSynced));
    let folder = s.home.join("Beebeeb Test");
    assert_eq!(s.engine.start_sync(folder.clone()), Ok(SyncStart::Started));
    assert_eq!(
        common::wait_for_rows(&s.state_db(), 7),
        7,
        "the first account's 7 items"
    );
    s.engine.stop();
    assert_eq!(refusal(), None, "the owner is handed its state");
    assert_eq!(
        owner_on_disk(),
        Some(s.this_account()),
        "recorded by the start, by id and email"
    );
    let firsts = names().expect("the owner's list");
    assert!(!firsts.is_empty(), "the control: the list has names");
    let held = s.engine.local_state().expect("a handle of the first account's session");
    assert_eq!(held.with(|db| db.list_files().unwrap().len()), Ok(7));

    // Signed out, keeping the local data: the handle it held is over, nothing was deleted.
    s.engine.sign_out_keeping_local_data().expect("the sign-out");
    assert_eq!(held.with(|_| ()), Err(LocalStateRefused::SessionChanged));
    assert_eq!(refusal(), Some(LocalStateRefused::SignedOut));
    assert_eq!(rows_on_disk(), 7);

    // The second account, on the same computer: signed in, unlocked, at the Ready gate, and refused at every door.
    assert_eq!(
        s.engine
            .sign_in_password(&second.email, Zeroizing::new(common::PASSWORD.into())),
        Ok(NextAfterSignIn::RecoveryPhrase)
    );
    assert_eq!(refusal(), Some(LocalStateRefused::Locked));
    s.engine
        .unlock_with_phrase(Zeroizing::new(SECOND_PHRASE.into()))
        .expect("the second account's phrase");
    assert_eq!(s.engine.gate(), GateState::Ready);
    assert_eq!(refusal(), Some(LocalStateRefused::OtherAccount));
    assert_eq!(s.engine.local_state().err(), Some(LocalStateRefused::OtherAccount));
    assert_eq!(
        s.engine.local_state_for_changes().err(),
        Some(LocalStateRefused::OtherAccount)
    );
    assert!(names().is_err(), "no list for the second account");
    assert_eq!(held.with(|_| ()), Err(LocalStateRefused::SessionChanged));
    // Its start is refused, and nothing of the first account's is reset, re-owned or deleted.
    assert_eq!(
        s.engine.start_sync(folder.clone()),
        Err(SyncStartRefused::OtherAccountsData)
    );
    assert_eq!(
        s.engine.view().start,
        Start::Refused(SyncStartRefused::OtherAccountsData)
    );
    assert_eq!(
        refusal(),
        Some(LocalStateRefused::OtherAccount),
        "also after the refused start"
    );
    assert_eq!(owner_on_disk(), Some(s.this_account()));
    assert_eq!(rows_on_disk(), 7);
    // Locked, the second account is still handed nothing.
    s.engine.lock();
    assert_eq!(refusal(), Some(LocalStateRefused::Locked));
    assert!(names().is_err());

    // The second account leaves; the first is back to its list as it was.
    s.engine.sign_out_keeping_local_data().expect("the second sign-out");
    assert_eq!(refusal(), Some(LocalStateRefused::SignedOut));
    s.sign_in();
    s.unlock();
    assert_eq!(refusal(), None);
    assert_eq!(names().expect("the first account's list again"), firsts);
    assert_eq!(rows_on_disk(), 7, "nothing was lost");
    assert_eq!(
        held.with(|_| ()),
        Err(LocalStateRefused::SessionChanged),
        "an old handle stays over"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}

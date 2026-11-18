//! Shared's link calls (w17-shared-links), against the fake accounts of `shared_tests.rs` (a child of it).
//! Every name and password is invented.

use super::*;

fn patches(fake: &FakeApi) -> Vec<(String, Value)> {
    fake.requests()
        .into_iter()
        .filter(|req| req.route().starts_with("PATCH /api/v1/shares/"))
        .map(|req| (req.route(), req.json()))
        .collect()
}

#[test]
fn change_expiry_on_a_link_sends_one_patch_with_the_chosen_hours_and_only_for_a_listed_link() {
    let fake = fake("link-expiry");
    let mut source = EngineSharedSource::new(signed_in(&fake));
    // Before the lists are here nothing can be changed.
    assert!(!source.change_link(REQ, LinkEdit::Expiry(Some(24))));
    opened(&mut source);
    // Not a link row of the lists: nothing is sent.
    assert!(!source.change_link(OTHERS, LinkEdit::Expiry(Some(24))));
    assert!(patches(&fake.fake).is_empty());

    assert!(source.change_link(REQ, LinkEdit::Expiry(Some(24 * 7))));
    assert!(!source.change_link(REQ, LinkEdit::Expiry(None)), "an answer is on its way");
    assert!(settle(&mut source).is_empty());
    let sent = patches(&fake.fake);
    assert_eq!(sent.len(), 1, "one PATCH per press");
    assert_eq!(sent[0].0, format!("PATCH /api/v1/shares/{REQ}"));
    assert_eq!(sent[0].1["expires_in_hours"], json!(168));

    assert!(source.change_link(REQ, LinkEdit::Expiry(None)));
    assert!(settle(&mut source).is_empty());
    assert_eq!(patches(&fake.fake)[1].1["expires_in_hours"], Value::Null);
}

#[test]
fn a_link_password_is_sent_once_and_a_too_short_one_is_refused_without_a_request() {
    let fake = fake("link-password");
    let mut source = EngineSharedSource::new(signed_in(&fake));
    opened(&mut source);
    let typed = "an invented link phrase";
    let edit = LinkEdit::Password(Some(Zeroizing::new(typed.into())));
    assert!(!format!("{edit:?}").contains(typed), "a password is never named");
    assert!(source.change_link(REQ, edit));
    assert!(settle(&mut source).is_empty());
    let sent = patches(&fake.fake);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1["passphrase"], json!(typed));

    // Too short: the engine refuses it, nothing is sent, and the view says the change was not made.
    assert!(source.change_link(REQ, LinkEdit::Password(Some(Zeroizing::new("x".into())))));
    assert_eq!(settle(&mut source), vec![Notice::ChangeRefused]);
    assert_eq!(patches(&fake.fake).len(), 1);

    assert!(source.change_link(REQ, LinkEdit::Password(None)));
    assert!(settle(&mut source).is_empty());
    assert_eq!(patches(&fake.fake)[1].1["passphrase"], Value::Null);
}

#[test]
fn change_expiry_on_a_request_goes_out_once_by_id() {
    let fake = fake("request-expiry");
    let mut source = EngineSharedSource::new(signed_in(&fake));
    opened(&mut source);
    assert!(!source.change_request_expiry(INV_A, Some(24)), "not a request");
    assert!(source.change_request_expiry(REQ, Some(24)));
    assert!(settle(&mut source).is_empty());
    assert_eq!(changes(&fake.fake), vec![format!("PATCH /api/v1/file-requests/{REQ}")]);
}

#[test]
fn copy_link_asks_only_for_listed_rows_and_a_refusal_hands_no_link_to_take_link() {
    let fake = fake("copy-link");
    let mut source = EngineSharedSource::new(signed_in(&fake));
    assert!(!source.copy_row_link(REQ), "before the lists are here");
    opened(&mut source);
    assert!(!source.copy_row_link(OTHERS), "not a link or request row");
    assert!(source.take_link().is_none());
    // The fake holds no key for this file: the engine refuses, the view says so, and nothing reaches the clipboard.
    assert!(source.copy_row_link(REQ));
    assert!(!source.copy_row_link(REQ), "an answer is on its way");
    assert_eq!(settle(&mut source), vec![Notice::ChangeRefused]);
    assert!(source.take_link().is_none());
}

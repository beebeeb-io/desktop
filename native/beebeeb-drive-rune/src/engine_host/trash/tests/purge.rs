//! Empty Trash and Delete permanently on the real engine (`EngineTrashSource::purge`), against the engine's fake
//! account on loopback: the password is proven with real OPAQUE for every item, one single-use token deletes one
//! item, a wrong password deletes nothing, a refusal is told with what is still in Trash, and an answer that lands
//! under another session touches nothing. Every name is invented.

use std::sync::atomic::{AtomicU8, AtomicUsize};

use beebeeb_engine::test_api::ConfirmTokens;

use super::*;

const OK: u8 = 0;
const REFUSE: u8 = 1;

/// The fake's permanent delete: the two step-up legs (the account's own OPAQUE record), the single-use tokens, and
/// the ids it deleted for good.
#[derive(Clone)]
pub(super) struct PurgeFake {
    account: Arc<FakeAccount>,
    tokens: Arc<ConfirmTokens>,
    purged: Arc<Mutex<Vec<String>>>,
    minted: Arc<AtomicUsize>,
    mode: Arc<AtomicU8>,
}

impl PurgeFake {
    pub(super) fn new(account: Arc<FakeAccount>) -> Self {
        PurgeFake {
            account,
            tokens: Arc::default(),
            purged: Arc::default(),
            minted: Arc::default(),
            mode: Arc::new(AtomicU8::new(OK)),
        }
    }

    /// The answer to a step-up leg or a permanent delete of the first account; `None`: not one.
    pub(super) fn answer(&self, req: &Request) -> Option<Reply> {
        let route = req.route();
        if req.bearer() != Some(self.account.session_token.as_str()) {
            return None;
        }
        match route.as_str() {
            "POST /api/v1/auth/confirm-opaque-start" => Some(self.account.opaque.login_start(&req.json())),
            "POST /api/v1/auth/confirm-opaque-finish" => Some(if self.account.opaque.login_finish(&req.json()) {
                self.minted.fetch_add(1, Ordering::SeqCst);
                Reply::ok(json!({ "confirmation_token": self.tokens.mint(), "expires_at": "soon" }))
            } else {
                Reply::json(401, json!({ "error": "invalid_credentials" }))
            }),
            _ => {
                let id = route
                    .strip_prefix("DELETE /api/v1/files/")?
                    .strip_suffix("/permanent")?
                    .to_string();
                if self.mode.load(Ordering::SeqCst) == REFUSE {
                    return Some(Reply::json(500, json!({ "error": "a server body" })));
                }
                if !self.tokens.spend(req) {
                    return Some(Reply::json(403, json!({ "error": "step_up_required" })));
                }
                self.purged.lock().unwrap().push(id);
                Some(Reply::empty(204))
            }
        }
    }

    /// The listing's items without the ones deleted for good.
    pub(super) fn without_purged(&self, items: &Value) -> Value {
        let gone = self.purged.lock().unwrap();
        let kept: Vec<Value> = items
            .as_array()
            .map(|all| {
                all.iter()
                    .filter(|item| !item["id"].as_str().is_some_and(|id| gone.iter().any(|g| g == id)))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        Value::Array(kept)
    }
}

/// Pump until nothing is on its way; every purge end and notice told on the way.
fn settle_purge(source: &mut EngineTrashSource<MemoryCredentials>) -> (Vec<PurgeEnd>, Vec<Notice>) {
    let began = Instant::now();
    let (mut ends, mut notices) = (Vec::new(), Vec::new());
    loop {
        let pumped = source.pump();
        ends.extend(pumped.purge);
        notices.extend(pumped.notice);
        if !pumped.busy && !source.busy() && !source.purging() {
            return (ends, notices);
        }
        assert!(began.elapsed() < Duration::from_secs(20), "an answer did not come back");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn delete_permanently_deletes_that_one_item_after_the_password_and_nothing_else() {
    let fake = fake("purge-one");
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine);
    opened(&mut source);
    assert!(source.purge_available() && source.purge(Some(CONTRACT), PASSWORD));
    assert!(source.purging(), "the button waits while it is on its way");
    let (ends, notices) = settle_purge(&mut source);
    assert_eq!(ends, vec![PurgeEnd::Done]);
    assert!(notices.is_empty(), "{notices:?}");
    assert_eq!(*fake.purge.purged.lock().unwrap(), vec![CONTRACT.to_string()]);
    assert_eq!(fake.purge.minted.load(Ordering::SeqCst), 1, "one proof for one item");
    assert_eq!(names(&source), ["IMG_0001.HEIC", "Invented old invoices"]);
}

#[test]
fn empty_trash_deletes_every_listed_item_with_one_proof_each() {
    let fake = fake("purge-all");
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine);
    opened(&mut source);
    assert!(source.purge(None, PASSWORD));
    let (ends, notices) = settle_purge(&mut source);
    assert_eq!(ends, vec![PurgeEnd::Done]);
    assert!(notices.is_empty(), "{notices:?}");
    let mut purged = fake.purge.purged.lock().unwrap().clone();
    purged.sort();
    assert_eq!(purged, vec![DOCS.to_string(), CONTRACT.to_string(), PHOTO.to_string()]);
    assert_eq!(fake.purge.minted.load(Ordering::SeqCst), 3);
    assert!(names(&source).is_empty());
    assert_eq!(source.loaded(), Loaded::Ready);
    assert!(source.emptying().is_none(), "nothing is on its way any more");
}

#[test]
fn a_wrong_password_deletes_nothing_and_leaves_every_row() {
    let fake = fake("purge-wrong");
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine);
    opened(&mut source);
    assert!(source.purge(None, "not the password"));
    let (ends, notices) = settle_purge(&mut source);
    assert_eq!(ends, vec![PurgeEnd::WrongPassword]);
    assert!(notices.is_empty(), "the dialog says it, not the list: {notices:?}");
    assert!(fake.purge.purged.lock().unwrap().is_empty());
    assert_eq!(names(&source).len(), 3);
    // Nothing started with no password, and nothing for an item that is not listed.
    assert!(!source.purge(None, ""));
    assert!(!source.purge(Some(KEPT), PASSWORD));
    assert!(!source.purging());
}

#[test]
fn a_refusal_stops_it_says_what_is_left_and_deletes_nothing_it_did_not() {
    let fake = fake("purge-refused");
    fake.purge.mode.store(REFUSE, Ordering::SeqCst);
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine);
    opened(&mut source);
    assert!(source.purge(None, PASSWORD));
    let (ends, notices) = settle_purge(&mut source);
    assert_eq!(ends, vec![PurgeEnd::Stopped]);
    assert_eq!(
        notices,
        vec![Notice::PurgeStopped {
            done: 0,
            why: "Beebeeb did not delete them.".into()
        }]
    );
    assert_eq!(names(&source).len(), 3, "every item is still in Trash");
}

#[test]
fn an_answer_that_lands_under_another_session_touches_no_row_and_tells_nothing() {
    let fake = fake("purge-late");
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine.clone());
    opened(&mut source);
    assert!(source.purge(None, PASSWORD));
    // The session ends while the delete is on its way.
    engine.sign_out_keeping_local_data().expect("signed out");
    let (mut ends, mut notices) = settle_purge(&mut source);
    // Give the worker's answer time to land, then take it in.
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(10));
        let pumped = source.pump();
        ends.extend(pumped.purge);
        notices.extend(pumped.notice);
    }
    assert!(ends.is_empty(), "nothing is told to a window that moved on: {ends:?}");
    assert!(notices.is_empty(), "{notices:?}");
    assert!(names(&source).is_empty(), "no name of a session that ended");
}

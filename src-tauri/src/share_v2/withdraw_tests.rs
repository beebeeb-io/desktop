//! Task 2440 item 4: a withdrawal the server never answered is written to the state DB and retried
//! once on the next share-list refresh, until the server answers. The rule is the one
//! `safety::send_withdrawal` already keeps: 200 and 404 are answers, everything else is not.
//!
//! No server here: a fake answer (`HttpResponse::new`, or a transport error) drives the same
//! decision the command and the refresh use.
use beebeeb_core::auth_flow::HttpResponse;
use serde_json::json;

use super::flows::{Withdrawal, note_withdrawal_pending, withdrawal_answer, withdrawal_outcome};
use super::reads::retry_pending_withdrawals;
use crate::state_db::StateDb;

const OWN: &str = "0b3c7a52-5d0e-4f1a-9d2e-1c6f3a8b9e01";
const OTHER: &str = "7f9e2d14-3a6b-4c8d-8e1f-2b4a6c8d0e12";
const INVITE: &str = "6f1c2b9e-1d1a-4c55-9a51-3f7a2b8c9d01";
const INVITE_2: &str = "1a2b3c4d-5e6f-4a1b-8c2d-3e4f5a6b7c8d";

fn db() -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    (dir, db)
}

/// The one status rule: 2xx and 404 are the server's answer, 5xx and a transport error are not, and
/// every other status is the server refusing (reported, never queued).
#[test]
fn only_a_missing_answer_is_worth_retrying() {
    let mut checked = 0;
    for (status, want) in [
        (200, Withdrawal::Answered),
        (201, Withdrawal::Answered),
        (204, Withdrawal::Answered),
        (404, Withdrawal::Answered),
        (500, Withdrawal::Retry),
        (502, Withdrawal::Retry),
        (503, Withdrawal::Retry),
        (400, Withdrawal::Refused),
        (401, Withdrawal::Refused),
        (403, Withdrawal::Refused),
        (409, Withdrawal::Refused),
        (422, Withdrawal::Refused),
        (429, Withdrawal::Refused),
    ] {
        assert_eq!(withdrawal_answer(status), want, "{status}");
        checked += 1;
    }
    assert_eq!(checked, 13);
    let err: super::Answer<HttpResponse> = Err("network".into());
    assert_eq!(withdrawal_outcome(&err), Withdrawal::Retry);
    assert_eq!(
        withdrawal_outcome(&Ok(HttpResponse::new(500, json!({})))),
        Withdrawal::Retry
    );
    assert_eq!(
        withdrawal_outcome(&Ok(HttpResponse::new(404, json!({})))),
        Withdrawal::Answered
    );
}

/// A dropped withdrawal (transport error) and a 5xx are recorded under the account and invite; a
/// refusal is not. The record holds no secret: the value is empty.
#[test]
fn a_dropped_withdrawal_is_recorded_under_its_account() {
    let (_dir, db) = db();
    let dropped: super::Answer<HttpResponse> = Err("network".into());
    let five_hundred: super::Answer<HttpResponse> = Ok(HttpResponse::new(503, json!({})));
    let refused: super::Answer<HttpResponse> = Ok(HttpResponse::new(403, json!({})));
    let mut recorded = 0;
    for result in [&dropped, &five_hundred] {
        if withdrawal_outcome(result) == Withdrawal::Retry {
            note_withdrawal_pending(Some(&db), Some(OWN), INVITE);
            recorded += 1;
        }
    }
    assert_eq!(recorded, 2);
    assert_eq!(withdrawal_outcome(&refused), Withdrawal::Refused, "not recorded");
    assert_eq!(db.share_withdraw_pending(OWN).unwrap(), vec![INVITE.to_string()]);
    assert!(db.is_share_withdraw_pending(OWN, INVITE).unwrap());
    assert!(
        !db.is_share_withdraw_pending(OTHER, INVITE).unwrap(),
        "another account never sees this cancellation"
    );
    assert_eq!(
        db.share_withdraw_pending(OTHER).unwrap().len(),
        0,
        "and its refresh retries nothing of ours"
    );
}

/// The next refresh retries the record once and drops it on 200 and on 404.
#[tokio::test]
async fn the_next_refresh_drops_the_record_on_200_and_on_404() {
    let (_dir, db) = db();
    for status in [200u16, 404] {
        note_withdrawal_pending(Some(&db), Some(OWN), INVITE);
        let mut calls = 0;
        retry_pending_withdrawals(&db, OWN, |req| {
            calls += 1;
            assert_eq!(req.path, format!("/api/v1/shares/invites/{INVITE}"));
            async move { Ok(HttpResponse::new(status, json!({}))) }
        })
        .await;
        assert_eq!(calls, 1, "{status}: one attempt per refresh");
        assert!(
            !db.is_share_withdraw_pending(OWN, INVITE).unwrap(),
            "{status}: the server answered, the record is done"
        );
    }
    assert_eq!(db.share_withdraw_pending(OWN).unwrap().len(), 0);
}

/// A retry that still gets no answer keeps the record, clears nothing else, and never fails the
/// list: the refresh goes on to its four fetches.
#[tokio::test]
async fn a_still_failing_retry_keeps_the_record_and_the_list_goes_on() {
    let (_dir, db) = db();
    note_withdrawal_pending(Some(&db), Some(OWN), INVITE);
    note_withdrawal_pending(Some(&db), Some(OWN), INVITE_2);
    let mut calls = 0;
    retry_pending_withdrawals(&db, OWN, |_req| {
        calls += 1;
        async { Err("network".to_string()) }
    })
    .await;
    assert_eq!(calls, 2, "every pending cancellation is tried once");
    let mut pending = db.share_withdraw_pending(OWN).unwrap();
    pending.sort();
    assert_eq!(
        pending,
        vec![INVITE_2.to_string(), INVITE.to_string()],
        "both records stay for the next refresh"
    );

    // The next refresh's 500 answers keep the records too; the calls before it answered only one.
    retry_pending_withdrawals(&db, OWN, |req| {
        let status = if req.path.ends_with(INVITE) { 200 } else { 500 };
        async move { Ok(HttpResponse::new(status, json!({}))) }
    })
    .await;
    assert_eq!(
        db.share_withdraw_pending(OWN).unwrap(),
        vec![INVITE_2.to_string()],
        "the answer drops one, the 5xx keeps the other"
    );
}

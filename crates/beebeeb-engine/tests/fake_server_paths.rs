//! The fake server's permanent delete, sessions and sharing routes, driven by the engine's own client calls
//! (`purge_one`, `purge_many`, `ApiClient`, core's `share_flow` requests): each write changes what the next read lists.
//! Every account, id and password here is invented.

use std::sync::Arc;

use beebeeb_core::auth_flow::{HttpRequest, Method};
use beebeeb_core::share_flow;
use beebeeb_engine::api_client::ApiClient;
use beebeeb_engine::app::trash_bulk_api::purge_many;
use beebeeb_engine::app::trash_purge_api::{PurgeError, purge_one};
use beebeeb_engine::auth_transport::{client_for, send_to};
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, start_fake};
use serde_json::{Value, json};

const PASSWORD: &str = "an invented password";

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn start() -> (Arc<FakeAccount>, FakeApi) {
    let (account, _phrase) = FakeAccount::register("fake-paths@example.invalid", PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let fake = start_fake(account.clone());
    (account, fake)
}

fn api(fake: &FakeApi, account: &FakeAccount) -> ApiClient {
    ApiClient::new(fake.base.clone(), account.session_token.clone(), account.master_key)
}

fn id(n: u8) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}

/// Send a request of core's builders (or the same method and path as `shares_api`) with the account's session.
fn send(fake: &FakeApi, token: &str, req: HttpRequest) -> (u16, Value) {
    let client = client_for(&fake.base).unwrap();
    let resp = runtime()
        .block_on(send_to(&fake.base, &client, Some(token), req))
        .expect("the fake answered");
    (resp.status, resp.body)
}

fn call(method: Method, path: String, body: Option<Value>) -> HttpRequest {
    let mut req = HttpRequest::session_post(path, Value::Null);
    req.method = method;
    req.body = body;
    req
}

fn trashed_ids(fake: &FakeApi, account: &FakeAccount) -> Vec<String> {
    let listed = runtime()
        .block_on(async {
            let client = client_for(&fake.base).unwrap();
            let req = call(Method::Get, "/api/v1/files?trashed=true".into(), None);
            send_to(&fake.base, &client, Some(&account.session_token), req).await
        })
        .unwrap();
    let mut ids: Vec<String> = listed.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

// ── Permanent delete ────────────────────────────────────────────────────────────

#[test]
fn a_permanent_delete_removes_the_item_and_lowers_the_count() {
    let (account, fake) = start();
    let (a, b) = (id(1), id(2));
    account.add_file_with_content(&a, None, "a.txt", b"a");
    account.add_file_with_content(&b, None, "b.txt", b"b");
    assert!(account.trash_item(&a) && account.trash_item(&b));
    let client = client_for(&fake.base).unwrap();
    let rt = runtime();
    assert_eq!(rt.block_on(api(&fake, &account).trashed_file_count()).unwrap(), 2);

    rt.block_on(purge_one(
        &fake.base,
        &client,
        &account.session_token,
        &a,
        PASSWORD.as_bytes(),
    ))
    .expect("deleted for good");
    assert_eq!(trashed_ids(&fake, &account), vec![b.clone()], "a is gone from Trash");
    assert_eq!(
        rt.block_on(api(&fake, &account).trashed_file_count()).unwrap(),
        1,
        "count down"
    );
    assert!(!account.live_file_ids().contains(&a));
}

#[test]
fn a_wrong_password_deletes_nothing() {
    let (account, fake) = start();
    let a = id(1);
    account.add_file_with_content(&a, None, "a.txt", b"a");
    assert!(account.trash_item(&a));
    let client = client_for(&fake.base).unwrap();
    let refused = runtime().block_on(purge_one(
        &fake.base,
        &client,
        &account.session_token,
        &a,
        b"not the password",
    ));
    assert_eq!(refused, Err(PurgeError::WrongPassword));
    assert_eq!(trashed_ids(&fake, &account), vec![a], "still in Trash");
}

#[test]
fn a_delete_without_a_confirmation_token_or_another_token_is_refused() {
    let (account, fake) = start();
    let a = id(1);
    account.add_file_with_content(&a, None, "a.txt", b"a");
    assert!(account.trash_item(&a));
    let path = format!("/api/v1/files/{a}/permanent");
    let (status, _) = send(&fake, &account.session_token, call(Method::Delete, path.clone(), None));
    assert_eq!(status, 403, "no confirmation token");
    let (status, _) = send(&fake, "another-accounts-token", call(Method::Delete, path, None));
    assert_eq!(status, 401);
    assert_eq!(trashed_ids(&fake, &account), vec![a], "nothing was deleted");
}

#[test]
fn a_live_item_is_not_deleted_for_good() {
    let (account, fake) = start();
    let a = id(1);
    account.add_file_with_content(&a, None, "a.txt", b"a");
    let client = client_for(&fake.base).unwrap();
    let refused = runtime().block_on(purge_one(
        &fake.base,
        &client,
        &account.session_token,
        &a,
        PASSWORD.as_bytes(),
    ));
    assert_eq!(refused, Err(PurgeError::Failed), "409 not_trashed");
    assert!(account.live_file_ids().contains(&a));
}

#[test]
fn a_bulk_delete_removes_the_trashed_and_keeps_a_folder_with_live_items() {
    let (account, fake) = start();
    let (clean, holder, inside, loose) = (id(1), id(2), id(3), id(4));
    account.add_file_with_content(&clean, None, "clean.txt", b"c");
    account.add_item(&holder, None, "holder", true, 0);
    account.add_file_with_content(&inside, Some(&holder), "inside.txt", b"i");
    account.add_file_with_content(&loose, None, "loose.txt", b"l");
    for trashed in [&clean, &holder, &loose] {
        assert!(account.trash_item(trashed));
    }
    let client = client_for(&fake.base).unwrap();
    let answer = runtime().block_on(purge_many(
        &fake.base,
        &client,
        &account.session_token,
        &[clean.clone(), holder.clone(), loose.clone()],
        PASSWORD.as_bytes(),
    ));
    assert_eq!(answer.error, None);
    let mut deleted = answer.deleted.clone();
    deleted.sort();
    assert_eq!(deleted, vec![clean.clone(), loose.clone()], "the clean ones are gone");
    assert_eq!(answer.kept.len(), 1, "the folder with a live item is kept");
    assert_eq!(answer.kept[0].id, holder);
    assert_eq!(
        trashed_ids(&fake, &account),
        vec![holder],
        "only the kept folder is left"
    );
    assert!(account.live_file_ids().contains(&inside), "the live item survived");
}

#[test]
fn a_folder_goes_with_its_trashed_subtree() {
    let (account, fake) = start();
    let (folder, child) = (id(1), id(2));
    account.add_item(&folder, None, "folder", true, 0);
    account.add_file_with_content(&child, Some(&folder), "child.txt", b"c");
    assert!(account.trash_item(&child) && account.trash_item(&folder));
    let client = client_for(&fake.base).unwrap();
    runtime()
        .block_on(purge_one(
            &fake.base,
            &client,
            &account.session_token,
            &folder,
            PASSWORD.as_bytes(),
        ))
        .unwrap();
    assert!(
        trashed_ids(&fake, &account).is_empty(),
        "the folder and its child are both gone"
    );
}

// ── Sessions ────────────────────────────────────────────────────────────────────

#[test]
fn a_revoked_session_leaves_the_list_and_the_others_can_all_be_revoked() {
    let (account, fake) = start();
    account.serve_security_routes(true);
    account.add_session("fixture-session-2", "Fixture phone");
    account.add_session("fixture-session-3", "Fixture tablet");
    let (client, rt) = (api(&fake, &account), runtime());
    let listed = rt.block_on(client.account_sessions()).unwrap();
    assert_eq!(listed.sessions.len(), 3);
    assert_eq!(listed.sessions.iter().filter(|s| s.is_current).count(), 1);

    rt.block_on(client.revoke_account_session("fixture-session-2")).unwrap();
    let ids: Vec<_> = rt
        .block_on(client.account_sessions())
        .unwrap()
        .sessions
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids, vec!["fixture-session-1", "fixture-session-3"]);
    assert!(
        rt.block_on(client.revoke_account_session("no-such-session")).is_err(),
        "404"
    );

    let all = rt.block_on(client.revoke_other_sessions()).unwrap();
    assert_eq!(all.revoked, 1);
    assert_eq!(
        account.session_ids(),
        vec!["fixture-session-1"],
        "only the current one is left"
    );
    assert!(
        rt.block_on(client.revoke_account_session("fixture-session-1")).is_err(),
        "the current session is not revoked from here"
    );
}

// ── Sharing ─────────────────────────────────────────────────────────────────────

#[test]
fn an_incoming_invite_is_declined_or_claimed() {
    let (account, fake) = start();
    let (declined, claimed) = (id(1), id(2));
    account.add_incoming_invite(&declined, &id(10), "sender@example.invalid");
    account.add_incoming_invite(&claimed, &id(11), "sender@example.invalid");
    let incoming = |fake: &FakeApi| {
        send(
            fake,
            &account.session_token,
            call(Method::Get, "/api/v1/shares/invites/incoming".into(), None),
        )
        .1
    };
    assert_eq!(incoming(&fake)["invites"].as_array().unwrap().len(), 2);

    let (status, _) = send(
        &fake,
        &account.session_token,
        call(Method::Post, format!("/api/v1/shares/invites/{declined}/decline"), None),
    );
    assert_eq!(status, 200);
    assert_eq!(account.incoming_invite_status(&declined), None, "declined: gone");
    let (status, _) = send(
        &fake,
        &account.session_token,
        call(Method::Post, format!("/api/v1/shares/invites/{claimed}/claim"), None),
    );
    assert_eq!(status, 200);
    assert_eq!(account.incoming_invite_status(&claimed).as_deref(), Some("claimed"));
    let (again, _) = send(
        &fake,
        &account.session_token,
        call(Method::Post, format!("/api/v1/shares/invites/{claimed}/claim"), None),
    );
    assert_eq!(again, 400, "an invite already claimed");
    let (missing, _) = send(
        &fake,
        &account.session_token,
        call(Method::Post, format!("/api/v1/shares/invites/{}/decline", id(99)), None),
    );
    assert_eq!(missing, 404);
}

#[test]
fn a_sent_invite_changes_access_and_is_cancelled() {
    let (account, fake) = start();
    let invite = id(1);
    account.add_sent_invite(&invite, &id(10), "friend@example.invalid");
    let client = api(&fake, &account);
    let sent = runtime().block_on(client.sent_folder_invites()).unwrap();
    assert_eq!(sent["invites"][0]["recipient_email"], "friend@example.invalid");

    let access = beebeeb_core::share_flow::access::set_member_access_request(
        &invite,
        beebeeb_core::share_flow::access::MemberPermission::parse("write").unwrap(),
    )
    .unwrap();
    assert_eq!(send(&fake, &account.session_token, access).0, 200);
    assert_eq!(
        account.sent_invite_state(&invite),
        Some(("invited".into(), "write".into()))
    );

    let cancel = share_flow::cancel_invite_request(&invite).unwrap();
    assert_eq!(send(&fake, &account.session_token, cancel).0, 200);
    assert_eq!(account.sent_invite_state(&invite), None, "cancelled: gone");
    let sent = runtime().block_on(client.sent_folder_invites()).unwrap();
    assert!(sent["invites"].as_array().unwrap().is_empty());
}

#[test]
fn a_file_request_is_closed_then_hidden() {
    let (account, fake) = start();
    let request = id(1);
    account.add_file_request(&request, "Tax papers", 5);
    let client = api(&fake, &account);
    let listed = runtime().block_on(client.list_file_requests()).unwrap();
    assert_eq!(listed["file_requests"][0]["title"], "Tax papers");
    assert_eq!(account.file_request_flag(&request, "closed"), Some(false));

    let close = beebeeb_core::share_flow::request::close_request(&request).unwrap();
    assert_eq!(send(&fake, &account.session_token, close).0, 200);
    assert_eq!(account.file_request_flag(&request, "closed"), Some(true));
    let hide = call(Method::Post, format!("/api/v1/file-requests/{request}/hide"), None);
    assert_eq!(send(&fake, &account.session_token, hide).0, 200);
    assert_eq!(account.file_request_flag(&request, "hidden"), Some(true));
    let none = call(Method::Post, format!("/api/v1/file-requests/{}/close", id(99)), None);
    assert_eq!(send(&fake, &account.session_token, none).0, 404);
}

#[test]
fn a_share_link_is_revoked_and_listed_as_revoked() {
    let (account, fake) = start();
    let file = id(1);
    account.add_file_with_content(&file, None, "f.txt", b"f");
    let made = send(
        &fake,
        &account.session_token,
        call(
            Method::Post,
            "/api/v1/shares".into(),
            Some(json!({ "file_id": file, "token": "t" })),
        ),
    );
    assert_eq!(made.0, 200);
    let link = made.1["id"].as_str().unwrap().to_string();
    let revoke = share_flow::revoke_link_request(&link).unwrap();
    assert_eq!(send(&fake, &account.session_token, revoke).0, 200);
    assert_eq!(account.links()[0]["revoked"], true);
    let again = share_flow::revoke_link_request(&id(99)).unwrap();
    assert_eq!(send(&fake, &account.session_token, again).0, 404);
}

#[test]
fn a_shared_folders_custody_record_is_stored_and_read_back() {
    let (account, fake) = start();
    let folder = id(1);
    let (client, rt) = (api(&fake, &account), runtime());
    assert!(
        rt.block_on(client.get_shared_folder(&folder)).is_err(),
        "404 until stored"
    );
    rt.block_on(client.register_shared_folder_key(&folder, "wrapped-key", 1, Some("l7")))
        .unwrap();
    let read = rt.block_on(client.get_shared_folder(&folder)).unwrap();
    assert_eq!(read["owner_wrapped_folder_key"], "wrapped-key");
    assert_eq!(read["key_version"], 1);
    let keys = rt.block_on(client.get_shared_folder_keys(&folder, None)).unwrap();
    assert!(keys["keys"].as_array().unwrap().is_empty());
    let stranger = ApiClient::new(fake.base.clone(), "another-accounts-token".into(), account.master_key);
    assert!(rt.block_on(stranger.get_shared_folder(&folder)).is_err(), "401");
}

// ── The rest of the account routes (two-factor, activity, region, resend, deletion, health, speedtest) ──────────

fn get(fake: &FakeApi, account: &FakeAccount, path: &str) -> (u16, Value) {
    send(fake, &account.session_token, call(Method::Get, path.into(), None))
}

fn post(fake: &FakeApi, account: &FakeAccount, path: &str, body: Value) -> (u16, Value) {
    send(
        fake,
        &account.session_token,
        call(Method::Post, path.into(), Some(body)),
    )
}

#[test]
fn the_rest_routes_stay_404_until_the_security_routes_are_on() {
    let (account, fake) = start();
    assert_eq!(get(&fake, &account, "/api/v1/health").0, 404);
    assert_eq!(get(&fake, &account, "/api/v1/account/activity").0, 404);
    account.serve_security_routes(true);
    assert_eq!(get(&fake, &account, "/api/v1/health").0, 200);
    let (status, _) = send(
        &fake,
        "another-accounts-token",
        call(Method::Get, "/api/v1/account/activity".into(), None),
    );
    assert_eq!(status, 401);
}

#[test]
fn two_factor_turns_on_with_the_first_code_and_off_with_a_current_one() {
    let (account, fake) = start();
    account.serve_security_routes(true);
    let (status, setup) = post(&fake, &account, "/api/v1/auth/2fa/setup", Value::Null);
    assert_eq!((status, setup["backup_codes"].as_array().unwrap().len()), (200, 3));
    assert!(setup["qr_uri"].as_str().unwrap().starts_with("otpauth://"));
    assert_eq!(
        post(&fake, &account, "/api/v1/auth/2fa/enable", json!({ "code": "000000" })).0,
        401
    );
    assert_eq!(get(&fake, &account, "/api/v1/me/two-factor").1["enabled"], false);
    assert_eq!(
        post(&fake, &account, "/api/v1/auth/2fa/enable", json!({ "code": "123456" })).0,
        200
    );
    assert_eq!(get(&fake, &account, "/api/v1/me/two-factor").1["enabled"], true);
    // On already: a new set-up needs the current code (or a step-up token).
    assert_eq!(post(&fake, &account, "/api/v1/auth/2fa/setup", Value::Null).0, 403);
    assert_eq!(
        post(&fake, &account, "/api/v1/auth/2fa/setup", json!({ "code": "123456" })).0,
        200
    );
    let (status, codes) = post(
        &fake,
        &account,
        "/api/v1/auth/2fa/backup-codes",
        json!({ "code": "123456" }),
    );
    assert_eq!((status, codes["backup_codes"].as_array().unwrap().len()), (200, 3));
    assert_eq!(
        get(&fake, &account, "/api/v1/account/security-score").1["score"],
        5,
        "two-factor is the fifth factor"
    );
    assert_eq!(
        post(&fake, &account, "/api/v1/auth/2fa/disable", json!({ "code": "123456" })).0,
        200
    );
    assert_eq!(get(&fake, &account, "/api/v1/me/two-factor").1["enabled"], false);
}

#[test]
fn the_second_factor_step_of_a_sign_in_takes_the_code_and_the_partial_token() {
    let (account, fake) = start();
    account.serve_security_routes(true);
    let body = |code: &str| json!({ "partial_token": "partial", "code": code });
    let (status, ok) = send(
        &fake,
        "",
        call(Method::Post, "/api/v1/auth/2fa/verify".into(), Some(body("123456"))),
    );
    assert_eq!(
        (status, ok["session_token"] == account.session_token.as_str()),
        (200, true)
    );
    let (status, _) = send(
        &fake,
        "",
        call(Method::Post, "/api/v1/auth/2fa/verify".into(), Some(body("999999"))),
    );
    assert_eq!(status, 401);
}

#[test]
fn the_activity_score_region_and_flag_routes_read_in_the_clients_shapes() {
    let (account, fake) = start();
    account.serve_security_routes(true);
    let (client, rt) = (api(&fake, &account), runtime());
    let activity = rt.block_on(client.account_activity()).unwrap();
    assert_eq!(activity.events.len(), 1);
    assert_eq!(activity.summary.active_sessions, 1);
    let score = rt.block_on(client.security_score()).unwrap();
    assert_eq!((score.score, score.factors.len()), (4, 5));
    let feed = rt.block_on(client.activity_feed(Some(1), Some(1))).unwrap();
    assert_eq!((feed.events.len(), feed.total), (1, 2));
    let page2 = rt.block_on(client.activity_feed(Some(2), Some(1))).unwrap();
    assert_eq!(page2.events[0].id, "fixture-feed-2");

    let region = rt.block_on(client.account_region()).unwrap();
    assert_eq!(region.preferred_region.as_deref(), Some("europe"));
    assert_eq!(region.choices.len(), 2);
    rt.block_on(client.set_preferred_region(Some("de"), Some(true)))
        .unwrap();
    let region = rt.block_on(client.account_region()).unwrap();
    assert_eq!(
        (region.preferred_region.as_deref(), region.region_strict),
        (Some("de"), true)
    );

    let (status, flags) = get(&fake, &account, "/api/v1/feature-flags/evaluate");
    assert_eq!(
        (status, flags["flags"]["shared_folders_v1"].clone()),
        (200, json!(true))
    );
}

#[test]
fn an_email_code_is_sent_again_three_times_and_then_refused() {
    let (account, fake) = start();
    account.serve_security_routes(true);
    assert_eq!(post(&fake, &account, "/api/v1/me/email/resend", Value::Null).0, 404);
    account.start_email_change("new-address@example.invalid");
    for left in [2u64, 1, 0] {
        let (status, body) = post(&fake, &account, "/api/v1/me/email/resend", Value::Null);
        assert_eq!(status, 200);
        assert_eq!(body["pending_email"], "new-address@example.invalid");
        assert_eq!(body["resends_left"], left);
    }
    let (status, body) = post(&fake, &account, "/api/v1/me/email/resend", Value::Null);
    assert_eq!((status, body["error"].as_str()), (429, Some("email_code_resend_limit")));
}

#[test]
fn the_deletion_state_follows_what_the_test_set() {
    let (account, fake) = start();
    account.serve_security_routes(true);
    for state in ["none", "requested", "scheduled"] {
        account.set_deletion_state(state);
        let (status, body) = get(&fake, &account, "/api/v1/auth/account/deletion");
        assert_eq!((status, body["state"].as_str()), (200, Some(state)));
    }
    assert!(get(&fake, &account, "/api/v1/auth/account/deletion").1["cancel_until"].is_string());
}

#[test]
fn health_and_the_speedtest_answer_for_the_clients_latency_and_speed_checks() {
    let (account, fake) = start();
    account.serve_security_routes(true);
    let (client, rt) = (api(&fake, &account), runtime());
    rt.block_on(client.ping_health()).unwrap();
    assert_eq!(rt.block_on(client.speedtest_download(100_000)).unwrap().len(), 100_000);
    rt.block_on(client.speedtest_upload(&[0u8; 5_000])).unwrap();
}

#[test]
fn a_links_expiry_and_password_change_in_place_and_a_revoked_or_unknown_link_is_not_found() {
    let (account, fake) = start();
    let file = id(1);
    account.add_file_with_content(&file, None, "f.txt", b"f");
    let made = send(
        &fake,
        &account.session_token,
        call(Method::Post, "/api/v1/shares".into(), Some(json!({ "file_id": file, "token": "t" }))),
    );
    let link = made.1["id"].as_str().unwrap().to_string();
    let path = format!("/api/v1/shares/{link}");
    let patch = |body: Value| send(&fake, &account.session_token, call(Method::Patch, path.clone(), Some(body)));

    let (status, body) = patch(json!({ "expires_in_hours": 24, "passphrase": "four" }));
    assert_eq!(status, 200);
    assert_eq!(body["has_passphrase"], true);
    assert!(body["expires_at"].is_string());
    // Absent is unchanged; null clears.
    let (_, body) = patch(json!({ "expires_in_hours": null }));
    assert_eq!((body["expires_at"].is_null(), body["has_passphrase"].clone()), (true, json!(true)));
    let (_, body) = patch(json!({ "passphrase": null }));
    assert_eq!(body["has_passphrase"], false);
    // Limits as create.
    assert_eq!(patch(json!({ "passphrase": "  a " })).0, 400);
    assert_eq!(patch(json!({ "expires_in_hours": 0 })).0, 400);
    assert_eq!(patch(json!({ "expires_in_hours": 87601 })).0, 400);

    assert_eq!(send(&fake, &account.session_token, share_flow::revoke_link_request(&link).unwrap()).0, 200);
    assert_eq!(patch(json!({ "expires_in_hours": 5 })).0, 404);
    let unknown = call(Method::Patch, format!("/api/v1/shares/{}", id(99)), Some(json!({})));
    assert_eq!(send(&fake, &account.session_token, unknown).0, 404);
}

#[test]
fn a_file_requests_expiry_changes_while_open_and_an_invite_is_left_once() {
    let (account, fake) = start();
    account.add_file_request(&id(5), "Invented request", 10);
    let path = format!("/api/v1/file-requests/{}", id(5));
    let patch = |body: Value| send(&fake, &account.session_token, call(Method::Patch, path.clone(), Some(body)));
    assert_eq!(patch(json!({ "expires_in_secs": 3600 })).0, 200);
    let listed = send(&fake, &account.session_token, call(Method::Get, "/api/v1/file-requests".into(), None)).1;
    assert!(listed["file_requests"][0]["expires_at"].is_string());
    assert_eq!(patch(json!({ "expires_in_secs": null })).0, 200);
    let listed = send(&fake, &account.session_token, call(Method::Get, "/api/v1/file-requests".into(), None)).1;
    assert!(listed["file_requests"][0]["expires_at"].is_null());
    let close = beebeeb_core::share_flow::request::close_request(&id(5)).unwrap();
    assert_eq!(send(&fake, &account.session_token, close).0, 200);
    assert_eq!(patch(json!({ "expires_in_secs": 60 })).0, 409);
    let missing = call(Method::Patch, format!("/api/v1/file-requests/{}", id(9)), Some(json!({ "expires_in_secs": 1 })));
    assert_eq!(send(&fake, &account.session_token, missing).0, 404);

    account.add_incoming_invite(&id(7), &id(8), "sender@example.eu");
    let leave = |n: u8| call(Method::Post, format!("/api/v1/shares/invites/{}/leave", id(n)), None);
    // Only an approved invite can be left.
    assert_eq!(send(&fake, &account.session_token, leave(7)).0, 404);
    account.approve_incoming_invite(&id(7));
    let (status, body) = send(&fake, &account.session_token, leave(7));
    assert_eq!((status, body["status"].clone()), (200, json!("left")));
    assert_eq!(account.incoming_invite_status(&id(7)), None);
    assert_eq!(send(&fake, &account.session_token, leave(7)).0, 404);
}

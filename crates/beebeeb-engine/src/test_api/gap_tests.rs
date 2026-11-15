//! The fake server's gaps the native checks on the build VM hit (task 2475 follow-up): the device and session
//! replies in the names the client reads, the `heartbeat-off` switch, the password-proof legs of the screen lock's
//! password unlock, and the account routes the Security page reads. Each runs the real client (or core's own OPAQUE
//! client side) against the real handler over loopback.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use base64::Engine as _;
use serde_json::{Value, json};

use super::fake_account::FakeAccount;
use super::server::{FIXTURE_PASSWORD, Switches, fixture_account, fixture_handler};
use super::{FakeApi, Request};
use crate::api_client::{ApiClient, HeartbeatBody};

fn fixture() -> Arc<FakeAccount> {
    static ACCOUNT: OnceLock<Arc<FakeAccount>> = OnceLock::new();
    ACCOUNT.get_or_init(fixture_account).clone()
}

/// The fixture behind the server's own handler (switches and all), on loopback.
fn serve(switches: Switches) -> (FakeApi, ApiClient, Arc<FakeAccount>) {
    let account = fixture();
    let handler = fixture_handler(
        account.clone(),
        Arc::new(|_: &str| {}),
        Arc::new(AtomicU64::new(0)),
        switches,
    );
    let api = FakeApi::start(handler);
    let client = ApiClient::new(api.base.clone(), account.session_token.clone(), [0; 32]);
    (api, client, account)
}

fn call(handler: &super::Handler, method: &str, path: &str, token: Option<&str>, body: Value) -> super::Reply {
    handler(&Request {
        method: method.into(),
        path: path.into(),
        headers: token
            .map(|t| vec![("Authorization".to_string(), format!("Bearer {t}"))])
            .unwrap_or_default(),
        body: body.to_string().into_bytes(),
    })
}

#[test]
fn the_device_and_session_replies_carry_the_names_the_client_reads() {
    let (_api, client, _account) = serve(Switches::default());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let outcome = runtime.block_on(async {
        let device = client.register_device("fixture-pc", "windows", "0.9.0").await?;
        let session = client
            .create_session(&device.id, "Beebeeb", "sync", Some("C:\\Beebeeb"), "/", 30)
            .await?;
        client
            .post_heartbeat(
                &session.id,
                &HeartbeatBody {
                    status: "watching".into(),
                    ..Default::default()
                },
            )
            .await?;
        let listed = client.client_devices().await?;
        anyhow::Ok((device.id, session.id, session.heartbeat_interval_secs, listed))
    });
    let (device_id, session_id, interval, listed) = outcome.expect("register, open a session, beat, list");
    assert!(!device_id.is_empty() && !session_id.is_empty());
    assert_eq!(interval, Some(30), "the interval the app asked for");
    assert_eq!(
        listed.devices.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
        [device_id.as_str()],
        "the list names the device by the same id"
    );
}

#[test]
fn heartbeat_off_refuses_the_device_route_and_nothing_else() {
    let switches = Switches::default();
    let (_api, client, account) = serve(switches.clone());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let before = runtime.block_on(client.register_device("fixture-pc", "windows", "0.9.0"));
    switches.heartbeat_off.store(true, Ordering::SeqCst);
    let after = runtime.block_on(client.register_device("fixture-pc", "windows", "0.9.0"));
    let handler = fixture_handler(
        account.clone(),
        Arc::new(|_: &str| {}),
        Arc::new(AtomicU64::new(0)),
        switches,
    );
    let token = Some(account.session_token.as_str());
    let device = call(
        &handler,
        "POST",
        "/api/v1/clients/devices",
        token,
        json!({ "hostname": "h" }),
    );
    let snapshot = call(&handler, "GET", "/api/v1/sync/snapshot", token, Value::Null);
    let me = call(&handler, "GET", "/api/v1/auth/me", token, Value::Null);
    assert_eq!(
        (
            before.is_ok(),
            after.is_err(),
            device.status,
            snapshot.status,
            me.status
        ),
        (true, true, 503, 200, 200),
        "registered before the switch, refused after it, the refusal's status, the sync and profile routes untouched"
    );
}

/// The client side of core's OPAQUE sign-in for `password`, against the password-proof legs: the finish status.
fn password_proof(handler: &super::Handler, token: &str, password: &str) -> (u16, Value) {
    let start = beebeeb_core::opaque_protocol::client_login_start(password.as_bytes()).expect("client start");
    let b64 = base64::engine::general_purpose::STANDARD;
    let first = call(
        handler,
        "POST",
        "/api/v1/auth/password-proof/start",
        Some(token),
        json!({ "client_message": b64.encode(&start.message) }),
    );
    assert_eq!(first.status, 200, "the start leg answers");
    let first: Value = serde_json::from_slice(&first.body).unwrap();
    let server_message = b64.decode(first["server_message"].as_str().unwrap()).unwrap();
    let ksf = first["ksf_version"].as_u64().unwrap() as u32;
    let finished =
        beebeeb_core::opaque_protocol::client_login_finish(&start.state, password.as_bytes(), &server_message, ksf);
    // A wrong password stops in the client (the server's message does not open): send garbage, as a refused proof.
    let message = finished
        .map(|f| b64.encode(f.message))
        .unwrap_or_else(|_| "AA==".into());
    let second = call(
        handler,
        "POST",
        "/api/v1/auth/password-proof/finish",
        Some(token),
        json!({ "client_message": message, "server_state": first["server_state"] }),
    );
    (
        second.status,
        serde_json::from_slice(&second.body).unwrap_or(Value::Null),
    )
}

#[test]
fn the_password_proof_legs_prove_the_fixture_password_and_refuse_another() {
    let account = fixture();
    let handler = fixture_handler(
        account.clone(),
        Arc::new(|_: &str| {}),
        Arc::new(AtomicU64::new(0)),
        Switches::default(),
    );
    let token = account.session_token.as_str();
    let right = password_proof(&handler, token, FIXTURE_PASSWORD);
    let wrong = password_proof(&handler, token, "not the fixture password");
    let no_token = call(
        &handler,
        "POST",
        "/api/v1/auth/password-proof/start",
        None,
        json!({ "client_message": "AA==" }),
    )
    .status;
    assert_eq!(
        (right.0, right.1["proven"].clone(), wrong.0, no_token),
        (200, json!(true), 403, 401),
        "right password: status and proven; wrong password: status; no token: status"
    );
}

#[test]
fn the_security_page_routes_answer_in_the_shapes_the_client_reads() {
    let (_api, client, _account) = serve(Switches::default());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (sessions, activity, name, two_factor) = runtime.block_on(async {
        (
            client.account_sessions().await,
            client.my_activity(20).await,
            client.account_display_name().await,
            client.two_factor_status().await,
        )
    });
    let sessions = sessions.expect("account/sessions");
    let activity = activity.expect("me/activity");
    let name = name.expect("me/profile");
    let two_factor = two_factor.expect("me/two-factor");
    assert_eq!(
        (
            sessions.sessions.len(),
            sessions.sessions.iter().filter(|s| s.is_current).count(),
            activity.opted_in,
            activity.events.is_empty(),
            name.is_some(),
            two_factor.enabled,
        ),
        (1, 1, true, false, true, false),
        "sessions, of them current, activity recorded, activity events none?, a display name, two-factor on"
    );
}

#[test]
fn an_account_that_does_not_serve_the_security_routes_leaves_them_404_and_two_factor_follows_its_state() {
    use super::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
    let (account, _) = FakeAccount::register("plain@example.invalid", "a password", TEST_PHRASE);
    let account = Arc::new(account);
    let handler = account_handler(account.clone());
    let token = Some(account.session_token.as_str());
    let routes = [
        "/api/v1/account/sessions",
        "/api/v1/me/activity",
        "/api/v1/me/profile",
        "/api/v1/me/two-factor",
    ];
    let status = |h: &super::Handler| -> Vec<u16> {
        routes
            .iter()
            .map(|r| call(h, "GET", r, token, Value::Null).status)
            .collect()
    };
    let off = status(&handler);
    account.serve_security_routes(true);
    let on = status(&handler);
    let factor = |h: &super::Handler| {
        serde_json::from_slice::<Value>(&call(h, "GET", "/api/v1/me/two-factor", token, Value::Null).body).unwrap()["enabled"]
            .clone()
    };
    let before = factor(&handler);
    account.set_two_factor(true);
    let after = factor(&handler);
    assert_eq!(
        (off, on, before, after),
        (vec![404; 4], vec![200; 4], json!(false), json!(true)),
        "statuses off, statuses on, two-factor before and after the account's own state changed"
    );
}

//! dc-086 scratch (diagnosis only, never merged): `bb-fake-server`'s fixture account seeded with
//! `LargeAccount::GUUS`, on a loopback port, for the Cloud Files measurement at 40k files on the Windows build VM.
//!
//! `dc086_large_fake 127.0.0.1:47486 [switch-file]`. It prints `base:`, `email:`, `password:` and `phrase:` on
//! stdout (fixtures that open nothing but this process), then one line per answer on stderr:
//! `<unix ms> <METHOD /path -> status> <ms the handler took>`. While `switch-file` exists, it also answers
//! `GET`/`PUT /api/v1/backups/root` (the stock fake answers 404: no backup pass can start against it).

use std::io::Write;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use beebeeb_engine::test_api::fake_account::TEST_PHRASE;
use beebeeb_engine::test_api::large_account::{LargeAccount, local_rows, seed};
use beebeeb_engine::test_api::server::{
    FIXTURE_EMAIL, FIXTURE_PASSWORD, LineWriter, Switches, bind, fixture_account, fixture_handler, loopback_listen,
};
use beebeeb_engine::test_api::{FakeApi, Handler, Reply, Request};
use serde_json::json;

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let listen = args.next().unwrap_or_else(|| "127.0.0.1:47486".into());
    let switch = args.next().map(std::path::PathBuf::from);
    let listen = loopback_listen(&listen).expect("a loopback address");

    let started = Instant::now();
    let account = fixture_account();
    eprintln!("{} dc086: fixture account registered in {} ms", now_ms(), started.elapsed().as_millis());
    let started = Instant::now();
    seed(&account, &LargeAccount::GUUS);
    eprintln!("{} dc086: LargeAccount::GUUS seeded in {} ms", now_ms(), started.elapsed().as_millis());

    let lines: LineWriter = Arc::new(|_line: &str| {});
    let inner = fixture_handler(account.clone(), lines, Arc::new(AtomicU64::new(0)), Switches::default());
    let root: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let handler: Handler = Arc::new(move |request: &Request| {
        let started = Instant::now();
        let route = request.route();
        let backups_on = switch.as_ref().is_some_and(|file| file.exists());
        let reply = match route.as_str() {
            "GET /api/v1/backups/root" if backups_on => match root.lock().expect("root").clone() {
                Some(id) => Reply::ok(json!({ "id": id })),
                None => Reply::json(404, json!({ "error": "not_found" })),
            },
            "PUT /api/v1/backups/root" if backups_on => {
                let body = request.json();
                let mut root = root.lock().expect("root");
                match (root.clone(), body["folder_id"].as_str()) {
                    (Some(id), _) => Reply::ok(json!({ "id": id, "outcome": "existing" })),
                    (None, Some(id)) => {
                        let known = local_rows(&account).iter().any(|row| row.id == id);
                        if !known {
                            account.add_item(id, None, "Backups", true, 0);
                        }
                        *root = Some(id.to_string());
                        Reply::ok(json!({ "id": id, "outcome": if known { "claimed" } else { "created" } }))
                    }
                    (None, None) => Reply::error(400, "bad_request"),
                }
            }
            _ => inner(request),
        };
        let mut err = std::io::stderr().lock();
        let _ = writeln!(
            err,
            "{} {} {} -> {} {}ms {}b",
            now_ms(),
            request.method,
            request.path.split('?').next().unwrap_or_default(),
            reply.status,
            started.elapsed().as_millis(),
            reply.body.len()
        );
        reply
    });

    let fake = FakeApi::start_on(bind(listen).expect("bind"), handler).expect("start");
    let mut out = std::io::stdout();
    let _ = writeln!(out, "base: {}", fake.base);
    let _ = writeln!(out, "email: {FIXTURE_EMAIL}");
    let _ = writeln!(out, "password: {FIXTURE_PASSWORD}");
    let _ = writeln!(out, "phrase: {TEST_PHRASE}");
    let _ = out.flush();
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

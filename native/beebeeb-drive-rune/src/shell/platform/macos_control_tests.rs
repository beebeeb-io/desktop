//! Real sockets in a temp folder; the app's own folder is never touched.

use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;

use super::*;

const ID: AppIdentity = AppIdentity::Test;

fn recording(reply: Reply) -> (ControlHandler, Arc<Mutex<Vec<Request>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let handler: ControlHandler = Arc::new(move |request| {
        sink.lock().unwrap().push(request);
        reply
    });
    (handler, seen)
}

fn temp(what: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bbc-{what}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn a_request_round_trips_and_the_socket_is_private() {
    let dir = temp("rt");
    let (handler, seen) = recording(Reply::Ok);
    let server = serve_in(&dir, ID, handler).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&socket_path(&dir)), 0o600);
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(
        send_in(&dir, &Request::Show, Duration::from_secs(2)),
        Delivery::Acknowledged
    );
    assert_eq!(seen.lock().unwrap().as_slice(), [Request::Show]);
    drop(server);
    assert!(!socket_path(&dir).exists(), "dropping frees the name");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn replies_map_to_deliveries() {
    let dir = temp("rep");
    let (handler, _) = recording(Reply::OtherServer);
    let _server = serve_in(&dir, ID, handler).unwrap();
    assert_eq!(
        send_in(&dir, &Request::ShowOn("http://x".into()), Duration::from_secs(2)),
        Delivery::OtherServer
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_serve_says_already_running() {
    let dir = temp("two");
    let (handler, _) = recording(Reply::Ok);
    let _first = serve_in(&dir, ID, handler.clone()).unwrap();
    assert!(matches!(serve_in(&dir, ID, handler), Err(ServeError::AlreadyRunning)));
    // and the first still answers
    assert_eq!(
        send_in(&dir, &Request::Show, Duration::from_secs(2)),
        Delivery::Acknowledged
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_socket_left_by_a_dead_process_is_taken_over() {
    let dir = temp("stale");
    ensure_dir(&dir).unwrap();
    // A bound socket whose listener is gone leaves the file behind.
    drop(UnixListener::bind(socket_path(&dir)).unwrap());
    assert!(socket_path(&dir).exists());
    assert_eq!(
        send_in(&dir, &Request::Show, Duration::from_millis(200)),
        Delivery::Unreachable
    );
    let (handler, seen) = recording(Reply::Ok);
    let _server = serve_in(&dir, ID, handler).unwrap();
    assert_eq!(
        send_in(&dir, &Request::Show, Duration::from_secs(2)),
        Delivery::Acknowledged
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nobody_serving_is_unreachable_within_the_timeout() {
    let dir = temp("none");
    let started = Instant::now();
    assert_eq!(
        send_in(&dir, &Request::Show, Duration::from_millis(300)),
        Delivery::Unreachable
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn a_socket_open_to_others_is_not_written_to() {
    let dir = temp("open");
    let (handler, seen) = recording(Reply::Ok);
    let _server = serve_in(&dir, ID, handler).unwrap();
    std::fs::set_permissions(socket_path(&dir), std::fs::Permissions::from_mode(0o666)).unwrap();
    assert_eq!(
        send_in(&dir, &Request::Show, Duration::from_secs(1)),
        Delivery::Unreachable
    );
    assert!(seen.lock().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_socket_path_stays_under_sun_len() {
    let sandbox = Path::new("/Users/someone.with.a.longer.name/Library/Containers/io.beebeeb.app/Data");
    let long_home = PathBuf::from(format!("/Users/{}", "h".repeat(53)));
    assert_eq!(long_home.as_os_str().len(), 60);
    for home in [sandbox, long_home.as_path()] {
        for id in [AppIdentity::Real, AppIdentity::Test] {
            let path = socket_path(&control_dir_in(home, id));
            assert!(
                path.as_os_str().len() < SUN_LEN,
                "{} bytes: {path:?}",
                path.as_os_str().len()
            );
        }
    }
    assert_ne!(
        control_dir_in(sandbox, AppIdentity::Real),
        control_dir_in(sandbox, AppIdentity::Test)
    );
}

#[test]
fn a_second_instance_over_a_short_home_path_finds_the_first() {
    let home = temp("hs");
    std::fs::create_dir_all(&home).unwrap();
    let dir = control_dir_in(&home, ID);
    let (handler, seen) = recording(Reply::Ok);
    let server = serve_in(&dir, ID, handler).unwrap();
    assert!(matches!(
        serve_in(&dir, ID, recording(Reply::Ok).0),
        Err(ServeError::AlreadyRunning)
    ));
    assert_eq!(
        send_in(&dir, &Request::Show, Duration::from_secs(2)),
        Delivery::Acknowledged
    );
    assert_eq!(seen.lock().unwrap().as_slice(), [Request::Show]);
    drop(server);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_too_long_path_is_refused_not_bound() {
    let dir = PathBuf::from(format!("/tmp/{}", "d".repeat(100)));
    assert!(matches!(
        serve_in(&dir, ID, recording(Reply::Ok).0),
        Err(ServeError::Os(_))
    ));
}

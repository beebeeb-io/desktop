//! macOS: the control channel's transport, a Unix socket (`shell/control.rs` is the protocol).
//!
//! - **The place** is `control.sock` in this identity's Application Support folder
//!   (`AppIdentity::state_dir_id`), the folder mode 0700 and the socket 0600: this user only.
//! - **Serving it is being the instance.** A socket file that is there is tried first: if
//!   something answers a connect, that is the running instance (`AlreadyRunning`); if nothing
//!   does, its owner is gone and the file is replaced.
//! - **Every wait is bounded**: a client that connects and says nothing costs the server two
//!   seconds; a server that never answers costs the caller its timeout.
//! - **A launch checks the file before it writes**: the socket must belong to the same user as
//!   its folder and admit that user only; anything else is `Unreachable`, nothing is sent.

use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{ControlHandler, ControlServer, ServeError};
use crate::shell::control::{self, Reply, Request, MAX_REPLY, MAX_REQUEST};
use crate::shell::identity::AppIdentity;
use crate::shell::instance::Delivery;

const SOCKET_NAME: &str = "control.sock";
const REQUEST_WAIT: Duration = Duration::from_secs(2);

/// This identity's folder: `~/Library/Application Support/<bundle identifier>`.
pub(super) fn state_dir(identity: AppIdentity) -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .ok_or("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library/Application Support")
        .join(identity.state_dir_id()))
}

fn socket_path(dir: &Path) -> PathBuf {
    dir.join(SOCKET_NAME)
}

fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    // A folder that was there already keeps its mode from before: make it ours alone.
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

struct SocketServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl ControlServer for SocketServer {}

impl Drop for SocketServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the thread out of accept by connecting; it may be busy with a client first.
        let deadline = Instant::now() + Duration::from_secs(6);
        if let Some(worker) = self.worker.take() {
            while !worker.is_finished() && Instant::now() < deadline {
                drop(UnixStream::connect(&self.path));
                std::thread::sleep(Duration::from_millis(20));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn read_line(stream: &mut UnixStream, cap: usize, wait: Duration) -> Option<Vec<u8>> {
    let deadline = Instant::now() + wait;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 128];
    while !control::request_is_complete(&bytes) && bytes.len() <= cap {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        stream.set_read_timeout(Some(left)).ok()?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(_) => return None,
        }
    }
    Some(bytes)
}

fn serve_one(mut stream: UnixStream, identity: AppIdentity, handler: &ControlHandler) {
    let Some(bytes) = read_line(&mut stream, MAX_REQUEST, REQUEST_WAIT) else {
        return;
    };
    let Some(request) = Request::decode(&bytes, identity) else {
        return;
    };
    let reply = handler(request);
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    let _ = stream.write_all(reply.encode());
}

fn server_thread(listener: UnixListener, identity: AppIdentity, handler: ControlHandler, stop: Arc<AtomicBool>) {
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if let Ok(stream) = stream {
            serve_one(stream, identity, &handler);
        }
    }
}

/// Serves the channel in `dir`. `Err(AlreadyRunning)`: a live process does.
pub(super) fn serve_in(
    dir: &Path,
    identity: AppIdentity,
    handler: ControlHandler,
) -> Result<Box<dyn ControlServer>, ServeError> {
    let os = |what: &str, e: std::io::Error| ServeError::Os(format!("{what}: {e}"));
    ensure_dir(dir).map_err(|e| os("preparing the control folder failed", e))?;
    let path = socket_path(dir);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if UnixStream::connect(&path).is_ok() {
                return Err(ServeError::AlreadyRunning);
            }
            // Nobody answers: its owner is gone. Replace it.
            std::fs::remove_file(&path).map_err(|e| os("removing the stale socket failed", e))?;
            UnixListener::bind(&path).map_err(|e| match e.kind() {
                std::io::ErrorKind::AddrInUse => ServeError::AlreadyRunning,
                _ => os("creating the control socket failed", e),
            })?
        }
        Err(e) => return Err(os("creating the control socket failed", e)),
    };
    // The folder is 0700, so nobody else reaches the socket before this lands.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| os("setting the socket's mode failed", e))?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let worker = std::thread::Builder::new()
        .name("beebeeb-control".into())
        .spawn(move || server_thread(listener, identity, handler, thread_stop))
        .map_err(|e| ServeError::Os(format!("starting the control thread failed: {e}")))?;
    Ok(Box::new(SocketServer {
        path,
        stop,
        worker: Some(worker),
    }))
}

/// Whether the socket at `path` is this user's: owned like its folder, no access for anyone else.
fn socket_is_ours(path: &Path) -> bool {
    let (Ok(file), Some(Ok(dir))) = (std::fs::symlink_metadata(path), path.parent().map(std::fs::metadata)) else {
        return false;
    };
    file.file_type().is_socket_like() && file.uid() == dir.uid() && file.mode() & 0o077 == 0 && dir.mode() & 0o077 == 0
}

trait SocketLike {
    fn is_socket_like(&self) -> bool;
}

impl SocketLike for std::fs::FileType {
    fn is_socket_like(&self) -> bool {
        std::os::unix::fs::FileTypeExt::is_socket(self)
    }
}

/// Hands one request to the instance serving the channel in `dir`, waiting at most `timeout` in all.
pub(super) fn send_in(dir: &Path, request: &Request, timeout: Duration) -> Delivery {
    let path = socket_path(dir);
    if !socket_is_ours(&path) {
        return Delivery::Unreachable;
    }
    let deadline = Instant::now() + timeout;
    let Ok(mut stream) = UnixStream::connect(&path) else {
        return Delivery::Unreachable;
    };
    let left = |d: Instant| {
        d.saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1))
    };
    if stream.set_write_timeout(Some(left(deadline))).is_err() || stream.write_all(&request.encode()).is_err() {
        return Delivery::NoAnswer;
    }
    let mut bytes = Vec::new();
    let mut chunk = [0u8; MAX_REPLY];
    while !bytes.contains(&b'\n') && bytes.len() < MAX_REPLY {
        if Instant::now() >= deadline || stream.set_read_timeout(Some(left(deadline))).is_err() {
            return Delivery::NoAnswer;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
        }
    }
    match Reply::decode(&bytes) {
        Some(Reply::Ok) => Delivery::Acknowledged,
        Some(Reply::OtherServer) => Delivery::OtherServer,
        Some(Reply::Busy) | None => Delivery::NoAnswer,
    }
}

#[cfg(test)]
#[path = "macos_control_tests.rs"]
mod tests;

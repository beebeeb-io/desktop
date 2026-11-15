//! The confined helper and its two pipes: [`HelperLink`] starts `beebeeb-media-helper` through the sandbox crate
//! (`beebeeb-office-sandbox`: an AppContainer with no capabilities, in a job object of [`HELPER_LIMITS`], gone with
//! this app; fail closed) and speaks the protocol on its standard input and output, every message checked by the
//! protocol's decoder. There is no other way to start the helper: no unconfined path, no flag.
//!
//! Two threads per helper, so that nothing the helper does (or stops doing) can hold up the player, a lock or a
//! sign-out:
//! - the **reader** decodes what the helper writes ([`read_to_app`], every length checked before anything is
//!   allocated for it) and hands it over in batches: every message that is already whole in its buffer goes in the
//!   same batch, so the player sees together what the helper sent together (the player counts the reads in flight
//!   over a batch). At most [`READ_AHEAD_BATCHES`] batches wait; then the reader stops reading and the helper's
//!   next write waits.
//! - the **writer** writes what the player sends, in order, and wipes each `Data` (decrypted bytes) once it is
//!   written. [`HelperLink::send`] only queues: a helper that stops reading its input cannot block the player's
//!   thread, and the queue of `Data` is bounded ([`WRITE_QUEUE_MAX`]; past it `send` fails and the player kills the
//!   helper).
//!
//! [`HelperLink::kill`] ends the helper (it is not asked), lets go of both pipes and waits at most [`KILL_WAIT`] for
//! the two threads (they end once the helper's pipes broke). It never waits on the helper itself.
//!
//! The tests speak the protocol to a fake helper thread over two in-memory pipes ([`HelperLink::from_pipes`], test
//! builds only): the same threads, the same checks.

use std::collections::VecDeque;
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use beebeeb_media_protocol::{Codec, FromHelper, MAX_READ, ToHelper, WireError, read_to_app, write_msg};
use beebeeb_office_sandbox::{Child, Limits, Sandbox};
use zeroize::Zeroize;

use super::player::{Link, LinkFactory, Refusal};

/// The helper's committed memory: 1 GiB (decision D-MED-3).
pub const HELPER_LIMITS: Limits = Limits { memory_bytes: 1 << 30 };

/// The helper's file name, next to the app's exe.
const HELPER_NAME: &str = if cfg!(windows) {
    "beebeeb-media-helper.exe"
} else {
    "beebeeb-media-helper"
};

/// A sandbox profile other than today's (decision 2541 (a), answered yes: two helper profiles, chosen per file by
/// the route table). It has no value yet: today's profile (an AppContainer with no capabilities) is the only one a
/// helper can be started in. Lane `media-mf-route` replaces it with its own `Profile`.
pub enum HelperProfile {}

/// HOOK for lane `media-mf-route` (decision 2541): the profile for the file a helper is started for. `None`: today's
/// profile. That lane makes it `profile_for(route, codec)` (its helper/protocol side, the route from the codec's
/// row), and it alone writes the second profile. Until its branch lands, every file gets today's profile, and a
/// codec routed to the system's decoder is refused as before (`Refusal::NoSystemDecoder`).
pub fn profile_for_file(codec: Option<Codec>) -> Option<HelperProfile> {
    let _ = codec;
    None
}

/// How many batches the reader may have handed over and the player not yet taken.
const READ_AHEAD_BATCHES: usize = 2;
/// The most messages in one batch.
const BATCH_MAX: usize = 64;
/// The most bytes of `Data` queued for the writer: the two reads the helper may have in flight and one more. A
/// helper that stops reading its input while it keeps asking is refused here, not queued without bound.
const WRITE_QUEUE_MAX: usize = 3 * MAX_READ as usize;
/// How long [`HelperLink::kill`] waits for the two pipe threads to end.
const KILL_WAIT: Duration = Duration::from_millis(250);

/// What the reader hands over at once.
type Batch = Vec<Result<FromHelper, WireError>>;

/// Why nothing came from the helper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// Nothing arrived within the timeout.
    Timeout,
    /// The helper is gone (its pipe ended).
    Gone,
    /// What arrived broke the protocol: the player kills the helper.
    Protocol(WireError),
}

/// One running, confined helper.
pub struct HelperLink {
    /// `None` for a link over a test's pipes, and after [`HelperLink::kill`].
    child: Option<Child>,
    /// To the writer thread. `None` after a kill.
    to_writer: Option<mpsc::Sender<ToHelper>>,
    /// Bytes of `Data` queued and not yet written.
    queued: Arc<AtomicUsize>,
    /// The writer could not write (the helper's input is gone): every later `send` fails.
    write_failed: Arc<AtomicBool>,
    /// From the reader thread. `None` after a kill.
    from_reader: Option<mpsc::Receiver<Batch>>,
    /// The rest of the batch `recv` took last.
    inbox: VecDeque<Result<FromHelper, WireError>>,
    threads: Vec<thread::JoinHandle<()>>,
    /// Ends (disconnects) once both threads ended.
    ended: Option<mpsc::Receiver<()>>,
}

impl HelperLink {
    /// Starts the helper at `exe`, confined. Where the OS has no sandbox module, or the sandbox refuses:
    /// [`Refusal::NoSandbox`], and nothing runs.
    ///
    /// Every helper is started here, and its profile is asked of [`profile_for_file`] here (decision 2541): the
    /// player starts a helper before the file was read, so no codec is known yet.
    pub fn start(exe: &Path) -> Result<HelperLink, Refusal> {
        // The one seam: lane `media-mf-route` spawns in the profile this answers; `None`, today's profile (the only
        // one there is yet), spawns as below.
        let profile = profile_for_file(None);
        debug_assert!(profile.is_none(), "no second profile before lane media-mf-route");
        let sandbox = Sandbox::new(HELPER_LIMITS).map_err(|_| Refusal::NoSandbox)?;
        let mut child = sandbox.spawn(exe).map_err(|_| Refusal::NoSandbox)?;
        let Some(pipes) = child.take_pipes() else {
            let _ = child.kill();
            return Err(Refusal::NoSandbox);
        };
        match HelperLink::over(Box::new(pipes.from_helper), Box::new(pipes.to_helper)) {
            Ok(mut link) => {
                link.child = Some(child);
                Ok(link)
            }
            Err(_) => {
                let _ = child.kill();
                Err(Refusal::CouldNotPlay)
            }
        }
    }

    /// A link over a fake helper's two pipes (the player's tests): the same threads and checks, no process.
    #[cfg(test)]
    pub(crate) fn from_pipes(from: Box<dyn Read + Send>, to: Box<dyn Write + Send>) -> HelperLink {
        HelperLink::over(from, to).expect("the pipe threads start")
    }

    /// The reader and the writer thread over `from` and `to`.
    fn over(from: Box<dyn Read + Send>, to: Box<dyn Write + Send>) -> io::Result<HelperLink> {
        let (ended_tx, ended_rx) = mpsc::channel::<()>();
        let (msg_tx, msg_rx) = mpsc::channel::<ToHelper>();
        let (batch_tx, batch_rx) = mpsc::sync_channel::<Batch>(READ_AHEAD_BATCHES);
        let queued = Arc::new(AtomicUsize::new(0));
        let write_failed = Arc::new(AtomicBool::new(false));
        let writer = {
            let queued = Arc::clone(&queued);
            let failed = Arc::clone(&write_failed);
            let ended = ended_tx.clone();
            thread::Builder::new().name("media-helper-in".into()).spawn(move || {
                let _ended = ended;
                write_loop(to, msg_rx, &queued, &failed);
            })?
        };
        // If the reader does not start, `msg_tx` is dropped on the way out and the writer ends.
        let reader = thread::Builder::new().name("media-helper-out".into()).spawn(move || {
            let _ended = ended_tx;
            read_loop(from, batch_tx);
        })?;
        Ok(HelperLink {
            child: None,
            to_writer: Some(msg_tx),
            queued,
            write_failed,
            from_reader: Some(batch_rx),
            inbox: VecDeque::new(),
            threads: vec![writer, reader],
            ended: Some(ended_rx),
        })
    }

    /// Queues `msg` for the helper. Never waits for the helper: a full queue of `Data` (the helper stopped reading)
    /// or a broken pipe is an error, and the player kills the helper.
    pub fn send(&mut self, msg: &ToHelper) -> io::Result<()> {
        if self.write_failed.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let to = self.to_writer.as_ref().ok_or(io::ErrorKind::NotConnected)?;
        let size = data_len(msg);
        if self.queued.load(Ordering::SeqCst).saturating_add(size) > WRITE_QUEUE_MAX {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        self.queued.fetch_add(size, Ordering::SeqCst);
        // The queue's own copy; the writer wipes it once written (the player wipes the one it sent).
        if let Err(mpsc::SendError(mut back)) = to.send(msg.clone()) {
            wipe(&mut back);
            self.queued.fetch_sub(size, Ordering::SeqCst);
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        Ok(())
    }

    /// The next message, checked by the protocol's decoder. Nothing within `timeout` (zero: only what is already
    /// here): [`LinkError::Timeout`].
    pub fn recv(&mut self, timeout: Duration) -> Result<FromHelper, LinkError> {
        if self.inbox.is_empty() {
            let from = self.from_reader.as_ref().ok_or(LinkError::Gone)?;
            match from.recv_timeout(timeout) {
                Ok(batch) => self.inbox.extend(batch),
                Err(mpsc::RecvTimeoutError::Timeout) => return Err(LinkError::Timeout),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(LinkError::Gone),
            }
        }
        match self.inbox.pop_front() {
            Some(Ok(msg)) => Ok(msg),
            Some(Err(error)) => Err(link_error(error)),
            None => Err(LinkError::Timeout),
        }
    }

    /// Ends the helper now (it is not asked), lets go of both pipes, and waits at most [`KILL_WAIT`] for the two
    /// threads. Fine to call more than once.
    pub fn kill(&mut self) {
        if let Some(child) = self.child.take() {
            // Dropping the `Child` kills it too; asked first so the pipes break now.
            let _ = child.kill();
        }
        self.to_writer = None;
        self.from_reader = None;
        self.inbox.clear();
        if let Some(ended) = self.ended.take()
            && matches!(ended.recv_timeout(KILL_WAIT), Err(mpsc::RecvTimeoutError::Disconnected))
        {
            for thread in self.threads.drain(..) {
                let _ = thread.join();
            }
        }
        // Threads still running past the wait end on their own once their pipe breaks.
        self.threads.clear();
    }

    /// Started through the sandbox (`Sandbox::spawn` reads the confinement back before the helper runs).
    pub fn confined(&self) -> bool {
        self.child.is_some()
    }
}

impl Drop for HelperLink {
    fn drop(&mut self) {
        self.kill();
    }
}

impl Link for HelperLink {
    fn send(&mut self, msg: &ToHelper) -> io::Result<()> {
        HelperLink::send(self, msg)
    }

    fn recv(&mut self, timeout: Duration) -> Result<FromHelper, LinkError> {
        HelperLink::recv(self, timeout)
    }

    fn kill(&mut self) {
        HelperLink::kill(self)
    }

    fn confined(&self) -> bool {
        HelperLink::confined(self)
    }
}

/// Overwrites the decrypted bytes a `Data` holds. Every other message holds none.
pub(super) fn wipe(msg: &mut ToHelper) {
    if let ToHelper::Data { bytes, .. } = msg {
        bytes.zeroize();
    }
}

/// The bytes of `Data` a message queues.
fn data_len(msg: &ToHelper) -> usize {
    match msg {
        ToHelper::Data { bytes, .. } => bytes.len(),
        _ => 0,
    }
}

fn link_error(error: WireError) -> LinkError {
    match error {
        WireError::Closed | WireError::Io(_) => LinkError::Gone,
        other => LinkError::Protocol(other),
    }
}

/// Writes what the player queued, in order, and wipes each message after it. After a failed write nothing more is
/// written, but every message is still taken and wiped, until the link lets go of the queue.
fn write_loop(
    mut to: Box<dyn Write + Send>,
    messages: mpsc::Receiver<ToHelper>,
    queued: &AtomicUsize,
    failed: &AtomicBool,
) {
    for mut msg in messages.iter() {
        if !failed.load(Ordering::SeqCst) && write_msg(&mut to, &msg).is_err() {
            failed.store(true, Ordering::SeqCst);
        }
        let size = data_len(&msg);
        wipe(&mut msg);
        queued.fetch_sub(size, Ordering::SeqCst);
    }
}

/// Decodes what the helper writes and hands it over in batches, until the pipe ends, a message is refused (that
/// error is the batch's last entry) or the link lets go.
fn read_loop(from: Box<dyn Read + Send>, out: mpsc::SyncSender<Batch>) {
    let mut from = BufReader::new(from);
    loop {
        let mut batch = Batch::new();
        let last = loop {
            let msg = read_to_app(&mut from);
            let last = msg.is_err();
            batch.push(msg);
            if last || batch.len() >= BATCH_MAX || !holds_a_whole_message(from.buffer()) {
                break last;
            }
        };
        if out.send(batch).is_err() || last {
            return;
        }
    }
}

/// Whether `buffered` starts with a whole message (its length and all it says follows), so decoding it waits for
/// nothing.
fn holds_a_whole_message(buffered: &[u8]) -> bool {
    match buffered.first_chunk::<4>() {
        Some(len) => buffered.len() - 4 >= u32::from_le_bytes(*len) as usize,
        None => false,
    }
}

/// The helper next to the app's exe. `None`: it is not there (the player says playback isn't available).
pub fn helper_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.with_file_name(HELPER_NAME);
    exe.is_file().then_some(exe)
}

/// The real helpers: [`helper_exe`], started by [`HelperLink::start`]. The player's tests hand their own factory.
pub struct SandboxedHelpers;

impl LinkFactory for SandboxedHelpers {
    fn start(&self) -> Result<Box<dyn Link>, Refusal> {
        let exe = helper_exe().ok_or(Refusal::NotInstalled)?;
        Ok(Box::new(HelperLink::start(&exe)?))
    }
}

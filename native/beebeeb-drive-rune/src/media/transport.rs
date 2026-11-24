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
//! **Two profiles** (decision 2541): every helper starts in the sandbox's default profile (no capability). A file
//! whose stream the helper answers `NoSystemDecoder(codec)` for is restarted once in the system-decoder profile
//! ([`profile_for_file`]). The switch `MEDIA_SYSTEM_DECODER_ON` is on as shipped (the owner's ruling of 2026-11-20);
//! held off, that answer is always `None` and no helper ever starts in it.
//!
//! **Crash reports:** before the first helper of this process starts, the app writes the per-user Windows Error
//! Reporting exclusion for the helper's exe ([`ErrorReports`]), so a crash of the helper (which holds decrypted
//! bytes) leaves no dump on disk and sends none. A failed write starts no helper: [`Refusal::NoSandbox`].
//!
//! The tests speak the protocol to a fake helper thread over two in-memory pipes ([`HelperLink::from_pipes`], test
//! builds only): the same threads, the same checks.

use std::collections::VecDeque;
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::thread;
use std::time::Duration;

use beebeeb_media_protocol::{
    Codec, FromHelper, MAX_READ, MEDIA_SYSTEM_DECODER_ON, Route, ToHelper, WireError, profile_for_with, read_to_app,
    write_msg,
};
use beebeeb_office_sandbox::{Child, Limits, Profile, Sandbox, SandboxError};
use zeroize::Zeroize;

use super::player::{Link, LinkFactory, Refusal};
use super::refusal_log;

/// The helper's committed memory: 1 GiB (decision D-MED-3).
pub const HELPER_LIMITS: Limits = Limits { memory_bytes: 1 << 30 };

/// The helper's file name, next to the app's exe. Also the name its crash-report exclusion is written under.
pub(crate) const HELPER_NAME: &str = if cfg!(windows) {
    "beebeeb-media-helper.exe"
} else {
    "beebeeb-media-helper"
};

/// The profile to restart a helper in for a stream of `codec` (decision 2541), with the shipped switch
/// (`MEDIA_SYSTEM_DECODER_ON`, on). `None`: the default profile. The app has no route table: a helper started in the
/// default profile that answers `NoSystemDecoder(codec)` names a stream whose route is `System`.
pub fn profile_for_file(codec: Option<Codec>) -> Option<Profile> {
    profile_for_file_with(MEDIA_SYSTEM_DECODER_ON, codec)
}

/// [`profile_for_file`] with the switch given (the player's own field, so its tests hold it at both values).
pub(crate) fn profile_for_file_with(on: bool, codec: Option<Codec>) -> Option<Profile> {
    codec
        .and_then(|codec| profile_for_with(on, Route::System, codec))
        .map(|profile| Profile::SystemDecoder { hevc: profile.hevc })
}

/// Where the per-user Windows Error Reporting exclusion is written (a seam: the tests hand their own).
pub(crate) trait ErrorReports: Send + Sync {
    /// Excludes the exe called `exe_name` (its file name, never a path) from Windows Error Reporting for this user:
    /// no dump of it is kept or sent.
    fn exclude(&self, exe_name: &str) -> io::Result<()>;
}

/// The real one: `HKCU\Software\Microsoft\Windows\Windows Error Reporting\ExcludedApplications`, a `REG_DWORD` 1
/// named after the exe. Off Windows there is no such thing, and it does nothing.
pub(crate) struct WindowsErrorReports;

impl ErrorReports for WindowsErrorReports {
    fn exclude(&self, exe_name: &str) -> io::Result<()> {
        #[cfg(windows)]
        {
            wer::exclude(exe_name)
        }
        #[cfg(not(windows))]
        {
            let _ = exe_name;
            Ok(())
        }
    }
}

/// Whether this process wrote the exclusion. Once written it is never written again; a failed write is tried again
/// at the next start (and that start is refused).
pub(crate) struct Excluded(OnceLock<()>);

impl Excluded {
    pub(crate) const fn new() -> Excluded {
        Excluded(OnceLock::new())
    }
}

/// The process's own: the first helper's start writes the exclusion.
static EXCLUDED: Excluded = Excluded::new();

/// Writes the crash-report exclusion (once per `excluded`), THEN runs `spawn`. A failed write runs nothing:
/// [`Refusal::NoSandbox`]. Every helper is started through this.
pub(crate) fn excluded_then<T>(
    excluded: &Excluded,
    reports: &dyn ErrorReports,
    spawn: impl FnOnce() -> Result<T, Refusal>,
) -> Result<T, Refusal> {
    if excluded.0.get().is_none() {
        reports.exclude(HELPER_NAME).map_err(|_| {
            refusal_log::not_started(refusal_log::NotStarted::Exclusion);
            Refusal::NoSandbox
        })?;
        let _ = excluded.0.set(());
    }
    spawn()
}

/// The registry write, Windows only (`windows-sys`, HKEY_CURRENT_USER only, as the shell's Run value).
#[cfg(windows)]
mod wer {
    use std::io;
    use std::ptr;

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE, RegCloseKey, RegCreateKeyExW,
        RegSetValueExW,
    };

    const KEY: &str = "Software\\Microsoft\\Windows\\Windows Error Reporting\\ExcludedApplications";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub(super) fn exclude(exe_name: &str) -> io::Result<()> {
        let (key, name) = (wide(KEY), wide(exe_name));
        let mut hkey: HKEY = ptr::null_mut();
        // SAFETY: `key` is NUL-terminated and outlives the call; no class, no security attributes and no disposition
        // are asked for (null is allowed for each); `hkey` receives the opened key.
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                0,
                ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                ptr::null(),
                &mut hkey,
                ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let one = 1u32.to_le_bytes();
        // SAFETY: `hkey` was opened above with KEY_SET_VALUE; `name` is NUL-terminated; `one` is 4 bytes for the call.
        let status = unsafe { RegSetValueExW(hkey, name.as_ptr(), 0, REG_DWORD, one.as_ptr(), one.len() as u32) };
        // SAFETY: `hkey` was opened above and is closed once, here.
        unsafe { RegCloseKey(hkey) };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(())
    }
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

/// What a sandbox error means to the player: a Windows too old for the sandbox says so (logged once per app start,
/// the reason only); every other error is "no sandbox".
pub(crate) fn sandbox_refusal(error: SandboxError) -> Refusal {
    refusal_log::sandbox_failed(&error);
    if error == SandboxError::NeedsNewerWindows {
        crate::windows_floor::note_needs_windows_update();
        Refusal::NeedsWindowsUpdate
    } else {
        Refusal::NoSandbox
    }
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
    /// Starts the helper at `exe`, confined, in the default profile (no capability). Where the OS has no sandbox
    /// module, or the sandbox refuses: [`Refusal::NoSandbox`], and nothing runs.
    pub fn start(exe: &Path) -> Result<HelperLink, Refusal> {
        HelperLink::start_in(exe, Profile::default())
    }

    /// Starts the helper at `exe`, confined, in `profile` (decision 2541: the system-decoder profile only for a file
    /// whose stream the helper answered `NoSystemDecoder` for, [`profile_for_file`]). The process's first start
    /// writes the crash-report exclusion before anything runs; a failed write, no sandbox here, or a sandbox that
    /// refuses: [`Refusal::NoSandbox`], and nothing runs.
    pub fn start_in(exe: &Path, profile: Profile) -> Result<HelperLink, Refusal> {
        excluded_then(&EXCLUDED, &WindowsErrorReports, || HelperLink::spawn_in(exe, profile))
    }

    /// The spawn itself, through the sandbox only.
    fn spawn_in(exe: &Path, profile: Profile) -> Result<HelperLink, Refusal> {
        let sandbox = Sandbox::with_profile(HELPER_LIMITS, profile).map_err(sandbox_refusal)?;
        let mut child = sandbox.spawn(exe).map_err(sandbox_refusal)?;
        let Some(pipes) = child.take_pipes() else {
            let _ = child.kill();
            refusal_log::not_started(refusal_log::NotStarted::Pipes);
            return Err(Refusal::NoSandbox);
        };
        match HelperLink::over(Box::new(pipes.from_helper), Box::new(pipes.to_helper)) {
            Ok(mut link) => {
                link.child = Some(child);
                Ok(link)
            }
            Err(_) => {
                let _ = child.kill();
                refusal_log::not_started(refusal_log::NotStarted::Threads);
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
        // What the helper sent and nobody took (frames, sound, tags) is zeroed before it is let go: the inbox, then
        // the batches still queued by the reader (one it hands over after this is zeroed by the reader itself).
        wipe_batch(self.inbox.make_contiguous());
        self.inbox.clear();
        if let Some(from) = self.from_reader.take() {
            while let Ok(mut batch) = from.try_recv() {
                wipe_batch(&mut batch);
            }
        }
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

/// Overwrites what a helper's answer holds of the file: a frame's pixels, sound, the tags' words and picture.
pub(super) fn wipe_answer(msg: &mut FromHelper) {
    match msg {
        FromHelper::Frame { rgba, .. } => rgba.zeroize(),
        FromHelper::Audio { samples, .. } => samples.zeroize(),
        FromHelper::Tags(tags) => {
            tags.title.zeroize();
            tags.artist.zeroize();
            if let Some((_, _, rgba)) = tags.art.as_mut() {
                rgba.zeroize();
            }
        }
        FromHelper::ReadAt { .. } | FromHelper::Opened(_) | FromHelper::Ended | FromHelper::Error(_) => {}
    }
}

/// [`wipe_answer`] for each answer of a batch.
fn wipe_batch(batch: &mut [Result<FromHelper, WireError>]) {
    for msg in batch.iter_mut().flatten() {
        wipe_answer(msg);
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
        match out.send(batch) {
            Ok(()) if !last => {}
            Ok(()) => return,
            // The link let go: nobody takes this batch, so it is zeroed here.
            Err(mpsc::SendError(mut batch)) => {
                wipe_batch(&mut batch);
                return;
            }
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

/// The real helpers: [`helper_exe`], started by [`HelperLink::start_in`]. The player's tests hand their own factory.
pub struct SandboxedHelpers;

impl LinkFactory for SandboxedHelpers {
    fn start(&self) -> Result<Box<dyn Link>, Refusal> {
        self.start_in(Profile::default())
    }

    fn start_in(&self, profile: Profile) -> Result<Box<dyn Link>, Refusal> {
        let exe = helper_exe().ok_or_else(|| {
            refusal_log::not_started(refusal_log::NotStarted::NoHelper);
            Refusal::NotInstalled
        })?;
        Ok(Box::new(HelperLink::start_in(&exe, profile)?))
    }
}

/// The real image helpers (095): [`helper_exe`] in its `--image` mode, confined like every helper (the crash-report
/// exclusion first, the same limits), always in the system-decoder profile (WIC needs COM; `hevc` for a HEIC).
/// Nothing is started while the system decoders are switched off.
pub struct SandboxedPictures;

/// The process of one image helper. Dropping it ends the helper.
struct PictureChildProcess(std::sync::Mutex<Child>);

impl crate::engine_host::media::PictureChild for PictureChildProcess {
    fn kill(&self) {
        let _ = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).kill();
    }
}

impl crate::engine_host::media::PictureHelper for SandboxedPictures {
    fn start(&self, hevc: bool) -> Option<crate::engine_host::media::PictureProcess> {
        if !MEDIA_SYSTEM_DECODER_ON {
            return None;
        }
        let exe = helper_exe()?;
        let mut child = excluded_then(&EXCLUDED, &WindowsErrorReports, || {
            Sandbox::with_profile(HELPER_LIMITS, Profile::SystemDecoder { hevc })
                .and_then(|sandbox| sandbox.spawn_in_mode(&exe, beebeeb_office_sandbox::Mode::Image))
                .map_err(sandbox_refusal)
        })
        .ok()?;
        let Some(pipes) = child.take_pipes() else {
            let _ = child.kill();
            return None;
        };
        Some(crate::engine_host::media::PictureProcess {
            from: Box::new(pipes.from_helper),
            to: Box::new(pipes.to_helper),
            child: Arc::new(PictureChildProcess(std::sync::Mutex::new(child))),
        })
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;

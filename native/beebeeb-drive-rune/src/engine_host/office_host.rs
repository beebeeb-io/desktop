//! The app's side of the Office helper: one confined renderer per open document, every answer checked before use,
//! killed on close, lock and sign-out.
//!
//! An Office file is parsed by a separate program, the helper (`beebeeb-office-render`), never by this process,
//! which holds the keys. The helper starts confined (`beebeeb_office_sandbox`: no network, no files, no child
//! process, gone with this process) and talks over two pipes in frames (`beebeeb_office_protocol`). The document's
//! bytes and every rendered page travel in unnamed shared sections this side makes and grants.
//!
//! What this file holds to:
//! - **The window never waits.** [`OfficeHost::open`], [`OfficeHost::poll`], the `request_*` calls, `cancel`,
//!   `retry` and `close` return at once. One worker thread per document starts the helper, sends the requests,
//!   reads the answers (a reader thread per helper feeds them into the worker's one channel, beside the window's
//!   commands) and keeps the timers; a writer thread per helper puts frames on its pipe, so a helper that stops
//!   reading cannot hold the worker.
//! - **Every answer is untrusted.** Each one is decoded, must carry the id of the OLDEST request out (the helper
//!   answers one request at a time, in order), must be the kind of answer that request takes, and passes the
//!   protocol's check (`check::check_opened`, `check_sheets`, `check_rendered`, `check_outline`, `check_cells`)
//!   before anything of it is used. Anything else kills the helper: "Renderer stopped". A reason is kept only as a
//!   variant of [`Stop`], never the helper's bytes.
//! - **Sizes come from the protocol only.** A page's pixel size is `check::asked_size` at the caller's `scale`
//!   (`check::render_scale(zoom, dpi)`, one `f32`): that size makes the section, and that same `scale` goes on the
//!   wire. At most [`MAX_IN_FLIGHT`] renders are out; the rest wait here, in order.
//! - **Timers.** The first answer to `Open` within 30 s of the start request; after that the oldest request out
//!   within 15 s of becoming the oldest. A miss kills the helper.
//! - **Try again** starts one new helper, once per open.
//! - **Close, lock, sign-out, drop** kill the helper through a handle shared with the worker (a helper whose start
//!   returns after the close is killed at once), wipe every answer not yet taken, and the worker drops the document's
//!   section and every page's section within one turn of its loop.
//!
//! Plaintext: the document's bytes are in the caller's `Zeroizing` buffer (moved in, wiped once copied), in the
//! document's section (the sandbox zeroes a section when it is dropped) and in the helper. Pixels, outline text, cell
//! text and sheet names in events and states are plaintext too: wiped here when nobody took them. Nothing of it is
//! written to a file, a log or an error.
//!
//! This file is included by `examples/office_page.rs` as well (`#[path]`): it uses only `std`, `zeroize`, the two
//! Office crates and (for the crash-report exclusion on Windows) `windows-sys`; no item of the app, no `rune`, no
//! engine and no log call.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use beebeeb_office_protocol::check;
use beebeeb_office_protocol::frame::{Frame, FrameError};
use beebeeb_office_protocol::office::{self, Message};
use beebeeb_office_sandbox as sandbox;
use zeroize::{Zeroize, Zeroizing};

pub use beebeeb_office_protocol::office::{CellData, CellRange, DocKind, OutlineEntry, PageSize, Rect, SheetInfo};

// Windows Error Reporting keeps no copy of the helper's memory (the lead's answer 4, 2026-11-15 19:03).
#[path = "office_host/crash_reports.rs"]
mod crash_reports;
// The worker thread of one open document.
#[path = "office_host/worker.rs"]
mod worker;

/// The helper's file name, beside the app's own executable.
#[cfg(windows)]
pub const HELPER_EXE: &str = "beebeeb-office-render.exe";
/// The helper's file name, beside the app's own executable.
#[cfg(not(windows))]
pub const HELPER_EXE: &str = "beebeeb-office-render";

/// Render requests out at once (the visible pages and one screen each way); more wait here, in order.
pub const MAX_IN_FLIGHT: usize = 6;

/// The Office engine's raster side: no page is asked whose whole size is beyond it on either side, whatever the
/// region (a tile of a larger page is still that page's raster in the engine).
pub const MAX_PAGE_SIDE_PX: u32 = 16_000;

/// Called by the worker after it queued an event: the window then polls.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// The helper beside the app's executable, when that file exists.
pub fn helper_exe() -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?.with_file_name(HELPER_EXE);
    path.is_file().then_some(path)
}

/// Every host of this process.
static PROCESS: Registry = Registry::new();

/// Ends every live helper of the process: the hook for a lock and a sign-out.
pub fn kill_all(why: CloseWhy) {
    kill_all_in(&PROCESS, why);
}

/// The shell's session end (a lock, a sign-out, another account): every helper of the process is killed and what
/// it rendered is wiped.
pub fn end_session(locked: bool) {
    kill_all(if locked { CloseWhy::Locked } else { CloseWhy::SignedOut });
}

/// A render request, as [`OfficeHost::request_page`] handed it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RequestId(u32);

/// What the window shows for an Office file.
#[derive(Clone, Debug, PartialEq)]
pub enum OfficeState {
    /// DW-1563 "Opening…".
    Opening,
    /// DW-1507 (and slides): the checked page count and sizes.
    Pages { pages: u32, sizes: Vec<PageSize> },
    /// DW-1508: the checked sheets; `more` sheets after them have no name here.
    Sheets { sheets: Vec<SheetInfo>, more: u32 },
    /// DW-1556 "No preview for this file here." and Open in another app…
    NoPreview(NoPreview),
    /// DW-1556 with the reason in one line.
    CannotOpen(CannotOpen),
    /// DW-1556 "Renderer stopped", with Try again while `can_retry`.
    Stopped { why: Stop, can_retry: bool },
    /// Nothing is held: the lock screen or the closed view.
    Closed(CloseWhy),
}

/// Why there is no preview here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoPreview {
    /// The helper has no preview for this format or this file's variant of it.
    ThisType,
    /// The file is password-protected.
    Password,
    /// This operating system has no sandbox for the helper: it is never started unconfined.
    NoSandbox,
    /// The helper is not installed beside the app.
    NotInstalled,
}

/// Why the file cannot be opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CannotOpen {
    /// Damaged, or not what its kind says.
    Damaged,
    /// Over a limit of the viewer contract.
    TooLarge,
}

/// Why the renderer stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The helper ended on its own (a crash, out of memory).
    Exited,
    /// The helper did not answer in time.
    TimedOut,
    /// The helper said something that failed a check.
    Protocol,
    /// The helper could not do it, or it could not be started.
    Failed,
}

/// Why a host was closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseWhy {
    Closed,
    Locked,
    SignedOut,
}

/// What [`OfficeHost::poll`] hands the window. Every payload has passed its check.
pub enum Event {
    /// The state changed: read [`OfficeHost::state`].
    State,
    /// A page (or `region` of it) at `scale`: `w` x `h` RGBA8.
    Page {
        id: RequestId,
        page: u32,
        scale: f32,
        region: Option<Rect>,
        w: u32,
        h: u32,
        rgba: Vec<u8>,
        missing_content: bool,
    },
    /// The whole outline, once every page of it arrived.
    Outline(Vec<OutlineEntry>),
    /// The document's counts.
    Stats { words: u32 },
    /// The cells of `range` on `sheet`.
    Cells {
        sheet: u32,
        range: CellRange,
        data: CellData,
    },
}

/// Pixels and text are document content: lengths only.
impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::State => f.write_str("State"),
            Event::Page {
                id,
                page,
                scale,
                region,
                w,
                h,
                rgba,
                missing_content,
            } => f
                .debug_struct("Page")
                .field("id", id)
                .field("page", page)
                .field("scale", scale)
                .field("region", region)
                .field("w", w)
                .field("h", h)
                .field("rgba_len", &rgba.len())
                .field("missing_content", missing_content)
                .finish(),
            Event::Outline(entries) => f.debug_struct("Outline").field("entries", &entries.len()).finish(),
            Event::Stats { words } => f.debug_struct("Stats").field("words", words).finish(),
            Event::Cells { sheet, range, data } => f
                .debug_struct("Cells")
                .field("sheet", sheet)
                .field("range", range)
                .field("cells", &data.cells.len())
                .finish(),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The seam: the sandbox in the app, a fake helper in the tests (`office_host_tests.rs`).
// ---------------------------------------------------------------------------------------------------------------

/// A started helper, as far as the host needs it: end it, name it.
pub(crate) trait Helper: Send + Sync + 'static {
    /// Ends the helper now, without asking. Fine on a helper that already ended.
    fn end(&self);
    /// Its process id, where there is one.
    fn process_id(&self) -> Option<u32>;
}

/// A started helper and its two pipes.
pub(crate) struct Link<P> {
    pub(crate) helper: P,
    pub(crate) to_helper: Box<dyn Write + Send>,
    pub(crate) from_helper: Box<dyn Read + Send>,
}

/// Makes sections and starts helpers. Production: [`Confined`] (the sandbox). Tests: a fake helper.
pub(crate) trait Starter: Send + 'static {
    type Section: Send + 'static;
    type Process: Helper;
    /// Whether the helper's file is there.
    fn installed(&self) -> bool;
    /// Windows Error Reporting keeps no dump of the helper (asked before every start; the production one writes
    /// once per process). `Err` carries the system's code.
    fn exclude_from_crash_reports(&mut self) -> Result<(), u32>;
    /// A shared section of `len` bytes, all zero.
    fn section(&mut self, len: usize) -> Result<Self::Section, sandbox::SandboxError>;
    fn section_len(&self, section: &Self::Section) -> usize;
    fn fill(&self, section: &mut Self::Section, bytes: &[u8]);
    fn copy_out(&self, section: &Self::Section, out: &mut [u8]);
    /// Starts one helper. Never an unconfined one in production.
    fn start(&mut self) -> Result<Link<Self::Process>, sandbox::SandboxError>;
    /// Hands `section` to `helper` with exactly `access`.
    fn grant(
        &self,
        helper: &Self::Process,
        section: &Self::Section,
        access: sandbox::Access,
    ) -> Result<sandbox::Granted, sandbox::SandboxError>;
}

/// The production starter: the helper at `helper`, started through the sandbox crate.
pub(crate) struct Confined {
    helper: PathBuf,
}

impl Starter for Confined {
    type Section = sandbox::Section;
    type Process = sandbox::Child;

    fn installed(&self) -> bool {
        self.helper.is_file()
    }

    fn exclude_from_crash_reports(&mut self) -> Result<(), u32> {
        crash_reports::exclude_once(HELPER_EXE)
    }

    fn section(&mut self, len: usize) -> Result<sandbox::Section, sandbox::SandboxError> {
        sandbox::Section::new(len)
    }

    fn section_len(&self, section: &sandbox::Section) -> usize {
        section.len()
    }

    fn fill(&self, section: &mut sandbox::Section, bytes: &[u8]) {
        let n = bytes.len().min(section.len());
        section.as_mut_slice()[..n].copy_from_slice(&bytes[..n]);
    }

    fn copy_out(&self, section: &sandbox::Section, out: &mut [u8]) {
        let n = out.len().min(section.len());
        out[..n].copy_from_slice(&section.as_slice()[..n]);
    }

    fn start(&mut self) -> Result<Link<sandbox::Child>, sandbox::SandboxError> {
        let mut child = sandbox::Sandbox::new(sandbox::Limits::default())?.spawn(&self.helper)?;
        // Right after the spawn the pipes are there; without them `child` is dropped here, which ends it.
        let Some(pipes) = child.take_pipes() else {
            return Err(sandbox::SandboxError::Os {
                step: "take pipes",
                code: 0,
            });
        };
        Ok(Link {
            helper: child,
            to_helper: Box::new(pipes.to_helper),
            from_helper: Box::new(pipes.from_helper),
        })
    }

    fn grant(
        &self,
        helper: &sandbox::Child,
        section: &sandbox::Section,
        access: sandbox::Access,
    ) -> Result<sandbox::Granted, sandbox::SandboxError> {
        helper.grant(section, access)
    }
}

impl Helper for sandbox::Child {
    fn end(&self) {
        // An error here is a helper that is already gone or cannot be reached; its job ends it with this process.
        let _ = self.kill();
    }

    fn process_id(&self) -> Option<u32> {
        Some(self.id())
    }
}

/// The two timers. Production: the contract's 30 s and 15 s; the tests shorten them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Timeouts {
    /// From the start request to the first answer (to `Open`).
    pub(crate) open: Duration,
    /// For the oldest request out, from when it became the oldest.
    pub(crate) request: Duration,
}

impl Timeouts {
    pub(crate) const PRODUCTION: Timeouts = Timeouts {
        open: check::OPEN_TIMEOUT,
        request: check::REQUEST_TIMEOUT,
    };
}

/// The live hosts [`kill_all`] reaches. The process has one ([`PROCESS`]); each test makes its own, so one test's
/// `kill_all_in` never reaches another test's hosts.
pub(crate) struct Registry {
    hosts: Mutex<Vec<Weak<Shared>>>,
}

impl Registry {
    pub(crate) const fn new() -> Registry {
        Registry {
            hosts: Mutex::new(Vec::new()),
        }
    }

    fn add(&self, shared: &Arc<Shared>) {
        let mut hosts = lock(&self.hosts);
        hosts.retain(|host| host.strong_count() > 0);
        hosts.push(Arc::downgrade(shared));
    }
}

/// Closes every live host of `registry` (each kills its helper and wipes what it held). The handles are collected
/// first, then closed outside the registry's lock.
pub(crate) fn kill_all_in(registry: &Registry, why: CloseWhy) {
    let live: Vec<Arc<Shared>> = lock(&registry.hosts).iter().filter_map(Weak::upgrade).collect();
    for shared in live {
        shared.close(why);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// What the window's side and the worker share.
// ---------------------------------------------------------------------------------------------------------------

/// One lock for the closed reason, the kill handle and the answers not yet taken: a close and the worker's
/// "store the handle" or "queue an answer" never pass each other.
struct Shared {
    inner: Mutex<Inner>,
    /// The worker's channel: the window's commands, and the reader threads' frames.
    commands: Sender<Input>,
    wake: Wake,
    /// Bytes of plaintext wiped because nobody took them (a test hook).
    wiped: AtomicUsize,
}

struct Inner {
    closed: Option<CloseWhy>,
    helper: Option<Arc<dyn Helper>>,
    outs: VecDeque<Out>,
    worker_ended: bool,
}

/// What the worker queues for the window.
enum Out {
    State(OfficeState),
    Event(Event),
}

impl Shared {
    /// Kills the helper through the shared handle, wipes every answer not taken, tells the worker, wakes the window.
    /// Never waits for the worker. The first close's reason stays.
    fn close(&self, why: CloseWhy) {
        let (helper, outs) = {
            let mut inner = lock(&self.inner);
            if inner.closed.is_some() {
                return;
            }
            inner.closed = Some(why);
            (inner.helper.take(), std::mem::take(&mut inner.outs))
        };
        if let Some(helper) = helper {
            helper.end();
        }
        let mut wiped = 0;
        for mut out in outs {
            wiped += wipe_out(&mut out);
        }
        self.wiped.fetch_add(wiped, Ordering::SeqCst);
        // The worker may be waiting with no deadline: this wakes it so that it drops every section and ends.
        let _ = self.commands.send(Input::Close);
        (self.wake)();
    }

    fn is_closed(&self) -> bool {
        lock(&self.inner).closed.is_some()
    }

    /// Stores the handle of a helper that just started, so that a close can kill it. `false` when the host was
    /// closed first: the caller kills that helper at once.
    fn adopt(&self, helper: Arc<dyn Helper>) -> bool {
        let mut inner = lock(&self.inner);
        if inner.closed.is_some() {
            return false;
        }
        inner.helper = Some(helper);
        true
    }

    fn disown(&self) {
        lock(&self.inner).helper = None;
    }

    /// Queues `out` for the window and wakes it; after a close it is wiped instead.
    fn push(&self, mut out: Out) {
        {
            let mut inner = lock(&self.inner);
            if inner.closed.is_none() {
                inner.outs.push_back(out);
                drop(inner);
                (self.wake)();
                return;
            }
        }
        let wiped = wipe_out(&mut out);
        self.wiped.fetch_add(wiped, Ordering::SeqCst);
    }

    fn mark_worker_ended(&self) {
        let mut inner = lock(&self.inner);
        inner.helper = None;
        inner.worker_ended = true;
    }
}

/// What reaches the worker.
enum Input {
    /// A frame from helper number `link` (an earlier helper's frames are ignored).
    Frame {
        link: u64,
        frame: Result<Frame, FrameError>,
    },
    Command(Command),
    Close,
}

enum Command {
    Render(RenderAsk),
    Cancel(RequestId),
    Outline,
    Stats,
    Cells { sheet: u32, range: CellRange },
    Retry,
}

/// A page asked by the window; `w` x `h` from `check::asked_size`.
#[derive(Clone, Copy, Debug)]
struct RenderAsk {
    id: RequestId,
    page: u32,
    scale: f32,
    region: Option<Rect>,
    w: u32,
    h: u32,
}

// ---------------------------------------------------------------------------------------------------------------
// The window's side.
// ---------------------------------------------------------------------------------------------------------------

/// One open Office document and its helper.
pub struct OfficeHost {
    shared: Arc<Shared>,
    state: OfficeState,
    next_request: u32,
}

impl OfficeHost {
    /// Opens `bytes` (a document of `kind`) with the helper at `helper`. Never blocks: the helper starts on a worker.
    pub fn open(helper: &Path, kind: DocKind, bytes: Zeroizing<Vec<u8>>, wake: Wake) -> OfficeHost {
        let starter = Confined {
            helper: helper.to_path_buf(),
        };
        open_with(starter, &PROCESS, Timeouts::PRODUCTION, kind, bytes, wake)
    }

    pub fn state(&self) -> &OfficeState {
        &self.state
    }

    /// What arrived since the last poll; applies the state changes. Never blocks.
    pub fn poll(&mut self) -> Vec<Event> {
        let (outs, closed) = {
            let mut inner = lock(&self.shared.inner);
            (std::mem::take(&mut inner.outs), inner.closed)
        };
        let mut events = Vec::with_capacity(outs.len());
        for out in outs {
            match out {
                Out::State(state) => {
                    self.set_state(state);
                    events.push(Event::State);
                }
                Out::Event(event) => events.push(event),
            }
        }
        if let Some(why) = closed
            && self.become_closed(why)
        {
            events.push(Event::State);
        }
        events
    }

    /// Asks for page `page` (or `region` of it) at `scale` (from `check::render_scale(zoom, dpi)`). `None` unless
    /// the document is open, the page exists, the ask can be sized (`check::asked_size`, at most 4,096 px a side) and
    /// the whole page stays within [`MAX_PAGE_SIDE_PX`].
    pub fn request_page(&mut self, page: u32, scale: f32, region: Option<Rect>) -> Option<RequestId> {
        let OfficeState::Pages { pages, sizes } = &self.state else {
            return None;
        };
        if page >= *pages {
            return None;
        }
        let size = *sizes.get(page as usize)?;
        let probe = office::Render {
            page,
            scale,
            region,
            out: office::GrantRef { wire: 0, len: 0 },
        };
        let (w, h) = check::asked_size(&probe, &size, &check::Limits::CONTRACT).ok()?;
        let (page_w, page_h) = check::page_pixels(&size, scale)?;
        if page_w > MAX_PAGE_SIDE_PX || page_h > MAX_PAGE_SIDE_PX {
            return None;
        }
        let id = RequestId(self.next_request);
        self.next_request = self.next_request.wrapping_add(1);
        self.command(Command::Render(RenderAsk {
            id,
            page,
            scale,
            region,
            w,
            h,
        }));
        Some(id)
    }

    /// Cancels a page: still waiting here, it is never asked; already asked, its answer is checked and discarded.
    pub fn cancel(&mut self, id: RequestId) {
        if self.is_open() {
            self.command(Command::Cancel(id));
        }
    }

    /// Asks for the whole outline (in pages, until the helper says there is no more).
    pub fn request_outline(&mut self) -> bool {
        if !matches!(self.state, OfficeState::Pages { .. }) {
            return false;
        }
        self.command(Command::Outline);
        true
    }

    /// Asks for the document's counts.
    pub fn request_stats(&mut self) -> bool {
        if !matches!(self.state, OfficeState::Pages { .. }) {
            return false;
        }
        self.command(Command::Stats);
        true
    }

    /// Asks for the cells of `range` on sheet `sheet` (a range too large for one frame is asked again in halves,
    /// each half its own [`Event::Cells`]).
    pub fn request_cells(&mut self, sheet: u32, range: CellRange) -> bool {
        let OfficeState::Sheets { sheets, more } = &self.state else {
            return false;
        };
        let count = sheets.len() as u64 + u64::from(*more);
        if u64::from(sheet) >= count || range.rows == 0 || range.cols == 0 {
            return false;
        }
        self.command(Command::Cells { sheet, range });
        true
    }

    /// Try again: one new helper, once per open. `false` (and nothing started) otherwise.
    pub fn retry(&mut self) -> bool {
        if !matches!(self.state, OfficeState::Stopped { can_retry: true, .. }) {
            return false;
        }
        self.command(Command::Retry);
        self.set_state(OfficeState::Opening);
        true
    }

    /// Kills the helper now and drops what this host held. Never blocks.
    pub fn close(&mut self, why: CloseWhy) {
        self.shared.close(why);
        let why = lock(&self.shared.inner).closed.unwrap_or(why);
        self.become_closed(why);
    }

    /// The live helper's process id.
    pub fn helper_pid(&self) -> Option<u32> {
        lock(&self.shared.inner)
            .helper
            .as_ref()
            .and_then(|helper| helper.process_id())
    }

    /// Test hook: plaintext bytes wiped because nobody took them.
    pub(crate) fn wiped_bytes(&self) -> usize {
        self.shared.wiped.load(Ordering::SeqCst)
    }

    /// Test hook: the worker thread has ended (or never ran).
    pub(crate) fn worker_ended(&self) -> bool {
        lock(&self.shared.inner).worker_ended
    }

    fn is_open(&self) -> bool {
        matches!(self.state, OfficeState::Pages { .. } | OfficeState::Sheets { .. })
    }

    fn command(&self, command: Command) {
        // No worker (a refusal before any start, or one that ended): nothing to do.
        let _ = self.shared.commands.send(Input::Command(command));
    }

    fn set_state(&mut self, state: OfficeState) {
        let mut old = std::mem::replace(&mut self.state, state);
        let wiped = wipe_state(&mut old);
        self.shared.wiped.fetch_add(wiped, Ordering::SeqCst);
    }

    fn become_closed(&mut self, why: CloseWhy) -> bool {
        if matches!(self.state, OfficeState::Closed(_)) {
            return false;
        }
        self.set_state(OfficeState::Closed(why));
        true
    }
}

impl Drop for OfficeHost {
    fn drop(&mut self) {
        self.close(CloseWhy::Closed);
    }
}

/// [`OfficeHost::open`] with its parts named: the starter, the registry it joins, the timers. Never blocks.
pub(crate) fn open_with<S: Starter>(
    starter: S,
    registry: &Registry,
    timeouts: Timeouts,
    kind: DocKind,
    bytes: Zeroizing<Vec<u8>>,
    wake: Wake,
) -> OfficeHost {
    let (commands, inputs) = mpsc::channel();
    let shared = Arc::new(Shared {
        inner: Mutex::new(Inner {
            closed: None,
            helper: None,
            outs: VecDeque::new(),
            worker_ended: false,
        }),
        commands,
        wake,
        wiped: AtomicUsize::new(0),
    });
    let mut host = OfficeHost {
        shared: shared.clone(),
        state: OfficeState::Opening,
        next_request: 0,
    };
    // Refused before anything starts: `bytes` is wiped when this returns.
    let refused = if bytes.len() as u64 > check::MAX_INPUT_BYTES {
        Some(CannotOpen::TooLarge)
    } else if bytes.is_empty() {
        Some(CannotOpen::Damaged)
    } else {
        None
    };
    if let Some(why) = refused {
        host.state = OfficeState::CannotOpen(why);
        shared.mark_worker_ended();
        return host;
    }
    registry.add(&shared);
    if !worker::spawn(starter, kind, shared.clone(), inputs, timeouts, bytes) {
        // The worker (and the bytes in it) was dropped: wiped.
        host.state = OfficeState::Stopped {
            why: Stop::Failed,
            can_retry: false,
        };
        shared.mark_worker_ended();
    }
    host
}

// Each wipe counts the bytes BEFORE `zeroize()`, which also empties the buffer.

fn wipe_out(out: &mut Out) -> usize {
    match out {
        Out::State(state) => wipe_state(state),
        Out::Event(event) => wipe_event(event),
    }
}

fn wipe_state(state: &mut OfficeState) -> usize {
    match state {
        OfficeState::Sheets { sheets, .. } => sheets
            .iter_mut()
            .map(|sheet| {
                let n = sheet.name.len();
                sheet.name.zeroize();
                n
            })
            .sum(),
        _ => 0,
    }
}

fn wipe_event(event: &mut Event) -> usize {
    match event {
        Event::Page { rgba, .. } => {
            let n = rgba.len();
            rgba.zeroize();
            n
        }
        Event::Outline(entries) => wipe_entries(entries),
        Event::Cells { data, .. } => wipe_cells(data),
        Event::State | Event::Stats { .. } => 0,
    }
}

fn wipe_entries(entries: &mut [OutlineEntry]) -> usize {
    entries
        .iter_mut()
        .map(|entry| {
            let n = entry.text.len();
            entry.text.zeroize();
            n
        })
        .sum()
}

fn wipe_cells(data: &mut CellData) -> usize {
    data.cells
        .iter_mut()
        .map(|cell| {
            let n = cell.text.len();
            cell.text.zeroize();
            n
        })
        .sum()
}

fn wipe_message(message: &mut Message) {
    match message {
        Message::OutlineData(data) => {
            wipe_entries(&mut data.entries);
        }
        Message::CellData(data) => {
            wipe_cells(data);
        }
        Message::OpenedSheets(opened) => {
            for sheet in &mut opened.sheets {
                sheet.name.zeroize();
            }
        }
        _ => {}
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

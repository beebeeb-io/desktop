//! The worker of one open document: it starts the helper, sends the requests, checks every answer and keeps the
//! timers (the rules are in `office_host.rs`). It runs on its own thread; a reader thread per helper puts the helper's
//! frames into the same channel as the window's commands, and a writer thread per helper writes the requests.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SendError, Sender};
use std::sync::{Arc, Weak};
use std::thread;
use std::time::Instant;

use beebeeb_office_protocol::check;
use beebeeb_office_protocol::frame::{self, Frame, FrameError, Ids, Side};
use beebeeb_office_protocol::office::{self, ErrorCode, Message};
use beebeeb_office_sandbox as sandbox;
use zeroize::{Zeroize, Zeroizing};

use super::{
    Anchor, AnchorAsk, CannotOpen, CellData, Command, DocKind, Event, HANDLE_SLACK, HELPER_EXE, Helper, Input, Link,
    MAX_IN_FLIGHT, MAX_KNOWN_ANCHORS, MAX_SPLIT_OUTSTANDING, MAX_SPLIT_REQUESTS, NoPreview, OfficeState, Out,
    OutlineEntry, PageSize, Received, RenderAsk, RequestId, Shared, Starter, Stop, Timeouts, lock, wipe_cells,
    wipe_entries, wipe_message,
};

// The editing session's side of the worker: its requests, the checks of its answers, the save handshake.
#[path = "edit.rs"]
pub(crate) mod edit;

// The document the host keeps for a restart, and the restart itself with the snapshot timing.
#[path = "held.rs"]
mod held;
#[path = "restart.rs"]
pub(crate) mod restart;

use edit::Mode;
use held::Held;
use restart::Cadence;

/// Starts the worker of one document on a thread of its own. `false` when no thread could be made: the worker, and
/// the bytes in it, are dropped (wiped).
pub(super) fn spawn<S: Starter>(
    starter: S,
    mode: Mode,
    kind: DocKind,
    shared: Arc<Shared>,
    inputs: Receiver<Input>,
    timeouts: Timeouts,
    cadence: Cadence,
    bytes: Option<Zeroizing<Vec<u8>>>,
    scratch: Option<beebeeb_engine::app::office_scratch::ScratchStore>,
) -> bool {
    // An editing session keeps what it was opened with, for a restart; large copies spill sealed to scratch.
    let held = match mode {
        Mode::View => None,
        Mode::Edit(_) => Some(Held::with_scratch(bytes.clone(), scratch)),
        Mode::New(..) => Some(Held::with_scratch(None, scratch)),
    };
    let worker = Worker {
        held,
        rs: restart::State::new(cadence),
        starter,
        mode,
        kind,
        shared,
        inputs,
        timeouts,
        doc: bytes.map_or(Doc::Blank, Doc::Bytes),
        run: None,
        links: 0,
        retried: false,
        phase: Phase::Opening,
    };
    thread::Builder::new()
        .name("office-host".to_owned())
        .spawn(move || worker.run())
        .is_ok()
}

/// The document: its bytes until they are in a section, then the section (kept for Try again), then gone.
enum Doc<T> {
    /// A new document: there are no bytes.
    Blank,
    Bytes(Zeroizing<Vec<u8>>),
    Section(T),
    Gone,
}

enum Phase {
    Opening,
    Pages { sizes: Vec<PageSize> },
    Sheets,
    Stopped { can_retry: bool },
    Done,
}

type Outgoing = (u16, u32, Vec<u8>);

/// One started helper.
struct Run<S: Starter> {
    link: u64,
    helper: Arc<S::Process>,
    writer: Sender<Outgoing>,
    ids: Ids,
    /// When this helper had started (its start returned, after any wait for the launch lock): the open timer counts
    /// from here.
    started_at: Instant,
    /// When the oldest request out became the oldest: the request timer counts from here.
    oldest_since: Instant,
    /// The requests out, oldest first: the helper answers them in this order.
    sent: VecDeque<Sent<S::Section>>,
    /// Pages and anchors not sent yet (more than [`MAX_IN_FLIGHT`] asked).
    waiting: VecDeque<Ask>,
    /// A save is out (`SaveSize` to `SavedInto`): edits wait in `deferred`, so that the token stays current.
    save_busy: bool,
    /// Edit requests that came during a save, in order.
    deferred: VecDeque<Command>,
    /// The outline's entries so far, while it is being asked.
    outline: Option<Vec<OutlineEntry>>,
    /// When the outline's first page was asked: its budget counts from here.
    outline_since: Option<Instant>,
    /// The budget of each range the window asked, while any of its requests is out.
    splits: Vec<Split>,
    next_split: u64,
    /// The helper's open handles at its first checked answer (Opened/Sheets): the ceiling counts from here. The
    /// loader's handles (taken between the resume and that answer) belong to it. `None`: not taken yet, or the
    /// system could not say, nothing is bounded.
    handle_baseline: Option<u32>,
    /// Whether the baseline was taken (once, at the first checked answer).
    baselined: bool,
}

/// One range the window asked, and the halves asked for it after `RangeTooLarge`.
struct Split {
    id: u64,
    /// Its requests out now.
    outstanding: u32,
    /// Halves asked for it so far.
    halves: u32,
    /// When the window's ask was sent: [`Timeouts::split`] counts from here.
    since: Instant,
}

/// A page or a chart or picture the window asked: they share [`MAX_IN_FLIGHT`] and the waiting queue.
enum Ask {
    Page(RenderAsk),
    Anchor(AnchorAsk),
}

impl Ask {
    fn id(&self) -> RequestId {
        match self {
            Ask::Page(ask) => ask.id,
            Ask::Anchor(ask) => ask.id,
        }
    }
}

struct Sent<T> {
    wire: u32,
    asked: Asked<T>,
    cancelled: bool,
}

enum Asked<T> {
    Open,
    Render {
        ask: RenderAsk,
        request: office::Render,
        section: T,
    },
    Anchor {
        ask: AnchorAsk,
        request: office::RenderAnchor,
        section: T,
    },
    Outline(office::Outline),
    Stats,
    /// A request of an editing session.
    Edit(edit::EditAsked<T>),
    /// A range, or a half of one, and the window's ask it belongs to.
    Cells {
        asked: office::Cells,
        split: u64,
    },
}

struct Worker<S: Starter> {
    starter: S,
    mode: Mode,
    kind: DocKind,
    shared: Arc<Shared>,
    inputs: Receiver<Input>,
    timeouts: Timeouts,
    doc: Doc<S::Section>,
    run: Option<Run<S>>,
    /// Helpers started so far: numbers each helper's frames.
    links: u64,
    retried: bool,
    phase: Phase,
    /// What a restart opens (editing sessions only).
    held: Option<Held>,
    /// Snapshot timing and crash bookkeeping.
    rs: restart::State,
}

/// A panic on the worker thread: the shared helper is ended, a document parked for a start is dropped, the window says
/// "Renderer stopped" with no Try again, and the worker counts as ended. Does nothing when the worker returns normally.
struct PanicGuard(Arc<Shared>);

impl Drop for PanicGuard {
    fn drop(&mut self) {
        if !thread::panicking() {
            return;
        }
        let (helper, parked) = {
            let mut inner = lock(&self.0.inner);
            (inner.helper.take(), inner.parked.take())
        };
        drop(parked);
        if let Some(helper) = helper {
            helper.end();
        }
        self.0.push(Out::State(OfficeState::Stopped {
            why: Stop::Failed,
            can_retry: false,
        }));
        self.0.mark_worker_ended();
    }
}

impl<S: Starter> Worker<S> {
    fn run(mut self) {
        // Dropped before `self` on an unwind: the guard ends the helper while the worker's handles still exist.
        let _guard = PanicGuard(self.shared.clone());
        self.start_helper();
        while !self.done() {
            let input = match self.next_due() {
                Some((at, why)) => {
                    let now = Instant::now();
                    if at <= now {
                        self.fire(why);
                        continue;
                    }
                    match self.inputs.recv_timeout(at - now) {
                        Ok(input) => input,
                        Err(RecvTimeoutError::Timeout) => {
                            self.fire(why);
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                None => match self.inputs.recv() {
                    Ok(input) => input,
                    Err(_) => break,
                },
            };
            match input {
                Input::Close => break,
                Input::Frame { link, frame } => self.on_frame(link, frame),
                Input::Command(command) => self.on_command(command),
                Input::Flush => self.on_flush(),
            }
        }
        self.finish();
    }

    fn done(&self) -> bool {
        matches!(self.phase, Phase::Done) || self.shared.is_closed()
    }

    /// The next deadline and what missing it means. The oldest request's timer (`Open` from the start request, any
    /// other from when it became the oldest) is `TimedOut`; a budget (an asked range and its halves, an outline) is
    /// `Protocol`: every answer came in time, and there was no end to them.
    fn deadline(&self) -> Option<(Instant, Stop)> {
        let run = self.run.as_ref()?;
        let mut next = run.sent.front().map(|oldest| match oldest.asked {
            Asked::Open => (run.started_at + self.timeouts.open, Stop::TimedOut),
            _ => (run.oldest_since + self.timeouts.request, Stop::TimedOut),
        });
        let budgets = run
            .splits
            .iter()
            .map(|split| split.since + self.timeouts.split)
            .chain(run.outline_since.map(|since| since + self.timeouts.outline));
        for at in budgets {
            if next.is_none_or(|(soonest, _)| at < soonest) {
                next = Some((at, Stop::Protocol));
            }
        }
        next
    }

    /// Drops every section and the document, and wipes the helper frames still queued. A close killed the helper
    /// already (the shared handle); dropping its handle here ends it too.
    fn finish(mut self) {
        if let Some(run) = self.run.take() {
            wipe_run(run);
        }
        self.doc = Doc::Gone;
        // Frames nobody will handle: wiped and counted. Then the channel's receiving end goes, so that a frame read
        // later fails to send and its reader wipes it.
        while let Ok(input) = self.inputs.try_recv() {
            self.shared.wipe_input(input);
        }
        drop(self.inputs);
        self.shared.mark_worker_ended();
    }

    fn start_helper(&mut self) {
        if !self.starter.installed() {
            return self.end_with(OfficeState::NoPreview(NoPreview::NotInstalled));
        }
        // Only the helper of our own name is started: that is the name crash reports leave out.
        let Some(name) = self.starter.file_name().filter(|name| name == HELPER_EXE) else {
            return self.end_with(OfficeState::NoPreview(NoPreview::NotInstalled));
        };
        // Before EVERY start: Windows Error Reporting keeps no dump of it.
        if self.starter.exclude_from_crash_reports(&name).is_err() {
            return self.stopped(Stop::Failed);
        }
        // The document's section, once: the bytes are copied in and wiped.
        self.doc = match std::mem::replace(&mut self.doc, Doc::Gone) {
            Doc::Bytes(bytes) => match self.starter.section(bytes.len()) {
                Ok(mut section) => {
                    self.starter.fill(&mut section, &bytes);
                    Doc::Section(section)
                }
                Err(error) => {
                    self.doc = Doc::Bytes(bytes);
                    return self.start_failed(error);
                }
            },
            other => other,
        };
        if matches!(self.doc, Doc::Gone) {
            return self.stopped(Stop::Failed);
        }
        // The start may wait for the launch lock, which Windows' app chooser holds for as long as it is open. Meanwhile
        // the worker takes no close, so the document's section waits where a close drops it at once.
        if !self.shared.park(Box::new(std::mem::replace(&mut self.doc, Doc::Gone))) {
            return;
        }
        let started = self.starter.start();
        self.doc = self
            .shared
            .unpark()
            .and_then(|doc| doc.downcast::<Doc<S::Section>>().ok())
            .map_or(Doc::Gone, |doc| *doc);
        let link = match started {
            Ok(link) => link,
            Err(error) => return self.start_failed(error),
        };
        // The open timer counts from here: the wait for the lock was not the helper's time.
        let started_at = Instant::now();
        let Link {
            helper,
            to_helper,
            from_helper,
        } = link;
        let helper = Arc::new(helper);
        let handle: Arc<dyn Helper> = helper.clone();
        // A close that came while the helper was starting: it is killed now and nothing is sent.
        if !self.shared.adopt(handle) {
            helper.end();
            return;
        }
        self.links += 1;
        let link_no = self.links;
        let writer = spawn_writer(to_helper);
        let reading = spawn_reader(
            from_helper,
            link_no,
            self.shared.commands.clone(),
            Arc::downgrade(&self.shared),
        );
        let (Some(writer), true) = (writer, reading) else {
            helper.end();
            self.shared.disown();
            return self.stopped(Stop::Failed);
        };
        self.run = Some(Run {
            link: link_no,
            helper,
            writer,
            ids: Ids::new(Side::App),
            started_at,
            oldest_since: started_at,
            sent: VecDeque::new(),
            waiting: VecDeque::new(),
            save_busy: false,
            deferred: VecDeque::new(),
            outline: None,
            outline_since: None,
            splits: Vec::new(),
            next_split: 0,
            // Taken at the first checked answer, not at the start: the loader is still mapping then.
            handle_baseline: None,
            baselined: false,
        });
        let Some(first) = self.first_message() else {
            return self.stopped(Stop::Failed);
        };
        self.ask(first, Asked::Open);
    }

    fn start_failed(&mut self, error: sandbox::SandboxError) {
        if error == sandbox::SandboxError::Unsupported {
            self.end_with(OfficeState::NoPreview(NoPreview::NoSandbox));
        } else if error == sandbox::SandboxError::NeedsNewerWindows {
            crate::windows_floor::note_needs_windows_update();
            self.end_with(OfficeState::NoPreview(NoPreview::NeedsWindowsUpdate));
        } else {
            self.stopped(Stop::Failed);
        }
    }

    /// Kills the running helper and drops its sections. The anchors it named go with it.
    fn kill_run(&mut self) {
        if let Some(run) = self.run.take() {
            run.helper.end();
            self.shared.disown();
            lock(&self.shared.inner).anchors.clear();
            wipe_run(run);
        }
    }

    /// "Renderer stopped". The document's section stays for Try again; without one it goes now.
    fn stopped(&mut self, why: Stop) {
        // A snapshot a lock waits for can no longer come from this helper.
        self.flush_complete();
        // An editing session whose helper exited goes on with a new one (or read-only).
        if why == Stop::Exited && self.try_restart() {
            return;
        }
        self.kill_run();
        // An editing session has no Try again: a new helper would not hold the edits.
        let can_retry = !self.retried && !matches!(self.doc, Doc::Gone) && matches!(self.mode, Mode::View);
        if can_retry {
            self.phase = Phase::Stopped { can_retry: true };
        } else {
            self.phase = Phase::Done;
            self.doc = Doc::Gone;
        }
        self.shared.push(Out::State(OfficeState::Stopped { why, can_retry }));
    }

    /// A state with no Try again (no preview, cannot open): the helper and the document go.
    fn end_with(&mut self, state: OfficeState) {
        self.kill_run();
        self.doc = Doc::Gone;
        self.phase = Phase::Done;
        self.shared.push(Out::State(state));
    }

    fn retry(&mut self) {
        if !matches!(self.phase, Phase::Stopped { can_retry: true }) {
            return;
        }
        self.retried = true;
        self.phase = Phase::Opening;
        self.start_helper();
    }

    fn on_command(&mut self, command: Command) {
        match command {
            Command::Retry => self.retry(),
            Command::Render(ask) => {
                if !matches!(self.phase, Phase::Pages { .. }) {
                    return;
                }
                let Some(run) = self.run.as_mut() else {
                    return;
                };
                run.waiting.push_back(Ask::Page(ask));
                self.pump_renders();
            }
            Command::RenderAnchor(ask) => {
                if !matches!(self.phase, Phase::Sheets) {
                    return;
                }
                let Some(run) = self.run.as_mut() else {
                    return;
                };
                run.waiting.push_back(Ask::Anchor(ask));
                self.pump_renders();
            }
            Command::Edit(command) => self.edit_command(command),
            Command::Cancel(id) => self.cancel(id),
            Command::Outline => self.start_outline(),
            Command::Stats => {
                if matches!(self.phase, Phase::Pages { .. }) {
                    self.ask(Message::Stats(office::Stats), Asked::Stats);
                }
            }
            Command::Cells { sheet, range } => {
                if !matches!(self.phase, Phase::Sheets) {
                    return;
                }
                let Some(run) = self.run.as_mut() else {
                    return;
                };
                // The ask's budget: one request out, no halves yet, the clock running.
                let split = run.next_split;
                run.next_split += 1;
                run.splits.push(Split {
                    id: split,
                    outstanding: 1,
                    halves: 0,
                    since: Instant::now(),
                });
                let asked = office::Cells { sheet, range };
                self.ask(Message::Cells(asked), Asked::Cells { asked, split });
            }
        }
    }

    /// Sends `message` with a new id and keeps it as the newest request out.
    fn ask(&mut self, message: Message, asked: Asked<S::Section>) -> bool {
        let Ok(payload) = message.encode() else {
            self.stopped(Stop::Failed);
            return false;
        };
        let Some(run) = self.run.as_mut() else {
            return false;
        };
        let id = run.ids.next_id();
        if run.writer.send((message.kind(), id, payload)).is_err() {
            // The writer ended: the helper's pipe is gone.
            self.stopped(Stop::Exited);
            return false;
        }
        if run.sent.is_empty() {
            run.oldest_since = Instant::now();
        }
        run.sent.push_back(Sent {
            wire: id,
            asked,
            cancelled: false,
        });
        true
    }

    /// Sends waiting pages and anchors while fewer than [`MAX_IN_FLIGHT`] are out (a cancelled one counts until
    /// answered).
    fn pump_renders(&mut self) {
        loop {
            let Some(run) = self.run.as_mut() else {
                return;
            };
            let out = run
                .sent
                .iter()
                .filter(|sent| matches!(sent.asked, Asked::Render { .. } | Asked::Anchor { .. }))
                .count();
            if out >= MAX_IN_FLIGHT {
                return;
            }
            let Some(ask) = run.waiting.pop_front() else {
                return;
            };
            let sent = match ask {
                Ask::Page(ask) => self.send_render(ask),
                Ask::Anchor(ask) => self.send_anchor(ask),
            };
            if !sent {
                return;
            }
        }
    }

    /// A section of exactly the asked `w * h * 4` bytes, granted read-write, and the `RenderAnchor` at the asked scale.
    fn send_anchor(&mut self, ask: AnchorAsk) -> bool {
        let len = ask.w as usize * ask.h as usize * 4;
        let section = match self.starter.section(len) {
            Ok(section) => section,
            Err(_) => {
                self.stopped(Stop::Failed);
                return false;
            }
        };
        let granted = self
            .run
            .as_ref()
            .map(|run| self.starter.grant(&run.helper, &section, sandbox::Access::ReadWrite));
        let Some(Ok(granted)) = granted else {
            self.stopped(Stop::Failed);
            return false;
        };
        let request = office::RenderAnchor {
            sheet: ask.sheet,
            anchor: ask.anchor.id,
            scale: ask.scale,
            out: office::GrantRef {
                wire: granted.wire,
                len: granted.len,
            },
        };
        self.ask(Message::RenderAnchor(request), Asked::Anchor { ask, request, section })
    }

    /// A section of exactly the asked `w * h * 4` bytes, granted read-write, and the `Render` at the asked scale.
    fn send_render(&mut self, ask: RenderAsk) -> bool {
        let len = ask.w as usize * ask.h as usize * 4;
        let section = match self.starter.section(len) {
            Ok(section) => section,
            Err(_) => {
                self.stopped(Stop::Failed);
                return false;
            }
        };
        let granted = self
            .run
            .as_ref()
            .map(|run| self.starter.grant(&run.helper, &section, sandbox::Access::ReadWrite));
        let Some(Ok(granted)) = granted else {
            self.stopped(Stop::Failed);
            return false;
        };
        let request = office::Render {
            page: ask.page,
            scale: ask.scale,
            region: ask.region,
            out: office::GrantRef {
                wire: granted.wire,
                len: granted.len,
            },
        };
        self.ask(Message::Render(request), Asked::Render { ask, request, section })
    }

    fn cancel(&mut self, id: RequestId) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        if let Some(at) = run.waiting.iter().position(|ask| ask.id() == id) {
            let _ = run.waiting.remove(at);
            return;
        }
        let sent = run.sent.iter_mut().find(|sent| {
            !sent.cancelled
                && match &sent.asked {
                    Asked::Render { ask, .. } => ask.id == id,
                    Asked::Anchor { ask, .. } => ask.id == id,
                    _ => false,
                }
        });
        let Some(sent) = sent else {
            return;
        };
        sent.cancelled = true;
        let wire = sent.wire;
        if let Ok(payload) = office::Cancel.encode() {
            // The frame's id is the cancelled request's. A failed send: the helper is gone and its reader says so.
            let _ = run.writer.send((office::KIND_CANCEL, wire, payload));
        }
    }

    fn start_outline(&mut self) {
        if !matches!(self.phase, Phase::Pages { .. }) {
            return;
        }
        let Some(run) = self.run.as_mut() else {
            return;
        };
        if run.outline.is_some() {
            return;
        }
        run.outline = Some(Vec::new());
        run.outline_since = Some(Instant::now());
        let asked = office::Outline { from: 0 };
        self.ask(Message::Outline(asked), Asked::Outline(asked));
    }

    fn on_frame(&mut self, link: u64, frame: Result<Received, FrameError>) {
        let (current, oldest) = match &self.run {
            Some(run) => (run.link, run.sent.front().map(|sent| sent.wire)),
            // After a stop: no helper runs, nothing is asked. Wiped, not handled.
            None => return self.unhandled(frame),
        };
        if current != link {
            // An earlier helper's frame. Wiped, not handled.
            return self.unhandled(frame);
        }
        let frame = match frame {
            Ok(frame) => frame,
            Err(FrameError::TooLong | FrameError::TooShort) => return self.stopped(Stop::Protocol),
            // The pipe closed or failed: the helper is gone (a crash, out of memory).
            Err(FrameError::Closed | FrameError::Truncated | FrameError::Io(_)) => return self.stopped(Stop::Exited),
        };
        let Received { kind, id, mut payload } = frame;
        // An app id, and the oldest one out: the helper answers in request order.
        if check::check_answer_id(id, |id| Some(id) == oldest).is_err() {
            payload.zeroize();
            return self.stopped(Stop::Protocol);
        }
        let decoded = Message::decode(kind, &payload);
        payload.zeroize();
        let Ok(message) = decoded else {
            return self.stopped(Stop::Protocol);
        };
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let Some(sent) = run.sent.pop_front() else {
            let mut message = message;
            wipe_message(&mut message);
            return self.stopped(Stop::Protocol);
        };
        run.oldest_since = Instant::now();
        self.answer(sent, message);
    }

    /// A helper frame the worker does not handle: its payload is wiped and counted.
    fn unhandled(&self, frame: Result<Received, FrameError>) {
        if let Ok(mut frame) = frame {
            self.shared.wipe_frame(&mut frame);
        }
    }

    /// After a checked answer, the helper's open handles stay within an absolute ceiling: its count at its first
    /// checked answer (Opened/Sheets; the loader's handles are in it), plus one per render out ([`MAX_IN_FLIGHT`]), plus [`HANDLE_SLACK`]. Over
    /// it, the helper is stopped (`Protocol`) and `false` comes back: a helper that keeps the sections it was handed
    /// pins their memory. Where the count cannot be read (at the start or now), nothing is bounded.
    fn handles_ok(&mut self) -> bool {
        let Some(run) = self.run.as_mut() else {
            return false;
        };
        if !run.baselined {
            run.baselined = true;
            run.handle_baseline = run.helper.handle_count();
            return true;
        }
        let (Some(base), Some(count)) = (run.handle_baseline, run.helper.handle_count()) else {
            return true;
        };
        let over = u64::from(count) > u64::from(base) + MAX_IN_FLIGHT as u64 + u64::from(HANDLE_SLACK);
        if over {
            self.stopped(Stop::Protocol);
        }
        !over
    }

    fn kind_has_pages(&self) -> bool {
        matches!(self.kind, DocKind::Docx | DocKind::Doc | DocKind::Pptx)
    }

    /// The answer to the oldest request: it must be the kind that request takes, then pass its check.
    fn answer(&mut self, sent: Sent<S::Section>, message: Message) {
        let Sent { asked, cancelled, .. } = sent;
        let pages = self.kind_has_pages();
        match (asked, message) {
            (Asked::Open, Message::Opened(opened)) if pages => self.opened(opened),
            (Asked::Open, Message::OpenedSheets(opened)) if !pages => self.opened_sheets(opened),
            (Asked::Open, Message::Error(error)) => self.open_refused(error.code),
            (Asked::Edit(asked), message) => self.edit_answer(asked, message),
            // Cancelled: its answer is checked for id, order and kind (above), then discarded with its section.
            (Asked::Render { .. } | Asked::Anchor { .. }, Message::Rendered(_) | Message::Error(_)) if cancelled => {
                let _ = self.handles_ok();
            }
            (Asked::Render { ask, request, section }, Message::Rendered(got)) => {
                self.rendered(ask, request, section, got)
            }
            (Asked::Anchor { ask, request, section }, Message::Rendered(got)) => {
                self.rendered_anchor(ask, request, section, got)
            }
            (Asked::Outline(asked), Message::OutlineData(got)) => self.outline_page(asked, got),
            (Asked::Stats, Message::StatsData(got)) => {
                if self.handles_ok() {
                    self.shared.push(Out::Event(Event::Stats { words: got.words }));
                }
            }
            (Asked::Cells { asked, split }, Message::CellData(got)) => self.cells(asked, split, got),
            (Asked::Cells { asked, split }, Message::Error(error)) => self.cells_refused(asked, split, error.code),
            (Asked::Render { .. } | Asked::Anchor { .. } | Asked::Outline(_) | Asked::Stats, Message::Error(error)) => {
                self.request_refused(error.code)
            }
            // Any other kind of answer: not what was asked.
            (_, mut other) => {
                wipe_message(&mut other);
                self.stopped(Stop::Protocol);
            }
        }
        self.pump_renders();
    }

    fn opened(&mut self, opened: office::Opened) {
        if check::check_opened(&opened, &check::Limits::CONTRACT).is_err() {
            return self.stopped(Stop::Protocol);
        }
        if !self.handles_ok() {
            return;
        }
        let office::Opened { pages, sizes } = opened;
        self.phase = Phase::Pages { sizes: sizes.clone() };
        self.shared.push(Out::State(OfficeState::Pages { pages, sizes }));
        self.opened_after_restart();
    }

    fn opened_sheets(&mut self, mut opened: office::OpenedSheets) {
        if check::check_sheets(&opened, &check::Limits::CONTRACT).is_err() {
            for sheet in &mut opened.sheets {
                sheet.name.zeroize();
            }
            return self.stopped(Stop::Protocol);
        }
        if !self.handles_ok() {
            for sheet in &mut opened.sheets {
                sheet.name.zeroize();
            }
            return;
        }
        self.phase = Phase::Sheets;
        self.shared.push(Out::State(OfficeState::Sheets {
            sheets: opened.sheets,
            more: opened.more,
        }));
        self.opened_after_restart();
    }

    /// The helper's answer to `Open` with an error: its typed state; the helper goes.
    fn open_refused(&mut self, code: ErrorCode) {
        match code {
            ErrorCode::Unsupported => self.end_with(OfficeState::NoPreview(NoPreview::ThisType)),
            ErrorCode::Password => self.end_with(OfficeState::NoPreview(NoPreview::Password)),
            ErrorCode::Damaged => self.end_with(OfficeState::CannotOpen(CannotOpen::Damaged)),
            ErrorCode::TooLarge => self.end_with(OfficeState::CannotOpen(CannotOpen::TooLarge)),
            ErrorCode::Internal => self.stopped(Stop::Failed),
            // Only ever an answer to `Cells`.
            ErrorCode::RangeTooLarge => self.stopped(Stop::Protocol),
        }
    }

    /// An error to a request that was not cancelled (not `Cells`).
    fn request_refused(&mut self, code: ErrorCode) {
        if code == ErrorCode::RangeTooLarge {
            self.stopped(Stop::Protocol);
        } else {
            self.stopped(Stop::Failed);
        }
    }

    fn rendered(&mut self, ask: RenderAsk, request: office::Render, section: S::Section, got: office::Rendered) {
        let Phase::Pages { sizes } = &self.phase else {
            return self.stopped(Stop::Protocol);
        };
        let Some(size) = sizes.get(ask.page as usize).copied() else {
            return self.stopped(Stop::Protocol);
        };
        let len = self.starter.section_len(&section);
        if check::check_rendered(&request, &size, len as u64, &got, &check::Limits::CONTRACT).is_err() {
            return self.stopped(Stop::Protocol);
        }
        if !self.handles_ok() {
            return;
        }
        let mut rgba = vec![0u8; len];
        if !self.starter.copy_out(&section, &mut rgba) {
            // Never a page of zeros shown as the page.
            rgba.zeroize();
            return self.stopped(Stop::Failed);
        }
        drop(section);
        self.shared.push(Out::Event(Event::Page {
            id: ask.id,
            page: ask.page,
            scale: ask.scale,
            region: ask.region,
            w: got.w,
            h: got.h,
            rgba,
            missing_content: got.missing_content,
        }));
    }

    /// A chart or picture: checked against the anchor it was asked for (exactly the asked size), then copied out.
    fn rendered_anchor(
        &mut self,
        ask: AnchorAsk,
        request: office::RenderAnchor,
        section: S::Section,
        got: office::Rendered,
    ) {
        let len = self.starter.section_len(&section);
        if check::check_rendered_anchor(&request, &ask.anchor, len as u64, &got, &check::Limits::CONTRACT).is_err() {
            return self.stopped(Stop::Protocol);
        }
        if !self.handles_ok() {
            return;
        }
        let mut rgba = vec![0u8; len];
        if !self.starter.copy_out(&section, &mut rgba) {
            rgba.zeroize();
            return self.stopped(Stop::Failed);
        }
        drop(section);
        self.shared.push(Out::Event(Event::Anchor {
            id: ask.id,
            sheet: ask.sheet,
            anchor: ask.anchor.id,
            scale: ask.scale,
            w: got.w,
            h: got.h,
            rgba,
            missing_content: got.missing_content,
        }));
    }

    /// One page of the outline: checked (it must move forward and stay under the limit), then the next page is asked,
    /// or the whole outline goes to the window.
    fn outline_page(&mut self, asked: office::Outline, mut got: office::OutlineData) {
        let pages = match &self.phase {
            Phase::Pages { sizes } => sizes.len() as u32,
            _ => {
                wipe_entries(&mut got.entries);
                return self.stopped(Stop::Protocol);
            }
        };
        if check::check_outline(&asked, &got, pages, &check::Limits::CONTRACT).is_err() {
            wipe_entries(&mut got.entries);
            return self.stopped(Stop::Protocol);
        }
        if !self.handles_ok() {
            wipe_entries(&mut got.entries);
            return;
        }
        let office::OutlineData { entries, next } = got;
        let Some(run) = self.run.as_mut() else {
            return;
        };
        run.outline.get_or_insert_with(Vec::new).extend(entries);
        match next {
            Some(from) => {
                let asked = office::Outline { from };
                self.ask(Message::Outline(asked), Asked::Outline(asked));
            }
            None => {
                let all = run.outline.take().unwrap_or_default();
                run.outline_since = None;
                self.shared.push(Out::Event(Event::Outline(all)));
            }
        }
    }

    fn cells(&mut self, asked: office::Cells, split: u64, mut got: CellData) {
        if check::check_cells(&asked, &got, &check::Limits::CONTRACT).is_err() {
            wipe_cells(&mut got);
            return self.stopped(Stop::Protocol);
        }
        if !self.handles_ok() {
            wipe_cells(&mut got);
            return;
        }
        self.split_answered(split);
        self.remember_anchors(asked.sheet, &got.anchors);
        self.shared.push(Out::Event(Event::Cells {
            sheet: asked.sheet,
            range: asked.range,
            data: got,
        }));
    }

    /// `RangeTooLarge`: both halves are asked (each its own answer), within the ask's budget: at most
    /// [`MAX_SPLIT_OUTSTANDING`] of its requests out and [`MAX_SPLIT_REQUESTS`] halves in all. Past it, or for a
    /// range that cannot be halved, the helper is wrong. Any other code: stopped.
    fn cells_refused(&mut self, asked: office::Cells, split: u64, code: ErrorCode) {
        if code != ErrorCode::RangeTooLarge {
            return self.stopped(Stop::Failed);
        }
        let Ok(halves) = check::halve_range(&asked) else {
            return self.stopped(Stop::Protocol);
        };
        // The refused request is answered; its two halves go out.
        let within = self
            .run
            .as_mut()
            .and_then(|run| run.splits.iter_mut().find(|budget| budget.id == split))
            .is_some_and(|budget| {
                budget.outstanding = budget.outstanding.saturating_add(1);
                budget.halves = budget.halves.saturating_add(2);
                budget.outstanding <= MAX_SPLIT_OUTSTANDING && budget.halves <= MAX_SPLIT_REQUESTS
            });
        if !within {
            return self.stopped(Stop::Protocol);
        }
        for half in halves {
            if !self.ask(Message::Cells(half), Asked::Cells { asked: half, split }) {
                return;
            }
        }
    }

    /// The charts and pictures of checked cells of `sheet`, as the window may ask them: up to [`MAX_KNOWN_ANCHORS`]
    /// (one already known is updated).
    fn remember_anchors(&self, sheet: u32, anchors: &[Anchor]) {
        let mut inner = lock(&self.shared.inner);
        if inner.closed.is_some() {
            return;
        }
        for anchor in anchors {
            let key = (sheet, anchor.id);
            if inner.anchors.len() < MAX_KNOWN_ANCHORS || inner.anchors.contains_key(&key) {
                inner.anchors.insert(key, anchor.clone());
            }
        }
    }

    /// One request of the ask `split` was answered with its cells; with none left out, its budget ends.
    fn split_answered(&mut self, split: u64) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let Some(at) = run.splits.iter().position(|budget| budget.id == split) else {
            return;
        };
        let budget = &mut run.splits[at];
        budget.outstanding = budget.outstanding.saturating_sub(1);
        if budget.outstanding == 0 {
            run.splits.remove(at);
        }
    }
}

/// Reads frames from the helper into the worker's channel until the pipe ends (the helper is gone) or the worker is.
/// A frame the worker can no longer take is wiped here.
fn spawn_reader(
    mut from_helper: Box<dyn Read + Send>,
    link: u64,
    to_worker: Sender<Input>,
    shared: Weak<Shared>,
) -> bool {
    thread::Builder::new()
        .name("office-helper-read".to_owned())
        .spawn(move || {
            loop {
                let frame = frame::read_frame(&mut from_helper).map(|Frame { kind, id, payload }| Received {
                    kind,
                    id,
                    payload: Zeroizing::new(payload),
                });
                let last = frame.is_err();
                if let Err(SendError(input)) = to_worker.send(Input::Frame { link, frame }) {
                    // The worker has ended. Without the host, dropping it wipes it (`Zeroizing`).
                    if let Some(shared) = shared.upgrade() {
                        shared.wipe_input(input);
                    }
                    return;
                }
                if last {
                    return;
                }
            }
        })
        .is_ok()
}

/// Writes the worker's frames to the helper, so that a full pipe never holds the worker. Ends when the worker drops
/// its sender or a write fails.
fn spawn_writer(mut to_helper: Box<dyn Write + Send>) -> Option<Sender<Outgoing>> {
    let (sender, outgoing) = mpsc::channel::<Outgoing>();
    thread::Builder::new()
        .name("office-helper-write".to_owned())
        .spawn(move || {
            for (kind, id, payload) in outgoing {
                if frame::write_frame(&mut to_helper, kind, id, &payload).is_err() {
                    return;
                }
            }
        })
        .ok()?;
    Some(sender)
}

/// Drops a helper's state: the outline text so far is wiped, every page section dropped (zeroized by the sandbox).
fn wipe_run<S: Starter>(mut run: Run<S>) {
    if let Some(entries) = run.outline.as_mut() {
        wipe_entries(entries);
    }
    drop(run);
}

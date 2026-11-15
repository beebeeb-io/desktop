//! The fake helper of `office_host_tests`: sections in memory, one thread per start that speaks the protocol over
//! in-memory pipes, and counters for starts, kills and live sections. The sandbox answers `Unsupported` off Windows,
//! so without this fake the Linux run would count no host test at all.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use beebeeb_office_protocol::check;
use beebeeb_office_protocol::frame;
use beebeeb_office_protocol::office::{self, Message, PageSize};
use beebeeb_office_sandbox::{Access, Granted, SandboxError};
use zeroize::{Zeroize, Zeroizing};

use crate::engine_host::office_host::{
    DocKind, Event, Helper, Link, OfficeHost, OfficeState, Registry, Starter, Timeouts, Wake, open_with,
};

/// The tests' timers: 300 ms each.
pub(super) const TEST_TIMEOUTS: Timeouts = Timeouts {
    open: Duration::from_millis(300),
    request: Duration::from_millis(300),
};

/// How long a test waits for what must happen (generous, so a loaded machine does not flake).
pub(super) const WITHIN: Duration = Duration::from_secs(2);

/// US Letter in points.
pub(super) const LETTER: PageSize = PageSize {
    w_pt: 612.0,
    h_pt: 792.0,
};

/// A4 as a document states it, in points (the protocol's shared vectors use the same).
pub(super) const A4: PageSize = PageSize {
    w_pt: 595.3,
    h_pt: 841.9,
};

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A document's bytes: the fake never parses them.
pub(super) fn doc() -> Vec<u8> {
    b"PK\x03\x04 a fixture, not a document".to_vec()
}

pub(super) fn noop_wake() -> Wake {
    Arc::new(|| {})
}

/// Opens a document through `fake` in `registry`, with the tests' timers. Also hands back the fake's log and a count
/// of the host's wakes.
pub(super) fn open_fake(fake: Fake, registry: &Registry, kind: DocKind) -> (OfficeHost, Arc<Log>, Arc<AtomicUsize>) {
    let log = fake.log.clone();
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    let wake: Wake = Arc::new(move || {
        counter.fetch_add(1, SeqCst);
    });
    let host = open_with(fake, registry, TEST_TIMEOUTS, kind, Zeroizing::new(doc()), wake);
    (host, log, wakes)
}

/// Polls `host` until `done` holds (true) or [`WITHIN`] passed (false). Every event is kept in `events`.
pub(super) fn pump(
    host: &mut OfficeHost,
    events: &mut Vec<Event>,
    done: impl Fn(&OfficeHost, &[Event]) -> bool,
) -> bool {
    pump_for(host, events, WITHIN, done)
}

/// [`pump`] with its own bound.
pub(super) fn pump_for(
    host: &mut OfficeHost,
    events: &mut Vec<Event>,
    bound: Duration,
    done: impl Fn(&OfficeHost, &[Event]) -> bool,
) -> bool {
    let until = Instant::now() + bound;
    loop {
        events.extend(host.poll());
        if done(host, events) {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// Polls until the document is past `Opening`.
pub(super) fn until_open(host: &mut OfficeHost, events: &mut Vec<Event>) -> bool {
    pump(host, events, |host, _| !matches!(host.state(), OfficeState::Opening))
}

/// Waits until `what` holds (true) or [`WITHIN`] passed (false).
pub(super) fn wait_for(what: impl Fn() -> bool) -> bool {
    let until = Instant::now() + WITHIN;
    while !what() {
        if Instant::now() >= until {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
    true
}

/// How many pages reached the window.
pub(super) fn pages_of(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, Event::Page { .. }))
        .count()
}

/// `Opened` for a document of `sizes`.
pub(super) fn opened(sizes: &[PageSize]) -> Message {
    Message::Opened(office::Opened {
        pages: sizes.len() as u32,
        sizes: sizes.to_vec(),
    })
}

/// A whole Letter page at 100 % on a 96 DPI screen, as rendered.
pub(super) fn rendered_letter() -> Message {
    Message::Rendered(office::Rendered {
        w: 816,
        h: 1056,
        missing_content: false,
    })
}

/// A helper that answers every request well, for a document of `sizes`.
pub(super) fn serve(link: &mut FakeLink, sizes: &[PageSize]) {
    while let Some((id, message)) = link.next() {
        if let Some(reply) = well(link, message, sizes) {
            link.answer(id, reply);
        }
    }
}

/// The right answer to `message` for a document of `sizes` (a render's section painted grey). `None` for a `Cancel`,
/// which has no answer of its own (the request it names is answered in its turn).
pub(super) fn well(link: &FakeLink, message: Message, sizes: &[PageSize]) -> Option<Message> {
    Some(match message {
        Message::Open(_) => opened(sizes),
        Message::Render(render) => {
            let size = sizes.get(render.page as usize)?;
            let (w, h) = check::asked_size(&render, size, &check::Limits::CONTRACT).ok()?;
            link.paint(render.out, 0x80);
            Message::Rendered(office::Rendered {
                w,
                h,
                missing_content: false,
            })
        }
        Message::Stats(_) => Message::StatsData(office::StatsData { words: 42 }),
        Message::Cancel(_) => return None,
        _ => Message::Error(office::Error {
            code: office::ErrorCode::Internal,
        }),
    })
}

/// A latch the test opens; a fake helper waits on it before it answers.
#[derive(Clone, Default)]
pub(super) struct Gate(Arc<(Mutex<bool>, Condvar)>);

impl Gate {
    pub(super) fn open(&self) {
        let (open, changed) = &*self.0;
        *lock(open) = true;
        changed.notify_all();
    }

    /// Waits until the gate is open, at most [`WITHIN`]; `false` when it never opened.
    pub(super) fn wait(&self) -> bool {
        let (open, changed) = &*self.0;
        let until = Instant::now() + WITHIN;
        let mut is_open = lock(open);
        while !*is_open {
            let now = Instant::now();
            if now >= until {
                return false;
            }
            is_open = changed
                .wait_timeout(is_open, until - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }
}

/// What the fake saw and did, shared by the starter, its helpers and its sections.
#[derive(Default)]
pub(super) struct Log {
    starts: AtomicUsize,
    /// Helpers ended by an explicit kill, each counted once. Dropping a helper's handle ends it too (as the sandbox's
    /// `Child` does), but is not counted: a test that asks "killed?" sees only a kill somebody made.
    kills: AtomicUsize,
    live_sections: AtomicI64,
    /// The length of every section made, in order (the document's first).
    pub(super) section_sizes: Mutex<Vec<usize>>,
    /// The starter's calls in order: "exclude" (crash reports) and "start".
    pub(super) calls: Mutex<Vec<&'static str>>,
    /// Every frame the host wrote to a helper, as (kind, id), in order.
    pub(super) sent: Arc<Mutex<Vec<(u16, u32)>>>,
    /// The page of every `Render` a helper read, in order.
    pub(super) rendered_pages: Mutex<Vec<u32>>,
    memory: Mutex<HashMap<u64, Weak<Mutex<Vec<u8>>>>>,
    next_wire: AtomicU64,
}

impl Log {
    pub(super) fn starts(&self) -> usize {
        self.starts.load(SeqCst)
    }

    pub(super) fn kills(&self) -> usize {
        self.kills.load(SeqCst)
    }

    pub(super) fn live(&self) -> i64 {
        self.live_sections.load(SeqCst)
    }

    pub(super) fn renders_sent(&self) -> usize {
        lock(&self.sent)
            .iter()
            .filter(|(kind, _)| *kind == office::KIND_RENDER)
            .count()
    }
}

pub(super) type Script = Arc<dyn Fn(&mut FakeLink) + Send + Sync>;

/// The fake starter: each start runs `script` on a thread of its own, as the helper.
pub(super) struct Fake {
    pub(super) log: Arc<Log>,
    script: Script,
    start_delay: Duration,
}

impl Fake {
    pub(super) fn new(script: impl Fn(&mut FakeLink) + Send + Sync + 'static) -> Fake {
        Fake {
            log: Arc::new(Log::default()),
            script: Arc::new(script),
            start_delay: Duration::ZERO,
        }
    }

    /// Each start takes `delay` before it returns.
    pub(super) fn slow_start(mut self, delay: Duration) -> Fake {
        self.start_delay = delay;
        self
    }
}

impl Starter for Fake {
    type Section = FakeSection;
    type Process = FakeHelper;

    fn installed(&self) -> bool {
        true
    }

    fn exclude_from_crash_reports(&mut self) -> Result<(), u32> {
        lock(&self.log.calls).push("exclude");
        Ok(())
    }

    fn section(&mut self, len: usize) -> Result<FakeSection, SandboxError> {
        let wire = 0x100 + self.log.next_wire.fetch_add(4, SeqCst);
        let memory = Arc::new(Mutex::new(vec![0u8; len]));
        lock(&self.log.memory).insert(wire, Arc::downgrade(&memory));
        lock(&self.log.section_sizes).push(len);
        self.log.live_sections.fetch_add(1, SeqCst);
        Ok(FakeSection {
            wire,
            memory,
            log: self.log.clone(),
        })
    }

    fn section_len(&self, section: &FakeSection) -> usize {
        lock(&section.memory).len()
    }

    fn fill(&self, section: &mut FakeSection, bytes: &[u8]) {
        let mut memory = lock(&section.memory);
        let n = memory.len().min(bytes.len());
        memory[..n].copy_from_slice(&bytes[..n]);
    }

    fn copy_out(&self, section: &FakeSection, out: &mut [u8]) {
        let memory = lock(&section.memory);
        let n = memory.len().min(out.len());
        out[..n].copy_from_slice(&memory[..n]);
    }

    fn start(&mut self) -> Result<Link<FakeHelper>, SandboxError> {
        if !self.start_delay.is_zero() {
            thread::sleep(self.start_delay);
        }
        let index = self.log.starts.fetch_add(1, SeqCst);
        lock(&self.log.calls).push("start");
        let to_helper = Pipe {
            frames: Some(self.log.sent.clone()),
            ..Pipe::default()
        };
        let from_helper = Pipe::default();
        let mut link = FakeLink {
            input: to_helper.clone(),
            output: from_helper.clone(),
            log: self.log.clone(),
        };
        let script = self.script.clone();
        thread::spawn(move || {
            script(&mut link);
            // The helper ends: its pipes close, as a process's do when it exits.
            link.output.close();
            link.input.close();
        });
        Ok(Link {
            helper: FakeHelper {
                index,
                pipes: [to_helper.clone(), from_helper.clone()],
                ended: AtomicBool::new(false),
                log: self.log.clone(),
            },
            to_helper: Box::new(to_helper),
            from_helper: Box::new(from_helper),
        })
    }

    fn grant(&self, _helper: &FakeHelper, section: &FakeSection, _access: Access) -> Result<Granted, SandboxError> {
        Ok(Granted {
            wire: section.wire,
            len: lock(&section.memory).len() as u64,
        })
    }
}

/// A section in memory: zeroed and counted out when dropped, as the sandbox's.
pub(super) struct FakeSection {
    wire: u64,
    memory: Arc<Mutex<Vec<u8>>>,
    log: Arc<Log>,
}

impl Drop for FakeSection {
    fn drop(&mut self) {
        lock(&self.memory).zeroize();
        self.log.live_sections.fetch_sub(1, SeqCst);
    }
}

/// The host's handle on a fake helper: ending it closes both pipes (the helper's thread then sees its input end).
pub(super) struct FakeHelper {
    index: usize,
    pipes: [Pipe; 2],
    ended: AtomicBool,
    log: Arc<Log>,
}

impl Helper for FakeHelper {
    fn end(&self) {
        if !self.ended.swap(true, SeqCst) {
            self.log.kills.fetch_add(1, SeqCst);
        }
        for pipe in &self.pipes {
            pipe.close();
        }
    }

    fn process_id(&self) -> Option<u32> {
        Some(4000 + self.index as u32)
    }
}

impl Drop for FakeHelper {
    fn drop(&mut self) {
        // As the sandbox's `Child`: gone with its handle (not counted as a kill).
        for pipe in &self.pipes {
            pipe.close();
        }
    }
}

/// The helper's side of the pipes, for a script.
pub(super) struct FakeLink {
    input: Pipe,
    output: Pipe,
    log: Arc<Log>,
}

impl FakeLink {
    /// The next request and its id; `None` once the pipe closed (the helper was killed) or a frame did not decode.
    pub(super) fn next(&mut self) -> Option<(u32, Message)> {
        let frame = frame::read_frame(&mut self.input).ok()?;
        let message = Message::decode(frame.kind, &frame.payload).ok()?;
        if let Message::Render(render) = &message {
            lock(&self.log.rendered_pages).push(render.page);
        }
        Some((frame.id, message))
    }

    /// Answers request `id` with `message` (a closed pipe is ignored: the helper was killed).
    pub(super) fn answer(&mut self, id: u32, message: Message) {
        if let Ok(payload) = message.encode() {
            let _ = frame::write_frame(&mut self.output, message.kind(), id, &payload);
        }
    }

    /// Fills the granted section `out` with `byte`, as a render would.
    pub(super) fn paint(&self, out: office::GrantRef, byte: u8) {
        let memory = lock(&self.log.memory).get(&out.wire).and_then(Weak::upgrade);
        if let Some(memory) = memory {
            lock(&memory).fill(byte);
        }
    }
}

#[derive(Default)]
struct PipeState {
    bytes: VecDeque<u8>,
    closed: bool,
}

/// One direction of a pipe in memory: a read waits until bytes come or the pipe is closed.
#[derive(Clone, Default)]
pub(super) struct Pipe {
    state: Arc<(Mutex<PipeState>, Condvar)>,
    /// When set, each write is recorded as (kind, id): `write_frame` writes one whole frame at once.
    frames: Option<Arc<Mutex<Vec<(u16, u32)>>>>,
}

impl Pipe {
    fn close(&self) {
        let (state, changed) = &*self.state;
        lock(state).closed = true;
        changed.notify_all();
    }
}

impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let (state, changed) = &*self.state;
        let mut state = lock(state);
        loop {
            if !state.bytes.is_empty() {
                let n = buf.len().min(state.bytes.len());
                for (slot, byte) in buf.iter_mut().zip(state.bytes.drain(..n)) {
                    *slot = byte;
                }
                return Ok(n);
            }
            if state.closed {
                return Ok(0);
            }
            state = changed.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let (state, changed) = &*self.state;
        let mut state = lock(state);
        if state.closed {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        if let Some(frames) = &self.frames
            && buf.len() >= 10
        {
            let kind = u16::from_le_bytes([buf[4], buf[5]]);
            let id = u32::from_le_bytes([buf[6], buf[7], buf[8], buf[9]]);
            lock(frames).push((kind, id));
        }
        state.bytes.extend(buf.iter().copied());
        changed.notify_all();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

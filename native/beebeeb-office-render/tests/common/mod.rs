//! What the `serve` tests share, std only: an in-memory pipe each way, a fake memory that counts mappings, a fake
//! engine, and the helper running on its own thread.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use beebeeb_office_protocol::frame::{FrameError, read_frame, write_frame};
use beebeeb_office_protocol::office::{
    CellData, CellRange, DocKind, ErrorCode, GrantRef, Message, Open, Opened, OutlineEntry, PageSize, Rect, Render,
};
use beebeeb_office_render::{Engine, Mapped, MappedMut, Memory, OpenDoc, Opening, serve};

/// How long a test waits for one answer, or for the helper to end, before it fails. The real engine renders in a
/// debug build.
pub const WAIT: Duration = Duration::from_secs(60);

/// A4 as a document states it.
pub const A4: PageSize = PageSize {
    w_pt: 595.3,
    h_pt: 841.9,
};

/// The fake engine's page: 4 x 2 px, RGBA8.
pub const FAKE_SIZE: (u32, u32) = (4, 2);
pub const FAKE_PAGE_BYTES: usize = 4 * 2 * 4;

/// The fake engine's chart or picture: 5 x 4 px, RGBA8. Twenty pixels, so a drawing of ten colours or more fits.
pub const FAKE_ANCHOR_SIZE: (u32, u32) = (5, 4);
pub const FAKE_ANCHOR_BYTES: usize = 5 * 4 * 4;

/// A grant for an ask that is only sized, never sent.
pub const NO_GRANT: GrantRef = GrantRef { wire: 0, len: 0 };

// ---- the pipe

/// The reading end: bytes in the chunks the writer wrote. Counts what it handed out.
pub struct PipeReader {
    rx: Receiver<Vec<u8>>,
    chunk: Vec<u8>,
    at: usize,
    /// `None`: wait for ever (the helper's input). `Some`: fail after that long (the test's side).
    wait: Option<Duration>,
    handed: Arc<AtomicUsize>,
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        while self.at >= self.chunk.len() {
            let next = match self.wait {
                None => self.rx.recv().ok(),
                Some(wait) => match self.rx.recv_timeout(wait) {
                    Ok(chunk) => Some(chunk),
                    Err(RecvTimeoutError::Timeout) => {
                        return Err(io::Error::new(io::ErrorKind::TimedOut, "no bytes in time"));
                    }
                    Err(RecvTimeoutError::Disconnected) => None,
                },
            };
            match next {
                Some(chunk) => {
                    self.chunk = chunk;
                    self.at = 0;
                }
                // The writer is gone: end of input.
                None => return Ok(0),
            }
        }
        let n = (self.chunk.len() - self.at).min(out.len());
        out[..n].copy_from_slice(&self.chunk[self.at..self.at + n]);
        self.at += n;
        self.handed.fetch_add(n, Ordering::SeqCst);
        Ok(n)
    }
}

/// The writing end. Closed by dropping its sender.
pub struct PipeWriter {
    tx: Option<Sender<Vec<u8>>>,
    written: Arc<AtomicUsize>,
}

impl Write for PipeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let broken = || io::Error::from(io::ErrorKind::BrokenPipe);
        let tx = self.tx.as_ref().ok_or_else(broken)?;
        tx.send(bytes.to_vec()).map_err(|_| broken())?;
        self.written.fetch_add(bytes.len(), Ordering::SeqCst);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// ---- the fake memory

/// Sections by `wire`, each the bytes the app would share. Counts every mapping per grant, and the mappings alive.
#[derive(Default)]
pub struct FakeMemory {
    state: Mutex<MemoryState>,
    live_reads: Arc<AtomicUsize>,
    live_writes: Arc<AtomicUsize>,
}

#[derive(Default)]
struct MemoryState {
    sections: HashMap<u64, Arc<Mutex<Vec<u8>>>>,
    maps: HashMap<u64, usize>,
    next_wire: u64,
}

impl FakeMemory {
    pub fn new() -> Arc<FakeMemory> {
        Arc::new(FakeMemory::default())
    }

    /// A section holding `bytes`, as the app grants it.
    pub fn section(&self, bytes: Vec<u8>) -> GrantRef {
        let mut state = self.state.lock().expect("the fake memory's lock");
        // Handle-like values: 4, 8, 12, …
        state.next_wire += 4;
        let wire = state.next_wire;
        let len = bytes.len() as u64;
        state.sections.insert(wire, Arc::new(Mutex::new(bytes)));
        GrantRef { wire, len }
    }

    /// A zeroed section of `len` bytes (a page buffer).
    pub fn zeroed(&self, len: usize) -> GrantRef {
        self.section(vec![0; len])
    }

    /// The section's bytes now.
    pub fn bytes(&self, g: GrantRef) -> Vec<u8> {
        let state = self.state.lock().expect("the fake memory's lock");
        let section = state.sections.get(&g.wire).expect("a section the test made");
        section.lock().expect("the section's lock").clone()
    }

    /// How often the grant was mapped.
    pub fn maps(&self, g: GrantRef) -> usize {
        let state = self.state.lock().expect("the fake memory's lock");
        state.maps.get(&g.wire).copied().unwrap_or(0)
    }

    /// How often any grant was mapped.
    pub fn all_maps(&self) -> usize {
        let state = self.state.lock().expect("the fake memory's lock");
        state.maps.values().sum()
    }

    /// Read-only mappings not dropped yet.
    pub fn live_reads(&self) -> usize {
        self.live_reads.load(Ordering::SeqCst)
    }

    /// Read-write mappings not dropped yet.
    pub fn live_writes(&self) -> usize {
        self.live_writes.load(Ordering::SeqCst)
    }

    fn map(&self, g: GrantRef) -> Result<Arc<Mutex<Vec<u8>>>, ErrorCode> {
        let mut state = self.state.lock().expect("the fake memory's lock");
        *state.maps.entry(g.wire).or_default() += 1;
        let section = state.sections.get(&g.wire).cloned().ok_or(ErrorCode::Internal)?;
        let len = section.lock().expect("the section's lock").len() as u64;
        if len != g.len {
            return Err(ErrorCode::Internal);
        }
        Ok(section)
    }
}

struct FakeRead {
    bytes: Vec<u8>,
    live: Arc<AtomicUsize>,
}

impl Mapped for FakeRead {
    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }
}

impl Drop for FakeRead {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A read-write mapping: written back into the section when dropped, as an unmap makes the writes the app's.
struct FakeWrite {
    buf: Vec<u8>,
    section: Arc<Mutex<Vec<u8>>>,
    live: Arc<AtomicUsize>,
}

impl MappedMut for FakeWrite {
    fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.buf
    }
}

impl Drop for FakeWrite {
    fn drop(&mut self) {
        if let Ok(mut section) = self.section.lock() {
            *section = std::mem::take(&mut self.buf);
        }
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Memory for FakeMemory {
    fn read(&self, g: GrantRef) -> Result<Box<dyn Mapped>, ErrorCode> {
        let section = self.map(g)?;
        let bytes = section.lock().expect("the section's lock").clone();
        self.live_reads.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeRead {
            bytes,
            live: Arc::clone(&self.live_reads),
        }))
    }

    fn write(&self, g: GrantRef) -> Result<Box<dyn MappedMut>, ErrorCode> {
        let section = self.map(g)?;
        let buf = section.lock().expect("the section's lock").clone();
        self.live_writes.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeWrite {
            buf,
            section,
            live: Arc::clone(&self.live_writes),
        }))
    }
}

// ---- the fake engine

/// Opened by the test; the fake engine's first render waits for it.
#[derive(Clone, Default)]
pub struct Gate(Arc<(Mutex<bool>, Condvar)>);

impl Gate {
    pub fn open(&self) {
        let (open, changed) = &*self.0;
        *open.lock().expect("the gate's lock") = true;
        changed.notify_all();
    }

    fn wait(&self) {
        let (open, changed) = &*self.0;
        let deadline = Instant::now() + WAIT;
        let mut is_open = open.lock().expect("the gate's lock");
        while !*is_open {
            let now = Instant::now();
            assert!(now < deadline, "the test never opened the gate");
            is_open = changed
                .wait_timeout(is_open, deadline - now)
                .expect("the gate's lock")
                .0;
        }
    }
}

/// What the fake engine answers, and what it was asked.
pub struct Fake {
    pub opening: Result<Opening, ErrorCode>,
    pub size: Result<(u32, u32), ErrorCode>,
    pub outline: Vec<OutlineEntry>,
    pub words: u32,
    pub cells: Result<CellData, ErrorCode>,
    pub anchor_size: Result<(u32, u32), ErrorCode>,
    /// When set, the first render waits until the test opens it.
    pub gate: Option<Gate>,
    pub opens: AtomicUsize,
    pub renders: AtomicUsize,
    pub cells_asked: AtomicUsize,
    pub outlines: AtomicUsize,
    pub anchor_renders: AtomicUsize,
}

impl Default for Fake {
    fn default() -> Self {
        Fake {
            opening: Ok(Opening::Pages(Opened {
                pages: 1,
                sizes: vec![A4],
            })),
            size: Ok(FAKE_SIZE),
            outline: Vec::new(),
            words: 7,
            cells: Err(ErrorCode::Unsupported),
            anchor_size: Ok(FAKE_ANCHOR_SIZE),
            gate: None,
            opens: AtomicUsize::new(0),
            renders: AtomicUsize::new(0),
            cells_asked: AtomicUsize::new(0),
            outlines: AtomicUsize::new(0),
            anchor_renders: AtomicUsize::new(0),
        }
    }
}

impl Fake {
    pub fn renders(&self) -> usize {
        self.renders.load(Ordering::SeqCst)
    }

    pub fn cells_asked(&self) -> usize {
        self.cells_asked.load(Ordering::SeqCst)
    }

    pub fn outlines(&self) -> usize {
        self.outlines.load(Ordering::SeqCst)
    }

    pub fn anchor_renders(&self) -> usize {
        self.anchor_renders.load(Ordering::SeqCst)
    }
}

pub struct FakeEngine(pub Arc<Fake>);

impl Engine for FakeEngine {
    fn open(&self, _kind: DocKind, _bytes: &[u8]) -> Result<Box<dyn OpenDoc>, ErrorCode> {
        self.0.opens.fetch_add(1, Ordering::SeqCst);
        if let Err(code) = &self.0.opening {
            return Err(*code);
        }
        Ok(Box::new(FakeDoc(Arc::clone(&self.0))))
    }
}

struct FakeDoc(Arc<Fake>);

impl OpenDoc for FakeDoc {
    fn opened(&self) -> Opening {
        self.0.opening.clone().expect("asked only after a successful open")
    }

    fn render_size(&self, _page: u32, _scale: f32, _region: Option<Rect>) -> Result<(u32, u32), ErrorCode> {
        self.0.size
    }

    fn render(&self, _page: u32, _scale: f32, _region: Option<Rect>, out: &mut [u8]) -> Result<bool, ErrorCode> {
        let earlier = self.0.renders.fetch_add(1, Ordering::SeqCst);
        if earlier == 0
            && let Some(gate) = &self.0.gate
        {
            gate.wait();
        }
        // Not flat: every byte set, none zero.
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = (i % 251) as u8 + 1;
        }
        Ok(false)
    }

    fn outline(&self) -> Vec<OutlineEntry> {
        self.0.outlines.fetch_add(1, Ordering::SeqCst);
        self.0.outline.clone()
    }

    fn words(&self) -> u32 {
        self.0.words
    }

    fn cells(&self, _sheet: u32, _range: CellRange) -> Result<CellData, ErrorCode> {
        self.0.cells_asked.fetch_add(1, Ordering::SeqCst);
        self.0.cells.clone()
    }

    fn anchor_size(&self, _sheet: u32, _anchor: u32, _scale: f32) -> Result<(u32, u32), ErrorCode> {
        self.0.anchor_size
    }

    fn render_anchor(&self, _sheet: u32, _anchor: u32, _scale: f32, out: &mut [u8]) -> Result<bool, ErrorCode> {
        self.0.anchor_renders.fetch_add(1, Ordering::SeqCst);
        // Every pixel its own colour: bytes 1, 2, 3, … (never 0), as a page's.
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = (i % 251) as u8 + 1;
        }
        Ok(true)
    }
}

// ---- the helper on its own thread

/// `serve` running on its own thread, as the app sees the helper: frames in, frames out, an exit code at the end.
pub struct Running {
    to: PipeWriter,
    from: PipeReader,
    exit: Receiver<u8>,
    /// Bytes the helper's input has handed to its reader.
    handed: Arc<AtomicUsize>,
    /// Bytes the test has written to the helper.
    written: Arc<AtomicUsize>,
}

impl Running {
    pub fn start(memory: Arc<FakeMemory>, engine: Arc<dyn Engine + Send + Sync>) -> Running {
        let (to_tx, to_rx) = mpsc::channel();
        let (from_tx, from_rx) = mpsc::channel();
        let (exit_tx, exit_rx) = mpsc::channel();
        let handed = Arc::new(AtomicUsize::new(0));
        let written = Arc::new(AtomicUsize::new(0));
        let input = PipeReader {
            rx: to_rx,
            chunk: Vec::new(),
            at: 0,
            wait: None,
            handed: Arc::clone(&handed),
        };
        let mut output = PipeWriter {
            tx: Some(from_tx),
            written: Arc::new(AtomicUsize::new(0)),
        };
        thread::spawn(move || {
            let code = serve(Box::new(input), &mut output, &*memory, &*engine);
            // The output closes before the exit code is reported: once the test has the code, nothing more comes.
            drop(output);
            let _ = exit_tx.send(code);
        });
        Running {
            to: PipeWriter {
                tx: Some(to_tx),
                written: Arc::clone(&written),
            },
            from: PipeReader {
                rx: from_rx,
                chunk: Vec::new(),
                at: 0,
                wait: Some(WAIT),
                handed: Arc::new(AtomicUsize::new(0)),
            },
            exit: exit_rx,
            handed,
            written,
        }
    }

    /// Sends `message` as a frame with `id`.
    pub fn send(&mut self, id: u32, message: &Message) {
        let payload = message.encode().expect("a test message fits one frame");
        self.send_frame(message.kind(), id, &payload);
    }

    /// Sends a frame of any kind and payload.
    pub fn send_frame(&mut self, kind: u16, id: u32, payload: &[u8]) {
        write_frame(&mut self.to, kind, id, payload).expect("the helper's input is open");
    }

    /// Sends bytes as they are (a broken frame).
    pub fn send_raw(&mut self, bytes: &[u8]) {
        self.to.write_all(bytes).expect("the helper's input is open");
    }

    /// The next answer: its id and its message. Fails the test after [`WAIT`].
    pub fn answer(&mut self) -> (u32, Message) {
        let frame = match read_frame(&mut self.from) {
            Ok(frame) => frame,
            Err(e) => panic!("no answer: {e}"),
        };
        let message = Message::decode(frame.kind, &frame.payload).expect("the helper's answer decodes");
        (frame.id, message)
    }

    /// Ends the helper's input, as the app closing its end of the pipe.
    pub fn close(&mut self) {
        self.to.tx = None;
    }

    /// The helper's exit code. Fails the test after [`WAIT`], or when `serve` panicked.
    pub fn exit_code(&self) -> u8 {
        self.exit
            .recv_timeout(WAIT)
            .expect("the helper ended in time, without a panic")
    }

    /// After the helper ended: it wrote nothing more.
    pub fn nothing_more(&mut self) {
        assert_eq!(
            read_frame(&mut self.from).err(),
            Some(FrameError::Closed),
            "nothing more was written"
        );
    }

    /// Waits until the helper's input has handed out every byte the test wrote.
    pub fn wait_handed_out(&self) {
        let deadline = Instant::now() + WAIT;
        while self.handed.load(Ordering::SeqCst) < self.written.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "the helper never read its input");
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Opens a fake document (id 0) and returns the answer.
    pub fn open_fake(&mut self, memory: &FakeMemory) -> Message {
        let doc = memory.section(b"a document".to_vec());
        self.send(
            0,
            &Message::Open(Open {
                kind: DocKind::Docx,
                doc,
            }),
        );
        match self.answer() {
            (0, message) => message,
            (id, message) => panic!("an answer to {id} where 0 was due: {message:?}"),
        }
    }
}

/// A whole-page `Render` of page 0 at scale 1 into `out`.
pub fn render_into(out: GrantRef) -> Message {
    Message::Render(Render {
        page: 0,
        scale: 1.0,
        region: None,
        out,
    })
}

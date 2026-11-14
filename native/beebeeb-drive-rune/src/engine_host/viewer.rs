//! An open file on the real engine: [`EngineViewerSource`], the `ViewerSource` of the running app. One source per
//! opened file (`frames::viewers::RouteSource`).
//!
//! What it shows, and from where:
//! - the name, the folders above it and the size: the signed-in account's local state, through the engine's
//!   account-bound handle ([`Engine::open_file_facts`]); read once, when the file opens.
//! - the content, on a worker, from one of two places:
//!   - a file whose bytes are wholly ON THIS PC is read from there ([`Engine::open_local_copy_in_memory`]): it
//!     opens without a connection, and a change made here that is not sent yet is what is shown, never the older
//!     version Beebeeb has. The view says where the content came from ([`ViewerSource::origin`]), and never
//!     more than is known: a copy read while sync is off is not called the version Beebeeb has; a copy with an
//!     unsent change that cannot be read is a state of its own, never answered with a download; and Beebeeb's
//!     version of a file whose copy here is not settled with it (a conflict, a failed sync) is said to be that;
//!   - any other file is downloaded and decrypted IN MEMORY by the engine's own download path
//!     ([`Engine::open_file_in_memory`]: the path every hydrate uses, no crypto here), with the engine's
//!     progress.
//!   Text, Markdown, CSV / TSV (as text), PNG / JPEG / GIF / WebP pictures and PDFs are shown. A PDF's pages are
//!   rendered on the same worker, page by page as the view shows them (`viewer_pdf`), from memory.
//! - which app opens a file: a list's Open of a file that is kept on this PC while sync runs goes to the file's
//!   own app ([`ViewerSource::kept_on_this_pc`]) when that app shows it (a document, a picture); the viewer is
//!   for a file that is not on this PC (no download to disk), for a route that names a file directly, and for
//!   source code, scripts and web pages wherever they are (`kinds::its_app_shows_it`: Enter reads one, it
//!   never runs it).
//! - not shown here: a PDF that asks for a password or cannot be read, audio, archives, RAW and HEIC pictures, Office
//!   documents. A list's Open leaves those to their own app, as before ([`ViewerSource::previews`]); a route that
//!   names one anyway (Search, a link) gets the honest state, `Body::NotPreviewable`, and nothing is downloaded.
//!   The same state, with its own words, for any file over [`IN_MEMORY_MAX`], and for a picture that declares
//!   more than the viewer decodes (`viewer_picture`, asked of the header before anything is decoded).
//! - a file that cannot be opened at all says why ([`OpenError`]): gone, the session ended, or "not right now"
//!   (asked again on Try again), never one catch-all.
//! - "Open in another app…": the system opens the file from the account's folder on this PC, where the engine
//!   already puts opened files. No copy is made here.
//! - not bound: saving an edit, Available offline, a newer version's banner, find, play, a photo's strip.
//!
//! Where the decrypted bytes are: in the engine's `Zeroizing` buffer until the worker made of them what the view
//! draws (lines of text, or pixels), then wiped. The text is overwritten when the view lets go of it
//! (`OpenFile::wipe`, `viewers::Ui`'s drop) or when an answer is never taken ([`Content`]'s drop). Decoded pixels
//! are the toolkit's memory: released with the file, not overwritten (the toolkit has no call for it). Nothing
//! is written to disk, here or in the engine.
//!
//! Session rule: the file is opened for one session (the engine's session generation) of one account. Every
//! answer carries the generation it was made for; [`ViewerSource::poll`] (the window's poll, every frame) drops an
//! answer of another generation, and the moment the session is not that one any more (a lock, a sign-out, another
//! sign-in) it cancels the download, releases the picture and hands the view a body that holds nothing: on the
//! window's side, without asking a worker. Each accessor asks the same question before it hands anything out.
//!
//! Thread rule: the download runs on a worker of its own and sends ONE answer down a channel; the worker wakes
//! the window ([`Engine::wake_host`]) as progress moves and when it is done. The facts are a local read, as a
//! listing is. Opening in another app asks the system on the window's thread, as the Files window's open does.
//!
//! Must not: log a name, an id, a path or a byte; write a decrypted byte anywhere; show a sample.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::viewer_api::{LocalCopy, OnThisPc, OpenFileError, OpenFileFacts, OpenedFile};
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::file_ops;
use zeroize::Zeroize;

use super::log;
use crate::frames::viewers::kinds::{self, PdfDoc, PhotoDoc, Picture, Viewer};
use crate::frames::viewers::source::{
    Body, Cut, Details, Elsewhere, MdDoc, NoPreview, OpenError, OpenFile, Origin, PageState, Stopped, TextDoc,
    Truncated, ViewerSource, plain_lines,
};

#[path = "viewer_pdf.rs"]
pub(crate) mod viewer_pdf;
#[path = "viewer_picture.rs"]
pub(crate) mod viewer_picture;

/// How many rendered pages of the open PDF are kept (the page on screen and the ones just left).
pub const PAGES_KEPT: usize = 6;

#[cfg(test)]
#[path = "viewer_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "viewer_window_tests.rs"]
mod window_tests;

/// The largest file a preview decrypts: it is held in memory, whole, while it is open. Larger files are not
/// downloaded for a preview at all.
pub const IN_MEMORY_MAX: u64 = 64 * 1024 * 1024;

/// A text over this is shown read-only, its first lines only (the board's DW-1527).
pub const READ_WHOLE_MAX: usize = 2_000_000;
/// A text under [`READ_WHOLE_MAX`] with more lines than this, or a line longer than [`LINE_BYTES_MAX`], is shown
/// the same way: the reader draws every line it holds.
pub const WHOLE_LINES_MAX: usize = 5_000;
pub const LINE_BYTES_MAX: usize = 20_000;
/// What a text shown in part holds: its first lines, at most this many and at most [`SHOWN_BYTES`] of them.
pub const SHOWN_LINES: usize = 500;
pub const SHOWN_BYTES: usize = 64_000;
/// A Markdown note over this is shown as its text, not rendered (the note is parsed whenever it is drawn).
pub const MARKDOWN_RENDER_MAX: usize = 256_000;

/// What the adapter asks of the engine. The running app's is [`LiveViewerEngine`]; the tests' is a double with
/// hooks, so each rule here is tested without a network.
pub trait ViewerEngine: Send + Sync + 'static {
    /// The session's generation now ([`Engine::session_turn`]). Never waits.
    fn session_turn(&self) -> SessionGeneration;
    /// A session is in memory (signed in and unlocked). Never waits.
    fn has_session(&self) -> bool;
    /// Whose session it is (the email, else the user id), as `engine_host::settings` names an account.
    fn account(&self) -> Option<String>;
    /// [`Engine::open_file_facts`].
    fn facts(&self, file_id: &str) -> Result<OpenFileFacts, OpenFileError>;
    /// [`Engine::open_file_in_memory`]. BLOCKS: a worker's.
    fn fetch(
        &self,
        file_id: &str,
        max_bytes: u64,
        progress: &(dyn Fn(u64, u64) + Send + Sync + 'static),
        cancel: &AtomicBool,
    ) -> Result<OpenedFile, OpenFileError>;
    /// [`Engine::open_local_copy_in_memory`]: the copy on this PC, read into memory. No network. A worker's.
    fn local(&self, file_id: &str, max_bytes: u64, cancel: &AtomicBool) -> Result<LocalCopy, OpenFileError>;
    /// Sync runs for the account's folder on this PC ([`Engine::is_syncing`]): the system can open a file kept
    /// there.
    fn is_syncing(&self) -> bool;
    /// Ask the system to open the file with its own app, from the account's folder on this PC. Never
    /// `Elsewhere::NotBound`.
    fn open_elsewhere(&self, file_id: &str) -> Elsewhere;
    /// Wake the window's loop: an answer or progress is waiting.
    fn wake(&self);
}

/// The engine handle, as the viewer asks it.
pub struct LiveViewerEngine<C: CredentialStores> {
    engine: Arc<Engine<C>>,
}

impl<C: CredentialStores + 'static> ViewerEngine for LiveViewerEngine<C> {
    fn session_turn(&self) -> SessionGeneration {
        self.engine.session_turn()
    }

    fn has_session(&self) -> bool {
        self.engine.has_session()
    }

    fn account(&self) -> Option<String> {
        self.engine.account_email().or_else(|| self.engine.user_id())
    }

    fn facts(&self, file_id: &str) -> Result<OpenFileFacts, OpenFileError> {
        self.engine.open_file_facts(file_id)
    }

    fn fetch(
        &self,
        file_id: &str,
        max_bytes: u64,
        progress: &(dyn Fn(u64, u64) + Send + Sync + 'static),
        cancel: &AtomicBool,
    ) -> Result<OpenedFile, OpenFileError> {
        self.engine.open_file_in_memory(file_id, max_bytes, progress, cancel)
    }

    fn local(&self, file_id: &str, max_bytes: u64, cancel: &AtomicBool) -> Result<LocalCopy, OpenFileError> {
        self.engine.open_local_copy_in_memory(file_id, max_bytes, cancel)
    }

    fn is_syncing(&self) -> bool {
        self.engine.is_syncing()
    }

    /// What the Files window's own open does, for this one file by its id, through the same check
    /// (`engine_host::path_the_system_opens`): the path is resolved under the account-bound handle, a kind that
    /// is not opened for safety is refused, and the system is asked without a shell, after the handle answered.
    /// "Sync is off" and "refused" are two answers.
    fn open_elsewhere(&self, file_id: &str) -> Elsewhere {
        if !self.engine.is_syncing() {
            return Elsewhere::NeedsSync;
        }
        let Ok(sync_root) = self.engine.sync_root() else {
            return Elsewhere::NeedsSync;
        };
        let path = self.engine.local_state_for_changes().and_then(|state| {
            state.with(|db| {
                // A folder is not a file to open from a viewer.
                file_ops::resolve(db, file_id).ok().filter(|entry| !entry.is_dir())?;
                super::path_the_system_opens(db, &sync_root, file_id).ok()
            })
        });
        match path {
            Ok(Some(path)) if super::open_path(&path).is_ok() => Elsewhere::Opened,
            _ => Elsewhere::Refused,
        }
    }

    fn wake(&self) {
        self.engine.wake_host();
    }
}

/// What the worker made of the decrypted file, for the view. Its text is overwritten if nobody takes it (the
/// file was closed while the answer was on its way).
struct Content {
    body: Option<Body>,
    picture: Option<rune::Pixels>,
    /// Where the bytes came from.
    origin: Origin,
}

impl Drop for Content {
    fn drop(&mut self) {
        if let Some(body) = self.body.as_mut() {
            body.wipe();
        }
    }
}

/// The worker's one answer: what to show and the session it was decrypted for, or why not.
type Answer = Result<(SessionGeneration, Content), OpenFileError>;

/// What the download says while it runs.
#[derive(Default)]
struct Progress {
    done: AtomicU64,
    total: AtomicU64,
    /// The whole percent the window was last woken for.
    woken_at: AtomicU64,
}

/// The file this source has open, and the session it was opened for.
struct Opening {
    file_id: String,
    session: SessionGeneration,
    account: Option<String>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Progress>,
    inbox: mpsc::Receiver<Answer>,
    /// The worker has not answered yet.
    fetching: bool,
    /// The progress the view was last handed.
    drawn: (u64, u64),
    /// The view holds decrypted content of this file (text, or the picture below).
    holds: bool,
    picture: Option<rune::Pixels>,
    /// The open PDF's pages, while the file is a PDF.
    pages: Option<Pages>,
    /// Where the content the view holds came from (Beebeeb, until an answer says the copy on this PC).
    origin: Origin,
    /// The session moved on and the view was told: nothing more comes from this opening.
    ended: bool,
}

/// One page's answer from the worker: the page (from 1), its pixels or `None` when it could not be rendered, and
/// whether text of the page is not in the pixels (`viewer_pdf::Rendered::lacks_text`).
type PageAnswer = (u32, Option<rune::Pixels>, bool);

/// The window's side of an open PDF: pages are asked of the worker that holds the document, and the last few that
/// came back are kept. Asking happens while the view is drawn (`ViewerSource::picture` takes `&self`), so what
/// changes then sits in cells; this is the window's thread's alone.
struct Pages {
    ask: mpsc::Sender<u32>,
    got: mpsc::Receiver<PageAnswer>,
    /// The newest last. `None` pixels: that page could not be rendered.
    kept: RefCell<VecDeque<PageAnswer>>,
    /// The page the worker was last asked for and has not answered.
    asked: Cell<Option<u32>>,
    /// The document as the view was told, once the worker opened it.
    doc: RefCell<Option<PdfDoc>>,
}

impl Pages {
    /// Take what the worker rendered since the last call. `true`: something came.
    fn take_in(&self) -> bool {
        let mut kept = self.kept.borrow_mut();
        let mut came = false;
        while let Ok((page, pixels, lacks_text)) = self.got.try_recv() {
            kept.retain(|(held, ..)| *held != page);
            kept.push_back((page, pixels, lacks_text));
            while kept.len() > PAGES_KEPT {
                kept.pop_front();
            }
            if self.asked.get() == Some(page) {
                self.asked.set(None);
            }
            came = true;
        }
        came
    }

    fn held(&self, page: u32) -> Option<Option<rune::Pixels>> {
        self.kept
            .borrow()
            .iter()
            .find(|(held, ..)| *held == page)
            .map(|(_, pixels, _)| pixels.clone())
    }

    /// Whether the kept picture of `page` lacks text the renderer had no font for.
    fn lacks_text(&self, page: u32) -> bool {
        self.kept
            .borrow()
            .iter()
            .any(|(held, pixels, lacks_text)| *held == page && pixels.is_some() && *lacks_text)
    }
}

/// The worker's ends of an open PDF's channels, and the size the view is told.
struct PdfEnds {
    asks: mpsc::Receiver<u32>,
    answers: mpsc::Sender<PageAnswer>,
    size: String,
}

/// An open file on the real engine. See the module's doc.
pub struct EngineViewerSource<E: ViewerEngine> {
    engine: Arc<E>,
    open: Option<Opening>,
}

impl<C: CredentialStores + 'static> EngineViewerSource<LiveViewerEngine<C>> {
    /// Starts no thread and reads nothing until a file is opened.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        Self::on(Arc::new(LiveViewerEngine { engine }))
    }
}

/// How a file is shown here, by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shows {
    Text,
    Markdown,
    Picture,
    Pdf,
    /// No viewer here for this kind of file: nothing is downloaded.
    Nothing,
}

fn shows(name: &str) -> Shows {
    let ext = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match kinds::viewer_of(name) {
        // A CSV or TSV is shown as the text it is (the table view is not bound).
        Some(Viewer::Text | Viewer::Table) => Shows::Text,
        Some(Viewer::Markdown) => Shows::Markdown,
        // The formats the toolkit decodes. A RAW or a HEIC has no decoder here.
        Some(Viewer::Photo) if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp") => Shows::Picture,
        Some(Viewer::Pdf) => Shows::Pdf,
        _ => Shows::Nothing,
    }
}

/// "Markdown" of "Markdown · 1.4 KB": the kind in a word, by the name.
fn kind_word(name: &str) -> &'static str {
    match kinds::viewer_of(name) {
        Some(Viewer::Markdown) => "Markdown",
        Some(Viewer::Text) => "Text",
        Some(Viewer::Table) => "Table",
        Some(Viewer::Photo) => "Picture",
        Some(Viewer::Pdf) => "PDF",
        Some(Viewer::Audio) => "Audio",
        Some(Viewer::Archive) => "Archive",
        None => "File",
    }
}

/// The decrypted bytes as text the view draws, or `None` when they are not text (not UTF-8, or a NUL in them).
/// A text the reader cannot hold whole (over [`READ_WHOLE_MAX`], more than [`WHOLE_LINES_MAX`] lines, or a line
/// over [`LINE_BYTES_MAX`]) is its first lines, read-only: at most [`SHOWN_LINES`] and [`SHOWN_BYTES`].
fn text_body(bytes: &[u8], markdown: bool) -> Option<Body> {
    let text = std::str::from_utf8(bytes).ok()?;
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    if text.contains('\0') {
        return None;
    }
    let (mut total, mut longest) = (0usize, 0usize);
    for line in text.lines() {
        total += 1;
        longest = longest.max(line.len());
    }
    let why = if bytes.len() > READ_WHOLE_MAX {
        Some(Cut::OverTwoMb)
    } else if total > WHOLE_LINES_MAX || longest > LINE_BYTES_MAX {
        Some(Cut::TooLong)
    } else {
        None
    };
    if let Some(why) = why {
        let lines = first_lines(text);
        return Some(Body::Text(TextDoc {
            truncated: Some(Truncated {
                shown: u32::try_from(lines.len()).unwrap_or(u32::MAX),
                total: u32::try_from(total).unwrap_or(u32::MAX),
                why,
            }),
            lines,
        }));
    }
    Some(if markdown && bytes.len() <= MARKDOWN_RENDER_MAX {
        Body::Markdown(MdDoc {
            source: text.to_string(),
        })
    } else {
        Body::Text(TextDoc {
            lines: plain_lines(text),
            truncated: None,
        })
    })
}

/// The first lines of a text shown in part: at most [`SHOWN_LINES`] of them, each cut at [`LINE_BYTES_MAX`]
/// (on a character), and at most [`SHOWN_BYTES`] in all. No line the reader is handed is longer than the limit
/// it is said to hold.
fn first_lines(text: &str) -> Vec<Vec<crate::frames::viewers::source::Run>> {
    let cut = |line: &str, at_most: usize| {
        let mut end = line.len().min(at_most);
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        end
    };
    // Sized once: a buffer that grew would leave its earlier copies of the text behind, not overwritten.
    let mut shown = String::with_capacity(SHOWN_BYTES);
    for (i, line) in text.lines().take(SHOWN_LINES).enumerate() {
        let room = SHOWN_BYTES.saturating_sub(shown.len() + usize::from(i > 0));
        if room == 0 && i > 0 {
            break;
        }
        if i > 0 {
            shown.push('\n');
        }
        shown.push_str(&line[..cut(line, LINE_BYTES_MAX.min(room))]);
    }
    let lines = plain_lines(&shown);
    shown.zeroize();
    lines
}

/// The toolkit's decoders, one set for the app (started with the first picture). Its waker does nothing: the
/// worker waits for its own picture.
fn decoders() -> &'static rune::ImageCache {
    static DECODERS: std::sync::OnceLock<rune::ImageCache> = std::sync::OnceLock::new();
    DECODERS.get_or_init(|| rune::ImageCache::new(Arc::new(|| {})))
}

/// The decrypted bytes as a picture, or why not. What the file declares is held to the viewer's own limits FIRST
/// (`viewer_picture::check`, from the header): a file that is not one of the four formats by its content, or
/// that declares more than the viewer decodes, never reaches a decoder. `cancel` is asked before the decode
/// starts; the decode itself cannot be stopped, and is bounded by those limits.
fn picture_body(bytes: &[u8], cancel: &AtomicBool) -> Content {
    let nothing = |why| Content {
        body: Some(Body::NotPreviewable(why)),
        picture: None,
        origin: Origin::Beebeeb,
    };
    match viewer_picture::check(bytes) {
        Err(viewer_picture::Refused::NotAPicture) => return nothing(NoPreview::Unreadable),
        Err(viewer_picture::Refused::TooBig(declared)) => {
            return nothing(NoPreview::PictureTooBig {
                width: declared.width,
                height: declared.height,
            });
        }
        Ok(_) => {}
    }
    if cancel.load(Ordering::SeqCst) {
        return nothing(NoPreview::Unreadable);
    }
    match picture(bytes) {
        Some(pixels) => Content {
            body: Some(Body::Photo(PhotoDoc {
                pill: None,
                note: String::new(),
                strip: Vec::new(),
            })),
            picture: Some(pixels),
            origin: Origin::Beebeeb,
        },
        None => nothing(NoPreview::Unreadable),
    }
}

#[cfg(test)]
thread_local! {
    /// How many times this thread handed bytes to a picture decoder: a test asserts that a refusal handed none.
    static DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The bytes decoded by the toolkit's decoders, waited for on this (the worker's) thread. `None`: it does not
/// decode. The decoder reads a copy of the bytes; that copy is overwritten here once the decoder let go of it,
/// and a line is logged if it never did. Only for bytes [`viewer_picture::check`] passed.
fn picture(bytes: &[u8]) -> Option<rune::Pixels> {
    #[cfg(test)]
    DECODES.with(|count| count.set(count.get() + 1));
    let mut encoded: Arc<[u8]> = Arc::from(bytes);
    let image = decoders().load(encoded.clone());
    let decoded = settled(&image);
    // The decoder's job holds the other handle for a moment after it answered.
    let mut wiped = false;
    for _ in 0..2_000 {
        if let Some(copy) = Arc::get_mut(&mut encoded) {
            copy.zeroize();
            wiped = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    if !wiped {
        log("viewer: the picture decoder kept its copy of an open file; it was released, not overwritten");
    }
    decoded
}

/// What the view draws of the file's bytes.
fn content_of(shows: Shows, bytes: &[u8], cancel: &AtomicBool) -> Content {
    let unreadable = || Body::NotPreviewable(NoPreview::Unreadable);
    match shows {
        Shows::Text | Shows::Markdown => Content {
            body: Some(text_body(bytes, shows == Shows::Markdown).unwrap_or_else(unreadable)),
            picture: None,
            origin: Origin::Beebeeb,
        },
        Shows::Picture => picture_body(bytes, cancel),
        // A PDF is opened by the worker itself (`open_pdf`): it keeps the document for its pages.
        Shows::Pdf | Shows::Nothing => Content {
            body: Some(Body::NotPreviewable(NoPreview::NoViewer)),
            picture: None,
            origin: Origin::Beebeeb,
        },
    }
}

/// What the local state said of the file's copy on this PC when the file was opened.
#[derive(Clone, Copy)]
struct Recorded {
    /// Its bytes are here (`OpenFileFacts::on_this_pc`).
    here: bool,
    /// The copy here is not settled with Beebeeb's version (`OpenFileFacts::unsettled_here`).
    unsettled: bool,
}

/// The file's bytes: the copy on this PC when the local state says it is here and it really is (no network at
/// all), else the download. The answer says which, and the session the bytes were read for. The buffer is the
/// engine's own, wiped when it is dropped.
///
/// The origin never says more than is known: a copy in sync by the local state is only that while sync runs
/// (with sync off nothing has compared it with Beebeeb since), and Beebeeb's version of a file whose copy here
/// is not settled with it may not be what is on this PC.
fn read<E: ViewerEngine>(
    engine: &E,
    file_id: &str,
    recorded: Recorded,
    progress: &(dyn Fn(u64, u64) + Send + Sync + 'static),
    cancel: &AtomicBool,
) -> Result<(SessionGeneration, zeroize::Zeroizing<Vec<u8>>, Origin), OpenFileError> {
    if recorded.here {
        match engine.local(file_id, IN_MEMORY_MAX, cancel)? {
            LocalCopy::Opened(copy) => {
                let origin = match copy.copy {
                    OnThisPc::NotSentYet => Origin::ThisPc { unsent: true },
                    OnThisPc::InSync if engine.is_syncing() => Origin::ThisPc { unsent: false },
                    OnThisPc::InSync => Origin::ThisPcSyncOff,
                };
                return Ok((copy.session, copy.bytes, origin));
            }
            // Not wholly on this PC after all, and in sync: the download is the way.
            LocalCopy::NotHere => {}
        }
    }
    let opened = engine.fetch(file_id, IN_MEMORY_MAX, progress, cancel)?;
    let origin = if recorded.unsettled {
        Origin::BeebeebNotThisPcs
    } else {
        Origin::Beebeeb
    };
    Ok((opened.session, opened.bytes, origin))
}

/// Open the PDF in `bytes` (taken: the document reads the engine's buffer and wipes it when it is dropped). The
/// view's body, and the document for its pages when it opened.
fn open_pdf(bytes: zeroize::Zeroizing<Vec<u8>>, size: &str) -> (Body, Option<viewer_pdf::Document>) {
    match viewer_pdf::Document::open(bytes) {
        Ok(document) => (
            Body::Pdf(PdfDoc {
                pages: document.pages(),
                page: 1,
                size: size.to_string(),
            }),
            Some(document),
        ),
        Err(why) => (
            Body::NotPreviewable(match why {
                viewer_pdf::NotOpened::Locked => NoPreview::PdfLocked,
                viewer_pdf::NotOpened::TooLong { pages } => NoPreview::PdfTooLong { pages },
                viewer_pdf::NotOpened::Unreadable => NoPreview::PdfUnreadable,
            }),
            None,
        ),
    }
}

/// A rendered page as the toolkit draws it (with the smaller copies it scales down from). `None`: the toolkit
/// did not take it.
fn page_pixels(page: viewer_pdf::Rendered) -> Option<rune::Pixels> {
    settled(&decoders().rgba(page.width, page.height, page.rgba))
}

/// The pixels of `image` once the toolkit decoded it, waited for on this (a worker's) thread. `None`: it does
/// not decode. The toolkit marks an image decoded a moment BEFORE its pixel handle holds the picture, so the
/// handle is waited for too: handed out earlier it has no size yet, and a page or a photo drawn from it in that
/// moment has no height.
fn settled(image: &rune::Image) -> Option<rune::Pixels> {
    let [width, height] = image.wait().ok()?;
    let pixels = image.pixels().clone();
    for _ in 0..2_000 {
        if pixels.size() == (width, height) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Some(pixels)
}

/// What the view shows for a download that did not hand over content. `None`: nothing (it was cancelled).
fn body_of(error: OpenFileError) -> Option<Body> {
    Some(match error {
        OpenFileError::Cancelled => return None,
        OpenFileError::TooLarge { size } => Body::NotPreviewable(NoPreview::TooBig {
            size,
            limit: IN_MEMORY_MAX,
        }),
        // It passed the limit while it was read: no size is known, and none is said.
        OpenFileError::OverLimit => Body::NotPreviewable(NoPreview::OverLimit { limit: IN_MEMORY_MAX }),
        // The copy on this PC holds a change that is not sent, and could not be read: asked again on Try
        // again, never answered with the older version Beebeeb has.
        OpenFileError::CopyNotReadable => Body::Stopped(Stopped::CopyNotReadable),
        OpenFileError::Offline => Body::Stopped(Stopped::Offline),
        OpenFileError::SessionEnded => Body::Stopped(Stopped::SessionEnded),
        // The session, or whose local state this is, changed under the download.
        OpenFileError::NotSignedIn | OpenFileError::NotYours => Body::Stopped(Stopped::SessionChanged),
        OpenFileError::NotFound => Body::Stopped(Stopped::Gone),
        OpenFileError::NothingSynced => Body::Stopped(Stopped::NothingSynced),
        OpenFileError::Failed => Body::Failed,
    })
}

impl Opening {
    /// Stop everything of this opening: the download is cancelled, the picture released, an answer on its way
    /// dropped (its text overwritten).
    fn end(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.picture = None;
        // The worker that holds the document ends when nobody asks any more: the document and its bytes go.
        self.pages = None;
        while self.inbox.try_recv().is_ok() {}
        self.fetching = false;
        self.holds = false;
        self.ended = true;
    }
}

impl<E: ViewerEngine> EngineViewerSource<E> {
    pub(crate) fn on(engine: Arc<E>) -> Self {
        Self { engine, open: None }
    }

    /// Whether the session in memory is the one `opening` was made for: the same generation, the same account,
    /// and a session at all.
    fn is_current(engine: &E, opening: &Opening) -> bool {
        engine.has_session() && engine.session_turn() == opening.session && engine.account() == opening.account
    }

    /// The opening of `file_id`, while its session is the session.
    fn current(&self, file_id: &str) -> Option<&Opening> {
        self.open
            .as_ref()
            .filter(|opening| opening.file_id == file_id && !opening.ended && Self::is_current(&self.engine, opening))
    }

    /// Open `file_id` for the session in memory: its facts now, its content on a worker.
    fn begin(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        // What was open before is over (a retry, or the source asked again).
        if let Some(mut before) = self.open.take() {
            before.end();
        }
        let (session, account, facts) = self.facts_now(file_id)?;
        let recorded = Recorded {
            here: facts.on_this_pc.is_some(),
            unsettled: facts.unsettled_here,
        };
        let shows = shows(&facts.name);
        let (answers, inbox) = mpsc::channel();
        // A PDF: the worker keeps the document and renders the pages the view asks for.
        let (pages, pdf) = if shows == Shows::Pdf && facts.size <= IN_MEMORY_MAX {
            let ((ask, asks), (page_answers, got)) = (mpsc::channel(), mpsc::channel());
            let pages = Pages {
                ask,
                got,
                kept: RefCell::default(),
                asked: Cell::new(None),
                doc: RefCell::new(None),
            };
            let size = crate::source::format_size(i64::try_from(facts.size).unwrap_or(i64::MAX));
            let ends = PdfEnds {
                asks,
                answers: page_answers,
                size,
            };
            (Some(pages), Some(ends))
        } else {
            (None, None)
        };
        let mut opening = Opening {
            file_id: file_id.to_string(),
            session,
            account,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Progress::default()),
            inbox,
            fetching: false,
            drawn: (0, facts.size),
            holds: false,
            picture: None,
            pages,
            origin: Origin::Beebeeb,
            ended: false,
        };
        let body = if shows == Shows::Nothing {
            Body::NotPreviewable(NoPreview::NoViewer)
        } else if facts.size > IN_MEMORY_MAX {
            Body::NotPreviewable(NoPreview::TooBig {
                size: facts.size,
                limit: IN_MEMORY_MAX,
            })
        } else if self.fetch_on_a_worker(&opening, shows, recorded, answers, pdf) {
            opening.fetching = true;
            Body::Decrypting {
                done: 0,
                total: facts.size,
            }
        } else {
            log("viewer: could not start a thread for an open file");
            Body::Failed
        };
        self.open = Some(opening);
        Ok(file_of(file_id, facts, body))
    }

    /// The facts of `file_id` for the session in memory, with the session they are for. Each refusal keeps its
    /// kind: no session, a session Beebeeb ended, a file that is not there, a PC where nothing has synced yet,
    /// a local state that is not this account's, and "not right now" (the local state could not be read, or the
    /// session changed during the read; the read is tried once more first, whether the change shows as facts
    /// of another session or as a refused read).
    fn facts_now(&self, file_id: &str) -> Result<(SessionGeneration, Option<String>, OpenFileFacts), OpenError> {
        let mut again = true;
        loop {
            if !self.engine.has_session() {
                return Err(OpenError::SignedOut);
            }
            let (session, account) = (self.engine.session_turn(), self.engine.account());
            match self.engine.facts(file_id) {
                // Facts read across a session change are not this session's.
                Ok(facts) if facts.session == session => return Ok((session, account, facts)),
                Ok(_) => {}
                Err(OpenFileError::NotFound) => return Err(OpenError::NotFound),
                // Neither says anything about the file: it is not "gone".
                Err(OpenFileError::NothingSynced) => return Err(OpenError::NothingSynced),
                Err(OpenFileError::NotYours) => return Err(OpenError::NotThisAccounts),
                Err(OpenFileError::SessionEnded) => return Err(OpenError::SessionEnded),
                // The read was refused for the session: there is none any more (the next turn of this loop says
                // signed out), or it changed during the read (asked once more).
                Err(OpenFileError::NotSignedIn) => {}
                Err(_) => return Err(OpenError::NotNow),
            }
            if !std::mem::take(&mut again) {
                return Err(OpenError::NotNow);
            }
        }
    }

    /// Read (the copy on this PC, else download and decrypt) on a worker; its one answer goes down `answers`.
    /// `false`: no worker started.
    fn fetch_on_a_worker(
        &self,
        opening: &Opening,
        shows: Shows,
        recorded: Recorded,
        answers: mpsc::Sender<Answer>,
        pdf: Option<PdfEnds>,
    ) -> bool {
        let (engine, file_id) = (self.engine.clone(), opening.file_id.clone());
        let (cancel, progress) = (opening.cancel.clone(), opening.progress.clone());
        std::thread::Builder::new()
            .name("viewer-open".into())
            .spawn(move || {
                let (waker, moving) = (engine.clone(), progress.clone());
                let report = move |done: u64, total: u64| {
                    moving.done.store(done, Ordering::SeqCst);
                    moving.total.store(total, Ordering::SeqCst);
                    // The window is woken once per whole percent, not once per chunk.
                    let percent = if total == 0 { 100 } else { done.min(total) * 100 / total };
                    if moving.woken_at.swap(percent, Ordering::SeqCst) != percent {
                        waker.wake();
                    }
                };
                // A panic in this worker must not leave the view on "Decrypting" for good: it answers Failed.
                let (answer, document): (Answer, Option<viewer_pdf::Document>) =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let (session, bytes, origin) = match read(&*engine, &file_id, recorded, &report, &cancel) {
                            Ok(read) => read,
                            Err(error) => return (Err(error), None),
                        };
                        let (mut content, document) = match (&pdf, shows) {
                            (Some(ends), Shows::Pdf) => {
                                let (body, document) = open_pdf(bytes, &ends.size);
                                let content = Content {
                                    body: Some(body),
                                    picture: None,
                                    origin,
                                };
                                (content, document)
                            }
                            // `bytes` is dropped at the end of this arm: the engine's buffer is wiped there.
                            _ => (content_of(shows, &bytes, &cancel), None),
                        };
                        content.origin = origin;
                        (Ok((session, content)), document)
                    }))
                    .unwrap_or((Err(OpenFileError::Failed), None));
                // Nobody waits for the content any more (the file was closed, its session moved on, Cancel): it is
                // dropped here, its text overwritten, and never queued. What is sent says only "cancelled".
                let answer = if cancel.load(Ordering::SeqCst) {
                    drop(answer);
                    Err(OpenFileError::Cancelled)
                } else {
                    answer
                };
                let cancelled = answer.is_err();
                let _ = answers.send(answer);
                engine.wake();
                // An open PDF: this thread keeps the document and renders the pages the view asks for, until
                // the file is left (the asker goes) or its session moves on (`cancel`). Then the document is
                // dropped, and the bytes it read from are overwritten.
                if let (Some(document), Some(ends), false) = (document, pdf, cancelled) {
                    document.serve(&ends.asks, &cancel, &|page, rendered| {
                        let lacks_text = rendered.as_ref().is_some_and(|rendered| rendered.lacks_text);
                        let _ = ends.answers.send((page, rendered.and_then(page_pixels), lacks_text));
                        engine.wake();
                    });
                }
            })
            .is_ok()
    }
}

/// The file as the view draws it. The path starts at "Files", as the window's own crumbs do.
fn file_of(file_id: &str, facts: OpenFileFacts, body: Body) -> OpenFile {
    // The size as the Files window writes it.
    let size = crate::source::format_size(i64::try_from(facts.size).unwrap_or(i64::MAX));
    let kind = kind_word(&facts.name);
    let mut path = vec!["Files".to_string()];
    path.extend(facts.folders);
    OpenFile {
        id: file_id.to_string(),
        facts: format!("{kind} \u{00B7} {size}"),
        // Not read: the reader shows no version (the editor would, and it does not open here).
        version: 0,
        body,
        details: Details {
            kind: kind.to_string(),
            size,
            facts: vec![("Where", path.join(" \u{203A} "), false)],
            // Not read here, and not drawn (`knows_offline`).
            offline: false,
        },
        newer: None,
        path,
        name: facts.name,
    }
}

impl<E: ViewerEngine> ViewerSource for EngineViewerSource<E> {
    fn open(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        self.begin(file_id)
    }

    fn poll(&mut self, file_id: &str) -> Option<Body> {
        let opening = self.open.as_mut().filter(|opening| opening.file_id == file_id)?;
        if opening.ended {
            // An answer that slipped past the worker's own check is dropped here (its text overwritten).
            while opening.inbox.try_recv().is_ok() {}
            return None;
        }
        // The session is not the one the file was opened for: everything of it goes NOW, on the window's side.
        if !Self::is_current(&self.engine, opening) {
            let had = opening.fetching || opening.holds;
            // Nothing of the earlier session is held or on its way, and the account is the same (a lock and an
            // unlock): what is on screen says nothing a session decrypted, so the opening is this session's now.
            if !had && self.engine.has_session() && self.engine.account() == opening.account {
                opening.session = self.engine.session_turn();
                return None;
            }
            opening.end();
            return had.then_some(Body::Stopped(Stopped::SessionChanged));
        }
        match opening.inbox.try_recv() {
            Ok(answer) => {
                opening.fetching = false;
                match answer {
                    Ok((session, mut content)) if session == opening.session => {
                        opening.picture = content.picture.take();
                        opening.origin = content.origin;
                        opening.holds = true;
                        let body = content.body.take();
                        if let (Some(pages), Some(Body::Pdf(doc))) = (&opening.pages, &body) {
                            *pages.doc.borrow_mut() = Some(doc.clone());
                        }
                        body
                    }
                    // Decrypted for another session than the one this file was opened for: never shown.
                    Ok(_) => Some(Body::Stopped(Stopped::SessionChanged)),
                    Err(error) => {
                        if !matches!(
                            error,
                            OpenFileError::Cancelled | OpenFileError::TooLarge { .. } | OpenFileError::OverLimit
                        ) {
                            log(format!("viewer: a file did not open ({})", error.kind()));
                        }
                        body_of(error)
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) if opening.fetching => {
                let now = (
                    opening.progress.done.load(Ordering::SeqCst),
                    opening.progress.total.load(Ordering::SeqCst),
                );
                // Before the first report the total is the size the local state named.
                let now = if now.1 == 0 { (now.0, opening.drawn.1) } else { now };
                (now != opening.drawn).then(|| {
                    opening.drawn = now;
                    Body::Decrypting {
                        done: now.0,
                        total: now.1,
                    }
                })
            }
            // A page of the open PDF came back: the same document again, so the view is drawn with the page.
            Err(_) if opening.pages.as_ref().is_some_and(Pages::take_in) => opening
                .pages
                .as_ref()
                .and_then(|pages| pages.doc.borrow().clone())
                .map(Body::Pdf),
            Err(mpsc::TryRecvError::Empty) => None,
            // The worker ended without an answer.
            Err(mpsc::TryRecvError::Disconnected) => std::mem::take(&mut opening.fetching).then_some(Body::Failed),
        }
    }

    fn cancel(&mut self, file_id: &str) {
        if let Some(opening) = self.open.as_ref().filter(|opening| opening.file_id == file_id) {
            opening.cancel.store(true, Ordering::SeqCst);
        }
    }

    fn retry(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        self.begin(file_id)
    }

    fn picture(&self, file_id: &str, which: Picture) -> Option<rune::Pixels> {
        match which {
            Picture::Photo => self.current(file_id)?.picture.clone(),
            // A page of the open PDF: the one kept, else it is asked of the worker (once) and comes later.
            Picture::Page(page) => {
                let pages = self.current(file_id)?.pages.as_ref()?;
                pages
                    .doc
                    .borrow()
                    .as_ref()
                    .filter(|doc| (1..=doc.pages).contains(&page))?;
                if let Some(held) = pages.held(page) {
                    return held;
                }
                if pages.asked.get() != Some(page) && pages.ask.send(page).is_ok() {
                    pages.asked.set(Some(page));
                }
                None
            }
            // A strip and artwork are not bound: the frame draws its plain tile.
            _ => None,
        }
    }

    fn open_elsewhere(&mut self, file_id: &str) -> Elsewhere {
        if self.current(file_id).is_none() {
            return Elsewhere::Refused;
        }
        self.engine.open_elsewhere(file_id)
    }

    fn previews(&self, name: &str) -> bool {
        shows(name) != Shows::Nothing
    }

    /// The local state says the file's bytes are on this PC, and sync runs for the folder they are in (so the
    /// system can open the file there). Otherwise the viewer shows it.
    fn kept_on_this_pc(&self, file_id: &str) -> bool {
        self.engine.has_session()
            && self.engine.is_syncing()
            && self
                .engine
                .facts(file_id)
                .is_ok_and(|facts| facts.session == self.engine.session_turn() && facts.on_this_pc.is_some())
    }

    fn origin(&self, file_id: &str) -> Origin {
        self.current(file_id)
            .filter(|opening| opening.holds)
            .map_or(Origin::Beebeeb, |opening| opening.origin)
    }

    fn page_state(&self, file_id: &str, page: u32) -> PageState {
        let Some(pages) = self.current(file_id).and_then(|opening| opening.pages.as_ref()) else {
            return PageState::Ready;
        };
        match pages.held(page) {
            Some(Some(_)) if pages.lacks_text(page) => PageState::LacksText,
            Some(Some(_)) => PageState::Ready,
            Some(None) => PageState::Failed,
            None => PageState::Rendering,
        }
    }

    /// The text of a PDF is not searched here.
    fn finds(&self) -> bool {
        false
    }

    /// "Save a copy…" has nowhere to go yet: the button is not drawn.
    fn saves_copies(&self) -> bool {
        false
    }

    /// Saving an edit is not bound: the editor does not open.
    fn can_save(&self) -> bool {
        false
    }

    /// Whether the file is kept on this PC is not read here: the details panel draws no switch for it.
    fn knows_offline(&self) -> bool {
        false
    }
}

/// The file is left (closed, the route moved on, the account left): the download stops, the picture goes.
impl<E: ViewerEngine> Drop for EngineViewerSource<E> {
    fn drop(&mut self) {
        if let Some(opening) = self.open.as_mut() {
            opening.end();
        }
    }
}

//! An open file on the real engine: [`EngineViewerSource`], the `ViewerSource` of the running app. One source per
//! opened file (`frames::viewers::RouteSource`).
//!
//! What it shows, and from where:
//! - the name, the folders above it and the size: the signed-in account's local state, through the engine's
//!   account-bound handle ([`Engine::open_file_facts`]); read once, when the file opens.
//! - the content: downloaded and decrypted IN MEMORY by the engine's own download path
//!   ([`Engine::open_file_in_memory`]: the path every hydrate uses, no crypto here), on a worker, with the
//!   engine's progress. Text, Markdown, CSV / TSV (as text) and PNG / JPEG / GIF / WebP pictures are shown.
//! - not shown, honestly (`Body::NotPreviewable`, nothing downloaded): PDF (this app links no PDF renderer),
//!   audio, archives, RAW and HEIC pictures, Office documents, any file over [`IN_MEMORY_MAX`].
//! - "Open in another app…": the Files window's own open ([`EngineFilesSource`]): the system opens the file from
//!   the account's folder on this PC, where the engine already puts opened files. No copy is made here.
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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::viewer_api::{OpenFileError, OpenFileFacts, OpenedFile};
use beebeeb_engine::app::{CredentialStores, Engine};
use zeroize::Zeroize;

use super::{EngineFilesSource, log};
use crate::frames::files::source::FilesSource;
use crate::frames::viewers::kinds::{self, PhotoDoc, Picture, Viewer};
use crate::frames::viewers::source::{
    Body, Details, Elsewhere, MdDoc, NoPreview, OpenError, OpenFile, Stopped, TextDoc, Truncated, ViewerSource,
    plain_lines,
};
use crate::source::DriveItem;

#[cfg(test)]
#[path = "viewer_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "viewer_window_tests.rs"]
mod window_tests;

/// The largest file a preview decrypts: it is held in memory, whole, while it is open. Larger files are not
/// downloaded for a preview at all.
pub const IN_MEMORY_MAX: u64 = 64 * 1024 * 1024;

/// A text over this is shown read-only, its first [`SHOWN_LINES`] lines only (the board's DW-1527).
pub const READ_WHOLE_MAX: usize = 2_000_000;
pub const SHOWN_LINES: usize = 500;

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
    /// Ask the system to open the file with its own app, from the account's folder on this PC. `false`: refused.
    fn open_elsewhere(&self, file_id: &str) -> bool;
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

    /// The Files window's own open, for this one file: the path is resolved under the account-bound handle, the
    /// sync guard and the executable check apply, and the system is asked without a shell.
    fn open_elsewhere(&self, file_id: &str) -> bool {
        let item = DriveItem {
            id: file_id.to_string(),
            parent_id: None,
            name: String::new(),
            is_folder: false,
            size: None,
            child_count: None,
            modified_at: String::new(),
            mime: None,
            has_thumbnail: false,
            shared_type: None,
            shared_badges: Vec::new(),
            pc_status: None,
            is_protected: false,
        };
        EngineFilesSource::new(self.engine.clone()).open(&[item]).is_ok()
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
    /// The session moved on and the view was told: nothing more comes from this opening.
    ended: bool,
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
        Some(Viewer::Office) => "Document",
        None => "File",
    }
}

/// The decrypted bytes as text the view draws, or `None` when they are not text (not UTF-8, or a NUL in them).
/// Over [`READ_WHOLE_MAX`]: the first [`SHOWN_LINES`] lines, read-only.
fn text_body(bytes: &[u8], markdown: bool) -> Option<Body> {
    let text = std::str::from_utf8(bytes).ok()?;
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    if text.contains('\0') {
        return None;
    }
    if bytes.len() > READ_WHOLE_MAX {
        let total = text.lines().count();
        let end = text
            .match_indices('\n')
            .nth(SHOWN_LINES - 1)
            .map_or(text.len(), |(at, _)| at);
        return Some(Body::Text(TextDoc {
            lines: plain_lines(&text[..end]),
            truncated: Some(Truncated {
                shown: total.min(SHOWN_LINES) as u32,
                total: u32::try_from(total).unwrap_or(u32::MAX),
            }),
        }));
    }
    Some(if markdown {
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

/// The decrypted bytes as a picture: decoded by the toolkit's decoders, on this (the worker's) thread. `None`: it
/// does not decode. The copy the decoder read is overwritten once the decoder let go of it.
fn picture(bytes: &[u8]) -> Option<rune::Pixels> {
    let mut encoded: Arc<[u8]> = Arc::from(bytes);
    let cache = rune::ImageCache::new(Arc::new(|| {}));
    let image = cache.load(encoded.clone());
    let decoded = image.wait().is_ok();
    // The decoder's job holds the other handle for a moment after it answered.
    for _ in 0..200 {
        if let Some(copy) = Arc::get_mut(&mut encoded) {
            copy.zeroize();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    decoded.then(|| image.pixels().clone())
}

/// What the view draws of the decrypted file.
fn content_of(shows: Shows, bytes: &[u8]) -> Content {
    let unreadable = || Body::NotPreviewable(NoPreview::Unreadable);
    match shows {
        Shows::Text | Shows::Markdown => Content {
            body: Some(text_body(bytes, shows == Shows::Markdown).unwrap_or_else(unreadable)),
            picture: None,
        },
        Shows::Picture => match picture(bytes) {
            Some(pixels) => Content {
                body: Some(Body::Photo(PhotoDoc {
                    pill: None,
                    note: String::new(),
                    strip: Vec::new(),
                })),
                picture: Some(pixels),
            },
            None => Content {
                body: Some(unreadable()),
                picture: None,
            },
        },
        Shows::Nothing => Content {
            body: Some(Body::NotPreviewable(NoPreview::NoViewer)),
            picture: None,
        },
    }
}

/// What the view shows for a download that did not hand over content. `None`: nothing (it was cancelled).
fn body_of(error: OpenFileError) -> Option<Body> {
    Some(match error {
        OpenFileError::Cancelled => return None,
        OpenFileError::TooLarge { size } => Body::NotPreviewable(NoPreview::TooBig {
            size,
            limit: IN_MEMORY_MAX,
        }),
        OpenFileError::Offline => Body::Stopped(Stopped::Offline),
        OpenFileError::SessionEnded => Body::Stopped(Stopped::SessionEnded),
        // The session, or whose local state this is, changed under the download.
        OpenFileError::NotSignedIn | OpenFileError::NotYours => Body::Stopped(Stopped::SessionChanged),
        OpenFileError::NotFound => Body::Stopped(Stopped::Gone),
        OpenFileError::Failed => Body::Failed,
    })
}

impl Opening {
    /// Stop everything of this opening: the download is cancelled, the picture released, an answer on its way
    /// dropped (its text overwritten).
    fn end(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.picture = None;
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
        if !self.engine.has_session() {
            return Err(OpenError::NotFound);
        }
        let (session, account) = (self.engine.session_turn(), self.engine.account());
        let facts = self.engine.facts(file_id).map_err(|_| OpenError::NotFound)?;
        // Facts read across a session change are not this session's.
        if facts.session != session {
            return Err(OpenError::NotFound);
        }
        let shows = shows(&facts.name);
        let (answers, inbox) = mpsc::channel();
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
            ended: false,
        };
        let body = if shows == Shows::Nothing {
            Body::NotPreviewable(NoPreview::NoViewer)
        } else if facts.size > IN_MEMORY_MAX {
            Body::NotPreviewable(NoPreview::TooBig {
                size: facts.size,
                limit: IN_MEMORY_MAX,
            })
        } else if self.fetch_on_a_worker(&opening, shows, answers) {
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

    /// Download and decrypt on a worker; its one answer goes down `answers`. `false`: no worker started.
    fn fetch_on_a_worker(&self, opening: &Opening, shows: Shows, answers: mpsc::Sender<Answer>) -> bool {
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
                let answer: Answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let opened = engine.fetch(&file_id, IN_MEMORY_MAX, &report, &cancel)?;
                    // `opened` is dropped at the end of this closure: the engine's buffer is wiped there.
                    Ok((opened.session, content_of(shows, &opened.bytes)))
                }))
                .unwrap_or(Err(OpenFileError::Failed));
                // Nobody waits for it any more (the file was closed): the answer is dropped, its text overwritten.
                let _ = answers.send(answer);
                engine.wake();
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
            return None;
        }
        // The session is not the one the file was opened for: everything of it goes NOW, on the window's side.
        if !Self::is_current(&self.engine, opening) {
            let had = opening.fetching || opening.holds;
            opening.end();
            return had.then_some(Body::Stopped(Stopped::SessionChanged));
        }
        match opening.inbox.try_recv() {
            Ok(answer) => {
                opening.fetching = false;
                match answer {
                    Ok((session, mut content)) if session == opening.session => {
                        opening.picture = content.picture.take();
                        opening.holds = true;
                        content.body.take()
                    }
                    // Decrypted for another session than the one this file was opened for: never shown.
                    Ok(_) => Some(Body::Stopped(Stopped::SessionChanged)),
                    Err(error) => {
                        if !matches!(error, OpenFileError::Cancelled | OpenFileError::TooLarge { .. }) {
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
            // A strip, artwork and PDF pages are not bound: the frame draws its plain tile.
            _ => None,
        }
    }

    fn open_elsewhere(&mut self, file_id: &str) -> Elsewhere {
        if self.current(file_id).is_none() {
            return Elsewhere::Refused;
        }
        if self.engine.open_elsewhere(file_id) {
            Elsewhere::Opened
        } else {
            Elsewhere::Refused
        }
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

//! What the Files window's Preview panel shows of a file on the real engine: [`Previews`], the part of
//! `EngineFilesSource` behind `preview`, `preview_later` and `cancel_preview`.
//!
//! - ONE preview at a time. `preview` starts it: by the file's kind, a kind with no preview, a kind this version does
//!   not preview in the panel (a PDF, a picture) and a text over the preview limit are answered at once and NOTHING
//!   is downloaded for them; any other file is read on a worker, from the copy that is wholly on this PC when there
//!   is one (it opens without a connection), else downloaded and decrypted in memory by the engine's own path.
//! - The panel asks `preview_later` every frame while it waits. `Shown` is handed over ONCE (the page moves out);
//!   nothing decrypted stays here after that.
//! - Whose: the ask carries the session AND the account it was made under, read on the window's thread before the
//!   worker started. An answer counts only while both are still here, the content was decrypted for that session,
//!   the file is still the one asked and the panel did not stop it; otherwise it is dropped and wiped.
//! - Decrypted bytes: in the engine's `Zeroizing` buffer on the worker, then only the first lines, in a
//!   `Zeroizing<String>`, until they are handed over. Wiped on cancel, on a look under another session, and on drop.
//!   Nothing is written to disk and nothing is logged.
//!
//! Must not: log a name, an id or a line; keep a decrypted line after the panel let go of it; show text for a file
//! that is not the one in the panel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::viewer_api::{LocalCopy, OpenFileError};
use zeroize::Zeroizing;

use super::log;
use super::stamp::Stamp;
use super::viewer::{READ_WHOLE_MAX, Shows, ViewerEngine, kind_word, shows};
use crate::frames::files::actions_data::{Page, PreviewOf, Previewed};
use crate::frames::files::source::ActionError;
use crate::source::{DriveItem, format_size};

#[cfg(test)]
#[path = "files_preview_tests.rs"]
mod tests;

/// The first lines a preview holds: at most this many, and at most [`FIRST_BYTES`] of them.
const FIRST_LINES: usize = 60;
const FIRST_BYTES: usize = 6_000;

/// What the worker made of the content.
enum Outcome {
    /// The first lines, and whether anything was left out.
    Text {
        text: Zeroizing<String>,
        cut: bool,
    },
    /// Not text (not UTF-8, or a NUL in it).
    NotText,
    Failed(OpenFileError),
}

/// What a worker sends back. `session`: the one the content was decrypted for.
struct Answer {
    id: String,
    pressed: Stamp,
    session: Option<SessionGeneration>,
    outcome: Outcome,
}

/// What is held of the one preview.
enum Held {
    Loading,
    /// Read, not handed over yet: the page's text is in its `Zeroizing` buffer.
    Text {
        text: Zeroizing<String>,
        cut: bool,
    },
    /// Settled, with no text: handed out as it is.
    Settled(Previewed),
    /// `Shown` was handed over.
    Gone,
}

struct Ask {
    id: String,
    title: String,
    byline: String,
    cancel: Arc<AtomicBool>,
    held: Held,
}

/// No line of the content in a `{:?}`.
impl std::fmt::Debug for Ask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ask")
            .field("cancel", &self.cancel)
            .finish_non_exhaustive()
    }
}

/// See the module's doc.
pub struct Previews<E: ViewerEngine> {
    engine: Arc<E>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// The session and account the last look saw: when they move, the preview and what it read go.
    seen: Option<Stamp>,
    ask: Option<Ask>,
    /// Tests only: the next read waits on the first, on its worker, before it asks the engine anything, and says on
    /// the second that its answer is in the inbox.
    #[cfg(test)]
    next_call: Option<(mpsc::Receiver<()>, mpsc::Sender<()>)>,
}

fn stamp_of<E: ViewerEngine>(engine: &E) -> Stamp {
    (engine.session_turn(), engine.account())
}

impl<E: ViewerEngine> Previews<E> {
    /// Starts no thread and reads nothing.
    pub fn new(engine: Arc<E>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            answers,
            inbox,
            seen: None,
            ask: None,
            #[cfg(test)]
            next_call: None,
        }
    }

    /// The session or the account is not the one the last look saw: the preview and what it read go, here, now.
    fn look(&mut self, now: &Stamp) {
        if self.seen.as_ref() == Some(now) {
            return;
        }
        self.seen = Some(now.clone());
        self.cancel();
    }

    /// Stop the read and wipe what it decrypted.
    pub fn cancel(&mut self) {
        if let Some(ask) = self.ask.take() {
            ask.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Start the preview of `item`. What it is answered with is asked of [`Self::later`].
    pub fn start(&mut self, item: &DriveItem) {
        let pressed = stamp_of(&*self.engine);
        self.look(&pressed);
        self.cancel();
        let size = item.size.unwrap_or(0);
        let mut ask = Ask {
            id: item.id.clone(),
            title: item.name.clone(),
            byline: format!("{} \u{00B7} {}", kind_word(&item.name), format_size(size)),
            cancel: Arc::new(AtomicBool::new(false)),
            held: Held::Loading,
        };
        // The kind decides before anything is asked of the network: nothing is downloaded for these.
        let settled = match shows(&item.name) {
            Shows::Nothing => Some(Previewed::NoPreview),
            Shows::Picture | Shows::Pdf => Some(Previewed::NotYet),
            Shows::Text | Shows::Markdown if u64::try_from(size).is_ok_and(|size| size > READ_WHOLE_MAX as u64) => {
                Some(Previewed::TooLarge)
            }
            Shows::Text | Shows::Markdown => None,
        };
        if let Some(settled) = settled {
            ask.held = Held::Settled(settled);
        } else if !self.engine.has_session() {
            ask.held = Held::Settled(Previewed::Failed(ActionError::Unavailable));
        } else if !self.read(&ask, pressed) {
            ask.held = Held::Settled(Previewed::Failed(ActionError::Failed));
        }
        self.ask = Some(ask);
    }

    /// The worker of the read of `ask`. `false`: it could not start.
    fn read(&mut self, ask: &Ask, pressed: Stamp) -> bool {
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        let (id, cancel) = (ask.id.clone(), ask.cancel.clone());
        #[cfg(test)]
        let hooks = self.next_call.take();
        let started = std::thread::Builder::new().name("files-preview".into()).spawn(move || {
            #[cfg(test)]
            if let Some((hold, _)) = &hooks {
                let _ = hold.recv();
            }
            let read =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| first_lines_of(&*engine, &id, &cancel)));
            let (session, outcome) = read.unwrap_or_else(|_| {
                log("files: a preview ended early");
                (None, Outcome::Failed(OpenFileError::Failed))
            });
            let _ = answers.send(Answer {
                id,
                pressed,
                session,
                outcome,
            });
            #[cfg(test)]
            if let Some((_, ended)) = &hooks {
                let _ = ended.send(());
            }
            engine.wake();
        });
        if started.is_err() {
            log("files: could not start the preview");
        }
        started.is_ok()
    }

    /// Take in what the worker answered. An answer counts only for the session AND the account of its ask, for the
    /// content decrypted for that session, for the file asked, and while the panel did not stop it.
    fn pump(&mut self, now: &Stamp) {
        let here = self.engine.has_session();
        while let Ok(answer) = self.inbox.try_recv() {
            let Some(ask) = self.ask.as_mut() else {
                continue;
            };
            let current = here
                && answer.pressed == *now
                && ask.id == answer.id
                && !ask.cancel.load(Ordering::SeqCst)
                && answer.session.is_none_or(|session| session == now.0);
            if !current || !matches!(ask.held, Held::Loading) {
                log("files: a preview that is not the panel's was dropped");
                continue;
            }
            ask.held = match answer.outcome {
                Outcome::Text { text, cut } => Held::Text { text, cut },
                Outcome::NotText => Held::Settled(Previewed::NoPreview),
                Outcome::Failed(OpenFileError::TooLarge { .. } | OpenFileError::OverLimit) => {
                    Held::Settled(Previewed::TooLarge)
                }
                Outcome::Failed(OpenFileError::Cancelled) => continue,
                Outcome::Failed(why) => Held::Settled(Previewed::Failed(match why {
                    OpenFileError::Offline => ActionError::Offline,
                    OpenFileError::NotSignedIn
                    | OpenFileError::SessionEnded
                    | OpenFileError::NotYours
                    | OpenFileError::NothingSynced => ActionError::Unavailable,
                    _ => ActionError::Failed,
                })),
            };
        }
    }

    /// Where the preview of `item` stands: `Shown` is handed over once, and nothing of the text stays here.
    pub fn later(&mut self, item: &DriveItem) -> Previewed {
        let now = stamp_of(&*self.engine);
        self.look(&now);
        self.pump(&now);
        let Some(ask) = self.ask.as_mut().filter(|ask| ask.id == item.id) else {
            return Previewed::Failed(ActionError::Unavailable);
        };
        if !self.engine.has_session() {
            return Previewed::Failed(ActionError::Unavailable);
        }
        match std::mem::replace(&mut ask.held, Held::Gone) {
            Held::Loading => {
                ask.held = Held::Loading;
                Previewed::Loading
            }
            Held::Text { text, cut } => {
                // The page's own copy is the window's; this one is wiped here, now.
                Previewed::Shown(PreviewOf {
                    place: if cut { "The first lines" } else { "The whole file" }.to_string(),
                    page: Page {
                        title: ask.title.clone(),
                        byline: ask.byline.clone(),
                        sections: vec![(String::new(), text.as_str().to_string())],
                    },
                })
            }
            Held::Settled(settled) => {
                ask.held = Held::Settled(settled.clone());
                settled
            }
            Held::Gone => Previewed::NoPreview,
        }
    }
}

impl<E: ViewerEngine> Drop for Previews<E> {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// On a worker: the file's first lines, from the copy wholly on this PC when there is one, else downloaded. The
/// session the content was decrypted for is returned with them.
fn first_lines_of<E: ViewerEngine>(engine: &E, id: &str, cancel: &AtomicBool) -> (Option<SessionGeneration>, Outcome) {
    let (session, bytes) = match engine.local(id, READ_WHOLE_MAX as u64, cancel) {
        Ok(LocalCopy::Opened(copy)) => (copy.session, copy.bytes),
        // Not wholly on this PC: the download is the way.
        Ok(LocalCopy::NotHere) => match engine.fetch(id, READ_WHOLE_MAX as u64, &|_, _| {}, cancel) {
            Ok(opened) => (opened.session, opened.bytes),
            Err(why) => return (None, Outcome::Failed(why)),
        },
        Err(why) => return (None, Outcome::Failed(why)),
    };
    if cancel.load(Ordering::SeqCst) {
        return (None, Outcome::Failed(OpenFileError::Cancelled));
    }
    (Some(session), first_lines(&bytes))
}

/// The first lines of `bytes` as text: at most [`FIRST_LINES`] and [`FIRST_BYTES`], cut at a character. Bytes that
/// are not text (not UTF-8, or a NUL in them) have no preview. `bytes` is wiped by its owner; only the cut text is
/// copied, into a buffer that is wiped when dropped.
fn first_lines(bytes: &[u8]) -> Outcome {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Outcome::NotText;
    };
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    if text.contains('\0') {
        return Outcome::NotText;
    }
    let mut out = Zeroizing::new(String::with_capacity(text.len().min(FIRST_BYTES + 1)));
    let mut cut = false;
    for (n, line) in text.lines().enumerate() {
        if n >= FIRST_LINES {
            cut = true;
            break;
        }
        // The line and its break must fit; a line that does not is cut at a character.
        let room = FIRST_BYTES.saturating_sub(out.len());
        if line.len() + 1 > room {
            let mut end = line.len().min(room);
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            out.push_str(&line[..end]);
            cut = true;
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    while out.ends_with('\n') {
        out.pop();
    }
    Outcome::Text { text: out, cut }
}

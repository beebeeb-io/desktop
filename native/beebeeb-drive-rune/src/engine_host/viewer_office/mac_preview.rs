//! macOS shows the file itself (dm-70): a recording or a video AVFoundation plays, and a document Quick Look
//! previews (Word, Excel, PowerPoint, Pages, Numbers, Keynote, RTF, PSD, HEIC, RAW, …), in the system's Quick Look
//! panel over the app. Part of [`EngineViewerSource`]: `viewer_media.rs` asks it for the kinds the app has no viewer
//! for, `viewer_office/glue.rs` for an Office file (the Mac has no Office helper). macOS only.
//!
//! - **The type decides** ([`Host::shower_of`]): a kind the app shows itself (text, Markdown, pictures, PDF) stays
//!   the app's; a kind AVFoundation reads is a [`Shower::Player`] one, a kind Quick Look previews a
//!   [`Shower::QuickLook`] one; any other kind keeps the page with the other apps (dm-63). A player kind is asked
//!   again of the file itself (`AVURLAsset.isPlayable`) once its copy is there.
//! - **The copy:** the system reads a file, so the decrypted bytes are written to a temp copy where Files' Open puts
//!   its own (`<temp>/beebeeb-open/`, here `<id>.preview/<name>`, the folder 0700 and the file 0600). The bytes are
//!   read as any viewer's are (`viewer_read::read`: the copy on this Mac, else the download), up to [`PREVIEW_MAX`].
//! - **When the copy goes:** when the viewer closes or opens another file, when the session is not the one it was
//!   opened for (a lock, a sign-out, another account), when the person closes the panel, and when the panel could
//!   not show it. [`TempCopy`]'s drop removes it, so a copy nobody took is removed too. A copy a crash or a quit
//!   left behind is removed when the next one is made (`TempCopy::sweep`).
//! - **What the window says:** the viewer's own "Decrypting" page with the engine's progress while the copy is
//!   made, and at 100 percent while the panel shows it (its Cancel closes both); after the person closes the panel,
//!   the page with the other apps. There is no page of its own yet for "shown in Quick Look".
//!
//! Session rule, as the rest of the viewer: the copy is made for the session of the press and shown only while that
//! session is the session.
//!
//! Must not: log a name, an id, a path or a byte; leave a copy behind.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::viewer_api::OpenFileError;

use crate::engine_host::viewer::{EngineViewerSource, Recorded, Shows, ViewerEngine, body_of, log, read, shows};
use crate::frames::viewers::source::{Body, NoPreview, Stopped};

#[cfg(test)]
#[path = "mac_preview_tests.rs"]
mod tests;

/// The largest file shown this way: its bytes are held in memory, whole, while the copy is written.
pub(crate) const PREVIEW_MAX: u64 = 1024 * 1024 * 1024;

/// What of macOS shows a kind of file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shower {
    /// AVFoundation plays it (a recording, a video): the file itself is asked once its copy is there.
    Player,
    /// Quick Look previews it (a document, a picture the app does not decode).
    QuickLook,
}

/// The hosting seam: what is asked of macOS. The app's is [`SystemHost`]; a test's remembers what it was asked.
pub(crate) trait Host {
    /// What shows this kind of file, by its name. `None`: macOS does not show it.
    fn shower_of(&self, name: &str) -> Option<Shower>;
    /// Whether AVFoundation plays this file (`AVURLAsset.isPlayable`).
    fn playable(&self, copy: &Path) -> bool;
    /// Shows the temp copy. `false`: it could not be shown.
    fn show(&mut self, copy: &Path) -> bool;
    /// Whether the copy is still on screen (the person did not close it).
    fn showing(&self, copy: &Path) -> bool;
    /// Takes the copy off the screen.
    fn close(&mut self, copy: &Path);
}

/// AVFoundation and the Quick Look panel (`shell::platform::macos_preview`).
pub(crate) struct SystemHost;

impl Host for SystemHost {
    fn shower_of(&self, name: &str) -> Option<Shower> {
        use crate::shell::platform::macos_preview as mac;
        if mac::plays_type(name) {
            Some(Shower::Player)
        } else if mac::quicklook_shows_type(name) {
            Some(Shower::QuickLook)
        } else {
            None
        }
    }

    fn playable(&self, copy: &Path) -> bool {
        crate::shell::platform::macos_preview::playable(copy)
    }

    fn show(&mut self, copy: &Path) -> bool {
        crate::shell::platform::macos_preview::show(copy)
    }

    fn showing(&self, copy: &Path) -> bool {
        crate::shell::platform::macos_preview::showing(copy)
    }

    fn close(&mut self, copy: &Path) {
        crate::shell::platform::macos_preview::close(copy);
    }
}

/// A test's source shows nothing through macOS unless the test puts a host of its own in.
#[cfg(test)]
struct NoHost;

#[cfg(test)]
impl Host for NoHost {
    fn shower_of(&self, _name: &str) -> Option<Shower> {
        None
    }
    fn playable(&self, _copy: &Path) -> bool {
        false
    }
    fn show(&mut self, _copy: &Path) -> bool {
        false
    }
    fn showing(&self, _copy: &Path) -> bool {
        false
    }
    fn close(&mut self, _copy: &Path) {}
}

/// The decrypted copy the system reads. Its drop removes it and its folder.
pub(crate) struct TempCopy {
    path: PathBuf,
}

impl TempCopy {
    /// The folder of `file_id`'s copy below `root`. The id is one path component, whatever it holds.
    fn folder(root: &Path, file_id: &str) -> PathBuf {
        let id: String = file_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        root.join(format!("{id}.preview"))
    }

    /// Removes every preview copy below `root`: what a run that ended without closing its viewer (a crash, a quit)
    /// left there. Only folders this module names (`*.preview`), never a copy Files' Open made.
    fn sweep(root: &Path) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "preview") && path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            }
        }
    }

    /// Writes `bytes` as `<root>/<id>.preview/<name>`. One copy at a time (the system has one panel): any copy an
    /// earlier run left below `root` is removed first.
    fn write(root: &Path, file_id: &str, name: &str, bytes: &[u8]) -> std::io::Result<TempCopy> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let folder = Self::folder(root, file_id);
        Self::sweep(root);
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&folder)?;
        // The name's last part only: the system reads the kind from its extension.
        let name = Path::new(name).file_name().unwrap_or_else(|| "file".as_ref());
        // From here on the drop removes what was made, also when the write fails.
        let copy = TempCopy {
            path: folder.join(name),
        };
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&copy.path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(copy)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        if let Some(folder) = self.path.parent() {
            let _ = std::fs::remove_dir(folder);
        }
    }
}

/// The worker's one answer: the copy and the session its bytes were read for.
type Answer = Result<(SessionGeneration, TempCopy), OpenFileError>;

/// The source's macOS part: the seam, where copies go, and the file shown now.
pub(crate) struct Native {
    pub(crate) host: Box<dyn Host>,
    /// Where copies are made (`<temp>/beebeeb-open`; a test's own folder).
    pub(crate) root: PathBuf,
    pub(crate) max: u64,
    on: Option<On>,
}

impl Default for Native {
    fn default() -> Self {
        #[cfg(not(test))]
        let host: Box<dyn Host> = Box::new(SystemHost);
        #[cfg(test)]
        let host: Box<dyn Host> = Box::new(NoHost);
        Native {
            host,
            root: std::env::temp_dir().join("beebeeb-open"),
            max: PREVIEW_MAX,
            on: None,
        }
    }
}

/// The file macOS shows, or is about to, and the session it was opened for.
struct On {
    file_id: String,
    session: SessionGeneration,
    account: Option<String>,
    shower: Shower,
    cancel: Arc<AtomicBool>,
    /// Bytes done and in all, as the read reports them.
    progress: Arc<(AtomicU64, AtomicU64)>,
    /// The copy's answer, while it is on its way.
    inbox: Option<mpsc::Receiver<Answer>>,
    drawn: (u64, u64),
    /// The copy on screen.
    copy: Option<TempCopy>,
}

impl Native {
    /// Nothing of the file stays: the read is cancelled, the panel lets go of the copy, the copy is removed.
    fn end(&mut self) {
        if let Some(mut on) = self.on.take() {
            on.cancel.store(true, Ordering::SeqCst);
            on.inbox = None;
            if let Some(copy) = on.copy.take() {
                self.host.close(copy.path());
            }
        }
    }

    /// The copy on screen now (a test reads it).
    #[cfg(test)]
    pub(crate) fn shown_copy(&self) -> Option<&Path> {
        self.on.as_ref().and_then(|on| on.copy.as_ref()).map(TempCopy::path)
    }
}

impl<E: ViewerEngine> EngineViewerSource<E> {
    /// What of macOS shows `name`. `None`: the app shows it itself, or macOS does not show it.
    pub(in crate::engine_host::viewer) fn mac_shower(&self, name: &str) -> Option<Shower> {
        if shows(name) != Shows::Nothing {
            return None;
        }
        self.media.mac.host.shower_of(name)
    }

    /// Starts showing `file_id` through macOS: the body shown now. `None`: not a kind macOS shows (nothing started).
    pub(in crate::engine_host::viewer) fn mac_begin(
        &mut self,
        session: SessionGeneration,
        account: Option<String>,
        file_id: &str,
        name: &str,
    ) -> Option<Body> {
        self.mac_leave();
        let shower = self.mac_shower(name)?;
        let facts = self.engine.facts(file_id).ok()?;
        let max = self.media.mac.max;
        // Before anything is read: nothing is downloaded for a file the copy's read would refuse.
        if facts.size > max {
            return Some(Body::NotPreviewable(NoPreview::TooBig {
                size: facts.size,
                limit: max,
            }));
        }
        let recorded = Recorded {
            here: facts.on_this_pc.is_some(),
            unsettled: facts.unsettled_here,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new((AtomicU64::new(0), AtomicU64::new(facts.size)));
        let (answers, inbox) = mpsc::channel();
        let started = {
            let (engine, file_id, name) = (self.engine.clone(), file_id.to_string(), name.to_string());
            let (cancel, progress, root) = (cancel.clone(), progress.clone(), self.media.mac.root.clone());
            std::thread::Builder::new()
                .name("viewer-mac-preview".into())
                .spawn(move || {
                    let answer: Answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let report = {
                            let (engine, progress) = (engine.clone(), progress.clone());
                            // The window is woken once per whole percent, as the viewer's own download wakes it.
                            let woken = AtomicU64::new(0);
                            move |done: u64, total: u64| {
                                progress.0.store(done, Ordering::SeqCst);
                                progress.1.store(total, Ordering::SeqCst);
                                let percent = if total == 0 {
                                    0
                                } else {
                                    done.saturating_mul(100) / total
                                };
                                if woken.swap(percent, Ordering::SeqCst) != percent {
                                    engine.wake();
                                }
                            }
                        };
                        let (session, bytes, _) = read(&*engine, &file_id, recorded, max, &report, &cancel)?;
                        if cancel.load(Ordering::SeqCst) {
                            return Err(OpenFileError::Cancelled);
                        }
                        TempCopy::write(&root, &file_id, &name, &bytes)
                            .map(|copy| (session, copy))
                            .map_err(|_| OpenFileError::Failed)
                    }))
                    .unwrap_or(Err(OpenFileError::Failed));
                    // Nobody waits any more: the copy is removed here (its drop), never queued.
                    let answer = if cancel.load(Ordering::SeqCst) {
                        drop(answer);
                        Err(OpenFileError::Cancelled)
                    } else {
                        answer
                    };
                    let _ = answers.send(answer);
                    engine.wake();
                })
                .is_ok()
        };
        if !started {
            log("viewer: could not start a thread for an open file");
            return Some(Body::Failed);
        }
        self.media.mac.on = Some(On {
            file_id: file_id.to_string(),
            session,
            account,
            shower,
            cancel,
            progress,
            inbox: Some(inbox),
            drawn: (0, facts.size),
            copy: None,
        });
        Some(Body::Decrypting {
            done: 0,
            total: facts.size,
        })
    }

    /// The poll's macOS arm. `None`: macOS shows no file of this id here (the rest of the poll runs).
    pub(in crate::engine_host::viewer) fn mac_poll(&mut self, file_id: &str) -> Option<Option<Body>> {
        let on = self.media.mac.on.as_ref().filter(|on| on.file_id == file_id)?;
        let engine = &self.engine;
        if !(engine.has_session() && engine.session_turn() == on.session && engine.account() == on.account) {
            // A lock, a sign-out, another account: the panel closes and the copy is removed NOW.
            self.media.mac.end();
            return Some(Some(Body::Stopped(Stopped::SessionChanged)));
        }
        // Cancel and Esc reach the opening (`ViewerSource::cancel`): the read stops, and a copy on screen goes.
        let cancelled = self
            .open
            .as_ref()
            .is_some_and(|opening| opening.file_id == file_id && opening.cancel.load(Ordering::SeqCst));
        if cancelled {
            let shown = on.copy.is_some();
            self.media.mac.end();
            return Some(shown.then_some(Body::NotPreviewable(NoPreview::NoViewer)));
        }
        let Some(inbox) = &on.inbox else {
            // Shown. The person closed the panel: the copy goes, and the page offers the other apps.
            let closed = on
                .copy
                .as_ref()
                .is_some_and(|copy| !self.media.mac.host.showing(copy.path()));
            if closed {
                self.media.mac.end();
                return Some(Some(Body::NotPreviewable(NoPreview::NoViewer)));
            }
            return Some(None);
        };
        let answer = match inbox.try_recv() {
            Ok(answer) => answer,
            Err(mpsc::TryRecvError::Empty) => {
                let now = (
                    on.progress.0.load(Ordering::SeqCst),
                    on.progress.1.load(Ordering::SeqCst),
                );
                let on = self.media.mac.on.as_mut()?;
                return Some((now != on.drawn).then(|| {
                    on.drawn = now;
                    Body::Decrypting {
                        done: now.0,
                        total: now.1,
                    }
                }));
            }
            // The worker ended without an answer.
            Err(mpsc::TryRecvError::Disconnected) => {
                self.media.mac.end();
                return Some(Some(Body::Failed));
            }
        };
        let (session, shower, max) = (on.session, on.shower, self.media.mac.max);
        let body = match answer {
            Ok((read_for, copy)) if read_for == session => {
                let mac = &mut self.media.mac;
                if shower == Shower::Player && !mac.host.playable(copy.path()) {
                    // Its kind plays here, this file does not: the page with the other apps. The copy is removed.
                    drop(copy);
                    mac.on = None;
                    Body::NotPreviewable(NoPreview::PlayerRefused)
                } else if mac.host.show(copy.path()) {
                    let size = std::fs::metadata(copy.path()).map_or(0, |meta| meta.len());
                    if let Some(on) = mac.on.as_mut() {
                        on.inbox = None;
                        on.copy = Some(copy);
                    }
                    // All of it is decrypted on this Mac; the panel shows it.
                    Body::Decrypting {
                        done: size,
                        total: size,
                    }
                } else {
                    log("viewer: macOS could not show the file");
                    drop(copy);
                    mac.on = None;
                    Body::NotPreviewable(NoPreview::NoViewer)
                }
            }
            // Read for another session than the one this file was opened for: never shown (the copy is removed).
            Ok(_) => {
                self.media.mac.end();
                Body::Stopped(Stopped::SessionChanged)
            }
            Err(error) => {
                self.media.mac.end();
                return Some(match error {
                    OpenFileError::TooLarge { size } => {
                        Some(Body::NotPreviewable(NoPreview::TooBig { size, limit: max }))
                    }
                    OpenFileError::OverLimit => Some(Body::NotPreviewable(NoPreview::OverLimit { limit: max })),
                    error => {
                        if error != OpenFileError::Cancelled {
                            log(format!("viewer: a file did not open ({})", error.kind()));
                        }
                        body_of(error)
                    }
                });
            }
        };
        Some(Some(body))
    }

    /// The file is left (closed, another file opened): the panel lets go of it and its copy is removed.
    pub(in crate::engine_host::viewer) fn mac_leave(&mut self) {
        self.media.mac.end();
    }
}

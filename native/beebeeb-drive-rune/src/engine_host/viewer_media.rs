//! The viewer plays. A list's Open of an audio or a video file shows it through the
//! shell's one `PlayerHandle` (`media::shell_player`): audio with the file's folder as the queue (DW-1503), a video as
//! `Body::Video`. Part of [`EngineViewerSource`] (`viewer.rs`), which hands `can_play`, `play`, `player`, `previews`,
//! `opens_here_when_kept` and the poll's media branch here.
//!
//! Session rule (the binding shape of `files_trash.rs` and `stamp.rs`): the press stamps the session and the account
//! the file was opened for, and that press goes WITH every item to the player (`MediaItem::press`); the player opens
//! each item for its own press only (`engine_host::media`), and an answer that comes back under another session or
//! account stops the player and is never shown as playing. A lock, a sign-out or another account end it on the
//! window's side too, without asking a worker (the shell's poll calls `media::stop_all`, and this poll says so to the
//! view).
//!
//! A view is of ONE file: the doc shows the player's state only while the player's current item is this file
//! (`PlayerHandle::state_of`); another recording's title, times or refusal are never this one's. A Play on a file the
//! player does not hold live (another file plays, or this one was refused) opens it again.
//!
//! Nothing is downloaded to open the view or to list the queue: the queue is a read of the local state (owner-checked,
//! task 2488); a track's bytes are read by the player when it plays.
//!
//! Must not: log a name, an id, a title or a byte; keep the queue after the file is left.

use beebeeb_engine::drive_v2::model::{self, Sort as EngineSort};
use beebeeb_media_protocol::KindHint;

use super::*;
use crate::frames::viewers::kinds::{AudioDoc, Track, VideoDoc};
use crate::media::StopReason;
use crate::media::player::{MediaItem, PlayerHandle, Press};

/// A folder's audio files as the Files listing sorts them, and the folder's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaQueue {
    pub folder: String,
    /// (id, file name), in the folder's sort order.
    pub tracks: Vec<(String, String)>,
}

/// What the viewer holds of the one recording or video it shows.
#[derive(Default)]
pub(super) struct Media {
    on: Option<Shown>,
}

struct Shown {
    file_id: String,
    name: String,
    kind: KindHint,
    /// The session and account stamped when the file was opened (the press).
    session: SessionGeneration,
    account: Option<String>,
    queue: Vec<(String, String)>,
    folder: String,
    /// The last audio body handed to the view, so the poll answers only a change.
    drawn: Option<AudioDoc>,
    /// What the view last drew of a video's player, so the poll answers only a change.
    drawn_video: Option<VideoDrawn>,
}

/// What a video's view draws of the player's state: the place in quarter seconds (the scrubber moves; the text says
/// seconds), the length, the button, the lines under the stage, and whether there is a frame to put on the stage.
#[derive(Clone, PartialEq, Eq)]
struct VideoDrawn {
    quarter: u64,
    length_ms: u64,
    playing: bool,
    no_sound: Option<crate::media::player::NoSound>,
    refused: Option<crate::media::player::Refusal>,
    framed: bool,
}

/// [`VideoDrawn`] of what the player says of the view's file (`None`: it holds nothing of it).
fn video_drawn(state: Option<&crate::media::player::PlayState>, framed: bool) -> Option<VideoDrawn> {
    state.map(|state| VideoDrawn {
        quarter: state.at_ms / 250,
        length_ms: state.length_ms,
        playing: state.playing,
        no_sound: state.no_sound,
        refused: state.refused.clone(),
        framed,
    })
}

/// Whether the platform has a sandbox for the helper and the helper is next to the app's exe.
pub(super) fn playback_ready() -> bool {
    cfg!(windows) && crate::media::transport::helper_exe().is_some()
}

/// The folder's audio files, read from the account's local state (the owner is checked by the handle; a read across
/// a lock or a sign-out is dropped). `None`: not readable now; the file then plays alone.
pub(super) fn queue_of<C: CredentialStores>(
    engine: &Engine<C>,
    session: SessionGeneration,
    file_id: &str,
) -> Option<MediaQueue> {
    let read = |db: &beebeeb_engine::state_db::StateDb| -> Option<MediaQueue> {
        let entry = model::resolve(db, file_id).ok()?;
        let parent = entry.parent_id.clone();
        let (rows, _) = model::children_of(db, parent.as_deref(), EngineSort::default(), 0, usize::MAX).ok()?;
        let folder = model::resolve_folder(db, parent.as_deref())
            .ok()
            .and_then(|folder| model::crumbs(db, folder.as_ref()).ok())
            .and_then(|crumbs| crumbs.last().map(|crumb| crumb.name.clone()))
            .unwrap_or_else(|| "Files".to_string());
        let tracks = audio_queue(rows.into_iter().map(|row| (row.id, row.name, row.is_folder)));
        Some(MediaQueue { folder, tracks })
    };
    let queue = engine.local_state().ok()?.with(read).ok()??;
    (engine.session_turn() == session).then_some(queue)
}

/// The audio files among a folder's rows (id, name, is a folder), in the order the rows come: the listing's.
pub(super) fn audio_queue(rows: impl IntoIterator<Item = (String, String, bool)>) -> Vec<(String, String)> {
    rows.into_iter()
        .filter(|(_, name, is_folder)| !is_folder && kinds::viewer_of(name) == Some(Viewer::Audio))
        .map(|(id, name, _)| (id, name))
        .collect()
}

/// The file's kind as the player takes it: `None` for anything else.
fn kind_hint(name: &str) -> Option<KindHint> {
    match kinds::viewer_of(name) {
        Some(Viewer::Audio) => Some(KindHint::Audio),
        Some(Viewer::Video) => Some(KindHint::Video),
        _ => None,
    }
}

fn seconds(ms: u64) -> u32 {
    u32::try_from(ms / 1000).unwrap_or(u32::MAX)
}

impl<E: ViewerEngine> EngineViewerSource<E> {
    /// Whether this source plays here: the helper is installed and the platform has a sandbox (D-MED-9: elsewhere the
    /// file opens in another app), and the shell has a player.
    pub(super) fn media_can_play(&self) -> bool {
        self.engine.media_ready() && self.engine.media_player().is_some()
    }

    /// Whether a list's Open of `name` shows it here: an audio or a video file, when this source plays.
    pub(super) fn media_previews(&self, name: &str) -> bool {
        kind_hint(name).is_some() && self.media_can_play()
    }

    /// The body an audio or video file opens as, stamping the session at the press. `None`: not a recording or video,
    /// or playback is not available here (the file then gets the state of a kind without a viewer).
    pub(super) fn media_body(
        &mut self,
        session: SessionGeneration,
        account: Option<String>,
        file_id: &str,
        name: &str,
        folders: &[String],
    ) -> Option<Body> {
        self.media_leave();
        let kind = kind_hint(name).filter(|_| self.media_can_play())?;
        let asked = (kind == KindHint::Audio)
            .then(|| self.engine.media_queue(session, file_id))
            .flatten();
        let (folder, queue) = match asked {
            Some(queue) if queue.tracks.iter().any(|(id, _)| id == file_id) => (queue.folder, queue.tracks),
            // The queue could not be read, or does not hold the file (it moved): the file plays alone.
            _ => (
                folders.last().cloned().unwrap_or_else(|| "Files".to_string()),
                if kind == KindHint::Audio {
                    vec![(file_id.to_string(), name.to_string())]
                } else {
                    Vec::new()
                },
            ),
        };
        let shown = Shown {
            file_id: file_id.to_string(),
            name: name.to_string(),
            kind,
            session,
            account,
            queue,
            folder,
            drawn: None,
            drawn_video: None,
        };
        let body = match kind {
            KindHint::Audio => Body::Audio(audio_doc(&shown, None)),
            KindHint::Video => Body::Video(VideoDoc { length: 0 }),
        };
        self.media.on = Some(shown);
        Some(body)
    }

    fn media_is_current(&self, shown: &Shown) -> bool {
        self.engine.has_session()
            && self.engine.session_turn() == shown.session
            && self.engine.account() == shown.account
    }

    /// Play or pause `file_id`. A play of a file the player does not hold live (the first, one after another file, or
    /// one after a refusal) opens it with the queue, every item carrying the press's session and account.
    pub(super) fn media_play(&mut self, file_id: &str, on: bool) -> bool {
        let Some(player) = self.engine.media_player() else {
            return false;
        };
        let Some(shown) = self.media.on.as_ref().filter(|shown| shown.file_id == file_id) else {
            return false;
        };
        if !self.media_is_current(shown) {
            player.stop(StopReason::AccountChanged);
            return false;
        }
        // A refusal keeps the file's state on screen but ends its run: that file is not held, so Play opens it anew.
        let holds_this = player.holds(file_id);
        if !on {
            if holds_this {
                player.pause();
            }
            return true;
        }
        if holds_this {
            player.play();
            return true;
        }
        // The press goes WITH every item (the queue's too): the player opens each for this session of this account.
        let press = Press {
            session: shown.session,
            account: shown.account.clone(),
        };
        let item = MediaItem {
            file_id: file_id.to_string(),
            kind: shown.kind,
            length_ms: 0,
            press: press.clone(),
        };
        let queue = shown
            .queue
            .iter()
            .map(|(id, _)| MediaItem {
                file_id: id.clone(),
                kind: KindHint::Audio,
                length_ms: 0,
                press: press.clone(),
            })
            .collect();
        let opened = player.open(item, queue).is_ok();
        // A cheap early exit for a session that ended BEFORE the press returned. This is NOT the guarantee: the
        // player opens on its worker thread, later, and the opener there opens only for the item's own press (see
        // `engine_host/media.rs`); a refusal reaches the view through the player's state.
        let still = self.media.on.as_ref().is_some_and(|shown| self.media_is_current(shown));
        if !still {
            player.stop(StopReason::AccountChanged);
            return false;
        }
        opened
    }

    /// What the player says changed since the view was last given a body: the tags, the length, the place, the queue's
    /// current track. `None`: nothing new, or `file_id` is not the recording on screen.
    pub(super) fn media_poll(&mut self, file_id: &str) -> Option<Body> {
        let shown = self.media.on.as_ref().filter(|shown| shown.file_id == file_id)?;
        if !self.media_is_current(shown) {
            let had = shown.drawn.as_ref().is_some_and(|doc| doc.playing);
            // Nothing played and the same account is back (a lock, then an unlock): the view is this session's now.
            let same_account = self.engine.has_session() && self.engine.account() == shown.account;
            if !had && same_account {
                let turn = self.engine.session_turn();
                if let Some(shown) = self.media.on.as_mut() {
                    shown.session = turn;
                }
                return None;
            }
            self.media.on = None;
            return had.then_some(Body::Stopped(Stopped::SessionChanged));
        }
        if shown.kind != KindHint::Audio {
            // A video's view reads the player itself (`viewers::video::view`); the poll says when to run it again:
            // the file opened, its first frame came (the stage has no picture before it), the place moved, it
            // paused, ended or was refused. Without this the view is run only by a press.
            let player = self.engine.media_player()?;
            let drawn = video_drawn(player.state_of(&shown.file_id).as_ref(), player.has_frame());
            let shown = self.media.on.as_mut()?;
            if shown.drawn_video == drawn {
                return None;
            }
            let length = drawn.as_ref().map_or(0, |drawn| seconds(drawn.length_ms));
            shown.drawn_video = drawn;
            return Some(Body::Video(VideoDoc { length }));
        }
        let state = self.engine.media_player()?.state_of(&shown.file_id);
        let doc = audio_doc(shown, state.as_ref());
        let shown = self.media.on.as_mut()?;
        if shown.drawn.as_ref() == Some(&doc) {
            return None;
        }
        shown.drawn = Some(doc.clone());
        Some(Body::Audio(doc))
    }

    /// The recording on screen is left: the view's queue goes; what plays keeps playing (Back leaves audio playing).
    pub(super) fn media_leave(&mut self) {
        self.media.on = None;
    }

    /// The view is closed (the source is dropped): a video stops with its view; audio keeps playing (Back leaves it).
    pub(super) fn media_closed(&mut self) {
        if let (Some(shown), Some(player)) = (self.media.on.take(), self.engine.media_player())
            && shown.kind == KindHint::Video
            && player.state().is_some_and(|state| state.file_id == shown.file_id)
        {
            player.stop(StopReason::Closed);
        }
    }

    /// The shell's player, when this source plays.
    pub(super) fn media_handle(&self) -> Option<PlayerHandle> {
        self.media_can_play().then(|| self.engine.media_player()).flatten()
    }
}

/// The view's recording from what the player says of THIS file (`None`: the player holds nothing of it, so the file's
/// own name stands in, not playing). Another file's state is never taken, even one of the same queue.
fn audio_doc(shown: &Shown, state: Option<&crate::media::player::PlayState>) -> AudioDoc {
    let state = state.filter(|state| state.file_id == shown.file_id);
    let current = shown.file_id.as_str();
    let name_of = |id: &str| {
        shown
            .queue
            .iter()
            .find(|(track, _)| track == id)
            .map_or(shown.name.as_str(), |(_, name)| name.as_str())
    };
    AudioDoc {
        // The tag's title, else the file name (the board).
        title: state
            .filter(|state| !state.title.is_empty())
            .map_or_else(|| name_of(current).to_string(), |state| state.title.clone()),
        artist: state.map_or_else(String::new, |state| state.artist.clone()),
        length: state.map_or(0, |state| seconds(state.length_ms)),
        at: state.map_or(0, |state| seconds(state.at_ms)),
        playing: state.is_some_and(|state| state.playing),
        folder: shown.folder.clone(),
        queue: shown
            .queue
            .iter()
            .map(|(id, name)| Track {
                id: id.clone(),
                title: name.clone(),
                length: 0,
                current: id == current,
            })
            .collect(),
    }
}

#[cfg(test)]
#[path = "media_tests.rs"]
mod tests;

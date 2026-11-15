//! Slice M7: `EngineViewerSource` playing, against an engine of the test's own (`World`), a real `PlayerHandle` and a
//! helper that is a fake (`FakeLink`). What is asked, for which session, and what a late answer becomes. Every name,
//! id and byte is invented. Run at the media checkpoint, not by the lane.

use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use beebeeb_engine::app::viewer_api::{LocalCopy, OpenedFile};
use beebeeb_media_protocol::{AudioInfo, Codec, FromHelper, Opened, ToHelper};
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::media::refusal_of;
use crate::media::audio_out::AudioError;
use crate::media::player::{AudioFactory, AudioSink, Link, LinkFactory, MediaBytes, MediaOpener, Phase, Refusal};
use crate::media::transport::LinkError;

const FOLDER: &str = "Invented demos";

struct Inner {
    turn: u64,
    signed_in: bool,
    account: Option<String>,
    ready: bool,
    /// The folder's rows in listing order: (id, name, is a folder).
    rows: Vec<(String, String, bool)>,
    /// Every press's stamp.
    presses: Vec<SessionGeneration>,
    /// The session ends while the player opens the file (an answer that comes back late).
    ends_on_open: bool,
    opened: Vec<String>,
}

struct World {
    s: Mutex<Inner>,
    player: Mutex<Option<PlayerHandle>>,
}

impl World {
    fn new(ready: bool) -> Arc<Self> {
        let rows = [
            ("t1", "invented-one.mp3", false),
            ("d1", "invented-notes.pdf", false),
            ("t2", "invented-two.flac", false),
            ("t3", "invented-three.m4a", false),
            ("v1", "invented-clip.mp4", false),
        ];
        let world = Arc::new(Self {
            s: Mutex::new(Inner {
                turn: 1,
                signed_in: true,
                account: Some("a@example.eu".into()),
                ready,
                rows: rows
                    .iter()
                    .map(|(id, name, dir)| (id.to_string(), name.to_string(), *dir))
                    .collect(),
                presses: Vec::new(),
                ends_on_open: false,
                opened: Vec::new(),
            }),
            player: Mutex::new(None),
        });
        let player = PlayerHandle::new(
            Arc::new(FakeOpener(world.clone())),
            Arc::new(FakeLinks),
            Arc::new(FakeAudio),
            None,
            crate::media::video::VideoSurface::new(),
        );
        *world.player.lock().unwrap_or_else(PoisonError::into_inner) = Some(player);
        world
    }

    fn s(&self) -> MutexGuard<'_, Inner> {
        self.s.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn handle(&self) -> PlayerHandle {
        self.player
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .expect("the player")
    }
}

impl ViewerEngine for World {
    fn session_turn(&self) -> SessionGeneration {
        SessionGeneration::for_test(self.s().turn)
    }

    fn has_session(&self) -> bool {
        self.s().signed_in
    }

    fn account(&self) -> Option<String> {
        self.s().account.clone()
    }

    fn facts(&self, file_id: &str) -> Result<OpenFileFacts, OpenFileError> {
        let s = self.s();
        let (_, name, _) = s
            .rows
            .iter()
            .find(|(id, _, _)| id == file_id)
            .ok_or(OpenFileError::NotFound)?;
        Ok(OpenFileFacts {
            session: SessionGeneration::for_test(s.turn),
            name: name.clone(),
            folders: vec![FOLDER.to_string()],
            size: 1000,
            on_this_pc: None,
            unsettled_here: false,
        })
    }

    fn fetch(
        &self,
        _file_id: &str,
        _max_bytes: u64,
        _progress: &(dyn Fn(u64, u64) + Send + Sync + 'static),
        _cancel: &AtomicBool,
    ) -> Result<OpenedFile, OpenFileError> {
        panic!("a recording or a video is never downloaded whole");
    }

    fn local(&self, _file_id: &str, _max_bytes: u64, _cancel: &AtomicBool) -> Result<LocalCopy, OpenFileError> {
        panic!("a recording or a video is never read whole");
    }

    fn is_syncing(&self) -> bool {
        true
    }

    fn open_elsewhere(&self, _file_id: &str) -> Elsewhere {
        Elsewhere::Opened
    }

    fn wake(&self) {}

    fn media_ready(&self) -> bool {
        self.s().ready
    }

    fn media_player(&self) -> Option<PlayerHandle> {
        Some(self.handle())
    }

    fn media_queue(&self, _session: SessionGeneration, _file_id: &str) -> Option<MediaQueue> {
        Some(MediaQueue {
            folder: FOLDER.to_string(),
            tracks: audio_queue(self.s().rows.clone()),
        })
    }

    fn media_press(&self, session: SessionGeneration) {
        self.s().presses.push(session);
    }
}

/// The engine's range reader as a double: the bytes are zeros. Opening ends the session when the test says so.
struct FakeOpener(Arc<World>);

struct Zeros;

impl MediaBytes for Zeros {
    fn len(&self) -> u64 {
        1000
    }

    fn read_at(&mut self, _offset: u64, len: u32, _cancel: &AtomicBool) -> Result<Zeroizing<Vec<u8>>, ()> {
        Ok(Zeroizing::new(vec![0; len as usize]))
    }
}

impl MediaOpener for FakeOpener {
    fn open(&self, file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        let mut s = self.0.s();
        s.opened.push(file_id.to_string());
        if s.ends_on_open {
            // As the real opener answers a session that ended: the session is gone, the engine refuses it.
            s.turn += 1;
            return Err(Refusal::NotSignedIn);
        }
        Ok(Box::new(Zeros))
    }
}

struct FakeLinks;

struct FakeLink {
    inbox: Vec<FromHelper>,
    /// Where the next answered PCM starts.
    pts_ms: u64,
}

impl LinkFactory for FakeLinks {
    fn start(&self) -> Result<Box<dyn Link>, Refusal> {
        Ok(Box::new(FakeLink {
            inbox: Vec::new(),
            pts_ms: 0,
        }))
    }
}

impl Link for FakeLink {
    fn send(&mut self, msg: &ToHelper) -> io::Result<()> {
        match msg {
            ToHelper::Open { .. } => self.inbox.push(FromHelper::Opened(Opened {
                duration_ms: 90_000,
                video: None,
                audio: Some(AudioInfo {
                    rate: 48_000,
                    channels: 2,
                    codec: Codec::Mp3,
                }),
                reduced_quality: false,
                no_sound: None,
            })),
            // Silence for every ask, as a helper that decodes: an unanswered ask ends the run after the player's
            // answer deadline as "could not be played", which a test must never read as a pause.
            ToHelper::WantAudio { rate, channels, frames } => {
                self.inbox.push(FromHelper::Audio {
                    pts_ms: self.pts_ms,
                    channels: *channels,
                    samples: vec![0.0; *frames as usize * usize::from(*channels)],
                });
                self.pts_ms += u64::from(*frames) * 1000 / u64::from((*rate).max(1));
            }
            _ => {}
        }
        Ok(())
    }

    fn recv(&mut self, _timeout: Duration) -> Result<FromHelper, LinkError> {
        if self.inbox.is_empty() {
            return Err(LinkError::Timeout);
        }
        Ok(self.inbox.remove(0))
    }

    fn kill(&mut self) {}
}

struct FakeAudio;

struct Silence;

impl AudioFactory for FakeAudio {
    fn start(&self, _rate_hint: u32) -> Result<Box<dyn AudioSink>, AudioError> {
        Ok(Box::new(Silence))
    }
}

impl AudioSink for Silence {
    fn format(&self) -> (u32, u16) {
        (48_000, 2)
    }

    fn push(&mut self, _pts_ms: u64, samples: &[f32]) -> usize {
        samples.len()
    }

    fn clock_ms(&self) -> Option<u64> {
        Some(0)
    }

    fn pause(&mut self) {}

    fn resume(&mut self) {}

    fn flush(&mut self) {}
}

/// Polls every 10 ms; gives up after 2 s with `what` in the panic. The player opens on its own worker thread.
fn wait_until(what: &str, cond: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !cond() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn source(world: &Arc<World>) -> EngineViewerSource<World> {
    EngineViewerSource::on(world.clone())
}

#[test]
fn audio_open_plays_through_the_shell_player() {
    let world = World::new(true);
    let mut source = source(&world);
    assert!(
        source.can_play(),
        "playback is available: the helper is there and the platform has a sandbox"
    );
    assert!(source.player().is_some(), "the view gets the shell's player");
    let file = source.open("t1").expect("opens");
    assert!(
        matches!(file.body, Body::Audio(_)),
        "an audio file opens as a recording, not as 'no preview'"
    );
    assert!(source.play("t1", true), "the first play opens it in the player");
    wait_until("the engine opens the pressed file", || {
        world.s().opened == vec!["t1".to_string()]
    });
    assert_eq!(
        world.s().opened,
        vec!["t1".to_string()],
        "the engine opened the pressed file"
    );
    // `open` shows the file as playing at once; the worker's own word comes after the helper's `Opened`.
    wait_until("the player's worker plays the file", || {
        world.handle().phase() == Phase::Playing
    });
    let state = world.handle().state().expect("the player holds the file");
    assert_eq!(state.file_id, "t1");
    assert!(state.playing, "the player plays it");
    assert_eq!(state.refused, None);
    assert_eq!(
        world.s().presses,
        vec![SessionGeneration::for_test(1)],
        "the press stamped the session the file was opened for"
    );
    assert!(source.play("t1", false), "pause");
    // The pause is a command to the player's worker, applied at its next tick (`PlayerHandle::pause`): wait for it.
    wait_until("the pause lands in the player", || {
        world.handle().phase() == Phase::Paused
    });
    let state = world.handle().state().expect("a pause keeps the file");
    assert_eq!(state.file_id, "t1");
    assert!(!state.playing, "the player is paused");
    assert_eq!(state.refused, None, "paused by the press, not ended by a refusal");
}

#[test]
fn late_open_under_another_session_is_dropped() {
    let world = World::new(true);
    let mut source = source(&world);
    source.open("t1").expect("opens");
    // The session ends while the player opens the file: the answer comes back under another session.
    world.s().ends_on_open = true;
    // The press returns before the worker opens the file, so what it answers is not the property: the state is.
    source.play("t1", true);
    // The open happens on the player's worker: wait for it, then for the refusal to land in the player's state.
    wait_until("the opener was asked", || !world.s().opened.is_empty());
    wait_until("the player settled", || {
        world
            .handle()
            .state()
            .is_none_or(|state| state.refused.is_some() || !state.playing)
    });
    let state = world.handle().state();
    assert!(
        state.is_none() || state.as_ref().is_some_and(|s| s.refused == Some(Refusal::NotSignedIn)),
        "the late open is refused as not signed in, never played"
    );
    // The poll of the same view says so honestly, never "playing".
    let after = source.poll("t1");
    assert!(
        !matches!(after, Some(Body::Audio(doc)) if doc.playing),
        "a recording of another session is never drawn as playing"
    );
    // The same account is still signed in (a lock, then an unlock): that poll gave the view to the new session, so
    // the next one draws the recording from the player's state. The late open is never drawn as playing there either.
    let Some(Body::Audio(doc)) = source.poll("t1") else {
        panic!("the same account's view draws the recording again");
    };
    assert!(!doc.playing, "a late open is never drawn as playing");
    assert_eq!(
        doc.title, "invented-one.mp3",
        "nothing of the refused open is shown as its tags"
    );
}

#[test]
fn engine_refusals_map_to_the_players_words() {
    assert_eq!(refusal_of(OpenFileError::NotSignedIn), Refusal::NotSignedIn);
    assert_eq!(refusal_of(OpenFileError::SessionEnded), Refusal::NotSignedIn);
    assert_eq!(refusal_of(OpenFileError::NotYours), Refusal::NotSignedIn);
    assert_eq!(refusal_of(OpenFileError::NotFound), Refusal::CouldNotPlay);
    assert_eq!(refusal_of(OpenFileError::NothingSynced), Refusal::CouldNotPlay);
}

#[test]
fn queue_is_the_folders_audio_in_its_sort_order() {
    // The rows as the Files listing hands them (`model::children_of`, the same read and sort): 3 audio and a PDF.
    let rows = vec![
        ("a2".to_string(), "b-two.mp3".to_string(), false),
        ("p1".to_string(), "a-notes.pdf".to_string(), false),
        ("a1".to_string(), "c-one.flac".to_string(), false),
        ("a3".to_string(), "d-three.m4a".to_string(), false),
        ("f1".to_string(), "e-folder.mp3".to_string(), true),
    ];
    let queue = audio_queue(rows);
    assert_eq!(
        queue,
        vec![
            ("a2".to_string(), "b-two.mp3".to_string()),
            ("a1".to_string(), "c-one.flac".to_string()),
            ("a3".to_string(), "d-three.m4a".to_string()),
        ],
        "3 audio files in the listing's order; the PDF and a folder named like audio are not tracks"
    );
    // And in the view: the queue the open recording carries, the open file marked.
    let world = World::new(true);
    let mut source = source(&world);
    let Body::Audio(doc) = source.open("t2").expect("opens").body else {
        panic!("a recording");
    };
    let ids: Vec<&str> = doc.queue.iter().map(|track| track.id.as_str()).collect();
    assert_eq!(ids, ["t1", "t2", "t3"], "3 tracks, the folder's audio only");
    assert_eq!(doc.queue.iter().filter(|track| track.current).count(), 1);
    assert!(doc.queue[1].current, "the open file is the current track");
    assert_eq!(doc.folder, FOLDER);
    assert_eq!(doc.title, "invented-two.flac", "before the tags, the file's name");
}

#[test]
fn can_play_false_without_sandbox_or_helper() {
    let world = World::new(false);
    let mut source = source(&world);
    assert!(!source.can_play());
    assert!(source.player().is_none());
    assert!(
        !source.previews("a.mp3") && !source.previews("a.mp4"),
        "no player: a list's Open leaves them to another app"
    );
    assert!(!source.play("t1", true));
    // A route that names one anyway gets the honest state; nothing is downloaded or played.
    let file = source.open("t1").expect("opens");
    assert!(matches!(file.body, Body::NotPreviewable(NoPreview::NoViewer)));
    // The real check: no sandbox for the helper off Windows (D-MED-9), and no helper next to the test's exe.
    assert!(!playback_ready(), "the test binary has no helper next to it");
}

#[test]
fn leaving_the_view_keeps_audio_playing() {
    let world = World::new(true);
    let mut source = source(&world);
    source.open("t1").expect("opens");
    assert!(source.play("t1", true));
    drop(source);
    assert!(
        world.handle().state().is_some_and(|state| state.playing),
        "Back leaves the audio playing"
    );
}

#[test]
fn closing_a_video_stops_it() {
    let world = World::new(true);
    let mut source = source(&world);
    let file = source.open("v1").expect("opens");
    assert!(matches!(file.body, Body::Video(_)), "a video opens as a video");
    assert!(source.play("v1", true));
    assert!(world.handle().state().is_some(), "it plays");
    drop(source);
    assert!(world.handle().state().is_none(), "closing the video stops it");
}

#[test]
fn kept_audio_opens_in_the_player_when_playback_is_available() {
    let ready = source(&World::new(true));
    assert!(ready.opens_here_when_kept("a.mp3") && ready.opens_here_when_kept("b.mp4"));
    assert!(ready.previews("a.mp3") && ready.previews("b.mp4"));
    assert!(!ready.opens_here_when_kept("c.pdf") && !ready.opens_here_when_kept("d.txt"));
    let not_ready = source(&World::new(false));
    assert!(!not_ready.opens_here_when_kept("a.mp3") && !not_ready.opens_here_when_kept("b.mp4"));
}

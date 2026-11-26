//! dm-70 on a Mac: which kinds macOS shows, the temp copy the hosting seam is handed, and when that copy goes. The
//! engine is a double (no network, invented names and bytes), the seam a fake that remembers what it was asked; the
//! type questions are asked of the real system (AVFoundation, the type registry), which answer without a window.

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use beebeeb_engine::app::viewer_api::{LocalCopy, OpenFileFacts, OpenedFile};
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::viewer::viewer_media::playback_ready_for;
use crate::frames::viewers::source::{Elsewhere, ViewerSource};

const FILES: [(&str, &str, &[u8]); 4] = [
    ("v1", "invented-clip.mov", b"invented video bytes"),
    ("d1", "invented-letter.docx", b"invented document bytes"),
    ("x1", "invented-thing.xyz", b"invented unknown bytes"),
    ("t1", "invented-notes.txt", b"invented text"),
];

/// The engine: four files, a session that can end, and a count of what was downloaded.
struct World {
    turn: AtomicU64,
    fetched: Mutex<Vec<String>>,
}

impl World {
    fn new() -> Arc<Self> {
        Arc::new(World {
            turn: AtomicU64::new(1),
            fetched: Mutex::new(Vec::new()),
        })
    }

    fn fetched(&self) -> Vec<String> {
        self.fetched.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl ViewerEngine for World {
    fn session_turn(&self) -> SessionGeneration {
        SessionGeneration::for_test(self.turn.load(Ordering::SeqCst))
    }

    fn has_session(&self) -> bool {
        true
    }

    fn account(&self) -> Option<String> {
        Some("a@beebeeb.io".into())
    }

    fn facts(&self, file_id: &str) -> Result<OpenFileFacts, OpenFileError> {
        let (_, name, bytes) = FILES
            .iter()
            .find(|(id, _, _)| *id == file_id)
            .ok_or(OpenFileError::NotFound)?;
        Ok(OpenFileFacts {
            session: self.session_turn(),
            name: (*name).to_string(),
            folders: vec!["Invented".to_string()],
            size: bytes.len() as u64,
            on_this_pc: None,
            unsettled_here: false,
        })
    }

    fn fetch(
        &self,
        file_id: &str,
        _max_bytes: u64,
        progress: &(dyn Fn(u64, u64) + Send + Sync + 'static),
        _cancel: &AtomicBool,
    ) -> Result<OpenedFile, OpenFileError> {
        let (_, _, bytes) = FILES
            .iter()
            .find(|(id, _, _)| *id == file_id)
            .ok_or(OpenFileError::NotFound)?;
        self.fetched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(file_id.to_string());
        progress(bytes.len() as u64, bytes.len() as u64);
        Ok(OpenedFile {
            session: self.session_turn(),
            bytes: Zeroizing::new(bytes.to_vec()),
        })
    }

    fn local(&self, _file_id: &str, _max_bytes: u64, _cancel: &AtomicBool) -> Result<LocalCopy, OpenFileError> {
        Ok(LocalCopy::NotHere)
    }

    fn is_syncing(&self) -> bool {
        true
    }

    fn open_elsewhere(&self, _file_id: &str) -> Elsewhere {
        Elsewhere::Opened
    }

    fn wake(&self) {}
}

/// What the seam was asked.
#[derive(Default)]
struct Seen {
    /// Each copy shown, with the bytes that were in it when it was shown.
    shown: Vec<(PathBuf, Vec<u8>)>,
    closed: Vec<PathBuf>,
    asked_playable: Vec<PathBuf>,
    /// The person closed the panel.
    panel_closed: bool,
    /// AVFoundation's answer for the file itself.
    not_playable: bool,
}

/// The seam as a fake: a .mov is the player's, a .docx (and, to prove the app's own viewers come first, a .txt) is
/// Quick Look's, nothing else is shown.
struct FakeHost(Arc<Mutex<Seen>>);

impl FakeHost {
    fn seen(&self) -> MutexGuard<'_, Seen> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Host for FakeHost {
    fn shower_of(&self, name: &str) -> Option<Shower> {
        match name.rsplit_once('.')?.1 {
            "mov" => Some(Shower::Player),
            "docx" | "txt" => Some(Shower::QuickLook),
            _ => None,
        }
    }

    fn playable(&self, copy: &Path) -> bool {
        let mut seen = self.seen();
        seen.asked_playable.push(copy.to_path_buf());
        !seen.not_playable
    }

    fn show(&mut self, copy: &Path) -> bool {
        let bytes = std::fs::read(copy).expect("the copy is there when it is shown");
        self.seen().shown.push((copy.to_path_buf(), bytes));
        true
    }

    fn showing(&self, _copy: &Path) -> bool {
        !self.seen().panel_closed
    }

    fn close(&mut self, copy: &Path) {
        self.seen().closed.push(copy.to_path_buf());
    }
}

struct Rig {
    world: Arc<World>,
    seen: Arc<Mutex<Seen>>,
    root: tempfile::TempDir,
    source: EngineViewerSource<World>,
}

fn rig() -> Rig {
    let world = World::new();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let root = tempfile::tempdir().expect("a folder for the copies");
    let mut source = EngineViewerSource::on(world.clone());
    source.media.mac.host = Box::new(FakeHost(seen.clone()));
    source.media.mac.root = root.path().join("beebeeb-open");
    // As on a Mac: no Office helper beside the app.
    source.office.helper = || None;
    Rig {
        world,
        seen,
        root,
        source,
    }
}

impl Rig {
    fn seen(&self) -> MutexGuard<'_, Seen> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Polls until the body is not the copy's progress any more, or the copy is on screen.
    fn settle(&mut self, file_id: &str) -> Option<Body> {
        let until = Instant::now() + Duration::from_secs(10);
        let mut last = None;
        while Instant::now() < until {
            if let Some(body) = self.source.poll(file_id) {
                let waiting = matches!(body, Body::Decrypting { .. }) && self.seen().shown.is_empty();
                last = Some(body);
                if !waiting {
                    return last;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        last
    }

    /// Whatever is below the copies' folder now.
    fn left(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(self.root.path().join("beebeeb-open")) else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect()
    }
}

#[test]
fn playback_ready_is_true_for_a_mov_fixture_and_false_for_xyz() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../beebeeb-media-helper/tests/fixtures/hevc-main10.mov");
    assert!(playback_ready_for(&fixture), "AVFoundation plays the .mov fixture");
    let dir = tempfile::tempdir().unwrap();
    let unknown = dir.path().join("invented.xyz");
    std::fs::write(&unknown, b"invented bytes, no media in them").unwrap();
    assert!(!playback_ready_for(&unknown));
    // By the name alone, before anything is downloaded: the real system's answer.
    assert_eq!(SystemHost.shower_of("IMG_0001.MOV"), Some(Shower::Player));
    assert_eq!(SystemHost.shower_of("invented.xyz"), None);
}

#[test]
fn the_office_path_chooses_quick_look_for_a_docx() {
    // The real system's answer for the type: Quick Look, not the player.
    assert_eq!(SystemHost.shower_of("Dit is een test.docx"), Some(Shower::QuickLook));
    assert_eq!(SystemHost.shower_of("deck.pptx"), Some(Shower::QuickLook));
    // No PostScript converter on this macOS: an EPS keeps the page with the other apps.
    assert_eq!(SystemHost.shower_of("logo.eps"), None);

    let mut rig = rig();
    assert!(rig.source.previews("invented-letter.docx"));
    let file = rig.source.open("d1").expect("opens");
    assert!(matches!(file.body, Body::Decrypting { .. }), "{:?}", file.body);
    let body = rig.settle("d1");
    assert!(
        matches!(body, Some(Body::Decrypting { done, total }) if done == total),
        "{body:?}"
    );
    let seen = rig.seen();
    assert_eq!(seen.shown.len(), 1, "Quick Look was handed the document once");
    assert_eq!(seen.shown[0].0.file_name().unwrap(), "invented-letter.docx");
    assert_eq!(seen.shown[0].1, b"invented document bytes");
    assert!(seen.asked_playable.is_empty(), "a document is not asked of the player");
}

#[test]
fn the_seam_gets_the_temp_copys_url_and_the_copy_goes_when_the_viewer_closes() {
    let mut rig = rig();
    assert!(rig.source.previews("invented-clip.mov"));
    let file = rig.source.open("v1").expect("opens");
    assert!(matches!(file.body, Body::Decrypting { .. }), "{:?}", file.body);
    rig.settle("v1");
    let copy = rig
        .source
        .media
        .mac
        .shown_copy()
        .expect("the copy is on screen")
        .to_path_buf();
    {
        let seen = rig.seen();
        assert_eq!(seen.shown.len(), 1);
        assert_eq!(seen.shown[0].0, copy, "the seam was called with the temp copy's path");
        assert_eq!(seen.shown[0].1, b"invented video bytes");
        assert_eq!(
            seen.asked_playable,
            vec![copy.clone()],
            "the file itself was asked of AVFoundation"
        );
    }
    assert!(copy.starts_with(rig.root.path().join("beebeeb-open")));
    assert_eq!(copy.file_name().unwrap(), "invented-clip.mov");
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&copy), 0o600);
        assert_eq!(mode(copy.parent().unwrap()), 0o700);
    }
    // Nothing new while it is shown.
    assert!(rig.source.poll("v1").is_none());

    // The viewer closes: the panel lets go and the copy is removed.
    let Rig { seen, root, source, .. } = rig;
    drop(source);
    assert!(!copy.exists(), "the temp copy is removed when the viewer closes");
    assert!(!copy.parent().unwrap().exists(), "and so is its folder");
    assert_eq!(seen.lock().unwrap().closed, vec![copy]);
    drop(root);
}

#[test]
fn a_file_avfoundation_does_not_play_gets_the_page_and_no_copy_stays() {
    let mut rig = rig();
    rig.seen().not_playable = true;
    rig.source.open("v1").expect("opens");
    let body = rig.settle("v1");
    assert!(
        matches!(body, Some(Body::NotPreviewable(NoPreview::PlayerRefused))),
        "{body:?}"
    );
    assert!(rig.seen().shown.is_empty());
    assert!(rig.left().is_empty(), "the copy is removed: {:?}", rig.left());
}

#[test]
fn a_lock_closes_the_panel_and_removes_the_copy() {
    let mut rig = rig();
    rig.source.open("d1").expect("opens");
    rig.settle("d1");
    let copy = rig.source.media.mac.shown_copy().expect("shown").to_path_buf();
    // The session moves on (a lock, a sign-out).
    rig.world.turn.store(2, Ordering::SeqCst);
    let body = rig.source.poll("d1");
    assert!(matches!(body, Some(Body::Stopped(Stopped::SessionChanged))), "{body:?}");
    assert!(!copy.exists());
    assert_eq!(rig.seen().closed, vec![copy]);
}

#[test]
fn closing_the_panel_removes_the_copy_and_shows_the_other_apps() {
    let mut rig = rig();
    rig.source.open("d1").expect("opens");
    rig.settle("d1");
    let copy = rig.source.media.mac.shown_copy().expect("shown").to_path_buf();
    rig.seen().panel_closed = true;
    let body = rig.source.poll("d1");
    assert!(
        matches!(body, Some(Body::NotPreviewable(NoPreview::NoViewer))),
        "{body:?}"
    );
    assert!(!copy.exists());
    assert!(rig.left().is_empty());
}

#[test]
fn another_file_takes_the_place_of_the_one_shown() {
    let mut rig = rig();
    rig.source.open("d1").expect("opens");
    rig.settle("d1");
    let first = rig.source.media.mac.shown_copy().expect("shown").to_path_buf();
    rig.source.open("v1").expect("opens");
    assert!(!first.exists(), "the first file's copy went when the second opened");
    rig.settle("v1");
    assert_eq!(rig.left().len(), 1, "one copy at a time: {:?}", rig.left());
}

#[test]
fn only_what_macos_shows_is_shown_and_the_apps_own_viewers_come_first() {
    let mut rig = rig();
    // No viewer and macOS does not show it: the page with the other apps, and nothing is downloaded.
    assert!(!rig.source.previews("invented-thing.xyz"));
    let file = rig.source.open("x1").expect("opens");
    assert!(
        matches!(file.body, Body::NotPreviewable(NoPreview::NoViewer)),
        "{:?}",
        file.body
    );
    assert!(rig.world.fetched().is_empty());
    assert!(rig.left().is_empty());
    // A text is the app's own viewer's, whatever Quick Look could do with it.
    assert_eq!(rig.source.mac_shower("invented-notes.txt"), None);
    assert_eq!(rig.source.mac_shower("invented-clip.mov"), Some(Shower::Player));
    rig.source.open("t1").expect("opens");
    let until = Instant::now() + Duration::from_secs(10);
    let mut body = None;
    while body.is_none() && Instant::now() < until {
        body = rig
            .source
            .poll("t1")
            .filter(|body| !matches!(body, Body::Decrypting { .. }));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(body, Some(Body::Text(_))), "{body:?}");
    assert!(
        rig.seen().shown.is_empty(),
        "nothing was written to disk for the app's own viewer"
    );
    assert!(rig.left().is_empty());
}

#[test]
fn a_file_over_the_limit_is_not_downloaded() {
    let mut rig = rig();
    rig.source.media.mac.max = 4;
    let file = rig.source.open("v1").expect("opens");
    assert!(
        matches!(file.body, Body::NotPreviewable(NoPreview::TooBig { limit: 4, .. })),
        "{:?}",
        file.body
    );
    assert!(rig.world.fetched().is_empty());
    assert!(rig.left().is_empty());
}

#[test]
fn a_copy_nobody_takes_is_removed() {
    let dir = tempfile::tempdir().unwrap();
    let copy = TempCopy::write(dir.path(), "../odd id", "../../invented name.mov", b"bytes").expect("written");
    let path = copy.path().to_path_buf();
    assert!(
        path.starts_with(dir.path()),
        "the id and the name are one path part each: {path:?}"
    );
    assert_eq!(path.file_name().unwrap(), "invented name.mov");
    assert!(path.is_file());
    drop(copy);
    assert!(!path.exists() && !path.parent().unwrap().exists());
}

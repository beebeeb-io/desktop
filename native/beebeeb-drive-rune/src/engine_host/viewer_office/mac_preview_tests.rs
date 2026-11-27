//! dm-70 and dm-74 on a Mac: which kinds macOS shows, the file the hosting seam is handed (Finder's own copy by its
//! place, else a temp copy), the view inside the window and where it is put, and when all of it goes. The engine is
//! a double (no network, invented names and bytes), the seams are fakes that remember what they were asked (the
//! host: views, places, the panel; Finder: a folder of the test's own); the type questions are asked of the real
//! system (AVFoundation, the type registry), which answer without a window.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use beebeeb_engine::app::viewer_api::{LocalCopy, OpenFileFacts, OpenedFile};
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::office_host::EditKind;
use crate::engine_host::viewer::viewer_media::playback_ready_for;
use crate::frames::viewers::source::{Elsewhere, ViewerSource};

const FILES: [(&str, &str, &[u8]); 4] = [
    ("v1", "invented-clip.mov", b"invented video bytes"),
    ("d1", "invented-letter.docx", b"invented document bytes"),
    ("x1", "invented-thing.xyz", b"invented unknown bytes"),
    ("t1", "invented-notes.txt", b"invented text"),
];

/// What Finder's copy of a file holds in these tests: not what Beebeeb would decrypt, so a test can tell them apart.
const FINDERS: &[u8] = b"the bytes in Finder's own copy";

/// The engine: four files, a session that can end, a count of what was downloaded, and what it says of Finder.
struct World {
    turn: AtomicU64,
    fetched: Mutex<Vec<String>>,
    /// Beebeeb is in Finder and the files are the account's own.
    in_finder: AtomicBool,
    /// The transfer board's row for Finder's download moves (a byte more at every look).
    moving: AtomicBool,
    looks: AtomicU64,
    /// The account may change the files (dm-80's editor route). Off: no edit facts, as before.
    writable: AtomicBool,
}

impl World {
    fn new() -> Arc<Self> {
        Arc::new(World {
            turn: AtomicU64::new(1),
            fetched: Mutex::new(Vec::new()),
            in_finder: AtomicBool::new(false),
            moving: AtomicBool::new(true),
            looks: AtomicU64::new(0),
            writable: AtomicBool::new(false),
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

    fn in_finder(&self, _file_id: &str) -> bool {
        self.in_finder.load(Ordering::SeqCst)
    }

    fn edit_facts(
        &self,
        _session: SessionGeneration,
        _file_id: &str,
        _opening: u64,
    ) -> Option<beebeeb_engine::app::viewer_edit_api::EditFacts> {
        self.writable
            .load(Ordering::SeqCst)
            .then_some(beebeeb_engine::app::viewer_edit_api::EditFacts {
                version: 3,
                kept: false,
                can_write: true,
                own: false,
            })
    }

    fn download_progress(&self, _session: SessionGeneration, _file_id: &str) -> Option<(u64, u64)> {
        let looks = self.looks.fetch_add(1, Ordering::SeqCst) + 1;
        Some((if self.moving.load(Ordering::SeqCst) { looks } else { 7 }, 20))
    }
}

/// What the hosting seam was asked.
#[derive(Default)]
struct Seen {
    /// Each view hosted in the window: which, the file, and the bytes that were in the file then.
    embedded: Vec<(Shower, PathBuf, Vec<u8>)>,
    /// Where the view was put, in order (`None`: hidden).
    placed: Vec<(PathBuf, Option<[f32; 4]>)>,
    /// Each file shown in the panel, with its bytes.
    panel: Vec<(PathBuf, Vec<u8>)>,
    closed: Vec<PathBuf>,
    asked_playable: Vec<PathBuf>,
    /// The file is no longer on screen (the person closed the panel).
    off_screen: bool,
    /// AVFoundation's answer for the file itself.
    not_playable: bool,
    /// No view can be hosted in the window.
    no_view: bool,
    /// The panel does not open either.
    no_panel: bool,
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

    fn playable(&self, file: &Path) -> bool {
        let mut seen = self.seen();
        seen.asked_playable.push(file.to_path_buf());
        !seen.not_playable
    }

    fn embed(&mut self, shower: Shower, file: &Path) -> bool {
        if self.seen().no_view {
            return false;
        }
        let bytes = std::fs::read(file).expect("the file is there when its view is made");
        self.seen().embedded.push((shower, file.to_path_buf(), bytes));
        true
    }

    fn place(&mut self, file: &Path, rect: Option<[f32; 4]>) {
        self.seen().placed.push((file.to_path_buf(), rect));
    }

    fn show(&mut self, file: &Path) -> bool {
        if self.seen().no_panel {
            return false;
        }
        let bytes = std::fs::read(file).expect("the file is there when it is shown");
        self.seen().panel.push((file.to_path_buf(), bytes));
        true
    }

    fn showing(&self, _file: &Path) -> bool {
        !self.seen().off_screen
    }

    fn close(&mut self, file: &Path) {
        self.seen().closed.push(file.to_path_buf());
    }
}

/// Finder as a folder of the test's own: a place per file, some of them without their bytes.
#[derive(Default)]
struct FakeFinder {
    places: Mutex<HashMap<String, PathBuf>>,
    /// The places whose bytes are not on this Mac.
    cloud: Mutex<HashSet<PathBuf>>,
    requested: Mutex<Vec<String>>,
    /// The system refuses the download.
    refuses: AtomicBool,
    /// The download never ends.
    never_arrives: AtomicBool,
    /// How often a requested file was looked at since.
    looked: AtomicUsize,
    /// The read calls a thread is inside of now (each holds the scope of its file's place).
    inside: AtomicUsize,
}

impl FakeFinder {
    fn requested(&self) -> Vec<String> {
        self.requested.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn inside(&self) -> usize {
        self.inside.load(Ordering::SeqCst)
    }
}

impl Finder for FakeFinder {
    fn with_copy(&self, file_id: &str, read: &mut dyn FnMut(&Path)) -> bool {
        let place = self
            .places
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(file_id)
            .cloned();
        let Some(place) = place else {
            return false;
        };
        self.inside.fetch_add(1, Ordering::SeqCst);
        read(&place);
        self.inside.fetch_sub(1, Ordering::SeqCst);
        true
    }

    fn presence(&self, file: &Path) -> Presence {
        let mut cloud = self.cloud.lock().unwrap_or_else(PoisonError::into_inner);
        if !cloud.contains(file) {
            return presence_on_disk(file);
        }
        let asked = !self.requested().is_empty();
        // The system's download takes a few looks.
        if asked && !self.never_arrives.load(Ordering::SeqCst) && self.looked.fetch_add(1, Ordering::SeqCst) >= 3 {
            cloud.remove(file);
            return Presence::Here;
        }
        Presence::Cloud
    }

    fn request(&self, file_id: &str) -> Result<(), String> {
        if self.refuses.load(Ordering::SeqCst) {
            return Err("NSFileProviderErrorDomain -1005".into());
        }
        self.requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(file_id.to_string());
        Ok(())
    }
}

struct Rig {
    world: Arc<World>,
    seen: Arc<Mutex<Seen>>,
    finder: Arc<FakeFinder>,
    root: tempfile::TempDir,
    source: EngineViewerSource<World>,
}

fn rig() -> Rig {
    let world = World::new();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let finder = Arc::new(FakeFinder::default());
    let root = tempfile::tempdir().expect("a folder for the copies");
    let mut source = EngineViewerSource::on(world.clone());
    let host: Box<dyn Host> = Box::new(FakeHost(seen.clone()));
    source.media.mac.host = Arc::new(Mutex::new(host));
    source.media.mac.finder = finder.clone();
    source.media.mac.root = root.path().join("beebeeb-open");
    // As on a Mac: no Office helper beside the app.
    source.office.helper = || None;
    Rig {
        world,
        seen,
        finder,
        root,
        source,
    }
}

impl Rig {
    fn seen(&self) -> MutexGuard<'_, Seen> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Beebeeb is in Finder, and Finder has a place for `file_id` (`cloud`: without its bytes on this Mac).
    fn in_finder(&self, file_id: &str, cloud: bool) -> PathBuf {
        self.world.in_finder.store(true, Ordering::SeqCst);
        let (_, name, _) = FILES
            .iter()
            .find(|(id, _, _)| *id == file_id)
            .expect("a file of the test");
        let folder = self.root.path().join("Finder");
        std::fs::create_dir_all(&folder).unwrap();
        let place = folder.join(name);
        std::fs::write(&place, FINDERS).unwrap();
        self.finder
            .places
            .lock()
            .unwrap()
            .insert(file_id.to_string(), place.clone());
        if cloud {
            self.finder.cloud.lock().unwrap().insert(place.clone());
        }
        place
    }

    /// Polls until the body is not the progress any more. The last body that came.
    fn settle(&mut self, file_id: &str) -> Option<Body> {
        let until = Instant::now() + Duration::from_secs(10);
        let mut last = None;
        while Instant::now() < until {
            if let Some(body) = self.source.poll(file_id) {
                let waiting = matches!(body, Body::Decrypting { .. });
                last = Some(body);
                if !waiting {
                    return last;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        last
    }

    /// Waits until no thread holds a place in Finder any more.
    fn let_go(&self) -> bool {
        let until = Instant::now() + Duration::from_secs(10);
        while self.finder.inside() > 0 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.finder.inside() == 0
    }

    /// Whatever is below the temp copies' folder now.
    fn left(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(self.root.path().join("beebeeb-open")) else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect()
    }
}

/// The stage of a body the system shows inside the window.
fn stage_of(body: &Option<Body>, kind: NativeKind) -> Stage {
    match body {
        Some(Body::Native(doc)) if doc.kind == kind => doc.stage.clone().expect("the view is inside the window"),
        other => panic!("not a file the system shows: {other:?}"),
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
    // A Quick Look view inside the window: the page no longer says Decrypting.
    stage_of(&body, NativeKind::QuickLook);
    let seen = rig.seen();
    assert_eq!(seen.embedded.len(), 1, "Quick Look was handed the document once");
    assert_eq!(seen.embedded[0].0, Shower::QuickLook);
    assert_eq!(seen.embedded[0].1.file_name().unwrap(), "invented-letter.docx");
    assert_eq!(seen.embedded[0].2, b"invented document bytes");
    assert!(seen.asked_playable.is_empty(), "a document is not asked of the player");
    assert!(seen.panel.is_empty(), "the panel is only the fallback");
}

#[test]
fn a_mov_gets_a_player_view_at_the_bodys_rect_and_it_goes_on_back() {
    let mut rig = rig();
    assert!(rig.source.previews("invented-clip.mov"));
    let file = rig.source.open("v1").expect("opens");
    assert!(matches!(file.body, Body::Decrypting { .. }), "{:?}", file.body);
    let body = rig.settle("v1");
    let stage = stage_of(&body, NativeKind::Player);
    let copy = rig
        .source
        .media
        .mac
        .shown_copy()
        .expect("the file is on screen")
        .to_path_buf();
    {
        let seen = rig.seen();
        assert_eq!(seen.embedded.len(), 1, "one player view was added");
        assert_eq!(seen.embedded[0].0, Shower::Player);
        assert_eq!(seen.embedded[0].1, copy, "the view was made on the temp copy's path");
        assert_eq!(seen.embedded[0].2, b"invented video bytes");
        assert_eq!(
            seen.asked_playable,
            vec![copy.clone()],
            "the file itself was asked of AVFoundation"
        );
        assert!(seen.placed.is_empty(), "not placed before the body is laid out");
        assert!(seen.panel.is_empty(), "the panel is only the fallback");
    }
    assert!(copy.starts_with(rig.root.path().join("beebeeb-open")));
    assert_eq!(copy.file_name().unwrap(), "invented-clip.mov");
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&copy), 0o600);
        assert_eq!(mode(copy.parent().unwrap()), 0o700);
    }

    // The body is laid out: the view is sized to its rect.
    let body_rect = [240.0, 96.0, 1038.0, 670.0];
    stage.at(body_rect);
    assert_eq!(rig.seen().placed, vec![(copy.clone(), Some(body_rect))]);
    // The same rect again says nothing new.
    stage.at(body_rect);
    assert_eq!(rig.seen().placed.len(), 1);
    // The details panel opens, then the window is resized: the view follows each.
    let beside_details = [240.0, 96.0, 718.0, 670.0];
    stage.at(beside_details);
    let resized = [240.0, 96.0, 500.0, 400.0];
    stage.at(resized);
    assert_eq!(
        rig.seen().placed[1..],
        [(copy.clone(), Some(beside_details)), (copy.clone(), Some(resized))]
    );
    // A menu opens over the body: the view is hidden, and comes back where it was.
    stage.cover(true);
    stage.cover(true);
    stage.cover(false);
    assert_eq!(
        rig.seen().placed[3..],
        [(copy.clone(), None), (copy.clone(), Some(resized))]
    );
    // Nothing new while it is shown.
    assert!(rig.source.poll("v1").is_none());

    // Back (the viewer closes): the view is removed and the copy with it.
    let Rig { seen, root, source, .. } = rig;
    drop(source);
    assert!(!copy.exists(), "the temp copy is removed when the viewer closes");
    assert!(!copy.parent().unwrap().exists(), "and so is its folder");
    assert_eq!(seen.lock().unwrap().closed, vec![copy]);
    drop(root);
}

#[test]
fn a_file_finder_has_is_handed_over_by_finders_place_with_no_decrypt_and_no_copy() {
    let mut rig = rig();
    let place = rig.in_finder("v1", false);
    // Finder's copy has no limit: the app reads none of it.
    rig.source.media.mac.max = 4;
    rig.source.open("v1").expect("opens");
    let body = rig.settle("v1");
    stage_of(&body, NativeKind::Player);
    {
        let seen = rig.seen();
        assert_eq!(seen.embedded.len(), 1);
        assert_eq!(seen.embedded[0].1, place, "the player was handed Finder's own file");
        assert_eq!(seen.embedded[0].2, FINDERS);
        assert_eq!(seen.asked_playable, vec![place.clone()]);
    }
    assert!(
        rig.world.fetched().is_empty(),
        "nothing was downloaded or decrypted by the app"
    );
    assert!(rig.left().is_empty(), "no temp copy was made: {:?}", rig.left());
    assert!(
        rig.finder.requested().is_empty(),
        "a file that is here is not asked of the system"
    );
    assert_eq!(
        rig.finder.inside(),
        1,
        "the place's scope is held while the file is on screen"
    );

    // Back: the view goes, the scope is let go, and Finder's file is untouched.
    rig.source.media_leave();
    assert_eq!(rig.seen().closed, vec![place.clone()]);
    assert!(rig.let_go(), "the thread left Finder's place");
    assert_eq!(std::fs::read(&place).unwrap(), FINDERS);
}

#[test]
fn a_cloud_only_file_is_downloaded_by_finder_with_progress_and_then_shown_from_there() {
    let mut rig = rig();
    let place = rig.in_finder("d1", true);
    rig.source.open("d1").expect("opens");
    // The page shows how far Finder's download is while it runs.
    let until = Instant::now() + Duration::from_secs(10);
    let mut progress = Vec::new();
    let body = loop {
        match rig.source.poll("d1") {
            Some(Body::Decrypting { done, total }) => progress.push((done, total)),
            Some(body) => break Some(body),
            None => {}
        }
        assert!(Instant::now() < until, "the file never came");
        std::thread::sleep(Duration::from_millis(5));
    };
    stage_of(&body, NativeKind::QuickLook);
    assert!(
        progress.iter().any(|(done, total)| *done > 0 && *total == 20),
        "the transfer board's figures reached the page: {progress:?}"
    );
    assert_eq!(
        rig.finder.requested(),
        vec!["d1".to_string()],
        "the system was asked once"
    );
    assert_eq!(rig.seen().embedded[0].1, place);
    assert!(
        rig.world.fetched().is_empty(),
        "the File Provider's fetch is the one download"
    );
    assert!(rig.left().is_empty());
    rig.source.media_leave();
    assert!(rig.let_go());
}

#[test]
fn a_download_that_stops_moving_lands_on_the_page_and_nothing_is_shown() {
    let mut rig = rig();
    rig.in_finder("v1", true);
    rig.finder.never_arrives.store(true, Ordering::SeqCst);
    rig.world.moving.store(false, Ordering::SeqCst);
    rig.source.media.mac.stall = Duration::from_millis(60);
    rig.source.open("v1").expect("opens");
    let asked = std::time::Instant::now();
    let body = rig.settle("v1");
    // Dm-75: the file's page (dm-73's), with the reason on it; and the limit fired at the limit, also though nothing
    // said how far the download was (RC11 waited three times as long, in silence).
    assert!(
        matches!(body, Some(Body::NotPreviewable(NoPreview::NoViewer))),
        "{body:?}"
    );
    assert_eq!(
        crate::frames::viewers::unsupported::note_of("v1"),
        Some(crate::frames::viewers::unsupported::Note::Failed(
            NOT_MOVING.to_string()
        ))
    );
    assert!(asked.elapsed() < Duration::from_millis(60) * 3 + Duration::from_millis(100));
    assert_eq!(
        rig.finder.requested(),
        ["v1"],
        "the system was asked, by the item's identifier"
    );
    assert!(rig.seen().embedded.is_empty() && rig.seen().panel.is_empty());
    assert!(rig.world.fetched().is_empty(), "no second download by the app");
    assert!(rig.let_go(), "the thread left Finder's place");
    // The file is no longer this arm's: the rest of the poll runs.
    assert!(rig.source.mac_poll("v1").is_none());
}

#[test]
fn a_download_the_system_refuses_lands_on_the_page() {
    let mut rig = rig();
    rig.in_finder("d1", true);
    rig.finder.refuses.store(true, Ordering::SeqCst);
    rig.source.open("d1").expect("opens");
    let body = rig.settle("d1");
    assert!(
        matches!(body, Some(Body::NotPreviewable(NoPreview::NoViewer))),
        "{body:?}"
    );
    assert_eq!(
        crate::frames::viewers::unsupported::note_of("d1"),
        Some(crate::frames::viewers::unsupported::Note::Failed(
            NOT_DOWNLOADED.to_string()
        ))
    );
    assert!(rig.seen().embedded.is_empty() && rig.world.fetched().is_empty());
    assert!(rig.let_go());
}

#[test]
fn when_no_view_can_be_hosted_the_panel_shows_it_and_then_the_page() {
    let mut rig = rig();
    rig.seen().no_view = true;
    rig.source.open("d1").expect("opens");
    let body = rig.settle("d1");
    // In the panel: the body has no stage, and says so.
    assert!(
        matches!(&body, Some(Body::Native(doc)) if doc.kind == NativeKind::QuickLook && doc.stage.is_none()),
        "{body:?}"
    );
    let copy = rig.source.media.mac.shown_copy().expect("shown").to_path_buf();
    assert_eq!(rig.seen().panel.len(), 1);
    assert_eq!(rig.seen().panel[0].0, copy);
    assert!(rig.seen().embedded.is_empty() && rig.seen().placed.is_empty());
    // The person closes the panel: the copy goes, and the page offers the other apps.
    rig.seen().off_screen = true;
    let body = rig.source.poll("d1");
    assert!(
        matches!(body, Some(Body::NotPreviewable(NoPreview::NoViewer))),
        "{body:?}"
    );
    assert!(!copy.exists());
    assert!(rig.left().is_empty());
}

#[test]
fn when_neither_a_view_nor_the_panel_shows_it_the_page_does() {
    let mut rig = rig();
    rig.seen().no_view = true;
    rig.seen().no_panel = true;
    rig.source.open("v1").expect("opens");
    let body = rig.settle("v1");
    assert!(
        matches!(body, Some(Body::NotPreviewable(NoPreview::NoViewer))),
        "{body:?}"
    );
    assert!(rig.left().is_empty(), "the copy is removed: {:?}", rig.left());
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
    assert!(rig.seen().embedded.is_empty() && rig.seen().panel.is_empty());
    assert!(rig.left().is_empty(), "the copy is removed: {:?}", rig.left());
}

#[test]
fn a_lock_takes_the_view_away_and_removes_the_copy() {
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
        rig.seen().embedded.is_empty() && rig.seen().panel.is_empty(),
        "nothing was written to disk for the app's own viewer"
    );
    assert!(rig.left().is_empty());
}

#[test]
fn a_file_over_the_limit_that_finder_does_not_hold_is_not_downloaded() {
    let mut rig = rig();
    rig.source.media.mac.max = 4;
    rig.source.open("v1").expect("opens");
    let body = rig.settle("v1");
    assert!(
        matches!(body, Some(Body::NotPreviewable(NoPreview::TooBig { limit: 4, .. }))),
        "{body:?}"
    );
    assert!(rig.world.fetched().is_empty());
    assert!(rig.left().is_empty());
}

#[test]
fn the_temp_copys_limit_is_a_documents_never_a_films() {
    // A file that is not in Finder passes through memory: 64 MB at most, where dm-70 held up to 1 GiB.
    assert_eq!(PREVIEW_MAX, 64 * 1024 * 1024);
    assert_eq!(Native::default().max, PREVIEW_MAX);
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

#[test]
fn a_copy_a_crash_left_is_removed_when_the_next_is_made() {
    let dir = tempfile::tempdir().unwrap();
    // A run that ended without closing its viewer: its copy is still there. Files' Open has a copy of its own.
    let left = TempCopy::write(dir.path(), "old", "invented-old.docx", b"old bytes").expect("written");
    let left_path = left.path().to_path_buf();
    std::mem::forget(left);
    let opened = dir.path().join("opened-by-files");
    std::fs::create_dir(&opened).unwrap();
    std::fs::write(opened.join("invented.pdf"), b"another app has this open").unwrap();

    let next = TempCopy::write(dir.path(), "new", "invented-new.docx", b"new bytes").expect("written");
    assert!(
        !left_path.exists() && !left_path.parent().unwrap().exists(),
        "the copy left behind is gone"
    );
    assert!(next.path().is_file());
    assert!(
        opened.join("invented.pdf").is_file(),
        "a copy Files' Open made is not this module's to remove"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// dm-80: the helper is in the bundle. The sandbox (a fake here; nothing is started) decides where a document goes.
// ---------------------------------------------------------------------------------------------------------------

/// What the two renderer starts were asked: (editor, viewer).
type Starts = (Arc<Mutex<Vec<EditKind>>>, Arc<AtomicUsize>);

/// A rig with the helper beside the app, a sandbox that answers `says` and counts its asks, and fake renderers.
fn rig_with_helper(says: Result<(), String>) -> (Rig, Arc<AtomicUsize>, Starts) {
    use crate::engine_host::viewer::office_tests::Fake;
    let mut rig = rig();
    rig.world.writable.store(true, Ordering::SeqCst);
    rig.source.office.helper = || {
        Some(PathBuf::from(
            "/invented/Beebeeb.app/Contents/MacOS/beebeeb-office-render",
        ))
    };
    let asked = Arc::new(AtomicUsize::new(0));
    let count = asked.clone();
    rig.source.office.mac_sandbox = Box::new(move |helper| {
        assert_eq!(helper.file_name().unwrap(), "beebeeb-office-render");
        count.fetch_add(1, Ordering::SeqCst);
        says.clone()
    });
    let (edits, views) = (Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicUsize::new(0)));
    let (fake, log) = (Fake::default(), edits.clone());
    rig.source.office.edit_opener = Box::new(move |_, kind, _, _, _| {
        log.lock().unwrap().push(kind);
        fake.editor(kind)
    });
    let (fake, count) = (Fake::default(), views.clone());
    rig.source.office.opener = Box::new(move |_, kind, _, _| {
        count.fetch_add(1, Ordering::SeqCst);
        fake.view(kind)
    });
    (rig, asked, (edits, views))
}

#[test]
fn with_the_helper_and_a_sandbox_that_starts_it_a_docx_opens_in_the_editor() {
    let (mut rig, asked, (edits, views)) = rig_with_helper(Ok(()));
    let file = rig.source.open("d1").expect("opens");
    assert!(
        matches!(file.body, Body::OfficeState(_)),
        "the helper's route: {:?}",
        file.body
    );
    let until = Instant::now() + Duration::from_secs(10);
    while edits.lock().unwrap().is_empty() && Instant::now() < until {
        let _ = rig.source.poll("d1");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        *edits.lock().unwrap(),
        [EditKind::Docx],
        "an editing session, as on Windows"
    );
    assert_eq!(views.load(Ordering::SeqCst), 0, "not the viewer");
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert_eq!(rig.source.office.mac_refusals_logged, 0);
    let seen = rig.seen();
    assert!(
        seen.embedded.is_empty() && seen.panel.is_empty(),
        "Quick Look was not asked"
    );
}

#[test]
fn with_the_helper_and_a_sandbox_that_refuses_a_docx_goes_to_quick_look_and_the_log_says_so_once() {
    let refusal = String::from("no sandbox on this operating system; the helper was not started");
    let (mut rig, asked, (edits, views)) = rig_with_helper(Err(refusal));
    for _ in 0..2 {
        let file = rig.source.open("d1").expect("opens");
        // Quick Look's own start, never the page that says the helper is missing.
        assert!(matches!(file.body, Body::Decrypting { .. }), "{:?}", file.body);
        let body = rig.settle("d1");
        stage_of(&body, NativeKind::QuickLook);
    }
    assert_eq!(rig.seen().embedded.len(), 2, "Quick Look showed it both times");
    assert!(
        edits.lock().unwrap().is_empty() && views.load(Ordering::SeqCst) == 0,
        "no helper was started"
    );
    assert_eq!(asked.load(Ordering::SeqCst), 1, "the sandbox is asked once");
    assert_eq!(rig.source.office.mac_refusals_logged, 1, "one line in the log");
}

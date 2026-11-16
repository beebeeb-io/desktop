//! Windows' media controls: what they are given (never a path), how the picture is built (memory only), and the
//! buttons (drained once). Pure: none of these touches Windows.

use super::*;

/// Rule 1: with no tag title the title is the file's name alone, from a whole Windows path; a tag's title wins.
#[test]
fn now_playing_never_carries_a_path() {
    let path = r"C:\Users\someone\Studio demos\take 3.m4a";
    let now = NowPlaying::for_item(path, "", "", None, true, 0, 1000, false);
    assert_eq!(now.title, "take 3");
    assert!(!now.title.contains("Studio demos") && !now.title.contains('\\') && !now.title.contains(':'));
    let unix = NowPlaying::for_item(
        "/home/someone/Studio demos/take 3.m4a",
        "",
        "",
        None,
        true,
        0,
        1000,
        false,
    );
    assert_eq!(unix.title, "take 3");
    let tagged = NowPlaying::for_item(path, "Morning", "A band", None, true, 0, 1000, false);
    assert_eq!(tagged.title, "Morning");
    assert_eq!(tagged.artist, "A band");
    // A tag that is only control characters is no title.
    assert_eq!(title_for("\n\t", path), "take 3");
    // The debug text carries no word of a tag or a name.
    let shown = format!("{tagged:?}");
    assert!(!shown.contains("Morning") && !shown.contains("band"), "{shown}");
}

/// Rule 2: the picture is PNG bytes built in a `Vec`; this module never reaches a file or an address.
#[test]
fn art_is_encoded_in_memory() {
    let rgba: Vec<u8> = (0..(4 * 3 * 4)).map(|i| i as u8).collect();
    let png = encode_art_png(4, 3, &rgba).expect("a 4x3 picture encodes");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    // Wrong size, empty and oversize pictures are refused rather than encoded.
    assert!(encode_art_png(4, 3, &rgba[..10]).is_none());
    assert!(encode_art_png(0, 3, &[]).is_none());
    assert!(encode_art_png(beebeeb_media_protocol::MAX_ART + 1, 1, &[]).is_none());

    // The source scan (as `engine_host/tests.rs` does for `fn local`): no file, no folder, no address in this module.
    let source = include_str!("os_controls.rs");
    let code = &source[..source.find("#[cfg(test)]").expect("the tests' mount")];
    for needle in [
        ["std", "::", "fs"].concat(),
        ["fs", "::"].concat(),
        ["File", "::"].concat(),
        ["Create", "From", "File"].concat(),
        ["Create", "From", "Uri"].concat(),
        ["Storage", "File"].concat(),
        ["Uri", "::"].concat(),
    ] {
        assert!(!code.contains(&needle), "os_controls.rs must not contain {needle}");
    }
}

/// Rule 3: a press is handed to the player once.
#[test]
fn buttons_are_drained_once() {
    let queue = ButtonQueue::default();
    assert!(queue.drain().is_empty());
    queue.push(OsButton::Pause);
    queue.push(OsButton::Next);
    assert_eq!(queue.drain(), vec![OsButton::Pause, OsButton::Next]);
    assert!(queue.drain().is_empty(), "a second look sees nothing");
    // A press from the OS's thread reaches the same queue.
    let from_os = queue.clone();
    std::thread::spawn(move || from_os.push(OsButton::Previous))
        .join()
        .unwrap();
    assert_eq!(queue.drain(), vec![OsButton::Previous]);
}

/// C4: a video shows as a video in Quick Settings, a song as music.
#[test]
fn a_video_shows_as_video() {
    let video = NowPlaying::for_item("clip.mp4", "", "", None, true, 0, 1000, true);
    let music = NowPlaying::for_item("song.m4a", "", "", None, true, 0, 1000, false);
    assert_eq!(playback_kind(&video), PlaybackKind::Video);
    assert_eq!(playback_kind(&music), PlaybackKind::Music);
}

/// What a fake of Windows' controls was asked, and on which thread.
#[derive(Clone, Default)]
struct Calls(Arc<Mutex<Vec<(String, std::thread::ThreadId)>>>);

impl Calls {
    fn push(&self, what: String) {
        self.0.lock().expect("calls").push((what, std::thread::current().id()));
    }

    fn taken(&self) -> Vec<(String, std::thread::ThreadId)> {
        self.0.lock().expect("calls").clone()
    }
}

struct FakeControls(Calls);

impl NowPlayingSink for FakeControls {
    fn show(&mut self, now: &NowPlaying) {
        self.0.push(format!("show {}", now.title));
    }

    fn clear(&mut self) {
        self.0.push("clear".to_owned());
    }

    fn take_buttons(&mut self) -> Vec<OsButton> {
        panic!("the player takes the buttons from the shared queue, never from the window's controls");
    }
}

fn playing(title: &str) -> NowPlaying {
    NowPlaying::for_item("", title, "", None, true, 0, 1000, false)
}

/// P2-1 of the #462 review: the player's thread (which holds the player's lock while it feeds the controls) never
/// reaches Windows' controls. It only leaves what to show; the UI thread's pump attaches the controls once and
/// carries out the latest request, on its own thread. The buttons Windows presses reach the player's side.
#[test]
fn only_the_ui_thread_attaches_and_writes_the_controls() {
    let ui = UiControls::default();
    let calls = Calls::default();
    let attaches = Arc::new(Mutex::new(0usize));

    // The player's thread: two updates, nothing reaches the controls.
    let mut players = ui.clone();
    std::thread::spawn(move || {
        players.show(&playing("first"));
        players.show(&playing("second"));
    })
    .join()
    .expect("the player's thread");
    assert!(calls.taken().is_empty(), "no call from the player's thread");

    // The UI thread's pump: attached once, the latest update written there.
    let ui_thread = std::thread::current().id();
    let attach = |calls: &Calls, attaches: &Arc<Mutex<usize>>| {
        let (calls, attaches) = (calls.clone(), Arc::clone(attaches));
        move |buttons: ButtonQueue| {
            *attaches.lock().expect("count") += 1;
            buttons.push(OsButton::Next);
            Some(Box::new(FakeControls(calls)) as Box<dyn NowPlayingSink>)
        }
    };
    ui.pump_with(attach(&calls, &attaches));
    assert_eq!(calls.taken(), [("show second".to_owned(), ui_thread)]);
    assert_eq!(*attaches.lock().expect("count"), 1);

    // The button Windows pressed is the player's to take, once.
    let mut players = ui.clone();
    assert_eq!(players.take_buttons(), [OsButton::Next]);
    assert!(players.take_buttons().is_empty());

    // A stop clears; a clear asked after a show replaces it. Never attached twice.
    players.show(&playing("third"));
    players.clear();
    ui.pump_with(attach(&calls, &attaches));
    assert_eq!(calls.taken().last(), Some(&("clear".to_owned(), ui_thread)));
    assert_eq!(*attaches.lock().expect("count"), 1);

    // Nothing asked: nothing done.
    ui.pump_with(attach(&calls, &attaches));
    assert_eq!(calls.taken().len(), 2);
}

/// P2-1: before anything was shown a clear attaches nothing; where there are no controls (no window yet, another OS)
/// the update is dropped and the next one asks again.
#[test]
fn a_pump_without_controls_drops_the_update_and_tries_again() {
    let ui = UiControls::default();
    let mut players = ui.clone();
    players.clear();
    ui.pump_with(|_| panic!("a clear with nothing shown attaches nothing"));
    players.show(&playing("first"));
    ui.pump_with(|_| None);
    let calls = Calls::default();
    let fake = calls.clone();
    ui.pump_with(move |_| Some(Box::new(FakeControls(fake)) as Box<dyn NowPlayingSink>));
    assert!(calls.taken().is_empty(), "the dropped update is not written later");
    players.show(&playing("second"));
    let fake = calls.clone();
    ui.pump_with(move |_| Some(Box::new(FakeControls(fake)) as Box<dyn NowPlayingSink>));
    assert_eq!(calls.taken().len(), 1);
    assert_eq!(calls.taken()[0].0, "show second");
}

/// P3-2 of the #462 review: no picture reaches Windows while artwork is out of the release.
#[test]
fn no_picture_is_handed_to_windows_in_this_release() {
    const { assert!(!SHOWS_ART) };
}

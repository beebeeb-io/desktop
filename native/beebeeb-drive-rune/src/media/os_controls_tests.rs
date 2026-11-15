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

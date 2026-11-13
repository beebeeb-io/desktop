//! `EngineViewerSource`, round 2: the copy on this PC, a file that cannot be opened at all, and the viewer's own
//! picture limits. On the same engine double as `viewer_tests.rs` (`Fake`). Every name and byte is invented.

use super::*;

// ── The copy on this PC ─────────────────────────────────────────────────────────────────────────────

const LOCAL_WORDS: &str = "the words of the copy on this PC";

#[test]
fn a_file_kept_on_this_pc_is_read_from_its_copy_and_nothing_is_downloaded() {
    let fake = Fake::new();
    fake.keep(NOTE, OnThisPc::InSync, Some(LOCAL_WORDS.as_bytes()));
    let (source, _, body) = opened(&fake, NOTE);
    assert_eq!(words(&body).trim_end(), LOCAL_WORDS);
    assert!(fake.s().fetches.is_empty(), "nothing was downloaded");
    assert_eq!(fake.s().local_calls, [NOTE]);
    assert_eq!(source.origin(NOTE), Origin::ThisPc { unsent: false });
    assert_eq!(source.origin("another"), Origin::Beebeeb);
}

#[test]
fn a_copy_with_a_change_that_is_not_sent_is_what_is_shown_and_the_origin_says_so() {
    let fake = Fake::new();
    // Beebeeb has NOTE_TEXT; this PC has an edit of it that is on its way up.
    fake.keep(
        NOTE,
        OnThisPc::NotSentYet,
        Some(b"edited here, newer than what Beebeeb has"),
    );
    let (source, _, body) = opened(&fake, NOTE);
    assert_eq!(words(&body).trim_end(), "edited here, newer than what Beebeeb has");
    assert_ne!(words(&body), NOTE_TEXT, "never the older version from Beebeeb");
    assert!(fake.s().fetches.is_empty());
    assert_eq!(source.origin(NOTE), Origin::ThisPc { unsent: true });
}

#[test]
fn a_row_that_says_here_for_a_copy_that_is_not_wholly_here_is_downloaded_and_says_beebeeb() {
    let fake = Fake::new();
    fake.keep(NOTE, OnThisPc::InSync, None);
    let (source, _, body) = opened(&fake, NOTE);
    assert_eq!(words(&body), NOTE_TEXT);
    assert_eq!(fake.s().local_calls, [NOTE], "the copy was asked for first");
    assert_eq!(fake.s().fetches, [NOTE]);
    assert_eq!(source.origin(NOTE), Origin::Beebeeb);
    // A file the local state does not call here is never asked of the folder at all.
    fake.put("f-online", "Invented online.txt", b"online only");
    let (_source, _, body) = opened(&fake, "f-online");
    assert_eq!(words(&body).trim_end(), "online only");
    assert_eq!(fake.s().local_calls, [NOTE]);
}

#[test]
fn a_refused_read_of_the_copy_is_a_state_and_is_not_routed_around_by_a_download() {
    for (error, state) in [
        (
            OpenFileError::TooLarge { size: 1 << 30 },
            Body::NotPreviewable(NoPreview::TooBig {
                size: 1 << 30,
                limit: IN_MEMORY_MAX,
            }),
        ),
        (OpenFileError::NotSignedIn, Body::Stopped(Stopped::SessionChanged)),
        (OpenFileError::NotFound, Body::Stopped(Stopped::Gone)),
        (OpenFileError::Failed, Body::Failed),
    ] {
        let fake = Fake::new();
        fake.keep(NOTE, OnThisPc::InSync, Some(LOCAL_WORDS.as_bytes()));
        fake.s().local_error = Some(error);
        let (source, _, body) = opened(&fake, NOTE);
        assert_eq!(body, state, "{error:?}");
        assert!(fake.s().fetches.is_empty(), "{error:?}");
        assert_eq!(source.origin(NOTE), Origin::Beebeeb, "nothing is held: {error:?}");
    }
}

#[test]
fn a_copy_read_for_another_session_is_never_shown() {
    let fake = Fake::new();
    fake.keep(NOTE, OnThisPc::InSync, Some(LOCAL_WORDS.as_bytes()));
    fake.s().answers_for = Some(7);
    let (source, _, body) = opened(&fake, NOTE);
    assert_eq!(body, Body::Stopped(Stopped::SessionChanged));
    assert_eq!(source.origin(NOTE), Origin::Beebeeb);
}

#[test]
fn the_origin_goes_with_the_session_the_copy_was_read_for() {
    let fake = Fake::new();
    fake.keep(NOTE, OnThisPc::NotSentYet, Some(LOCAL_WORDS.as_bytes()));
    let (mut source, _, _) = opened(&fake, NOTE);
    assert_eq!(source.origin(NOTE), Origin::ThisPc { unsent: true });
    fake.new_session("a@example.eu");
    assert_eq!(source.origin(NOTE), Origin::Beebeeb);
    assert_eq!(source.poll(NOTE), Some(Body::Stopped(Stopped::SessionChanged)));
}

#[test]
fn kept_on_this_pc_is_said_only_for_a_file_that_is_here_while_sync_runs_for_this_session() {
    let fake = Fake::new();
    let source = Source::on(fake.clone());
    assert!(!source.kept_on_this_pc(NOTE), "online only");
    fake.keep(NOTE, OnThisPc::InSync, Some(LOCAL_WORDS.as_bytes()));
    assert!(source.kept_on_this_pc(NOTE));
    fake.keep(NOTE, OnThisPc::NotSentYet, Some(LOCAL_WORDS.as_bytes()));
    assert!(
        source.kept_on_this_pc(NOTE),
        "a copy with an unsent change is on this PC too"
    );
    assert!(!source.kept_on_this_pc("no-such-file"));
    // Sync is off: the system cannot be asked to open it from the folder, so the viewer shows it.
    fake.s().syncing = false;
    assert!(!source.kept_on_this_pc(NOTE));
    fake.s().syncing = true;
    // Facts of the session before this one say nothing about this one.
    fake.s().facts_are_old = true;
    assert!(!source.kept_on_this_pc(NOTE));
    fake.s().facts_are_old = false;
    fake.s().signed_in = false;
    let before = fake.s().facts_calls;
    assert!(!source.kept_on_this_pc(NOTE));
    assert_eq!(fake.s().facts_calls, before, "signed out, the local state is not asked");
}

// ── A file that cannot be opened at all ─────────────────────────────────────────────────────────────

#[test]
fn each_refusal_of_the_facts_keeps_its_kind_and_one_that_passes_opens_on_the_next_ask() {
    for (error, kind) in [
        (OpenFileError::NotFound, OpenError::NotFound),
        (OpenFileError::NotYours, OpenError::NotFound),
        (OpenFileError::SessionEnded, OpenError::SessionEnded),
        (OpenFileError::NotSignedIn, OpenError::SignedOut),
        (OpenFileError::Failed, OpenError::NotNow),
        (OpenFileError::Offline, OpenError::NotNow),
    ] {
        let fake = Fake::new();
        let mut source = Source::on(fake.clone());
        // Refused once (the local state could not be read at that moment), then readable again.
        fake.s().facts_error = Some((error, 1));
        assert_eq!(source.open(NOTE).err(), Some(kind), "{error:?}");
        assert!(fake.s().fetches.is_empty(), "{error:?}");
        assert_eq!(source.poll(NOTE), None, "{error:?}");
        let again = source.retry(NOTE).expect("the refusal passed");
        assert!(matches!(again.body, Body::Decrypting { .. }), "{error:?}");
        assert_eq!(words(&answered(&mut source, NOTE)), NOTE_TEXT, "{error:?}");
    }
}

// ── Pictures: the viewer's own limits ───────────────────────────────────────────────────────────────

/// The first 33 bytes of a PNG that declares `width` x `height`: its signature and IHDR, and no pixel.
fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    out
}

#[test]
fn a_picture_that_declares_more_than_the_viewer_decodes_is_refused_from_its_header() {
    use viewer_picture::{PIXELS_MAX, SIDE_MAX};
    let fake = Fake::new();
    // A few bytes that declare a canvas far over the limits: said with its figures, not "can't read".
    fake.put("f-wide", "Invented wide.png", &png_header(SIDE_MAX + 1, 10));
    fake.put("f-many", "Invented many.png", &png_header(8_000, 7_000));
    assert!(8_000 * 7_000 > PIXELS_MAX);
    for (id, width, height) in [("f-wide", SIDE_MAX + 1, 10), ("f-many", 8_000, 7_000)] {
        let (source, _, body) = opened(&fake, id);
        assert_eq!(
            body,
            Body::NotPreviewable(NoPreview::PictureTooBig { width, height }),
            "{id}"
        );
        assert!(source.picture(id, Picture::Photo).is_none(), "{id}");
    }
    // Within the limits it is decoded, as before.
    fake.put("f-fits", "Invented fits.png", &png_bytes(40, 30));
    let (source, _, body) = opened(&fake, "f-fits");
    assert!(matches!(body, Body::Photo(_)));
    assert_eq!(source.picture("f-fits", Picture::Photo).unwrap().size(), (40, 30));
}

#[test]
fn a_file_named_as_a_picture_that_is_another_format_by_its_content_never_reaches_a_decoder() {
    let fake = Fake::new();
    // The toolkit picks a decoder by content and has one for this: the viewer shows four formats, by content.
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>"#;
    fake.put("f-svg", "Invented drawing.png", svg);
    let (source, _, body) = opened(&fake, "f-svg");
    assert_eq!(body, Body::NotPreviewable(NoPreview::Unreadable));
    assert!(source.picture("f-svg", Picture::Photo).is_none());
}

#[test]
fn a_cancelled_open_decodes_no_picture() {
    let cancelled = AtomicBool::new(true);
    let content = picture_body(&png_bytes(6, 4), &cancelled);
    assert!(content.picture.is_none());
    let content = picture_body(&png_bytes(6, 4), &AtomicBool::new(false));
    assert!(content.picture.is_some());
}

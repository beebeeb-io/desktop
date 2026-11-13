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
        // It grew past the limit while it was read: no size is said.
        (
            OpenFileError::OverLimit,
            Body::NotPreviewable(NoPreview::OverLimit { limit: IN_MEMORY_MAX }),
        ),
        // A change that is not sent, in a copy that cannot be read: never the older version from Beebeeb.
        (OpenFileError::CopyNotReadable, Body::Stopped(Stopped::CopyNotReadable)),
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
    // The toolkit picks a decoder by content and HAS one for this: an icon container with a PNG inside. The viewer
    // shows four formats, by content, and this is none of them.
    let png = png_bytes(6, 4);
    let mut icon = b"icns".to_vec();
    icon.extend_from_slice(&(16 + png.len() as u32).to_be_bytes());
    icon.extend_from_slice(b"ic07");
    icon.extend_from_slice(&(8 + png.len() as u32).to_be_bytes());
    icon.extend_from_slice(&png);
    // The toolkit's own decoder does decode it (or this test would prove nothing about the check before it).
    assert!(picture(&icon).is_some(), "the toolkit decodes the container");
    fake.put("f-icon", "Invented drawing.png", &icon);
    let (source, _, body) = opened(&fake, "f-icon");
    assert_eq!(body, Body::NotPreviewable(NoPreview::Unreadable));
    assert!(source.picture("f-icon", Picture::Photo).is_none());
}

#[test]
fn a_cancelled_open_decodes_no_picture() {
    let cancelled = AtomicBool::new(true);
    let content = picture_body(&png_bytes(6, 4), &cancelled);
    assert!(content.picture.is_none());
    let content = picture_body(&png_bytes(6, 4), &AtomicBool::new(false));
    assert!(content.picture.is_some());
}

// ── A PDF ───────────────────────────────────────────────────────────────────────────────────────────

use viewer_pdf::tests::sample_pdf;

/// Poll until page `page` of `id` has a picture.
fn page_of(source: &mut Source, id: &str, page: u32) -> rune::Pixels {
    let began = Instant::now();
    loop {
        if let Some(pixels) = source.picture(id, Picture::Page(page)) {
            return pixels;
        }
        let _ = source.poll(id);
        assert!(
            began.elapsed() < Duration::from_secs(20),
            "page {page} was never rendered"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_pdf_opens_and_its_pages_are_rendered_when_the_view_asks_for_them() {
    let fake = Fake::new();
    fake.put("f-pdf", "Invented plan.pdf", &sample_pdf(3, 612, 792));
    let (mut source, file, body) = opened(&fake, "f-pdf");
    assert!(file.facts.starts_with("PDF \u{00B7} "), "{}", file.facts);
    let Body::Pdf(doc) = &body else { panic!("{body:?}") };
    assert_eq!((doc.pages, doc.page), (3, 1));
    assert!(!doc.size.is_empty());
    assert_eq!(fake.s().fetches, ["f-pdf"], "downloaded once, for every page");

    // Nothing is rendered before the view asks; the first ask says "rendering" and the page comes later.
    assert_eq!(source.page_state("f-pdf", 2), PageState::Rendering);
    assert!(source.picture("f-pdf", Picture::Page(2)).is_none());
    let wakes = fake.s().wakes;
    let began = Instant::now();
    let again = loop {
        // The page's arrival is an answer of `poll`: the same document, so the view is drawn again.
        if let Some(body) = source.poll("f-pdf") {
            break body;
        }
        assert!(began.elapsed() < Duration::from_secs(20), "the page never came");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(again, body);
    assert!(fake.s().wakes > wakes, "the window is woken for the page");
    assert_eq!(source.page_state("f-pdf", 2), PageState::Ready);
    let pixels = source.picture("f-pdf", Picture::Page(2)).expect("page 2");
    let (w, h) = pixels.size();
    assert!(
        (1_259..=1_260).contains(&w) && (1_629..=1_631).contains(&h),
        "{w} x {h}"
    );
    assert_eq!(fake.s().fetches.len(), 1, "a page costs no download");

    // A page the document does not have is not asked of the worker, and another file's page is nobody's.
    assert!(source.picture("f-pdf", Picture::Page(0)).is_none());
    assert!(source.picture("f-pdf", Picture::Page(4)).is_none());
    assert!(source.picture("another", Picture::Page(1)).is_none());
    assert_eq!(
        source.page_state("another", 1),
        PageState::Ready,
        "not this source's to wait for"
    );
    assert!(!source.finds(), "the text is not searched here");
}

#[test]
fn a_file_named_pdf_that_is_not_one_is_said_so_and_no_page_is_waited_for() {
    let fake = Fake::new();
    fake.put(
        "f-not-pdf",
        "Invented broken.pdf",
        b"%PDF-1.7 invented, and nothing after",
    );
    let (source, _, body) = opened(&fake, "f-not-pdf");
    assert_eq!(body, Body::NotPreviewable(NoPreview::Unreadable));
    assert!(source.picture("f-not-pdf", Picture::Page(1)).is_none());
}

#[test]
fn only_the_last_pages_are_kept_and_an_earlier_one_is_rendered_again_when_it_is_shown_again() {
    let fake = Fake::new();
    let pages = PAGES_KEPT as u32 + 2;
    fake.put("f-long", "Invented long.pdf", &sample_pdf(pages as usize, 200, 200));
    let (mut source, _, _) = opened(&fake, "f-long");
    for page in 1..=pages {
        let _ = page_of(&mut source, "f-long", page);
    }
    assert_eq!(source.page_state("f-long", pages), PageState::Ready);
    assert_eq!(source.page_state("f-long", 3), PageState::Ready);
    assert_eq!(
        source.page_state("f-long", 1),
        PageState::Rendering,
        "page 1 was let go"
    );
    assert_eq!(source.page_state("f-long", 2), PageState::Rendering);
    // Shown again: rendered again.
    let _ = page_of(&mut source, "f-long", 1);
    assert_eq!(source.page_state("f-long", 1), PageState::Ready);
}

#[test]
fn a_pdfs_pages_leave_with_its_session_and_a_page_on_its_way_is_never_shown() {
    let fake = Fake::new();
    fake.put("f-pdf", "Invented plan.pdf", &sample_pdf(2, 612, 792));
    let (mut source, _, _) = opened(&fake, "f-pdf");
    let _ = page_of(&mut source, "f-pdf", 1);
    // Page 2 is asked for, and the session moves on before it is taken.
    assert!(source.picture("f-pdf", Picture::Page(2)).is_none());
    fake.new_session("b@example.eu");
    assert!(
        source.picture("f-pdf", Picture::Page(1)).is_none(),
        "the kept page is not handed out"
    );
    assert_eq!(source.poll("f-pdf"), Some(Body::Stopped(Stopped::SessionChanged)));
    // The kept pages and the asker's end are let go with the session: with nobody to ask, the worker ends and
    // the document (and the decrypted bytes it reads from) is dropped, not held until the file is closed.
    assert!(
        source.open.as_ref().is_some_and(|opening| opening.pages.is_none()),
        "the pages outlived their session"
    );
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        source.poll("f-pdf"),
        None,
        "what the worker rendered meanwhile is not an answer"
    );
    assert!(source.picture("f-pdf", Picture::Page(2)).is_none());
    assert!(source.picture("f-pdf", Picture::Page(1)).is_none());
}

#[test]
fn a_pdf_kept_on_this_pc_is_rendered_from_its_copy() {
    let fake = Fake::new();
    // Beebeeb has two pages; the copy on this PC, edited and not sent yet, has three.
    fake.put("f-pdf", "Invented plan.pdf", &sample_pdf(2, 612, 792));
    fake.keep("f-pdf", OnThisPc::NotSentYet, Some(&sample_pdf(3, 612, 792)));
    let (source, _, body) = opened(&fake, "f-pdf");
    assert!(matches!(body, Body::Pdf(ref doc) if doc.pages == 3), "{body:?}");
    assert!(fake.s().fetches.is_empty());
    assert_eq!(source.origin("f-pdf"), Origin::ThisPc { unsent: true });
}

#[test]
fn find_on_a_source_that_cannot_search_says_it_is_not_built_never_no_matches() {
    let fake = Fake::new();
    fake.put("f-pdf", "Invented plan.pdf", &sample_pdf(2, 612, 792));
    let mut ui = crate::frames::viewers::Ui::open(Box::new(Source::on(fake.clone())), "f-pdf").expect("it opens");
    let began = Instant::now();
    while !matches!(ui.file.body, Body::Pdf(_)) {
        ui.pump();
        assert!(began.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(2));
    }
    ui.open_find();
    assert!(
        ui.media.find.is_none(),
        "no find bar that would answer for words nobody looked for"
    );
    assert_eq!(ui.toast.as_deref(), Some("Not built yet: Find in this document"));
}

#[test]
fn a_pdf_page_whose_font_is_not_in_the_file_is_said_for_that_page_and_for_no_other() {
    use viewer_pdf::tests::{Writes, fonts_pdf};
    let fake = Fake::new();
    fake.put(
        "f-pdf",
        "Invented letter.pdf",
        &fonts_pdf(&[Writes::Japanese, Writes::Latin]),
    );
    let (mut source, _, _) = opened(&fake, "f-pdf");
    // Until its picture is there the page is "rendering": what it lacks is said of the picture, not before it.
    assert_eq!(source.page_state("f-pdf", 1), PageState::Rendering);
    page_of(&mut source, "f-pdf", 1);
    assert_eq!(source.page_state("f-pdf", 1), PageState::LacksText);
    page_of(&mut source, "f-pdf", 2);
    assert_eq!(source.page_state("f-pdf", 2), PageState::Ready);
    assert_eq!(
        source.page_state("f-pdf", 1),
        PageState::LacksText,
        "kept with its page"
    );
    // Another session: nothing of the page is said any more.
    fake.new_session("b@example.eu");
    let _ = source.poll("f-pdf");
    assert_ne!(source.page_state("f-pdf", 1), PageState::LacksText);
}

#[test]
fn a_copy_read_while_sync_is_off_is_not_called_what_beebeeb_has() {
    let fake = Fake::new();
    fake.keep(NOTE, OnThisPc::InSync, Some(LOCAL_WORDS.as_bytes()));
    fake.s().syncing = false;
    let (source, _, body) = opened(&fake, NOTE);
    // It opens from the copy (no connection is needed), and the origin says what is not known.
    assert_eq!(words(&body).trim_end(), LOCAL_WORDS);
    assert!(fake.s().fetches.is_empty());
    assert_eq!(source.origin(NOTE), Origin::ThisPcSyncOff);
    // With sync running the same copy is the version Beebeeb has.
    fake.s().syncing = true;
    let (source, _, _) = opened(&fake, NOTE);
    assert_eq!(source.origin(NOTE), Origin::ThisPc { unsent: false });
    // A change that is not sent is that, whether sync runs or not.
    fake.keep(NOTE, OnThisPc::NotSentYet, Some(LOCAL_WORDS.as_bytes()));
    fake.s().syncing = false;
    let (source, _, _) = opened(&fake, NOTE);
    assert_eq!(source.origin(NOTE), Origin::ThisPc { unsent: true });
}

#[test]
fn beebeebs_version_of_a_file_that_is_not_in_sync_here_is_not_called_the_file() {
    let fake = Fake::new();
    fake.s().files.get_mut(NOTE).unwrap().unsettled = true;
    let (source, _, body) = opened(&fake, NOTE);
    // The local state calls no copy of it "here": Beebeeb's version is downloaded, and the origin says which.
    assert_eq!(words(&body), NOTE_TEXT);
    assert_eq!(fake.s().fetches, [NOTE]);
    assert!(fake.s().local_calls.is_empty());
    assert_eq!(source.origin(NOTE), Origin::BeebeebNotThisPcs);
    // A file that is settled is plainly Beebeeb's.
    fake.s().files.get_mut(NOTE).unwrap().unsettled = false;
    let (source, _, _) = opened(&fake, NOTE);
    assert_eq!(source.origin(NOTE), Origin::Beebeeb);
}

#[test]
fn a_download_that_passed_the_limit_on_its_way_claims_no_size() {
    let fake = Fake::new();
    fake.s().error = Some(OpenFileError::OverLimit);
    let (_source, _, body) = opened(&fake, NOTE);
    assert_eq!(
        body,
        Body::NotPreviewable(NoPreview::OverLimit { limit: IN_MEMORY_MAX })
    );
}

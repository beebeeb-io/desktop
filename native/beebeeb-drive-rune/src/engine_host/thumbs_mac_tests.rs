//! Drive-macos-92: this process decodes no picture. A HEIC or RAW goes to the confined helper; its page comes back as
//! the thumbnails' source; anything but a whole page of a fitting size, or no helper at all, is no thumbnail.

use std::cell::Cell;
use std::path::Path;

use super::*;
use crate::engine_host::office_host::{PageSize, Registry};

/// The app's sources hold no ImageIO call: the only picture decoder of a Mac runs in the confined helper
/// (`beebeeb-office-render/src/picture.rs`). The names are built from parts so this file does not match itself.
#[test]
fn no_imageio_call_remains_in_the_app() {
    let needles = [
        ["CGImage", "Source"].concat(),
        ["CGImage", "Destination"].concat(),
        ["name = \"Image", "IO\""].concat(),
        ["QLThumbnail", "Generator"].concat(),
        ["NSBitmap", "ImageRep"].concat(),
    ];
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![root.join("build.rs")];
    let mut dirs = vec![root.join("src")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("the sources are readable").flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    // The tray's icon: the app's own two PNGs, built into the binary (`TEMPLATE_1X`, `TEMPLATE_2X`), never a file's.
    let allowed = [("src/shell/platform/macos_tray.rs", ["NSBitmap", "ImageRep"].concat())];
    assert!(files.len() > 100, "the sources were found: {}", files.len());
    let mut found = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        for needle in &needles {
            let name = file.strip_prefix(root).unwrap_or(file).display().to_string();
            if text.contains(needle.as_str()) && !allowed.iter().any(|(at, what)| *at == name && what == needle) {
                found.push(format!("{name} has {needle}"));
            }
        }
    }
    assert!(found.is_empty(), "a picture decoder in the app: {found:?}");
}

#[test]
fn only_heic_and_raw_kinds_go_to_the_helper() {
    assert_eq!(kind_of("IMAGE/HEIC"), Some(PictureKind::Heic));
    assert_eq!(kind_of("image/heif"), Some(PictureKind::Heif));
    assert_eq!(kind_of("image/x-adobe-dng"), Some(PictureKind::Dng));
    for mime in [
        "image/x-canon-cr2",
        "image/x-canon-cr3",
        "image/x-nikon-nef",
        "image/x-sony-arw",
        "image/x-fuji-raf",
        "image/x-olympus-orf",
        "image/x-panasonic-rw2",
    ] {
        assert!(kind_of(mime).is_some(), "{mime}");
    }
    assert!(kind_of("image/jpeg").is_none() && kind_of("image/png").is_none());
    assert!(kind_of("video/quicktime").is_none());
}

/// A stand-in for the host: the states it goes through, the page it answers once asked.
struct Scripted {
    states: Vec<OfficeState>,
    at: Cell<usize>,
    page: Option<(u32, u32, Vec<u8>)>,
    asks: usize,
    answer_unasked: bool,
}

impl Scripted {
    fn new(states: Vec<OfficeState>, page: Option<(u32, u32, Vec<u8>)>) -> Scripted {
        Scripted {
            states,
            at: Cell::new(0),
            page,
            asks: 0,
            answer_unasked: false,
        }
    }
}

impl PictureHost for Scripted {
    fn state(&self) -> OfficeState {
        let at = self.at.get().min(self.states.len() - 1);
        self.at.set(at + 1);
        self.states[at].clone()
    }

    fn take_page(&mut self) -> Option<(u32, u32, Zeroizing<Vec<u8>>)> {
        if self.asks == 0 && !self.answer_unasked {
            return None;
        }
        self.page.take().map(|(w, h, rgba)| (w, h, Zeroizing::new(rgba)))
    }

    fn ask(&mut self) -> bool {
        self.asks += 1;
        true
    }

    fn wait(&self, _: Duration) {}
}

fn pages(w: f32, h: f32) -> OfficeState {
    OfficeState::Pages {
        pages: 1,
        sizes: vec![PageSize { w_pt: w, h_pt: h }],
    }
}

#[test]
fn the_helpers_page_is_the_picture() {
    let rgba: Vec<u8> = (0..4 * 2 * 4).map(|i| i as u8).collect();
    let mut host = Scripted::new(
        vec![OfficeState::Opening, OfficeState::Opening, pages(4.0, 2.0)],
        Some((4, 2, rgba.clone())),
    );
    let thumbs = decode_with(&mut host, Duration::from_secs(5)).expect("the page is the picture");
    assert_eq!((thumbs.width, thumbs.height), (4, 2));
    assert_eq!(*thumbs.rgba, rgba);
    assert_eq!(host.asks, 1, "page 0 is asked once");
}

#[test]
fn a_page_that_does_not_fit_or_was_not_asked_is_no_thumbnail() {
    // Larger than the thumbnails' source.
    let mut host = Scripted::new(vec![pages(4096.0, 2.0)], Some((4096, 2, vec![0; 4096 * 2 * 4])));
    assert!(decode_with(&mut host, Duration::from_secs(5)).is_none());
    assert_eq!(host.asks, 0, "a page larger than SOURCE_PX is not even asked");
    // Its length is not its size.
    let mut host = Scripted::new(vec![pages(4.0, 2.0)], Some((4, 2, vec![0; 7])));
    assert!(decode_with(&mut host, Duration::from_secs(5)).is_none());
    // More than one page.
    let mut host = Scripted::new(
        vec![OfficeState::Pages {
            pages: 2,
            sizes: vec![PageSize { w_pt: 4.0, h_pt: 2.0 }; 2],
        }],
        Some((4, 2, vec![0; 32])),
    );
    assert!(decode_with(&mut host, Duration::from_secs(5)).is_none());
    // A page that came before it was asked.
    let mut host = Scripted::new(vec![OfficeState::Opening], Some((4, 2, vec![0; 32])));
    host.answer_unasked = true;
    assert!(decode_with(&mut host, Duration::from_secs(5)).is_none());
}

#[test]
fn a_helper_that_refuses_stops_or_never_opens_is_no_thumbnail() {
    for end in [
        OfficeState::NoPreview(office_host::NoPreview::NotInstalled),
        OfficeState::NoPreview(office_host::NoPreview::NoSandbox),
        OfficeState::CannotOpen(office_host::CannotOpen::Damaged),
        OfficeState::Stopped {
            why: office_host::Stop::Exited,
            can_retry: true,
        },
        OfficeState::Closed(office_host::CloseWhy::Locked),
    ] {
        let mut host = Scripted::new(vec![OfficeState::Opening, end.clone()], Some((4, 2, vec![0; 32])));
        assert!(decode_with(&mut host, Duration::from_secs(5)).is_none(), "{end:?}");
        assert_eq!(host.asks, 0);
    }
    // Open, asked, and the page never comes: the backstop ends it.
    let mut host = Scripted::new(vec![pages(4.0, 2.0)], None);
    let started = Instant::now();
    assert!(decode_with(&mut host, Duration::from_millis(50)).is_none());
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// The service is not there: the real host, on a helper that does not exist, gives no thumbnail, and nothing is
/// decoded in this process instead.
#[test]
fn no_service_is_no_thumbnail() {
    let registry = Registry::new();
    let missing = Path::new("/nonexistent/beebeeb-dm92/beebeeb-office-render");
    let bytes = Zeroizing::new(b"\0\0\0\x18ftypheic\0\0\0\0 not a photo".to_vec());
    let mut host =
        Waited::open(|wake| OfficeHost::open_in(&registry, missing, DocKind::Picture(PictureKind::Heic), bytes, wake));
    let started = Instant::now();
    assert!(decode_with(&mut host, Duration::from_secs(60)).is_none());
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "it does not wait for the backstop"
    );
    assert!(
        !matches!(host.state(), OfficeState::Opening | OfficeState::Pages { .. }),
        "{:?}",
        host.state()
    );
    drop(host);
    // The test binary has no helper beside it: `generate` asks nothing and makes nothing.
    if office_host::helper_exe().is_none() {
        assert!(generate(b"\0\0\0\x18ftypheic", PictureKind::Heic).is_none());
        assert!(super::super::generate_for(b"\0\0\0\x18ftypheic", "image/heic").is_none());
    }
}

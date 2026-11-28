//! macOS: the thumbnails' source of a HEIC/HEIF photo or a camera's RAW (CR2, CR3, NEF, ARW, DNG, RAF, ORF, RW2).
//! The media helper's Rust decoders read neither, and this process, which holds the keys, never parses one
//! (drive-macos-92): the bytes go to the confined Office helper (on a Mac its XPC service,
//! `io.beebeeb.app.office-render`, in an App Sandbox of its own, checked before anything is granted), where ImageIO
//! decodes them (`beebeeb-office-render/src/picture.rs`), and the picture comes back as one page of RGBA.
//!
//! It goes through [`OfficeHost`] as a document does, so it keeps that path's rules: one document per process, the
//! service's memory watch, the open and request timers that kill a helper that does not answer, every answer checked,
//! and the kill on close, lock and sign-out. Pictures are decoded one at a time here ([`ONE`]); the service runs one
//! document at a time, so a picture asked while an Office file is open waits for it, and then gets no thumbnail.
//!
//! No service, a service that refuses or stops, an answer that fails a check: no thumbnail. Nothing is decoded
//! anywhere else instead.
//!
//! Plaintext: the bytes are copied into the host's `Zeroizing` buffer (and the section the sandbox zeroes); the
//! pixels come back in a page event and are moved into a wiped buffer. Nothing is written to a file or a log.

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use beebeeb_office_protocol::office::PictureKind;

use super::super::office_host::{self, DocKind, Event, OfficeHost, OfficeState, Wake};
use super::{SOURCE_PX, Thumbs};

/// The picture kind the helper is told the bytes are, for the kinds decoded there; `None`: not decoded there.
pub(super) fn kind_of(mime: &str) -> Option<PictureKind> {
    Some(match mime.to_ascii_lowercase().as_str() {
        "image/heic" | "image/heic-sequence" => PictureKind::Heic,
        "image/heif" | "image/heif-sequence" => PictureKind::Heif,
        "image/x-adobe-dng" | "image/dng" => PictureKind::Dng,
        "image/x-canon-cr2" => PictureKind::Cr2,
        "image/x-canon-cr3" => PictureKind::Cr3,
        "image/x-nikon-nef" => PictureKind::Nef,
        "image/x-sony-arw" => PictureKind::Arw,
        "image/x-fuji-raf" | "image/x-fujifilm-raf" => PictureKind::Raf,
        "image/x-olympus-orf" => PictureKind::Orf,
        "image/x-panasonic-rw2" | "image/x-panasonic-raw" => PictureKind::Rw2,
        _ => return None,
    })
}

/// One picture at a time goes to the service.
static ONE: Mutex<()> = Mutex::new(());

/// The longest this waits for one picture, past the host's own timers (the service's start, 30 s for the open, 15 s
/// for the page): then the host is dropped, which kills the helper.
const BACKSTOP: Duration = Duration::from_secs(90);

/// `bytes` (a `kind` photo) as an upright picture of at most [`SOURCE_PX`] a side, decoded by the confined helper.
/// `None`: no helper, or it did not decode. BLOCKS: the helper; a worker's.
pub(super) fn generate(bytes: &[u8], kind: PictureKind) -> Option<Thumbs> {
    if bytes.is_empty() {
        return None;
    }
    let helper = office_host::helper_exe()?;
    let _one = ONE.lock().unwrap_or_else(PoisonError::into_inner);
    let bytes = Zeroizing::new(bytes.to_vec());
    let mut host = Waited::open(|wake| OfficeHost::open(&helper, DocKind::Picture(kind), bytes, wake));
    decode_with(&mut host, BACKSTOP)
}

/// What [`decode_with`] asks of a host: [`OfficeHost`], or a test's stand-in.
pub(super) trait PictureHost {
    fn state(&self) -> OfficeState;
    /// Takes what arrived: page 0, whole, `w` x `h` RGBA, if it is among it (the rest is dropped, and wiped by the
    /// host).
    fn take_page(&mut self) -> Option<(u32, u32, Zeroizing<Vec<u8>>)>;
    /// Page 0, whole, at scale 1: `false` when it cannot be asked.
    fn ask(&mut self) -> bool;
    /// Waits until the host wakes it, at most `for_at_most`.
    fn wait(&self, for_at_most: Duration);
}

/// An [`OfficeHost`] with the wake it was opened with.
pub(super) struct Waited {
    host: OfficeHost,
    woken: Arc<(Mutex<bool>, Condvar)>,
}

impl Waited {
    pub(super) fn open(open: impl FnOnce(Wake) -> OfficeHost) -> Waited {
        let woken = Arc::new((Mutex::new(false), Condvar::new()));
        let wake: Wake = {
            let woken = woken.clone();
            Arc::new(move || {
                *woken.0.lock().unwrap_or_else(PoisonError::into_inner) = true;
                woken.1.notify_all();
            })
        };
        Waited {
            host: open(wake),
            woken,
        }
    }
}

impl PictureHost for Waited {
    fn state(&self) -> OfficeState {
        self.host.state()
    }

    fn take_page(&mut self) -> Option<(u32, u32, Zeroizing<Vec<u8>>)> {
        let mut page = None;
        for event in self.host.poll() {
            if let Event::Page {
                page: 0,
                region: None,
                w,
                h,
                rgba,
                ..
            } = event
            {
                page = Some((w, h, Zeroizing::new(rgba)));
            }
        }
        page
    }

    fn ask(&mut self) -> bool {
        self.host.request_page(0, 1.0, None).is_some()
    }

    fn wait(&self, for_at_most: Duration) {
        let (flag, wake) = &*self.woken;
        let woken = flag.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut woken, _) = wake
            .wait_timeout_while(woken, for_at_most, |woken| !*woken)
            .unwrap_or_else(PoisonError::into_inner);
        *woken = false;
    }
}

/// Opens, asks page 0 at scale 1 once the picture is open, and takes that page. Ends with `None` on any state but
/// "opening" or "open", and after `backstop`. The caller drops the host, which ends the helper.
pub(super) fn decode_with(host: &mut impl PictureHost, backstop: Duration) -> Option<Thumbs> {
    let until = Instant::now() + backstop;
    let mut asked = false;
    loop {
        if let Some((w, h, rgba)) = host.take_page() {
            let fits = w > 0 && h > 0 && w.max(h) <= SOURCE_PX;
            let whole = rgba.len() as u64 == u64::from(w) * u64::from(h) * 4;
            return (asked && fits && whole).then_some(Thumbs {
                width: w,
                height: h,
                rgba,
            });
        }
        match host.state() {
            OfficeState::Opening => {}
            OfficeState::Pages { pages: 1, sizes } if !asked => {
                let fits = sizes.first().is_some_and(|size| {
                    size.w_pt >= 1.0 && size.h_pt >= 1.0 && size.w_pt.max(size.h_pt) <= SOURCE_PX as f32
                });
                if !fits || !host.ask() {
                    return None;
                }
                asked = true;
            }
            OfficeState::Pages { pages: 1, .. } => {}
            _ => return None,
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        host.wait(left.min(Duration::from_millis(250)));
    }
}

#[cfg(test)]
#[path = "thumbs_mac_tests.rs"]
mod tests;

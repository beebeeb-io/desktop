//! The player's bytes from the engine. [`EngineMediaOpener`] opens a file with the
//! engine's range reader (`Engine::open_media_for`) for the press that came WITH the item (`MediaItem::press`: the
//! session and the account the press was made under), and refuses an item of another account or with no press (the
//! binding shape of `files_trash.rs` and `stamp.rs`). M7 also creates the shell's one `PlayerHandle` with the real
//! helper, sound and media-control factories and hands it to `media::set_shell_player`.
//!
//! **The late-open guarantee lives here, in the opener.** The player opens a file on its own worker thread, after
//! the press has returned. The opener, on that thread, opens only for the item's own press
//! (`open_for(press, ..)` -> `open_media_for(press.session, ..)`), and only while the signed-in account is the
//! press's: the engine refuses a session that has ended, and [`refusal_of`] turns that into `Refusal::NotSignedIn`,
//! which the player shows. There is no process-wide stamp: a press made in Photos opens its video as one made in
//! Files does, and a stale session number of another account (each account counts its sessions from its own start)
//! never opens anything, because the account is compared too. Nothing of a file opened late plays under another
//! session.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use beebeeb_engine::app::bindings::media_stream::MediaReader;
use beebeeb_engine::app::viewer_api::OpenFileError;
use beebeeb_engine::app::{CredentialStores, Engine};
use zeroize::Zeroizing;

use super::stamp::account_of;
use crate::media::audio_out::DefaultAudio;
use crate::media::player::{MediaBytes, MediaOpener, PlayerHandle, Press, Refusal};
use crate::media::transport::SandboxedHelpers;
use crate::media::video::VideoSurface;

// The engine mirrors the protocol's longest read; one number, or the native crate (which sees both) does not build.
const _: () = assert!(beebeeb_engine::app::bindings::media_stream::MEDIA_MAX_READ == beebeeb_media_protocol::MAX_READ);

/// The engine's range reader as the player's [`MediaOpener`].
pub struct EngineMediaOpener<C: CredentialStores> {
    engine: Arc<Engine<C>>,
}

impl<C: CredentialStores> EngineMediaOpener<C> {
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        Self { engine }
    }
}

/// What the player says when the engine refuses: not signed in for a session that is gone or locked, else the file
/// could not be played. Never the engine's own words.
pub(super) fn refusal_of(error: OpenFileError) -> Refusal {
    // ux-02: the kind is written (fixed words, no size and nothing of the file); the window says less.
    tracing::warn!(
        kind = match error {
            OpenFileError::NotSignedIn => "not signed in",
            OpenFileError::SessionEnded => "session ended",
            OpenFileError::NotYours => "not yours",
            OpenFileError::NothingSynced => "nothing synced",
            OpenFileError::NotFound => "not found",
            OpenFileError::TooLarge { .. } => "too large",
            OpenFileError::OverLimit => "over limit",
            OpenFileError::CopyNotReadable => "copy not readable",
            OpenFileError::Cancelled => "cancelled",
            OpenFileError::Offline => "offline",
            OpenFileError::NoAccess => "no access",
            OpenFileError::ChangedWhileOpening => "changed while opening",
            OpenFileError::Failed => "failed",
        },
        "media file not opened"
    );
    match error {
        OpenFileError::NotSignedIn | OpenFileError::SessionEnded | OpenFileError::NotYours => Refusal::NotSignedIn,
        _ => Refusal::CouldNotPlay,
    }
}

impl<C: CredentialStores> EngineMediaOpener<C> {
    /// Whether the signed-in account is the one `press` was made under (in memory only: never waits). A press that
    /// names no account opens nothing.
    fn is_the_press_account(&self, press: &Press) -> bool {
        press.account.is_some() && account_of(&self.engine) == press.account
    }
}

impl<C: CredentialStores + 'static> MediaOpener for EngineMediaOpener<C> {
    /// No press came with it: nothing is opened (a session is never guessed from "now").
    fn open(&self, _file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        Err(Refusal::NotSignedIn)
    }

    fn open_for(&self, press: &Press, file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        if !self.is_the_press_account(press) {
            return Err(Refusal::NotSignedIn);
        }
        let mut reader = self.engine.open_media_for(press.session, file_id).map_err(refusal_of)?;
        // The account is asked again once the reader is there: another account that came in while the bytes were
        // being opened gets nothing of them (the reader is dropped, and with it its chunk cache).
        if !self.is_the_press_account(press) {
            return Err(Refusal::NotSignedIn);
        }
        // RE3-b: the tail chunk (a phone's `moov`) is fetched on its own thread from now on, so the player's first
        // read of the head (`ranged::prime`) runs beside it, not after it.
        reader.start_tail_fetch();
        Ok(Box::new(EngineBytes(reader)))
    }
}

/// Makes the shell's one player with the real helper, sound device and media controls, and registers it
/// (`media::set_shell_player`). Called once when the window is built on the engine (`shell::state::init_on_engine`).
pub fn install_shell_player<C: CredentialStores + 'static>(engine: Arc<Engine<C>>) {
    let host = Arc::clone(&engine);
    let player = PlayerHandle::new(
        Arc::new(EngineMediaOpener::new(engine)),
        Arc::new(SandboxedHelpers),
        Arc::new(DefaultAudio),
        Some(Box::new(crate::media::os_controls::shell_controls())),
        VideoSurface::new(),
    );
    // What the player's worker changes reaches the screen: the window's loop is woken for it (the first frame of a
    // video is in no scene yet, so no frame alone would).
    player.set_waker(Arc::new(move || host.wake_host()));
    crate::media::set_shell_player(player);
    // The engine's video thumbnails come from the same confined helper, never from a program found by name.
    crate::media::thumb_frame::install();
}

/// One opened file's bytes, read by the engine for the session it was opened for.
struct EngineBytes(MediaReader);

impl MediaBytes for EngineBytes {
    fn len(&self) -> u64 {
        self.0.len()
    }

    fn read_at(&mut self, offset: u64, len: u32, cancel: &AtomicBool) -> Result<Zeroizing<Vec<u8>>, ()> {
        self.0.read_at(offset, len, cancel).map_err(|_| ())
    }
}

// ---- A picture decoded by the confined helper (095) -------------------------------------------------------------

use std::io::{Read, Write};

pub use beebeeb_media_protocol::image::ImageRefusal;
use beebeeb_media_protocol::image::{DecodeImage, ImageAnswer, ImageFromHelper, MAX_IMAGE_FILE, MAX_IMAGE_PX};
use beebeeb_media_protocol::{ToHelper, write_msg};

/// A picture decoded on this PC: upright, RGBA (straight alpha), at most the `max_px` asked for on its longest side.
pub struct DecodedPicture {
    pub width: u32,
    pub height: u32,
    pub rgba: Zeroizing<Vec<u8>>,
}

/// A started image helper: its two pipes, and the process behind them. Dropping it ends the helper.
pub struct PictureProcess {
    /// What the app reads from the helper.
    pub from: Box<dyn Read + Send>,
    /// What the app writes to the helper.
    pub to: Box<dyn Write + Send>,
    /// Ends the helper (it is not asked): both pipes break, a read of `from` ends.
    pub child: Arc<dyn PictureChild>,
}

/// The process of a started image helper.
pub trait PictureChild: Send + Sync {
    /// Ends it now. Called at most a few times; a helper that is gone already is fine.
    fn kill(&self);
}

/// Starts an image helper (the helper's `--image` mode, confined, system-decoder profile; `hevc`: with the
/// capabilities of the HEVC decoder too, for a HEIC). `None`: it cannot be started (no helper, no sandbox): the
/// picture has no decoder here.
pub trait PictureHelper: Send + Sync {
    fn start(&self, hevc: bool) -> Option<PictureProcess>;
}

static PICTURE_HELPER: std::sync::OnceLock<Arc<dyn PictureHelper>> = std::sync::OnceLock::new();

/// One decode may take this long; then the helper is ended and the picture is `Damaged`.
const DECODE_WITHIN: std::time::Duration = std::time::Duration::from_secs(30);

/// One helper decodes at a time (each may hold up to the helper's memory limit).
static ONE_DECODE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Whether `bytes` start like a HEIC/HEIF (an ISO media file that is not a camera's CR3): its decoder is HEVC.
pub(super) fn looks_like_heif(bytes: &[u8]) -> bool {
    bytes.get(4..8) == Some(b"ftyp") && bytes.get(8..12) != Some(b"crx ")
}

/// A picture only the helper decodes (Windows' own decoders), by what must be installed for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelperPicture {
    /// HEIC/HEIF: the HEIF and HEVC extensions.
    Heif,
    /// A camera's RAW: the Raw Image Extension.
    Raw,
}

impl HelperPicture {
    /// By the file's name. `None`: not a kind the helper is asked for.
    pub fn of_name(name: &str) -> Option<HelperPicture> {
        let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
        match ext.as_str() {
            "heic" | "heif" => Some(HelperPicture::Heif),
            "dng" | "cr2" | "cr3" | "arw" | "nef" | "raf" | "orf" | "rw2" => Some(HelperPicture::Raw),
            _ => None,
        }
    }

    /// By the file's first bytes: an ISO media file (HEIF, or a CR3), a TIFF (DNG, CR2, NEF, ARW, ORF, RW2) or a
    /// RAF. `None`: the helper is not started for these bytes.
    pub fn of_bytes(bytes: &[u8]) -> Option<HelperPicture> {
        if looks_like_heif(bytes) {
            return Some(HelperPicture::Heif);
        }
        let raw = bytes.get(4..12) == Some(b"ftypcrx ")
            || bytes.starts_with(b"II*\0")
            || bytes.starts_with(b"MM\0*")
            || bytes.starts_with(b"IIRO")
            || bytes.starts_with(b"IIRS")
            || bytes.starts_with(b"IIU\0")
            || bytes.starts_with(b"FUJIFILM");
        (raw && bytes.len() > 16).then_some(HelperPicture::Raw)
    }

    fn known(self) -> &'static std::sync::atomic::AtomicU8 {
        static HEIF: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(UNKNOWN);
        static RAW: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(UNKNOWN);
        match self {
            HelperPicture::Heif => &HEIF,
            HelperPicture::Raw => &RAW,
        }
    }

    /// Whether this PC is known to have no decoder for this kind: a decode of this run answered `NoDecoder` (and
    /// none since succeeded). Not known yet: `false`, the first picture finds out. Until the app is started again
    /// a decoder installed meanwhile is not seen. In a test with no helper registered: `true`.
    pub fn decoder_missing(self) -> bool {
        if cfg!(test) && PICTURE_HELPER.get().is_none() {
            return true;
        }
        self.known().load(std::sync::atomic::Ordering::Relaxed) == MISSING
    }
}

const UNKNOWN: u8 = 0;
const HERE: u8 = 1;
const MISSING: u8 = 2;

/// `bytes` (a HEIC, a RAW) as the pixels a viewer draws, at most [`MAX_IMAGE_PX`] a side, through the helper.
/// BLOCKS like [`decode_picture`].
pub fn helper_pixels(bytes: &[u8]) -> Result<rune::Pixels, ImageRefusal> {
    let decoded = decode_picture(bytes, MAX_IMAGE_PX)?;
    let pixels = rune::Pixels::new();
    pixels.set(decoded.width, decoded.height, decoded.rgba.to_vec());
    Ok(pixels)
}

/// Decodes `bytes` (a HEIC, a camera's RAW) to at most `max_px` on its longest side through the helper. Never
/// decodes in this process. No decoder, a damaged file, a file too large: the refusal, never a half picture.
/// BLOCKS: the helper, at most [`DECODE_WITHIN`] (and while another decode runs). Never call it on the window's thread.
pub fn decode_picture(bytes: &[u8], max_px: u32) -> Result<DecodedPicture, ImageRefusal> {
    let answer = decode_in_a_helper(bytes, max_px);
    // What this PC can decode is learned from the answers: a kind with no decoder is said so before any download.
    if let Some(kind) = HelperPicture::of_bytes(bytes) {
        match &answer {
            Ok(_) => kind.known().store(HERE, std::sync::atomic::Ordering::Relaxed),
            Err(ImageRefusal::NoDecoder) => kind.known().store(MISSING, std::sync::atomic::Ordering::Relaxed),
            Err(_) => {}
        }
    }
    answer
}

fn decode_in_a_helper(bytes: &[u8], max_px: u32) -> Result<DecodedPicture, ImageRefusal> {
    // The app's own helper unless another was registered first; a test starts none it did not register.
    #[cfg(not(test))]
    let helper = PICTURE_HELPER.get_or_init(|| Arc::new(crate::media::transport::SandboxedPictures));
    #[cfg(test)]
    let helper = PICTURE_HELPER.get().ok_or(ImageRefusal::NoDecoder)?;
    let _one = ONE_DECODE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let PictureProcess { from, to, child } = helper.start(looks_like_heif(bytes)).ok_or(ImageRefusal::NoDecoder)?;
    // A helper that stops answering is ended, so the read below ends too.
    let (done, late) = std::sync::mpsc::channel::<()>();
    let watched = child.clone();
    let watch = std::thread::Builder::new()
        .name("picture-helper-watch".into())
        .spawn(move || {
            if late.recv_timeout(DECODE_WITHIN) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                watched.kill();
            }
        });
    if watch.is_err() {
        child.kill();
        return Err(ImageRefusal::Damaged);
    }
    let answer = decode_over(from, to, bytes, max_px);
    let _ = done.send(());
    child.kill();
    answer
}

/// One decode over the helper's two pipes: the helper reads the ranges it asks for, and answers once.
pub(super) fn decode_over(
    mut from: Box<dyn Read + Send>,
    mut to: Box<dyn Write + Send>,
    bytes: &[u8],
    max_px: u32,
) -> Result<DecodedPicture, ImageRefusal> {
    if max_px == 0 || max_px > MAX_IMAGE_PX {
        return Err(ImageRefusal::Unsupported);
    }
    if bytes.len() as u64 > MAX_IMAGE_FILE {
        return Err(ImageRefusal::TooLarge);
    }
    write_msg(
        &mut to,
        &DecodeImage {
            max_px,
            len: bytes.len() as u64,
        },
    )
    .map_err(|_| ImageRefusal::Damaged)?;
    loop {
        match beebeeb_media_protocol::image::read_image_to_app(&mut from).map_err(|_| ImageRefusal::Damaged)? {
            ImageFromHelper::ReadAt { id, offset, len } => {
                let start = usize::try_from(offset).unwrap_or(usize::MAX);
                let answer = match bytes.get(start..) {
                    Some(rest) => ToHelper::Data {
                        id,
                        bytes: rest[..rest.len().min(len as usize)].to_vec(),
                    },
                    None => ToHelper::DataFailed { id },
                };
                if write_msg(&mut to, &answer).is_err() {
                    return Err(ImageRefusal::Damaged);
                }
            }
            ImageFromHelper::Answer(ImageAnswer::Refused(why)) => return Err(why),
            ImageFromHelper::Answer(ImageAnswer::Pixels { width, height, rgba }) => {
                // The helper is untrusted: what it says is checked against what was asked.
                if width == 0
                    || height == 0
                    || width.max(height) > max_px
                    || rgba.len() as u64 != u64::from(width) * u64::from(height) * 4
                {
                    return Err(ImageRefusal::Damaged);
                }
                let _ = write_msg(&mut to, &ToHelper::Close);
                return Ok(DecodedPicture {
                    width,
                    height,
                    rgba: Zeroizing::new(rgba),
                });
            }
        }
    }
}

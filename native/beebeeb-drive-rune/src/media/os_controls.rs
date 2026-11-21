//! Windows' media controls (DW-1504, decision D-MED-8): while something plays, the title, the artist and the picture
//! show in Windows' Quick Settings and on the media keys' overlay; never the file's path or its folder. They clear at
//! once on a stop, a lock, a sign-out or an account change. The picture goes to Windows from memory
//! (`InMemoryRandomAccessStream`), never as a file or a URI.
//!
//! System Media Transport Controls through `ISystemMediaTransportControlsInterop::GetForWindow`. macOS and Linux:
//! [`OsControls::attach`] answers `None` (the macOS Now Playing slice is in the macOS phase).
//!
//! **Threads (P2-1 of the #462 review).** The controls belong to the window: they are attached and written on the UI
//! thread only. The player feeds them from its own thread while it holds the player's lock, so the player is handed
//! a [`UiControls`]: its `show` and `clear` only leave what to show next, and [`UiControls::pump`], on the UI thread
//! (the shell's poll), attaches the window's controls once and carries out the latest of it. Nothing the player
//! calls waits on Windows. The process's one is [`shell_controls`].
//!
//! **No artwork in 0.9.0** (P3-2 of the #462 review): the player keeps no picture, and [`SHOWS_ART`] keeps the one
//! path that hands Windows a picture shut.
//!
//! **The wake cover** (`shell::away`, D-B2-3). While the away lock's cover is on (a wake the engine has not decided
//! yet, also a gap the window's poll has not found yet: [`WakeCover::covers_now`]), a button Windows presses waits in
//! the queue: the player is handed nothing. It acts once the cover is gone, asked again at every take. A stop (also
//! at a lock, a sign-out or another account: `media::stop_all`) drops it. Nothing of the item reaches Windows while
//! covered: the pump takes down what shows, once, and the latest item goes up again when the cover is gone.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use beebeeb_media_protocol::{MAX_ART, cut_text};

use super::player::NowPlayingSink;
use crate::shell::away::WakeCover;

/// What the OS's controls show.
#[derive(Clone, PartialEq, Eq)]
pub struct NowPlaying {
    /// The tag's title, else the file's name without its folder.
    pub title: String,
    pub artist: String,
    /// RGBA, at most the protocol's `MAX_ART` a side.
    pub art: Option<(u32, u32, Vec<u8>)>,
    pub playing: bool,
    pub at_ms: u64,
    pub length_ms: u64,
    /// A video shows as a video in Quick Settings, not as music.
    pub video: bool,
}

/// What Windows is told the item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackKind {
    Music,
    Video,
}

/// The kind the OS's controls show for this item.
pub fn playback_kind(now: &NowPlaying) -> PlaybackKind {
    if now.video {
        PlaybackKind::Video
    } else {
        PlaybackKind::Music
    }
}

impl NowPlaying {
    /// Builds what the controls show from an item. `file_name` may be a whole path (either separator): only its last
    /// part, without the extension, is ever used, and only when the tag has no title. The folder never reaches the OS.
    pub fn for_item(
        file_name: &str,
        tag_title: &str,
        artist: &str,
        art: Option<(u32, u32, Vec<u8>)>,
        playing: bool,
        at_ms: u64,
        length_ms: u64,
        video: bool,
    ) -> NowPlaying {
        NowPlaying {
            title: title_for(tag_title, file_name),
            artist: clean_text(artist),
            art,
            playing,
            at_ms,
            length_ms,
            video,
        }
    }
}

/// The tag's title; without one, the file's name without its folder and its extension.
pub fn title_for(tag_title: &str, file_name: &str) -> String {
    let tag = clean_text(tag_title);
    if !tag.is_empty() {
        return tag;
    }
    let last = file_name.rsplit(['/', '\\']).next().unwrap_or("");
    let stem = match last.rfind('.') {
        Some(dot) if dot > 0 => &last[..dot],
        _ => last,
    };
    clean_text(stem)
}

/// No control characters, trimmed, at most the protocol's text length.
fn clean_text(text: &str) -> String {
    let plain: String = text.chars().filter(|c| !c.is_control()).collect();
    cut_text(plain.trim())
}

/// Times and flags, never a word of a tag.
impl fmt::Debug for NowPlaying {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NowPlaying")
            .field("playing", &self.playing)
            .field("at_ms", &self.at_ms)
            .field("length_ms", &self.length_ms)
            .finish_non_exhaustive()
    }
}

/// A button of the OS's controls or a media key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsButton {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
}

/// The buttons pressed since the player last looked: the OS pushes from its own thread, the player drains on its tick.
#[derive(Clone, Default)]
pub struct ButtonQueue {
    pressed: Arc<Mutex<Vec<OsButton>>>,
}

impl ButtonQueue {
    pub fn push(&self, button: OsButton) {
        if let Ok(mut pressed) = self.pressed.lock() {
            // A held key must not pile up for ever while the player is busy.
            if pressed.len() < 64 {
                pressed.push(button);
            }
        }
    }

    /// Everything pressed so far, once: a second call answers nothing until a new press.
    pub fn drain(&self) -> Vec<OsButton> {
        self.pressed
            .lock()
            .map(|mut pressed| std::mem::take(&mut *pressed))
            .unwrap_or_default()
    }
}

/// Whether a picture is handed to Windows' controls. Off while artwork is out of the release: nothing is shown and
/// nothing on the UI thread waits for the picture's stream to be written.
pub const SHOWS_ART: bool = false;

/// The picture as PNG bytes, built in memory. `None` for a picture that is empty, over `MAX_ART` a side, or whose
/// bytes are not `w * h * 4`.
pub fn encode_art_png(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || w > MAX_ART || h > MAX_ART {
        return None;
    }
    if rgba.len() as u64 != u64::from(w) * u64::from(h) * 4 {
        return None;
    }
    let mut out: Vec<u8> = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(rgba).ok()?;
    writer.finish().ok()?;
    Some(out)
}

/// The OS's media controls of the app's window.
pub struct OsControls {
    #[cfg(windows)]
    smtc: Option<windows::Media::SystemMediaTransportControls>,
    #[cfg(windows)]
    handler_token: Option<windows::Foundation::EventRegistrationToken>,
    buttons: ButtonQueue,
}

impl OsControls {
    /// The controls of `window` (its native handle, on the UI thread). `None`: this OS has none here.
    pub fn attach(window: &rune::WindowHandle) -> Option<OsControls> {
        OsControls::attach_with(window, ButtonQueue::default())
    }

    /// [`OsControls::attach`], with the buttons Windows presses pushed to `buttons`.
    pub fn attach_with(window: &rune::WindowHandle, buttons: ButtonQueue) -> Option<OsControls> {
        #[cfg(windows)]
        {
            // The window's native handle, the way `shell/platform/windows_hello.rs` reads it (UI thread).
            let hwnd = window
                .with_raw_handle(|handle| match handle.as_raw() {
                    raw_window_handle::RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
                    _ => None,
                })
                .ok()
                .flatten()
                .filter(|hwnd| *hwnd != 0)?;
            imp::attach(hwnd, buttons)
        }
        #[cfg(not(windows))]
        {
            let _ = (window, buttons);
            None
        }
    }

    pub fn show(&mut self, now: &NowPlaying) {
        #[cfg(windows)]
        imp::show(self, now);
        #[cfg(not(windows))]
        let _ = now;
    }

    pub fn clear(&mut self) {
        #[cfg(windows)]
        imp::clear(self);
    }

    pub fn take_buttons(&mut self) -> Vec<OsButton> {
        self.buttons.drain()
    }
}

impl Drop for OsControls {
    fn drop(&mut self) {
        // Nothing of the item stays in Windows' controls once they are not ours any more.
        self.clear();
        #[cfg(windows)]
        imp::detach(self);
    }
}

impl NowPlayingSink for OsControls {
    fn show(&mut self, now: &NowPlaying) {
        OsControls::show(self, now)
    }

    fn clear(&mut self) {
        OsControls::clear(self)
    }

    fn take_buttons(&mut self) -> Vec<OsButton> {
        OsControls::take_buttons(self)
    }
}

/// What the player last asked of the controls and the UI thread has not carried out yet.
enum Pending {
    Show(NowPlaying),
    Clear,
}

/// The controls as the player holds them: see the module's "Threads". Clones share them.
#[derive(Clone, Default)]
pub struct UiControls {
    shared: Arc<UiShared>,
}

#[derive(Default)]
struct UiShared {
    /// The latest show or clear the player asked for; a newer one replaces it.
    pending: Mutex<Option<Pending>>,
    /// The buttons Windows pressed: its handler pushes, the player takes them (none while the cover is on).
    buttons: ButtonQueue,
    /// Only [`UiControls::pump`] (the UI thread) touches it.
    os: Mutex<OsSide>,
    /// The away lock's wake cover (the module's "The wake cover"). Never on for the default controls.
    cover: WakeCover,
}

/// The UI thread's side.
#[derive(Default)]
struct OsSide {
    /// The window's controls once attached.
    attached: Option<Box<dyn NowPlayingSink>>,
    /// The latest show the player asked for, kept while the cover hides it; none after a clear.
    latest: Option<NowPlaying>,
    /// The controls were taken down for the wake cover: the latest show goes up again once it is gone.
    hidden: bool,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl UiControls {
    /// Controls whose buttons wait, and whose item Windows is not shown, while `cover` is on. The shell's are
    /// [`shell_controls`].
    pub fn under(cover: WakeCover) -> UiControls {
        UiControls {
            shared: Arc::new(UiShared {
                cover,
                ..UiShared::default()
            }),
        }
    }

    /// On the UI thread, with the main window: attaches the window's controls the first time there is something to
    /// show, then carries out the latest show or clear. Cheap when nothing is pending; call it on every poll.
    pub fn pump(&self, window: &rune::WindowHandle) {
        self.pump_with(|buttons| {
            OsControls::attach_with(window, buttons).map(|controls| Box::new(controls) as Box<dyn NowPlayingSink>)
        });
    }

    /// [`UiControls::pump`] with the attaching given (the tests hand a fake). An attach that answers `None` (no window
    /// yet, no controls on this OS) drops the update: the player feeds the next one within a second.
    fn pump_with(&self, attach: impl FnOnce(ButtonQueue) -> Option<Box<dyn NowPlayingSink>>) {
        let pending = locked(&self.shared.pending).take();
        let mut os = locked(&self.shared.os);
        match &pending {
            Some(Pending::Show(now)) => os.latest = Some(now.clone()),
            Some(Pending::Clear) => os.latest = None,
            None => {}
        }
        if self.shared.cover.covers_now() {
            // Nothing of the item reaches Windows under the wake cover: what shows is taken down once, also with
            // nothing new asked (a paused item), and the latest show waits for the cover's end.
            if !std::mem::replace(&mut os.hidden, true)
                && let Some(controls) = os.attached.as_mut()
            {
                controls.clear();
            }
            if matches!(pending, Some(Pending::Clear)) {
                // The player stopped: a press Windows sent before its controls went off belongs to what stopped.
                self.shared.buttons.drain();
            }
            return;
        }
        let pending = if std::mem::take(&mut os.hidden) {
            // The cover is gone: the latest show goes up again (none: the player stopped meanwhile, already down).
            os.latest.clone().map(Pending::Show)
        } else {
            pending
        };
        let Some(pending) = pending else {
            return;
        };
        if os.attached.is_none() {
            if matches!(pending, Pending::Clear) {
                // Nothing was shown: nothing to clear.
                return;
            }
            os.attached = attach(self.shared.buttons.clone());
        }
        let Some(controls) = os.attached.as_mut() else {
            return;
        };
        match pending {
            Pending::Show(now) => controls.show(&now),
            Pending::Clear => {
                controls.clear();
                // A press Windows sent before its controls went off belongs to what stopped.
                self.shared.buttons.drain();
            }
        }
    }
}

/// The player's side: never a call into Windows, only what to do next.
impl NowPlayingSink for UiControls {
    fn show(&mut self, now: &NowPlaying) {
        *locked(&self.shared.pending) = Some(Pending::Show(now.clone()));
    }

    fn clear(&mut self) {
        // The player stopped (also at a lock, a sign-out or another account): a press that waited for the cover's end
        // goes with what it was for.
        self.shared.buttons.drain();
        *locked(&self.shared.pending) = Some(Pending::Clear);
    }

    fn take_buttons(&mut self) -> Vec<OsButton> {
        // Under the wake cover a press waits in the queue: nothing starts or moves behind the cover.
        if self.shared.cover.covers_now() {
            return Vec::new();
        }
        self.shared.buttons.drain()
    }
}

/// The process's one: the shell's player is handed a clone (`engine_host::media::install_shell_player`) and the
/// shell's poll pumps it on the UI thread.
/// Under the shell's wake cover ([`WakeCover::shell`]).
pub fn shell_controls() -> UiControls {
    static SHELL: OnceLock<UiControls> = OnceLock::new();
    SHELL.get_or_init(|| UiControls::under(WakeCover::shell())).clone()
}

#[cfg(windows)]
mod imp {
    use windows::Foundation::{TimeSpan, TypedEventHandler};
    use windows::Media::{
        MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls, SystemMediaTransportControlsButton,
        SystemMediaTransportControlsButtonPressedEventArgs, SystemMediaTransportControlsTimelineProperties,
    };
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
    use windows::core::HSTRING;

    use super::{
        ButtonQueue, NowPlaying, OsButton, OsControls, PlaybackKind, SHOWS_ART, encode_art_png, playback_kind,
    };

    /// Windows counts time in 100 ns steps.
    fn span(ms: u64) -> TimeSpan {
        TimeSpan {
            Duration: i64::try_from(ms).unwrap_or(i64::MAX / 10_000).saturating_mul(10_000),
        }
    }

    fn map_button(button: SystemMediaTransportControlsButton) -> Option<OsButton> {
        match button {
            SystemMediaTransportControlsButton::Play => Some(OsButton::Play),
            SystemMediaTransportControlsButton::Pause => Some(OsButton::Pause),
            SystemMediaTransportControlsButton::Next => Some(OsButton::Next),
            SystemMediaTransportControlsButton::Previous => Some(OsButton::Previous),
            _ => None,
        }
    }

    pub(super) fn attach(hwnd: isize, buttons: ButtonQueue) -> Option<OsControls> {
        let interop =
            windows::core::factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>().ok()?;
        // SAFETY: a COM call on a live interface. The handle is only a number to Windows: one that names no window
        // is refused with an error, and nothing is read through it here.
        let smtc: SystemMediaTransportControls = unsafe { interop.GetForWindow(HWND(hwnd as *mut _)) }.ok()?;
        smtc.SetIsPlayEnabled(true).ok()?;
        smtc.SetIsPauseEnabled(true).ok()?;
        smtc.SetIsNextEnabled(true).ok()?;
        smtc.SetIsPreviousEnabled(true).ok()?;
        let queue = buttons.clone();
        let handler = TypedEventHandler::<
            SystemMediaTransportControls,
            SystemMediaTransportControlsButtonPressedEventArgs,
        >::new(move |_, args| {
            if let Some(button) = args.as_ref().and_then(|a| a.Button().ok()).and_then(map_button) {
                queue.push(button);
            }
            Ok(())
        });
        let token = smtc.ButtonPressed(&handler).ok()?;
        // Nothing shows until something plays.
        let _ = smtc.SetIsEnabled(false);
        Some(OsControls {
            smtc: Some(smtc),
            handler_token: Some(token),
            buttons,
        })
    }

    pub(super) fn detach(controls: &mut OsControls) {
        if let (Some(smtc), Some(token)) = (controls.smtc.as_ref(), controls.handler_token.take()) {
            let _ = smtc.RemoveButtonPressed(token);
        }
        controls.smtc = None;
    }

    pub(super) fn show(controls: &mut OsControls, now: &NowPlaying) {
        let Some(smtc) = controls.smtc.as_ref() else { return };
        // A failed call leaves the controls as they were; nothing here may take the player down.
        let _ = write(smtc, now);
    }

    fn write(smtc: &SystemMediaTransportControls, now: &NowPlaying) -> windows::core::Result<()> {
        smtc.SetIsEnabled(true)?;
        smtc.SetPlaybackStatus(if now.playing {
            MediaPlaybackStatus::Playing
        } else {
            MediaPlaybackStatus::Paused
        })?;
        let updater = smtc.DisplayUpdater()?;
        match playback_kind(now) {
            PlaybackKind::Music => {
                updater.SetType(MediaPlaybackType::Music)?;
                let music = updater.MusicProperties()?;
                music.SetTitle(&HSTRING::from(now.title.as_str()))?;
                music.SetArtist(&HSTRING::from(now.artist.as_str()))?;
            }
            PlaybackKind::Video => {
                updater.SetType(MediaPlaybackType::Video)?;
                let video = updater.VideoProperties()?;
                video.SetTitle(&HSTRING::from(now.title.as_str()))?;
                video.SetSubtitle(&HSTRING::from(now.artist.as_str()))?;
            }
        }
        let art = now.art.as_ref().filter(|_| SHOWS_ART);
        match art.and_then(|(w, h, rgba)| encode_art_png(*w, *h, rgba)) {
            Some(png) => updater.SetThumbnail(&reference_from_memory(&png)?)?,
            None => updater.SetThumbnail(None::<&RandomAccessStreamReference>)?,
        }
        updater.Update()?;
        let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
        timeline.SetStartTime(span(0))?;
        timeline.SetEndTime(span(now.length_ms))?;
        timeline.SetMinSeekTime(span(0))?;
        timeline.SetMaxSeekTime(span(now.length_ms))?;
        timeline.SetPosition(span(now.at_ms.min(now.length_ms)))?;
        smtc.UpdateTimelineProperties(&timeline)?;
        Ok(())
    }

    /// The picture goes to Windows from this process's memory: an in-memory stream, never a file or an address.
    fn reference_from_memory(png: &[u8]) -> windows::core::Result<RandomAccessStreamReference> {
        let stream = InMemoryRandomAccessStream::new()?;
        let writer = DataWriter::CreateDataWriter(&stream)?;
        writer.WriteBytes(png)?;
        writer.StoreAsync()?.get()?;
        // The writer would close the stream with itself: let go of it first.
        let _ = writer.DetachStream();
        stream.Seek(0)?;
        RandomAccessStreamReference::CreateFromStream(&stream)
    }

    pub(super) fn clear(controls: &mut OsControls) {
        let Some(smtc) = controls.smtc.as_ref() else { return };
        if let Ok(updater) = smtc.DisplayUpdater() {
            let _ = updater.ClearAll();
            let _ = updater.Update();
        }
        let _ = smtc.SetPlaybackStatus(MediaPlaybackStatus::Closed);
        let _ = smtc.SetIsEnabled(false);
    }
}

#[cfg(test)]
#[path = "os_controls_tests.rs"]
mod tests;

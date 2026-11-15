//! Windows' media controls (DW-1504, decision D-MED-8): while something plays, the title, the artist and the picture
//! show in Windows' Quick Settings and on the media keys' overlay; never the file's path or its folder. They clear at
//! once on a stop, a lock, a sign-out or an account change. The picture goes to Windows from memory
//! (`InMemoryRandomAccessStream`), never as a file or a URI.
//!
//! System Media Transport Controls through `ISystemMediaTransportControlsInterop::GetForWindow`. macOS and Linux:
//! [`OsControls::attach`] answers `None` (the macOS Now Playing slice is in the macOS phase).

use std::fmt;
use std::sync::{Arc, Mutex};

use beebeeb_media_protocol::{MAX_ART, cut_text};

use super::player::NowPlayingSink;

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
    ) -> NowPlaying {
        NowPlaying {
            title: title_for(tag_title, file_name),
            artist: clean_text(artist),
            art,
            playing,
            at_ms,
            length_ms,
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
            imp::attach(hwnd)
        }
        #[cfg(not(windows))]
        {
            let _ = window;
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

#[cfg(windows)]
mod imp {
    use windows::Foundation::{TimeSpan, TypedEventHandler};
    use windows::Media::{
        MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls, SystemMediaTransportControlsButton,
        SystemMediaTransportControlsButtonPressedEventArgs, SystemMediaTransportControlsTimelineProperties,
    };
    use windows::Storage::Streams::{
        DataWriter, IRandomAccessStreamReference, InMemoryRandomAccessStream, RandomAccessStreamReference,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
    use windows::core::{HSTRING, Interface};

    use super::{ButtonQueue, NowPlaying, OsButton, OsControls, encode_art_png};

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

    pub(super) fn attach(hwnd: isize) -> Option<OsControls> {
        let interop =
            windows::core::factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>().ok()?;
        // SAFETY: a COM call on a live interface. The handle is only a number to Windows: one that names no window
        // is refused with an error, and nothing is read through it here.
        let smtc: SystemMediaTransportControls = unsafe { interop.GetForWindow(HWND(hwnd as *mut _)) }.ok()?;
        let buttons = ButtonQueue::default();
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
        // `NowPlaying` carries no kind: every item shows as music (title and artist). See the branch notes.
        updater.SetType(MediaPlaybackType::Music)?;
        let music = updater.MusicProperties()?;
        music.SetTitle(&HSTRING::from(now.title.as_str()))?;
        music.SetArtist(&HSTRING::from(now.artist.as_str()))?;
        match now.art.as_ref().and_then(|(w, h, rgba)| encode_art_png(*w, *h, rgba)) {
            Some(png) => updater.SetThumbnail(&reference_from_memory(&png)?)?,
            None => updater.SetThumbnail(None::<&IRandomAccessStreamReference>)?,
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
    fn reference_from_memory(png: &[u8]) -> windows::core::Result<IRandomAccessStreamReference> {
        let stream = InMemoryRandomAccessStream::new()?;
        let writer = DataWriter::CreateDataWriter(&stream)?;
        writer.WriteBytes(png)?;
        writer.StoreAsync()?.get()?;
        // The writer would close the stream with itself: let go of it first.
        let _ = writer.DetachStream();
        stream.Seek(0)?;
        RandomAccessStreamReference::CreateFromStream(&stream)?.cast::<IRandomAccessStreamReference>()
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

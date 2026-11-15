//! Windows' media controls (DW-1504, decision D-MED-8): while something plays, the title, the artist and the picture
//! show in Windows' Quick Settings and on the media keys' overlay; never the file's path or its folder. They clear at
//! once on a stop, a lock, a sign-out or an account change. The picture goes to Windows from memory
//! (`InMemoryRandomAccessStream`), never as a file or a URI.
//!
//! Slice M5 (`media-os-controls`) fills it (System Media Transport Controls through
//! `ISystemMediaTransportControlsInterop::GetForWindow`). macOS and Linux: [`OsControls::attach`] answers `None` (the
//! macOS Now Playing slice is in the macOS phase). The scaffold attaches nothing.

use std::fmt;

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

/// The OS's media controls of the app's window.
pub struct OsControls {
    #[cfg(windows)]
    smtc: Option<windows::Media::SystemMediaTransportControls>,
    buttons: Vec<OsButton>,
}

impl OsControls {
    /// The controls of `window` (its native handle, on the UI thread). `None`: this OS has none here.
    pub fn attach(window: &rune::WindowHandle) -> Option<OsControls> {
        let _ = window;
        None
    }

    pub fn show(&mut self, now: &NowPlaying) {
        let _ = now;
    }

    pub fn clear(&mut self) {}

    pub fn take_buttons(&mut self) -> Vec<OsButton> {
        std::mem::take(&mut self.buttons)
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

/// The WinRT types M5 uses (named here so the scaffold's one check proves the `windows` features are on).
#[cfg(windows)]
fn winrt_types() {
    let _: Option<windows::Media::SystemMediaTransportControls> = None;
    let _: Option<windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop> = None;
    let _: Option<windows::Storage::Streams::InMemoryRandomAccessStream> = None;
}

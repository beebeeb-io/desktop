//! Where a video's thumbnail frame comes from: the app's confined media helper, and nowhere else.
//!
//! The engine parses and decodes nothing of a video, and it starts no program for one (no program found by a name on
//! the search path, no environment override). The app that hosts the engine registers ONE [`FirstFrameSource`]
//! ([`set_first_frame_source`]); it asks the sandboxed helper for the first frame of the file at a path and answers
//! with RGBA pixels. No source registered, or a source that refuses, is no thumbnail (the file keeps its type icon):
//! there is no other way to get one.
//!
//! What a source answers is checked here ([`checked`]): the size is inside the limits asked for and the pixel
//! count is exactly `w * h * 4`. Anything else is refused.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use zeroize::Zeroizing;

/// The largest frame the engine asks for and accepts, per side (the helper letterboxes the video into it).
pub const MAX_SIDE: u32 = 1024;

/// What the engine asks of a source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameRequest {
    /// The most the frame may be wide and high, in pixels.
    pub max_w: u32,
    pub max_h: u32,
}

impl FrameRequest {
    /// The one request the engine makes: a frame no larger than [`MAX_SIDE`] square.
    pub const THUMBNAIL: FrameRequest = FrameRequest {
        max_w: MAX_SIDE,
        max_h: MAX_SIDE,
    };
}

/// One decoded frame, RGBA, 8 bits a channel.
pub struct Frame {
    pub rgba: Zeroizing<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

/// Why no frame came. The reason is never more than this: no path, no name and no byte of the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// No helper could be started (not installed, no sandbox here, the sandbox refused).
    NoHelper,
    /// The helper started and could not give a frame (a codec it does not decode, a damaged file, too long).
    NoFrame,
    /// The frame did not fit what was asked (a size over the limit, pixels that are not `w * h * 4`).
    BadFrame,
}

/// Gives the first frame of a video file. Implemented by the host, over the confined helper.
pub trait FirstFrameSource: Send + Sync {
    /// The first frame of the file at `path`, no larger than `request`. Blocks until it has one or refuses.
    fn first_frame(&self, path: &Path, request: FrameRequest) -> Result<Frame, Refused>;
}

static SOURCE: OnceLock<Arc<dyn FirstFrameSource>> = OnceLock::new();

/// Registers the process's source. `false`: one was registered already (it stays).
pub fn set_first_frame_source(source: Arc<dyn FirstFrameSource>) -> bool {
    SOURCE.set(source).is_ok()
}

/// The first frame of the video at `path` from the registered source, checked. No source: [`Refused::NoHelper`].
pub fn first_frame(path: &Path) -> Result<Frame, Refused> {
    first_frame_from(SOURCE.get().map(|s| &**s), path)
}

/// [`first_frame`] over a given source (the tests hand their own).
pub(crate) fn first_frame_from(source: Option<&dyn FirstFrameSource>, path: &Path) -> Result<Frame, Refused> {
    let source = source.ok_or(Refused::NoHelper)?;
    checked(
        source.first_frame(path, FrameRequest::THUMBNAIL)?,
        FrameRequest::THUMBNAIL,
    )
}

/// `frame` if its size is inside `request` and its pixels are exactly `w * h * 4`.
pub(crate) fn checked(frame: Frame, request: FrameRequest) -> Result<Frame, Refused> {
    let fits = frame.width > 0 && frame.height > 0 && frame.width <= request.max_w && frame.height <= request.max_h;
    let bytes = (frame.width as usize)
        .checked_mul(frame.height as usize)
        .and_then(|n| n.checked_mul(4));
    if fits && bytes == Some(frame.rgba.len()) {
        Ok(frame)
    } else {
        Err(Refused::BadFrame)
    }
}

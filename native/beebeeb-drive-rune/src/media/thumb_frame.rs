//! A video's thumbnail frame, asked of the confined media helper (the same sandboxed process the player uses).
//!
//! The engine makes thumbnails of videos it uploads and shows, and it parses and decodes nothing of a video: it asks
//! the [`FirstFrameSource`] this module registers ([`install`]). One call starts ONE helper for the ONE file, serves
//! the byte ranges the helper asks for (only those, read from the file at the path the engine gave; the helper gets
//! no name and no path), asks for the first frame, checks the frame against what was asked, and ends the helper.
//! A helper that cannot start, is not confined, refuses, breaks the protocol or is too slow is no frame (the file
//! keeps its type icon), never another program in its place.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use beebeeb_engine::video_frame_source::{FirstFrameSource, Frame, FrameRequest, Refused};
use beebeeb_media_protocol::{FromHelper, KindHint, MAX_FRAME_H, MAX_FRAME_W, MAX_READ, ToHelper};
use zeroize::Zeroizing;

use super::player::{Link, LinkFactory};
use super::transport::{LinkError, SandboxedHelpers, wipe, wipe_answer};

/// How long one message of the helper may take.
const STEP: Duration = Duration::from_secs(10);
/// How long the whole frame may take.
const TOTAL: Duration = Duration::from_secs(30);

/// The engine's source over the real, confined helper. Registered once (the first wins).
pub fn install() {
    let _ = beebeeb_engine::video_frame_source::set_first_frame_source(Arc::new(HelperFrames {
        factory: Arc::new(SandboxedHelpers),
        require_confined: true,
    }));
}

pub(crate) struct HelperFrames {
    pub(crate) factory: Arc<dyn LinkFactory>,
    /// A helper that does not run confined is killed unused. Only the tests' fake links turn this off.
    pub(crate) require_confined: bool,
}

impl FirstFrameSource for HelperFrames {
    fn first_frame(&self, path: &Path, request: FrameRequest) -> Result<Frame, Refused> {
        let mut file = File::open(path).map_err(|_| Refused::NoFrame)?;
        let len = file.metadata().map_err(|_| Refused::NoFrame)?.len();
        let mut link = self.factory.start().map_err(|_| Refused::NoHelper)?;
        if self.require_confined && !link.confined() {
            link.kill();
            return Err(Refused::NoHelper);
        }
        let result = ask(link.as_mut(), &mut file, len, request);
        link.kill();
        result
    }
}

/// The size to ask for: the video's own, shrunk to fit `request` and the protocol's frame limit, never zero.
fn asked_size(w: u32, h: u32, request: FrameRequest) -> (u32, u32) {
    let max_w = request.max_w.min(MAX_FRAME_W).max(1) as u64;
    let max_h = request.max_h.min(MAX_FRAME_H).max(1) as u64;
    let (w, h) = (w.max(1) as u64, h.max(1) as u64);
    if w <= max_w && h <= max_h {
        return (w as u32, h as u32);
    }
    // The smaller of the two ratios, in integers.
    if max_w * h <= max_h * w {
        (max_w as u32, (h * max_w / w).max(1) as u32)
    } else {
        ((w * max_h / h).max(1) as u32, max_h as u32)
    }
}

fn ask(link: &mut dyn Link, file: &mut File, len: u64, request: FrameRequest) -> Result<Frame, Refused> {
    let mut open = ToHelper::Open {
        len,
        hint: KindHint::Video,
    };
    link.send(&open).map_err(|_| Refused::NoFrame)?;
    wipe(&mut open);
    let started = Instant::now();
    let mut asked: Option<(u32, u32)> = None;
    loop {
        let left = TOTAL.checked_sub(started.elapsed()).ok_or(Refused::NoFrame)?;
        let mut msg = link.recv(STEP.min(left)).map_err(|e| match e {
            LinkError::Protocol(_) | LinkError::Timeout | LinkError::Gone => Refused::NoFrame,
        })?;
        let step = step(link, file, len, request, &mut asked, &msg);
        wipe_answer(&mut msg);
        if let Some(done) = step {
            return done;
        }
    }
}

/// One helper message: `Some` ends the call.
fn step(
    link: &mut dyn Link,
    file: &mut File,
    len: u64,
    request: FrameRequest,
    asked: &mut Option<(u32, u32)>,
    msg: &FromHelper,
) -> Option<Result<Frame, Refused>> {
    match msg {
        FromHelper::ReadAt { id, offset, len: want } => {
            let mut reply = match read_range(file, len, *offset, *want) {
                Some(bytes) => ToHelper::Data {
                    id: *id,
                    bytes: bytes.to_vec(),
                },
                None => ToHelper::DataFailed { id: *id },
            };
            let sent = link.send(&reply);
            wipe(&mut reply);
            sent.is_err().then_some(Err(Refused::NoFrame))
        }
        FromHelper::Opened(opened) => {
            let Some(video) = opened.video.filter(|_| asked.is_none()) else {
                return Some(Err(Refused::NoFrame));
            };
            let (w, h) = asked_size(video.w, video.h, request);
            *asked = Some((w, h));
            link.send(&ToHelper::WantVideo { w, h })
                .is_err()
                .then_some(Err(Refused::NoFrame))
        }
        FromHelper::Frame { w, h, rgba, .. } => {
            let (aw, ah) = (*asked)?;
            let exact = (*w, *h) == (aw, ah) && rgba.len() == aw as usize * ah as usize * 4;
            Some(if exact {
                Ok(Frame {
                    rgba: Zeroizing::new(rgba.clone()),
                    width: aw,
                    height: ah,
                })
            } else {
                Err(Refused::BadFrame)
            })
        }
        // Tags are of no use here; sound is never asked for.
        FromHelper::Tags(_) => None,
        FromHelper::Audio { .. } | FromHelper::Ended | FromHelper::Error(_) => Some(Err(Refused::NoFrame)),
    }
}

/// `want` bytes at `offset`, only when the whole range is inside the file and within the protocol's read limit.
fn read_range(file: &mut File, len: u64, offset: u64, want: u32) -> Option<Zeroizing<Vec<u8>>> {
    let end = offset.checked_add(u64::from(want))?;
    if want == 0 || want > MAX_READ || end > len {
        return None;
    }
    let mut bytes = Zeroizing::new(vec![0u8; want as usize]);
    file.seek(SeekFrom::Start(offset)).ok()?;
    file.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

#[cfg(test)]
#[path = "thumb_frame_tests.rs"]
mod tests;

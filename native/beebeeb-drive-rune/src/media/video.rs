//! The video's frames, into the toolkit's pixels (`rune::Pixels::set`, its streaming path: a frame of the same size
//! reuses its place in the atlas). Frames arrive at the size the app asked for (the drawn stage in device pixels,
//! capped at 1920 x 1080).
//!
//! On a stop the pixels are set to a frame of zeros of the same size, then released. The toolkit's own copy in its
//! atlas is overwritten only if the view is drawn again; otherwise the GPU page keeps the last frame until it is
//! reused (the gap the photo viewer states too; the media plan, (e)).
//!
//! Slice M6 (`media-surface`) fills it. The scaffold shows nothing.

/// The surface the player's frames go to; the view draws [`VideoSurface::pixels`].
pub struct VideoSurface {
    pixels: rune::Pixels,
}

impl Default for VideoSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoSurface {
    pub fn new() -> Self {
        VideoSurface {
            pixels: rune::Pixels::new(),
        }
    }

    /// Shows a `w` x `h` RGBA frame (exactly `w * h * 4` bytes, checked by the player).
    pub fn show(&self, w: u32, h: u32, rgba: Vec<u8>) {
        let _ = (w, h, rgba);
    }

    /// A frame of zeros of the same size, then the pixels let go.
    pub fn blank_and_release(&mut self) {}

    pub fn pixels(&self) -> rune::Pixels {
        self.pixels.clone()
    }
}

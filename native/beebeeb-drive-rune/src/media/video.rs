//! The video's frames, into the toolkit's pixels (`rune::Pixels::set`, its streaming path: a frame of the same size
//! reuses its place in the atlas). Frames arrive at the size the app asked for (the drawn stage in device pixels,
//! capped at 1920 x 1080).
//!
//! On a stop the pixels are set to a frame of zeros of the same size, then released. The toolkit's own copy in its
//! atlas is overwritten only if the view is drawn again; otherwise the GPU page keeps the last frame until it is
//! reused (the gap the photo viewer states too; the media plan, (e)).
//!

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
        self.pixels.set(w, h, rgba);
    }

    /// A frame of zeros of the same size, then the pixels let go.
    pub fn blank_and_release(&mut self) {
        let (w, h) = self.pixels.size();
        if w > 0 && h > 0 {
            // `w * h * 4` zero bytes: the same size, so the toolkit overwrites the place it already holds.
            self.pixels.set(w, h, vec![0u8; (w as usize) * (h as usize) * 4]);
        }
        // Let go of our clone; the view draws a fresh `Pixels` if it is shown again.
        self.pixels = rune::Pixels::new();
    }

    pub fn pixels(&self) -> rune::Pixels {
        self.pixels.clone()
    }
}

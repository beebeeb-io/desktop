//! Reading the live window: its element tree and its own pixels.
//!
//! The pixels are the window's own render, never a desktop screenshot: the frame's instances
//! are drawn a second time into an offscreen texture on the window's own device and read back.
//! Nothing outside this window can be in them.

use rune::compose::{Compositor, Frame};
use rune::{ElementInfo, Runtime};

use super::model::ElementSnap;
#[cfg(test)]
use super::model::Rgba;
use crate::State;

fn rgba(c: rune::Color) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

/// The element tree as of the last frame, with secure elements marked and their text dropped.
pub fn snapshot<S: 'static>(rt: &Runtime<S>) -> (Vec<ElementSnap>, bool) {
    let infos: Vec<ElementInfo> = rt.elements();
    let mut rigid = true;
    let tree = infos
        .iter()
        .map(|e| {
            let secure = rt.element_is_secure(&e.path);
            if secure && (e.scale - 1.0).abs() > 0.001 {
                rigid = false;
            }
            ElementSnap {
                path: e.path.clone(),
                key: e.key.clone(),
                role: e.role.to_string(),
                label: e.label.clone(),
                text: if secure { None } else { e.text.clone() },
                rect: e.rect,
                visible: e.visible,
                fill: rgba(e.fill),
                text_color: e.text_color.map(rgba),
                radius: e.radius,
                opacity: e.opacity,
                secure,
                focused: e.focused,
                parent: e.parent,
            }
        })
        .collect();
    (tree, rigid)
}

/// Draws `$rt`'s current instances into an offscreen target of `$format` on `$device` with the
/// renderer `$ui`, and reads it back as tightly packed RGBA8. A macro so the window's frame and
/// a headless GPU share it without this crate naming wgpu's types.
macro_rules! render_rgba {
    ($ui:expr, $device:expr, $queue:expr, $rt:expr, $size:expr, $format:expr) => {{
        let (w, h): (u32, u32) = $size;
        let target = rune::render::Offscreen::with_format($device, w, h, $format);
        let clear = $rt.clear_color();
        $ui.prepare($device, $queue, &mut $rt.text.atlas, &$rt.instances, (w, h));
        let mut encoder = $device.create_command_encoder(&Default::default());
        $ui.draw(&mut encoder, &target.view, clear);
        target.copy(&mut encoder);
        $ui.submit($queue, encoder);
        let mut px = target.read($device);
        if format!("{:?}", $format).starts_with("Bgra") {
            for p in px.chunks_exact_mut(4) {
                p.swap(0, 2);
            }
        }
        $crate::feedback::model::Rgba { w, h, px }
    }};
}

/// A headless render of `rt` as it stands, for tests and evidence: the same renderer and the
/// same instances the window draws.
#[cfg(test)]
pub fn render_headless<S: 'static>(rt: &mut Runtime<S>, scale: f32) -> Rgba {
    use rune::render::{Gpu, OFFSCREEN_FORMAT, Renderer};
    // One device for the whole test process: a thread-local one is torn down while wgpu's own
    // thread-locals are already gone.
    static GPU: std::sync::OnceLock<std::sync::Mutex<Gpu>> = std::sync::OnceLock::new();
    let gpu = GPU
        .get_or_init(|| std::sync::Mutex::new(Gpu::headless()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (vw, vh) = rt.window_viewport();
    let size = ((vw * scale).round() as u32, (vh * scale).round() as u32);
    // A fresh renderer has none of this runtime's glyphs yet: mark the whole atlas for upload.
    // (The window never needs this: it captures with its own renderer.)
    rt.text.atlas.invalidate();
    let mut ui = Renderer::new(&gpu.device, OFFSCREEN_FORMAT);
    render_rgba!(ui, &gpu.device, &gpu.queue, rt, size, OFFSCREEN_FORMAT)
}

/// The window's compositor while the `feedback` feature is on: the default frame, plus the
/// element pick under the pointer and, on the one frame a note is saved, the capture.
#[derive(Default)]
pub struct FeedbackCompositor {
    again: bool,
}

impl Compositor<State> for FeedbackCompositor {
    fn render(&mut self, f: &mut Frame<'_, State>) {
        self.again = super::sync(f.rt);
        if super::capture_due(&f.rt.state) {
            let img = render_rgba!(f.ui, f.device, f.queue, f.rt, f.size, f.format);
            super::finish_capture(f.rt, img);
            self.again = true;
        }
        if self.again {
            f.rt.invalidate();
        }
        f.draw_ui();
    }

    /// One more frame after a pick or a capture changed what the overlay shows.
    fn animating(&self) -> bool {
        self.again
    }
}

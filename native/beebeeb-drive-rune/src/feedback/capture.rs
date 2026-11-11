//! Reading the live window: its element tree and its own pixels.
//!
//! The pixels are the window's own render, never a desktop screenshot: the frame's instances
//! are drawn a second time into an offscreen texture on the window's own device and read back.
//! Nothing outside this window can be in them.

use rune::compose::{Compositor, Frame};
use rune::render::{Offscreen, Renderer};
use rune::{ElementInfo, Runtime};

use super::model::{ElementSnap, Rgba};
use crate::State;

/// A frame lane wraps a group of secret-bearing elements in one element whose key starts with
/// this: the whole subtree is redacted and the whole box blanked, whatever is inside.
pub const SECURE_KEY_PREFIX: &str = "secure-";

fn rgba(c: rune::Color) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

/// The element tree as of the last frame. An element is secure when Rune says so (`El::secure`,
/// a secure `TextEdit`), when its key starts with [`SECURE_KEY_PREFIX`], or when any ancestor is:
/// secure elements have no text here. The flag says whether every secure box is drawn unscaled.
pub fn snapshot<S: 'static>(rt: &Runtime<S>) -> (Vec<ElementSnap>, bool) {
    let infos: Vec<ElementInfo> = rt.elements();
    let mut rigid = true;
    let mut tree: Vec<ElementSnap> = Vec::with_capacity(infos.len());
    for (i, e) in infos.iter().enumerate() {
        let own = rt.element_is_secure(&e.path) || e.key.as_deref().is_some_and(|k| k.starts_with(SECURE_KEY_PREFIX));
        // A parent comes before its children in Rune's list; one that does not is treated as secure.
        let inherited = e.parent.is_some_and(|p| p >= i || tree[p].secure);
        let secure = own || inherited;
        if secure && (e.scale - 1.0).abs() > 0.001 {
            rigid = false;
        }
        tree.push(ElementSnap {
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
            moving: e.moving,
            exiting: e.exiting,
            parent: e.parent,
        });
    }
    (tree, rigid)
}

/// Whether a readback of `format` has blue first. Only the two 8-bit surface formats are known;
/// anything else is refused rather than written as garbage.
pub fn blue_first(format: wgpu::TextureFormat) -> Result<bool, String> {
    match format.remove_srgb_suffix() {
        wgpu::TextureFormat::Rgba8Unorm => Ok(false),
        wgpu::TextureFormat::Bgra8Unorm => Ok(true),
        other => Err(format!("this window's surface format ({other:?}) cannot be captured")),
    }
}

/// Draws `rt`'s current instances into an offscreen target of `format` on `device` with the
/// renderer `ui`, and reads it back as tightly packed RGBA8.
pub fn render_rgba<S: 'static>(
    ui: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    rt: &mut Runtime<S>,
    (w, h): (u32, u32),
    format: wgpu::TextureFormat,
) -> Result<Rgba, String> {
    let swap = blue_first(format)?;
    let target = Offscreen::with_format(device, w, h, format);
    let clear = rt.clear_color();
    ui.prepare(device, queue, &mut rt.text.atlas, &rt.instances, (w, h));
    let mut encoder = device.create_command_encoder(&Default::default());
    ui.draw(&mut encoder, &target.view, clear);
    target.copy(&mut encoder);
    ui.submit(queue, encoder);
    let mut px = target.read(device);
    if swap {
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
    }
    Ok(Rgba { w, h, px })
}

/// A headless render of `rt` as it stands, for tests and evidence: the same renderer and the
/// same instances the window draws.
#[cfg(test)]
pub fn render_headless<S: 'static>(rt: &mut Runtime<S>, scale: f32) -> Rgba {
    use rune::render::{Gpu, OFFSCREEN_FORMAT};
    // One device for the whole test process: a thread-local one is torn down while wgpu's own
    // thread-locals are already gone.
    static GPU: std::sync::OnceLock<std::sync::Mutex<Gpu>> = std::sync::OnceLock::new();
    let gpu = GPU
        .get_or_init(|| std::sync::Mutex::new(Gpu::headless()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (vw, vh) = rt.window_viewport();
    let size = ((vw * scale).round() as u32, (vh * scale).round() as u32);
    // A fresh renderer has none of this runtime's glyphs or images yet: mark them all for upload.
    // (The window never needs this: it captures with its own renderer.)
    rt.text.atlas.invalidate();
    let mut ui = Renderer::new(&gpu.device, OFFSCREEN_FORMAT);
    rt.images.invalidate();
    ui.upload_images(&gpu.device, &gpu.queue, &mut rt.images);
    render_rgba(&mut ui, &gpu.device, &gpu.queue, rt, size, OFFSCREEN_FORMAT).expect("the offscreen format is RGBA")
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
            let img = render_rgba(f.ui, f.device, f.queue, f.rt, f.size, f.format);
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

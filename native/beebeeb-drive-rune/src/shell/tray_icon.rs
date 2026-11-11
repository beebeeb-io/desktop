//! The b's pixels: one icon per state and taskbar mode (DW-1230), drawn from the repo's one b
//! glyph (`assets/tray/b-template@2x.png`, the 0.8.x tray template) with the state's badge
//! drawn in. Pure: state, mode, size and animation step in, straight-alpha RGBA out, so every
//! icon can be rendered and looked at without a taskbar.
//!
//! Windows does not tint notification-area icons, so the b is white on a dark taskbar and
//! near-black on a light one. Badges sit bottom-right on a cleared ring: paused and syncing in
//! the b's own colour, "needs a decision" in orange (amber-deep mixed with red), "stopped" in
//! the red token. Offline is the b faded with a stroke through it.

use std::sync::OnceLock;

use super::tray::TrayIconState;
use crate::theme::tokens;

static TEMPLATE_PNG: &[u8] = include_bytes!("../../assets/tray/b-template@2x.png");

/// The animation has this many steps per turn (one turn a second: DW-1230).
pub const SYNC_STEPS: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TaskbarTheme {
    Dark,
    Light,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    /// Straight (not premultiplied) RGBA rows.
    pub pixels: Vec<u8>,
}

struct Template {
    size: u32,
    alpha: Vec<u8>,
}

fn template() -> &'static Template {
    static TEMPLATE: OnceLock<Template> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let mut reader = png::Decoder::new(std::io::Cursor::new(TEMPLATE_PNG))
            .read_info()
            .expect("the bundled b is a PNG");
        let mut buf = vec![0; reader.output_buffer_size().expect("the bundled b is small")];
        let info = reader.next_frame(&mut buf).expect("the bundled b decodes");
        assert_eq!(
            (info.color_type, info.bit_depth),
            (png::ColorType::Rgba, png::BitDepth::Eight),
            "the bundled b is RGBA8"
        );
        assert_eq!(info.width, info.height, "the bundled b is square");
        Template {
            size: info.width,
            alpha: buf[..(info.width * info.height * 4) as usize]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[3])
                .collect(),
        }
    })
}

/// The glyph's coverage at a point of the unit square, bilinear. Outside it: nothing.
fn glyph_at(u: f32, v: f32) -> f32 {
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return 0.0;
    }
    let t = template();
    let n = t.size as f32;
    let (x, y) = ((u * n - 0.5).clamp(0.0, n - 1.0), (v * n - 0.5).clamp(0.0, n - 1.0));
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(t.size as usize - 1), (y0 + 1).min(t.size as usize - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |x: usize, y: usize| f32::from(t.alpha[y * t.size as usize + x]) / 255.0;
    let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
    let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// The glyph's coverage of the output pixel at `p`, when the glyph is drawn `extent` pixels
/// wide from the top-left corner: nine samples, so a 16 px icon is an average, not a pick.
fn glyph(p: (f32, f32), extent: f32) -> f32 {
    let mut sum = 0.0;
    for dy in [-1.0_f32, 0.0, 1.0] {
        for dx in [-1.0_f32, 0.0, 1.0] {
            sum += glyph_at((p.0 + dx / 3.0) / extent, (p.1 + dy / 3.0) / extent);
        }
    }
    sum / 9.0
}

fn srgb8(linear: f32) -> u8 {
    let c = linear.clamp(0.0, 1.0);
    let encoded = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

fn token_rgb(color: rune::Color) -> [u8; 3] {
    [srgb8(color.r), srgb8(color.g), srgb8(color.b)]
}

/// Coverage of a shape from its signed distance in pixels (negative inside).
fn cover(distance_px: f32) -> f32 {
    (0.5 - distance_px).clamp(0.0, 1.0)
}

/// Distance from `p` to the segment `a`-`b`.
fn segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (px, py, bx, by) = (p.0 - a.0, p.1 - a.1, b.0 - a.0, b.1 - a.1);
    let h = ((px * bx + py * by) / (bx * bx + by * by)).clamp(0.0, 1.0);
    ((px - bx * h).powi(2) + (py - by * h).powi(2)).sqrt()
}

/// Source-over of a straight-alpha colour onto a straight-alpha pixel.
fn over(dst: &mut [f32; 4], rgb: [u8; 3], a: f32) {
    let out_a = a + dst[3] * (1.0 - a);
    if out_a <= 0.0 {
        return;
    }
    for i in 0..3 {
        dst[i] = (f32::from(rgb[i]) / 255.0 * a + dst[i] * dst[3] * (1.0 - a)) / out_a;
    }
    dst[3] = out_a;
}

/// One icon. `size` is the icon's edge in physical pixels (16 at 100%, 24 at 150%, 32 at 200%);
/// `step` turns the syncing badge (`0..SYNC_STEPS`) and is ignored by the other states.
pub fn render(state: TrayIconState, theme: TaskbarTheme, size: u32, step: u8) -> Rgba {
    let size = size.clamp(8, 256);
    let n = size as f32;
    let ink: [u8; 3] = match theme {
        TaskbarTheme::Dark => [255, 255, 255],
        TaskbarTheme::Light => [26, 26, 26],
    };
    let palette = tokens::theme(theme == TaskbarTheme::Dark);
    let red = token_rgb(palette.red);
    let orange = token_rgb(tokens::amber_deep().mix(palette.red, 0.5));
    let badge = !matches!(state, TrayIconState::UpToDate | TrayIconState::Offline);
    // Badge: a disc in the bottom-right corner, on a cleared ring one pixel-unit wide. The b
    // is drawn a little smaller beside it, from the top-left, so it still reads as a b.
    let (cx, cy, r) = (n * 0.75, n * 0.75, n * 0.24);
    let extent = if badge { n * 0.86 } else { n };
    let unit = n / 16.0;
    let turn = f32::from(step % SYNC_STEPS) / f32::from(SYNC_STEPS) * std::f32::consts::TAU;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let mut px = [0.0_f32; 4];
            let mut b = glyph(p, extent);
            let d = ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt();
            if state == TrayIconState::Offline {
                // Faded, with a stroke from bottom-left to top-right on its own cleared band.
                let stroke = segment(p, (n * 0.16, n * 0.88), (n * 0.84, n * 0.12));
                b = b * 0.45 * (1.0 - cover(stroke - unit * 1.6));
                over(&mut px, ink, b);
                over(&mut px, ink, cover(stroke - unit * 0.7));
            } else if badge {
                b *= 1.0 - cover(d - (r + unit));
                over(&mut px, ink, b);
                let disc = cover(d - r);
                match state {
                    TrayIconState::Paused => {
                        // A disc with two bars knocked out.
                        let bar = |bx: f32| {
                            let dx = (p.0 - (cx + bx)).abs() - r * 0.16;
                            let dy = (p.1 - cy).abs() - r * 0.48;
                            cover(dx.max(dy))
                        };
                        over(&mut px, ink, disc * (1.0 - bar(-r * 0.30).max(bar(r * 0.30))));
                    }
                    TrayIconState::Syncing => {
                        // A three-quarter ring; the gap turns with `step`.
                        let ring = cover((d - r * 0.72).abs() - r * 0.24);
                        let angle = (p.1 - cy).atan2(p.0 - cx) - turn;
                        let gap = angle.rem_euclid(std::f32::consts::TAU) < std::f32::consts::FRAC_PI_2;
                        over(&mut px, ink, if gap { 0.0 } else { ring });
                    }
                    _ => {
                        let fill = if state == TrayIconState::Stopped { red } else { orange };
                        over(&mut px, fill, disc);
                        // A white "!": a stem and a dot.
                        let stem = segment(p, (cx, cy - r * 0.50), (cx, cy + r * 0.04)) - r * 0.15;
                        let dot = ((p.0 - cx).powi(2) + (p.1 - (cy + r * 0.52)).powi(2)).sqrt() - r * 0.17;
                        over(&mut px, [255, 255, 255], cover(stem.min(dot)) * disc);
                    }
                }
            } else {
                over(&mut px, ink, b);
            }
            pixels.extend(px.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8));
        }
    }
    Rgba {
        width: size,
        height: size,
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_sum(icon: &Rgba) -> u64 {
        icon.pixels.as_chunks::<4>().0.iter().map(|p| u64::from(p[3])).sum()
    }
    fn count(icon: &Rgba, pred: impl Fn(&[u8]) -> bool) -> usize {
        icon.pixels.as_chunks::<4>().0.iter().filter(|p| pred(*p)).count()
    }

    #[test]
    fn every_state_mode_and_size_renders_a_full_buffer() {
        for state in TrayIconState::ALL {
            for theme in [TaskbarTheme::Dark, TaskbarTheme::Light] {
                for size in [16, 20, 24, 32, 40, 48, 64] {
                    let icon = render(state, theme, size, 0);
                    assert_eq!(icon.pixels.len(), (size * size * 4) as usize);
                    assert!(alpha_sum(&icon) > 0, "{state:?} {theme:?} {size} draws something");
                }
            }
        }
    }

    #[test]
    fn the_six_states_are_six_different_icons_in_both_modes() {
        for theme in [TaskbarTheme::Dark, TaskbarTheme::Light] {
            let icons: Vec<Rgba> = TrayIconState::ALL.iter().map(|s| render(*s, theme, 32, 0)).collect();
            for a in 0..icons.len() {
                for b in (a + 1)..icons.len() {
                    assert_ne!(
                        icons[a],
                        icons[b],
                        "{:?} vs {:?}",
                        TrayIconState::ALL[a],
                        TrayIconState::ALL[b]
                    );
                }
            }
        }
    }

    #[test]
    fn the_b_is_white_on_a_dark_taskbar_and_dark_on_a_light_one() {
        let dark = render(TrayIconState::UpToDate, TaskbarTheme::Dark, 48, 0);
        let light = render(TrayIconState::UpToDate, TaskbarTheme::Light, 48, 0);
        let white = count(&dark, |p| p[3] >= 250 && p[0] == 255 && p[1] == 255 && p[2] == 255);
        let ink = count(&light, |p| p[3] >= 250 && p[0] == 26 && p[1] == 26 && p[2] == 26);
        assert!(white > 100, "{white} solid white pixels");
        assert_eq!(white, ink);
        // No pixel of the dark-taskbar b is anything but white, at any alpha.
        assert_eq!(
            count(&dark, |p| p[3] > 0 && (p[0] < 254 || p[1] < 254 || p[2] < 254)),
            0
        );
        // Same shape, other ink.
        assert_eq!(alpha_sum(&dark), alpha_sum(&light));
    }

    #[test]
    fn stopped_is_red_and_needs_a_decision_is_orange() {
        let reddish = |p: &[u8]| p[3] > 200 && p[0] > 150 && p[1] < 110 && p[2] < 110;
        let stopped = render(TrayIconState::Stopped, TaskbarTheme::Dark, 32, 0);
        let decision = render(TrayIconState::NeedsDecision, TaskbarTheme::Dark, 32, 0);
        assert!(
            count(&stopped, reddish) > 80,
            "a red badge: {}",
            count(&stopped, reddish)
        );
        assert_eq!(
            count(&render(TrayIconState::Paused, TaskbarTheme::Dark, 32, 0), reddish),
            0
        );
        // Orange: clearly more green than the red badge has, still warm.
        let warm = |p: &[u8]| p[3] > 200 && p[0] > 120 && p[0] > p[1] && p[1] > p[2] && p[0] != p[1];
        assert!(count(&decision, warm) > 80);
        assert_ne!(stopped, decision);
    }

    #[test]
    fn offline_is_fainter_than_the_plain_b() {
        let plain = render(TrayIconState::UpToDate, TaskbarTheme::Dark, 32, 0);
        let offline = render(TrayIconState::Offline, TaskbarTheme::Dark, 32, 0);
        // The plain b is solid ink; the offline b is the same shape at under half the alpha,
        // with only the stroke solid.
        let faint = |icon: &Rgba| count(icon, |p| (60..=130).contains(&p[3]));
        let solid = |icon: &Rgba| count(icon, |p| p[3] >= 250);
        assert!(
            faint(&offline) > faint(&plain) * 3,
            "{} vs {}",
            faint(&offline),
            faint(&plain)
        );
        assert!(solid(&offline) > 10, "the stroke is solid");
        assert!(alpha_sum(&offline) < alpha_sum(&plain));
    }

    #[test]
    fn the_syncing_badge_turns_and_comes_back_after_a_full_turn() {
        let frames: Vec<Rgba> = (0..SYNC_STEPS)
            .map(|step| render(TrayIconState::Syncing, TaskbarTheme::Dark, 32, step))
            .collect();
        for step in 1..frames.len() {
            assert_ne!(frames[step - 1], frames[step], "step {step} moved");
        }
        assert_eq!(
            render(TrayIconState::Syncing, TaskbarTheme::Dark, 32, SYNC_STEPS),
            frames[0]
        );
        // The other states ignore the step.
        assert_eq!(
            render(TrayIconState::Paused, TaskbarTheme::Dark, 32, 3),
            render(TrayIconState::Paused, TaskbarTheme::Dark, 32, 0)
        );
    }

    #[test]
    fn absurd_sizes_are_clamped() {
        assert_eq!(render(TrayIconState::UpToDate, TaskbarTheme::Dark, 0, 0).width, 8);
        assert_eq!(
            render(TrayIconState::UpToDate, TaskbarTheme::Dark, 100_000, 0).width,
            256
        );
    }
}

/// Writes every icon to one sheet, to be looked at: `BB_SHELL_EVIDENCE_DIR=<dir> cargo test
/// tray_icon_sheet`. Each state in both taskbar modes at 16, 24, 32 and 48 px, enlarged, on
/// the taskbar colour it is for.
#[cfg(test)]
mod sheet {
    use super::*;

    #[test]
    fn tray_icon_sheet() {
        let Ok(dir) = std::env::var("BB_SHELL_EVIDENCE_DIR") else {
            return;
        };
        let sizes = [16u32, 24, 32, 48];
        let zoom = 4u32;
        let cell = 48 * zoom + 16;
        let (cols, rows) = (TrayIconState::ALL.len() as u32, 2 * sizes.len() as u32);
        let (w, h) = (cols * cell, rows * cell);
        let mut sheet = vec![0u8; (w * h * 4) as usize];
        for (theme_i, theme) in [TaskbarTheme::Dark, TaskbarTheme::Light].into_iter().enumerate() {
            let bg: [u8; 3] = if theme == TaskbarTheme::Dark {
                [32, 32, 32]
            } else {
                [243, 243, 243]
            };
            for (size_i, size) in sizes.into_iter().enumerate() {
                let row = (theme_i * sizes.len() + size_i) as u32;
                for (col, state) in TrayIconState::ALL.into_iter().enumerate() {
                    let icon = render(state, theme, size, 1);
                    let (ox, oy) = (col as u32 * cell, row * cell);
                    for y in 0..cell {
                        for x in 0..cell {
                            let i = (((oy + y) * w + ox + x) * 4) as usize;
                            let mut px = [bg[0], bg[1], bg[2], 255];
                            let (ix, iy) = (x.saturating_sub(8) / zoom, y.saturating_sub(8) / zoom);
                            if x >= 8 && y >= 8 && ix < size && iy < size {
                                let p = &icon.pixels[((iy * size + ix) * 4) as usize..][..4];
                                let a = f32::from(p[3]) / 255.0;
                                for c in 0..3 {
                                    px[c] = (f32::from(p[c]) * a + f32::from(bg[c]) * (1.0 - a)).round() as u8;
                                }
                            }
                            sheet[i..i + 4].copy_from_slice(&px);
                        }
                    }
                }
            }
        }
        let path = std::path::Path::new(&dir).join("tray-icons.png");
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&path).unwrap()), w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&sheet).unwrap();
        eprintln!("tray icon sheet: {} states x {} rows -> {}", cols, rows, path.display());
    }
}

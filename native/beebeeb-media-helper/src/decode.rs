//! A decoded frame made what the app asked for: scaled to fit `w × h`, padded with black to exactly `w × h`, and
//! converted to RGBA with the stream's own matrix and range (FilmCraft's `VideoFrame::to_rgba8` does the colour);
//! PCM mixed to the asked channels. Pure functions.

/// The size a frame is SHOWN at: its pixel size with the pixel aspect ratio applied to the width.
pub fn display_size(w: u32, h: u32, par: (u32, u32)) -> (u32, u32) {
    if par.0 == 0 || par.1 == 0 {
        return (w.max(1), h.max(1));
    }
    let shown = (u64::from(w) * u64::from(par.0) / u64::from(par.1)).clamp(1, u64::from(u32::MAX));
    (shown as u32, h.max(1))
}

/// The largest size with the aspect of `display` that fits inside `dw × dh` (both at least 1).
pub fn fit(display: (u32, u32), dw: u32, dh: u32) -> (u32, u32) {
    let (aw, ah) = (u64::from(display.0.max(1)), u64::from(display.1.max(1)));
    let (dw, dh) = (u64::from(dw.max(1)), u64::from(dh.max(1)));
    if dw * ah <= dh * aw {
        let h = ((ah * dw + aw / 2) / aw).clamp(1, dh);
        (dw as u32, h as u32)
    } else {
        let w = ((aw * dh + ah / 2) / ah).clamp(1, dw);
        (w as u32, dh as u32)
    }
}

/// The scaler's weights are fixed point with this many bits: a channel times two weights still fits 32 bits.
const WEIGHT_BITS: u32 = 11;
const WEIGHT_ONE: u32 = 1 << WEIGHT_BITS;

/// The two source positions (columns or rows) an output position is mixed from, and the weight of the second (of
/// [`WEIGHT_ONE`]): the sample at the centre of the output position; the edges clamp.
struct Tap {
    at0: usize,
    at1: usize,
    w1: u32,
}

fn taps(src: usize, dst: usize) -> Vec<Tap> {
    let step = src as f32 / dst as f32;
    (0..dst)
        .map(|i| {
            let f = ((i as f32 + 0.5) * step - 0.5).clamp(0.0, (src - 1) as f32);
            let at0 = f.floor() as usize;
            Tap {
                at0,
                at1: (at0 + 1).min(src - 1),
                w1: ((f - at0 as f32) * WEIGHT_ONE as f32).round() as u32,
            }
        })
        .collect()
}

/// One source row mixed across to the output's width: each channel times [`WEIGHT_ONE`].
fn across(row: &[u8], xs: &[Tap], out: &mut [u32]) {
    for (tap, px) in xs.iter().zip(out.chunks_exact_mut(4)) {
        let a = &row[tap.at0 * 4..tap.at0 * 4 + 4];
        let b = &row[tap.at1 * 4..tap.at1 * 4 + 4];
        let (w0, w1) = (WEIGHT_ONE - tap.w1, tap.w1);
        for c in 0..4 {
            px[c] = u32::from(a[c]) * w0 + u32::from(b[c]) * w1;
        }
    }
}

/// `src` (RGBA, `sw × sh`) scaled to fit `dw × dh` with the aspect of `display`, centred, the rest opaque black:
/// exactly `dw × dh × 4` bytes. `None` for a zero size or a `src` of the wrong length.
///
/// Bilinear, in two passes of integer arithmetic: each source row is mixed across once (and kept while the output
/// rows that need it follow each other), then two such rows are mixed down. This runs once per frame shown, at the
/// size of the window: the per-channel floating-point loop it replaces took about 54 ns a pixel, which at a
/// 1578 × 866 stage was 60 ms of every frame of a 640 × 360 video (w12-video-start, VM 401: 13 frames a second).
pub fn scale_letterbox(src: &[u8], sw: u32, sh: u32, display: (u32, u32), dw: u32, dh: u32) -> Option<Vec<u8>> {
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() != sw.checked_mul(sh)?.checked_mul(4)? {
        return None;
    }
    let (fw, fh) = fit(display, dw as u32, dh as u32);
    let (fw, fh) = (fw as usize, fh as usize);
    let (ox, oy) = ((dw - fw) / 2, (dh - fh) / 2);
    let mut out = vec![0u8; dw * dh * 4];
    // The bars: opaque black. The picture's own pixels are written below.
    for (y, row) in out.chunks_exact_mut(dw * 4).enumerate() {
        let picture = if (oy..oy + fh).contains(&y) { ox..ox + fw } else { 0..0 };
        for (x, px) in row.chunks_exact_mut(4).enumerate() {
            if !picture.contains(&x) {
                px[3] = 255;
            }
        }
    }
    let xs = taps(sw, fw);
    let ys = taps(sh, fh);
    // The two source rows mixed across last, and which rows they are.
    let mut mixed = [vec![0u32; fw * 4], vec![0u32; fw * 4]];
    let mut held = [usize::MAX; 2];
    let half = 1u32 << (2 * WEIGHT_BITS - 1);
    for (y, tap) in ys.iter().enumerate() {
        let mut slot_of = |row: usize, keep: usize, mixed: &mut [Vec<u32>; 2], held: &mut [usize; 2]| -> usize {
            if let Some(slot) = held.iter().position(|have| *have == row) {
                return slot;
            }
            // Not the slot of the row this output row also needs.
            let slot = usize::from(held[0] == keep);
            across(&src[row * sw * 4..(row + 1) * sw * 4], &xs, &mut mixed[slot]);
            held[slot] = row;
            slot
        };
        let top = slot_of(tap.at0, tap.at1, &mut mixed, &mut held);
        let bottom = slot_of(tap.at1, tap.at0, &mut mixed, &mut held);
        let (w0, w1) = (WEIGHT_ONE - tap.w1, tap.w1);
        let at = ((y + oy) * dw + ox) * 4;
        let picture = &mut out[at..at + fw * 4];
        for ((px, t), b) in picture.iter_mut().zip(&mixed[top]).zip(&mixed[bottom]) {
            *px = ((t * w0 + b * w1 + half) >> (2 * WEIGHT_BITS)) as u8;
        }
    }
    Some(out)
}

/// Planar channels mixed to `out_channels` and interleaved, at most `max_frames` frames. A non-finite sample is
/// silence. Same count: copied; one to many: spread; many to one: the average; many to two: even channels left, odd
/// right (averaged); any other: the first channels, the rest silent.
pub fn mix_channels(channels: &[Vec<f32>], out_channels: u16, max_frames: usize) -> Vec<f32> {
    let out_n = usize::from(out_channels);
    let frames = channels.iter().map(Vec::len).min().unwrap_or(0).min(max_frames);
    if out_n == 0 || channels.is_empty() {
        return Vec::new();
    }
    let in_n = channels.len();
    let sample = |c: usize, i: usize| -> f32 {
        let s = channels[c][i];
        if s.is_finite() { s } else { 0.0 }
    };
    let mut out = vec![0.0f32; frames * out_n];
    for i in 0..frames {
        for c in 0..out_n {
            out[i * out_n + c] = if in_n == out_n {
                sample(c, i)
            } else if in_n == 1 {
                sample(0, i)
            } else if out_n == 1 {
                (0..in_n).map(|k| sample(k, i)).sum::<f32>() / in_n as f32
            } else if out_n == 2 {
                let group: Vec<usize> = (0..in_n).filter(|k| k % 2 == c).collect();
                group.iter().map(|&k| sample(k, i)).sum::<f32>() / group.len().max(1) as f32
            } else if c < in_n {
                sample(c, i)
            } else {
                0.0
            };
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        rgba.iter().copied().cycle().take((w * h * 4) as usize).collect()
    }

    #[test]
    fn scale_letterbox_to_the_exact_size() {
        // 4x2 white into 8x8: fits as 8x4, centred: rows 0-1 and 6-7 black, rows 2-5 white.
        let out = scale_letterbox(&solid(4, 2, [255; 4]), 4, 2, (4, 2), 8, 8).expect("scaled");
        assert_eq!(out.len(), 8 * 8 * 4);
        let px = |x: usize, y: usize| &out[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4];
        for x in 0..8 {
            for y in [0, 1, 6, 7] {
                assert_eq!(px(x, y), [0, 0, 0, 255], "bar at {x},{y}");
            }
            for y in 2..6 {
                assert_eq!(px(x, y), [255, 255, 255, 255], "picture at {x},{y}");
            }
        }
    }

    /// The scaler this one replaced: bilinear in floating point, a channel at a time. Its output is what the fast one
    /// is held to.
    fn reference(src: &[u8], sw: u32, sh: u32, display: (u32, u32), dw: u32, dh: u32) -> Option<Vec<u8>> {
        let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
        if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() != sw.checked_mul(sh)?.checked_mul(4)? {
            return None;
        }
        let (fw, fh) = fit(display, dw as u32, dh as u32);
        let (fw, fh) = (fw as usize, fh as usize);
        let (ox, oy) = ((dw - fw) / 2, (dh - fh) / 2);
        let mut out = vec![0u8; dw * dh * 4];
        for px in out.chunks_exact_mut(4) {
            px[3] = 255;
        }
        // Bilinear sampling at the centre of each output pixel; the edges clamp.
        let xs = sw as f32 / fw as f32;
        let ys = sh as f32 / fh as f32;
        for y in 0..fh {
            let fy = ((y as f32 + 0.5) * ys - 0.5).clamp(0.0, (sh - 1) as f32);
            let y0 = fy.floor() as usize;
            let y1 = (y0 + 1).min(sh - 1);
            let ty = fy - y0 as f32;
            for x in 0..fw {
                let fx = ((x as f32 + 0.5) * xs - 0.5).clamp(0.0, (sw - 1) as f32);
                let x0 = fx.floor() as usize;
                let x1 = (x0 + 1).min(sw - 1);
                let tx = fx - x0 as f32;
                let at = |px: usize, py: usize, c: usize| f32::from(src[(py * sw + px) * 4 + c]);
                let dst = ((y + oy) * dw + x + ox) * 4;
                for c in 0..4 {
                    let top = at(x0, y0, c) * (1.0 - tx) + at(x1, y0, c) * tx;
                    let bottom = at(x0, y1, c) * (1.0 - tx) + at(x1, y1, c) * tx;
                    out[dst + c] = (top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Some(out)
    }

    /// A picture with something in every pixel: a different ramp per channel and a checker of single pixels.
    fn busy(w: usize, h: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let checker = if (x + y) % 2 == 0 { 255 } else { 0 };
                out.extend_from_slice(&[(x * 7 + y * 3) as u8, (x * 255 / w.max(1)) as u8, checker, 255]);
            }
        }
        out
    }

    /// w12-video-start: the fast scaler gives the picture of the one it replaced, to within one level a channel
    /// (its weights have 11 bits), larger and smaller, with bars at the sides and above and below, with a pixel
    /// aspect; a frame asked at its own size comes back as it is.
    #[test]
    fn the_fast_scaler_gives_the_reference_picture() {
        for (sw, sh, display, dw, dh) in [
            (64usize, 36usize, (64u32, 36u32), 157u32, 86u32),
            (64, 36, (64, 36), 200, 86),
            (64, 36, (64, 36), 157, 200),
            (64, 36, (64, 36), 31, 17),
            (64, 36, (64, 36), 9, 40),
            (72, 57, (77, 57), 160, 90),
            (3, 2, (3, 2), 1, 1),
            (1, 1, (1, 1), 7, 5),
            (64, 36, (64, 36), 64, 36),
        ] {
            let src = busy(sw, sh);
            let fast = scale_letterbox(&src, sw as u32, sh as u32, display, dw, dh).expect("scaled");
            let slow = reference(&src, sw as u32, sh as u32, display, dw, dh).expect("scaled");
            assert_eq!(fast.len(), slow.len());
            let worst = fast.iter().zip(&slow).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            assert!(worst <= 1, "{sw}x{sh} into {dw}x{dh}: a channel is {worst} levels off");
            if (sw, sh) == (dw as usize, dh as usize) {
                assert_eq!(fast, src, "its own size: the frame as it is");
            }
        }
    }

    /// A measurement: `cargo test --release --lib scaler_speed -- --ignored --nocapture`. A 640 x 360 frame to the
    /// sizes a window asks for, the fast scaler against the one it replaced.
    #[test]
    #[ignore = "a measurement, not a gate"]
    fn scaler_speed() {
        let src = busy(640, 360);
        for (dw, dh) in [(1036u32, 582u32), (1578, 866), (1920, 1080)] {
            let time = |f: &dyn Fn() -> Option<Vec<u8>>| {
                let started = std::time::Instant::now();
                for _ in 0..20 {
                    std::hint::black_box(f());
                }
                started.elapsed().as_secs_f64() * 1000.0 / 20.0
            };
            let fast = time(&|| scale_letterbox(&src, 640, 360, (640, 360), dw, dh));
            let slow = time(&|| reference(&src, 640, 360, (640, 360), dw, dh));
            println!("SCALER 640x360 to {dw}x{dh}: {fast:.1} ms a frame (the one before: {slow:.1} ms)");
        }
    }

    #[test]
    fn scale_refuses_a_wrong_length_or_a_zero_size() {
        assert!(scale_letterbox(&[0; 7], 1, 2, (1, 2), 4, 4).is_none());
        assert!(scale_letterbox(&[0; 8], 0, 2, (1, 2), 4, 4).is_none());
        assert!(scale_letterbox(&[0; 8], 1, 2, (1, 2), 0, 4).is_none());
    }

    #[test]
    fn fit_keeps_the_aspect_and_never_exceeds_the_box() {
        assert_eq!(fit((1920, 1080), 320, 180), (320, 180));
        assert_eq!(fit((1080, 1920), 320, 180), (101, 180));
        assert_eq!(fit((100, 100), 320, 180), (180, 180));
        assert_eq!(fit((1, 1000), 10, 10), (1, 10));
    }

    #[test]
    fn display_size_applies_the_pixel_aspect() {
        assert_eq!(display_size(720, 576, (16, 15)), (768, 576));
        assert_eq!(display_size(720, 576, (0, 0)), (720, 576));
    }

    #[test]
    fn mix_matches_the_asked_channels() {
        let l = vec![1.0, 1.0];
        let r = vec![3.0, f32::NAN];
        assert_eq!(mix_channels(&[l.clone(), r.clone()], 2, 10), vec![1.0, 3.0, 1.0, 0.0]);
        assert_eq!(mix_channels(&[l.clone(), r], 1, 10), vec![2.0, 0.5]);
        assert_eq!(mix_channels(&[l], 2, 1), vec![1.0, 1.0]);
        assert!(mix_channels(&[], 2, 4).is_empty());
    }
}

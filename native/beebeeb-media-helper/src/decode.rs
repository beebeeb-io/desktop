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

/// `src` (RGBA, `sw × sh`) scaled to fit `dw × dh` with the aspect of `display`, centred, the rest opaque black:
/// exactly `dw × dh × 4` bytes. `None` for a zero size or a `src` of the wrong length.
pub fn scale_letterbox(src: &[u8], sw: u32, sh: u32, display: (u32, u32), dw: u32, dh: u32) -> Option<Vec<u8>> {
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

//! PCM from Windows' decoders at the rate the app asks for. Media Foundation's source reader hands float PCM at the
//! stream's own rate; [`Resampler`] brings it to the device's rate (linear, its position kept across calls) and
//! [`deinterleave`] hands it to the helper's own channel mixing (`crate::decode::mix_channels`), the same rules as
//! FilmCraft's path. Pure; tested on every system.

/// Interleaved float PCM from one rate to another. Equal rates pass every sample through unchanged. The position is
/// kept exactly, in whole units of `1 / to` input frames, so pieces give the same output as one push.
#[derive(Debug)]
pub struct Resampler {
    channels: usize,
    /// Input frames per output frame is `from / to`.
    from: u64,
    to: u64,
    /// Input not used up yet, interleaved.
    input: Vec<f32>,
    /// Where the next output frame lies, in units of `1 / to` input frames from the start of `input`.
    position: u64,
}

impl Resampler {
    /// From `from` Hz to `to` Hz, `channels` channels (each at least 1).
    pub fn new(from: u32, to: u32, channels: u16) -> Resampler {
        Resampler {
            channels: usize::from(channels.max(1)),
            from: u64::from(from.max(1)),
            to: u64::from(to.max(1)),
            input: Vec::new(),
            position: 0,
        }
    }

    /// Adds interleaved input; a trailing partial frame is dropped.
    pub fn push(&mut self, interleaved: &[f32]) {
        let whole = interleaved.len() - interleaved.len() % self.channels;
        self.input.extend_from_slice(&interleaved[..whole]);
    }

    fn frames_in(&self) -> u64 {
        (self.input.len() / self.channels) as u64
    }

    /// Output frame `k` from now lies at `at / to` input frames.
    fn at(&self, k: u64) -> u64 {
        self.position + k * self.from
    }

    /// Whether output frame `k` from now can be made: at the end of the stream its left input frame is enough; before
    /// it, the frame after it must be there too.
    fn can_make(&self, k: u64, end: bool) -> bool {
        let left = self.at(k) / self.to;
        left + if end { 1 } else { 2 } <= self.frames_in()
    }

    /// How many output frames [`Resampler::take`] can give now without the end of the stream: every `k` with
    /// `at(k) < (frames - 1) * to`.
    pub fn ready(&self) -> usize {
        let frames = self.frames_in();
        if frames < 2 {
            return 0;
        }
        let limit = (frames - 1) * self.to;
        if limit <= self.position {
            return 0;
        }
        ((limit - self.position).div_ceil(self.from)) as usize
    }

    /// Up to `frames` output frames, interleaved. With `end`, the last input frames are used up too (the last one
    /// held, as nothing follows it).
    pub fn take(&mut self, frames: usize, end: bool) -> Vec<f32> {
        let channels = self.channels;
        let available = self.frames_in();
        let mut out = Vec::with_capacity(frames * channels);
        let mut made = 0u64;
        while (made as usize) < frames && self.can_make(made, end) {
            let at = self.at(made);
            let left = (at / self.to) as usize;
            let right = (left + 1).min(available as usize - 1);
            let fraction = at % self.to;
            let t = fraction as f32 / self.to as f32;
            for c in 0..channels {
                let a = self.input[left * channels + c];
                let b = self.input[right * channels + c];
                out.push(if fraction == 0 { a } else { a + (b - a) * t });
            }
            made += 1;
        }
        // Drop the input frames no later output frame needs.
        let next = self.at(made);
        let used = (next / self.to).min(available);
        self.input.drain(..used as usize * channels);
        self.position = next - used * self.to;
        out
    }

    /// Forgets all input (a seek).
    pub fn clear(&mut self) {
        self.input.clear();
        self.position = 0;
    }
}

/// Interleaved samples as one vector per channel; a trailing partial frame is dropped.
pub fn deinterleave(interleaved: &[f32], channels: u16) -> Vec<Vec<f32>> {
    let channels = usize::from(channels.max(1));
    let frames = interleaved.len() / channels;
    (0..channels)
        .map(|c| (0..frames).map(|f| interleaved[f * channels + c]).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_rates_pass_every_sample_through_unchanged() {
        let input: Vec<f32> = (0..20).map(|i| i as f32 * 0.01 - 0.07).collect();
        let mut r = Resampler::new(48_000, 48_000, 2);
        r.push(&input);
        assert_eq!(r.ready(), 9, "the last frame waits for its neighbour");
        let mut out = r.take(100, false);
        out.extend(r.take(100, true));
        assert_eq!(out, input);
    }

    #[test]
    fn half_the_rate_keeps_every_other_frame() {
        let input: Vec<f32> = (0..10).map(|i| i as f32).collect();
        let mut r = Resampler::new(2, 1, 1);
        r.push(&input);
        let out = r.take(100, true);
        assert_eq!(out, vec![0.0, 2.0, 4.0, 6.0, 8.0]);
    }

    #[test]
    fn double_the_rate_puts_the_midpoint_between_frames() {
        let mut r = Resampler::new(1, 2, 1);
        r.push(&[0.0, 1.0]);
        assert_eq!(r.ready(), 2);
        assert_eq!(r.take(10, false), vec![0.0, 0.5]);
        assert_eq!(r.take(10, true), vec![1.0, 1.0], "at the end the last frame is held");
        assert!(r.take(10, true).is_empty());
    }

    /// The position carries over from one call to the next: pushing in pieces gives the same samples as at once.
    #[test]
    fn pieces_give_the_same_output_as_one_push() {
        let input: Vec<f32> = (0..441).map(|i| (i as f32 * 0.37).sin()).collect();
        let mut whole = Resampler::new(44_100, 48_000, 1);
        whole.push(&input);
        let all = whole.take(10_000, true);
        let mut pieces = Resampler::new(44_100, 48_000, 1);
        let mut out = Vec::new();
        for chunk in input.chunks(37) {
            pieces.push(chunk);
            out.extend(pieces.take(pieces.ready(), false));
        }
        out.extend(pieces.take(10_000, true));
        assert_eq!(out, all);
        // 441 frames at 44.1 kHz are 10 ms: exactly 480 frames at 48 kHz.
        assert_eq!(all.len(), 480);
    }

    #[test]
    fn a_partial_frame_is_dropped_and_clear_forgets_everything() {
        let mut r = Resampler::new(1, 1, 2);
        r.push(&[1.0, 2.0, 3.0]);
        assert_eq!(r.take(10, true), vec![1.0, 2.0]);
        r.push(&[5.0, 6.0, 7.0, 8.0]);
        r.clear();
        assert!(r.take(10, true).is_empty());
    }

    #[test]
    fn deinterleave_splits_by_channel() {
        assert_eq!(
            deinterleave(&[1.0, 2.0, 3.0, 4.0, 5.0], 2),
            vec![vec![1.0, 3.0], vec![2.0, 4.0]]
        );
    }
}

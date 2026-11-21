//! The TEST build's lines about a run (the build's identity, never the real app): `media.played`, `media.start`,
//! `media.seen` and `media.pace`, read by the Windows test machine rungs. What they count ([`Played`], [`Trace`]) and how the worker
//! writes them. Milliseconds and counts only: no name, id, tag or byte.

use std::time::Instant;

use beebeeb_engine::identity::EngineIdentity;
use beebeeb_media_protocol::Codec;

use super::Worker;
use crate::media::player::{PLAYED_AFTER_MS, lock};

/// What the test build's `media.played` line counts in the first second of playback.
#[derive(Default)]
pub(in crate::media::player) struct Played {
    /// The time playback started at (the first frame shown or sample played).
    pub(in crate::media::player) start: Option<u64>,
    pub(in crate::media::player) frames: u32,
    pub(in crate::media::player) sum_sq: f64,
    pub(in crate::media::player) samples: u64,
    pub(in crate::media::player) written: bool,
}

impl Played {
    pub(in crate::media::player) fn begin(&mut self, at: u64) {
        self.start.get_or_insert(at);
    }

    fn in_first_second(&self, pts: u64) -> bool {
        self.start
            .is_some_and(|start| pts >= start && pts < start.saturating_add(PLAYED_AFTER_MS))
    }

    pub(in crate::media::player) fn frame(&mut self, pts: u64) {
        if self.in_first_second(pts) {
            self.frames += 1;
        }
    }

    pub(in crate::media::player) fn sound(&mut self, samples: &[f32], pts: u64, rate: u32, channels: u16) {
        if !self.in_first_second(pts) {
            return;
        }
        let second = u64::from(rate) * u64::from(channels);
        for sample in samples {
            if self.samples >= second {
                break;
            }
            self.sum_sq += f64::from(*sample) * f64::from(*sample);
            self.samples += 1;
        }
    }

    pub(in crate::media::player) fn rms(&self) -> f64 {
        if self.samples == 0 {
            return 0.0;
        }
        let rms = (self.sum_sq / self.samples as f64).sqrt();
        (rms * 10_000.0).round() / 10_000.0
    }
}

/// Where the time from the press to the first frame goes, and how the picture keeps up after it: the TEST build's
/// `media.start` line (once per item) and `media.pace` line (once a second). Milliseconds and counts only.
pub(in crate::media::player) struct Trace {
    /// The press (the worker is made on the caller's thread).
    pub(in crate::media::player) from: Instant,
    /// Each step's time since the press, the first time it happened.
    pub(in crate::media::player) steps: Vec<(&'static str, u64)>,
    /// The sound device opened.
    pub(in crate::media::player) sound: bool,
    pub(in crate::media::player) written: bool,
    /// Since the last `media.pace` line: frames that passed the checks, frames shown, frames dropped as late.
    pub(in crate::media::player) received: u32,
    pub(in crate::media::player) shown: u32,
    pub(in crate::media::player) dropped: u32,
    pub(in crate::media::player) shown_pts: u64,
    pub(in crate::media::player) paced_at: u64,
    /// Since the last `media.pace` line, in microseconds: from each want for a frame to its answer (summed), in the
    /// hand-off of frames to the surface, in the reads of the item's bytes; and the worker's ticks.
    pub(in crate::media::player) want_at: Option<Instant>,
    pub(in crate::media::player) answer_us: u64,
    pub(in crate::media::player) show_us: u64,
    pub(in crate::media::player) read_us: u64,
    pub(in crate::media::player) ticks: u32,
}

impl Trace {
    pub(in crate::media::player) fn new() -> Trace {
        Trace {
            from: Instant::now(),
            steps: Vec::new(),
            sound: false,
            written: false,
            received: 0,
            shown: 0,
            dropped: 0,
            shown_pts: 0,
            paced_at: 0,
            want_at: None,
            answer_us: 0,
            show_us: 0,
            read_us: 0,
            ticks: 0,
        }
    }

    pub(in crate::media::player) fn ms(&self) -> u64 {
        u64::try_from(self.from.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// `step` happened now (kept the first time only).
    pub(in crate::media::player) fn mark(&mut self, step: &'static str) {
        if !self.steps.iter().any(|(name, _)| *name == step) {
            self.steps.push((step, self.ms()));
        }
    }

    pub(in crate::media::player) fn us(since: Instant) -> u64 {
        u64::try_from(since.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    /// A step's time; -1 when it never happened.
    pub(in crate::media::player) fn at(&self, step: &str) -> i64 {
        self.steps
            .iter()
            .find(|(name, _)| *name == step)
            .map_or(-1, |(_, ms)| i64::try_from(*ms).unwrap_or(i64::MAX))
    }
}

impl Worker {
    /// The test build's `media.start` line: once per item, at its first frame shown (or when the run ends without
    /// one).
    pub(super) fn write_start(&mut self) {
        if self.trace.written {
            return;
        }
        self.trace.written = true;
        #[cfg(test)]
        {
            let steps = self.trace.steps.clone();
            let _ = self.publish(|player| player.record.steps = steps);
        }
        let t = &self.trace;
        if !writes_played_line(crate::shell::identity::engine_identity()) {
            return;
        }
        tracing::info!(
            joined_ms = t.at("joined"),
            bytes_ms = t.at("bytes"),
            helper_ms = t.at("helper"),
            opened_ms = t.at("opened"),
            sound_ms = t.at("sound"),
            sound = t.sound,
            asked_ms = t.at("asked"),
            frame_ms = t.at("frame"),
            shown_ms = t.at("shown"),
            "media.start"
        );
    }

    /// The test build's `media.pace` line, once a second from `Opened`: the frames received, shown and dropped in
    /// that second, the clock, the time of the frame on screen and the position the view is told.
    pub(super) fn write_pace(&mut self, clock: u64, at_ms: u64) {
        let now = self.trace.ms();
        if self.opened.is_none() || now.saturating_sub(self.trace.paced_at) < 1_000 {
            return;
        }
        self.trace.paced_at = now;
        let (received, shown, dropped) = (self.trace.received, self.trace.shown, self.trace.dropped);
        self.trace.received = 0;
        self.trace.shown = 0;
        self.trace.dropped = 0;
        let shown_pts = self.trace.shown_pts;
        // Per frame: how long the helper took to answer a want; in all: the hand-off to the surface and the reads.
        let answer_ms = self.trace.answer_us / 1000 / u64::from(received.max(1));
        let (show_ms, read_ms, ticks) = (self.trace.show_us / 1000, self.trace.read_us / 1000, self.trace.ticks);
        self.trace.answer_us = 0;
        self.trace.show_us = 0;
        self.trace.read_us = 0;
        self.trace.ticks = 0;
        #[cfg(test)]
        let _ = self.publish(|player| {
            player.record.pace.push([
                now,
                u64::from(received),
                u64::from(shown),
                u64::from(dropped),
                clock,
                shown_pts,
                at_ms,
            ]);
        });
        if !writes_played_line(crate::shell::identity::engine_identity()) {
            return;
        }
        tracing::info!(
            t_ms = now,
            received,
            shown,
            dropped,
            clock_ms = clock,
            shown_pts,
            at_ms,
            answer_ms,
            show_ms,
            read_ms,
            ticks,
            "media.pace"
        );
    }

    /// The test build's one line per item, after the first second of playback.
    pub(super) fn write_played(&mut self, clock: u64) {
        let due = self
            .played
            .start
            .is_some_and(|start| clock >= start.saturating_add(PLAYED_AFTER_MS));
        if self.played.written || !due {
            return;
        }
        self.played.written = true;
        let codec = self
            .opened
            .and_then(|opened| {
                opened
                    .video
                    .map(|video| video.codec)
                    .or_else(|| opened.audio.map(|audio| audio.codec))
            })
            .unwrap_or(Codec::Other);
        let confined = lock(&self.slots.link).as_ref().is_some_and(|link| link.confined());
        log_played(codec, self.played.frames, self.played.rms(), confined);
    }
}

/// Whether this build writes the `media.played` line: the TEST build only (the Windows test machine rung reads it), never the
/// real app, in any profile. Asked of the identity fixed when the binary was built.
pub(in crate::media::player) fn writes_played_line(identity: &EngineIdentity) -> bool {
    !identity.is_real()
}

/// The `media.played` line: the codec as its protocol number (`Codec::ALL`'s order), the frames shown and the RMS of
/// the samples played in the first second, and whether the helper ran confined. Nothing else.
fn log_played(codec: Codec, frames_1s: u32, rms_1s: f64, confined: bool) {
    if !writes_played_line(crate::shell::identity::engine_identity()) {
        return;
    }
    tracing::info!(codec = codec as u8, frames_1s, rms_1s, confined, "media.played");
}

/// The test build's `media.seen` line: how long after the press a view first took the run's frame to draw it.
pub(in crate::media::player) fn log_seen(seen_ms: u64) {
    if !writes_played_line(crate::shell::identity::engine_identity()) {
        return;
    }
    tracing::info!(seen_ms, "media.seen");
}

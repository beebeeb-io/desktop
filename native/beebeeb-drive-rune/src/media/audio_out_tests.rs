//! The ring and the clock: pure parts, no device (slice M4). The null sink's timer test runs on Linux only.

use super::*;

fn ring(rate: u32, channels: u16) -> Ring {
    Ring::new(rate, channels)
}

#[test]
fn ring_never_holds_more_than_two_seconds() {
    // 1000 Hz stereo: 2 s = 4000 samples.
    let mut r = ring(1000, 2);
    assert_eq!(r.push(0, &vec![0.5; 3000]), 3000);
    // Only 1000 samples are free: the rest is refused, and the count says so.
    assert_eq!(r.push(0, &vec![0.5; 3000]), 1000);
    assert_eq!(r.buf.len(), 4000);
    assert_eq!(r.push(0, &[0.5; 2]), 0, "a full ring takes nothing");
    // Whole frames only: three samples of stereo take one frame.
    let mut r = ring(1000, 2);
    assert_eq!(r.push(0, &[0.1, 0.2, 0.3]), 2);
}

#[test]
fn ring_underrun_writes_silence() {
    let mut r = ring(1000, 1);
    r.push(0, &[0.25, 0.5, 0.75, 1.0]);
    let mut out = [9.0f32; 10];
    assert_eq!(r.fill(&mut out), 4);
    assert_eq!(&out[..4], &[0.25, 0.5, 0.75, 1.0]);
    assert!(out[4..].iter().all(|s| *s == 0.0), "the rest is silence");
    assert_eq!(r.underruns, 1);
    // Silence is not played sound: the clock counts the 4 real frames only.
    assert_eq!(r.played_frames, 4);
}

#[test]
fn clock_counts_played_frames_from_the_first_pts() {
    let mut r = ring(1000, 1);
    assert_eq!(r.clock_ms(0), None, "nothing queued");
    r.push(5000, &vec![0.1; 1000]);
    assert_eq!(r.clock_ms(0), None, "queued but not yet played");
    let mut out = [0.0f32; 500];
    r.fill(&mut out);
    assert_eq!(r.clock_ms(0), Some(5500));
    // The device latency (100 frames) is not heard yet.
    assert_eq!(r.clock_ms(100), Some(5400));
    // A later push keeps the first pts as the base.
    r.push(9999, &[0.1; 10]);
    assert_eq!(r.clock_ms(0), Some(5500));
}

#[test]
fn flush_resets_the_clock_base() {
    let mut r = ring(1000, 1);
    r.push(5000, &vec![0.1; 1000]);
    r.fill(&mut [0.0f32; 500]);
    assert_eq!(r.clock_ms(0), Some(5500));
    r.flush();
    assert!(r.buf.is_empty());
    assert_eq!(r.clock_ms(0), None);
    r.push(9000, &vec![0.1; 1000]);
    r.fill(&mut [0.0f32; 100]);
    assert_eq!(
        r.clock_ms(0),
        Some(9100),
        "the new base is the first sample pushed after the flush"
    );
}

/// P2-2 of the #462 review: on a device with latency the clock used to stop `latency` short of the end once the ring
/// ran empty, so the player never said `Ended`. The silence the device takes after the last sound pushes it out:
/// after `latency` frames of it the clock is at the end of the sound, and not before.
#[test]
fn the_clock_reaches_the_end_once_the_device_played_out_its_latency() {
    let mut r = ring(1000, 1);
    // One second of sound at pts 0, all of it taken by the device in buffers of 100 frames.
    assert_eq!(r.push(0, &vec![0.1; 1000]), 1000);
    for _ in 0..10 {
        r.fill(&mut [0.0f32; 100]);
    }
    assert_eq!(r.clock_ms(50), Some(950), "the last 50 frames are still in the device");
    // The ring is empty: the device takes silence after the sound.
    r.fill(&mut [0.0f32; 30]);
    assert_eq!(
        r.clock_ms(50),
        Some(980),
        "30 frames of silence pushed 30 frames of sound out"
    );
    r.fill(&mut [0.0f32; 100]);
    assert_eq!(r.clock_ms(50), Some(1000), "all of the sound was heard: the end");
    r.fill(&mut [0.0f32; 100]);
    assert_eq!(r.clock_ms(50), Some(1000), "and never past it");
    // Sound again (after an underrun): its own last frames are in the device again, the ones before were heard.
    r.push(1000, &[0.1; 20]);
    r.fill(&mut [0.0f32; 20]);
    assert_eq!(r.clock_ms(50), Some(1000), "the 20 new frames are not heard yet");
    r.fill(&mut [0.0f32; 50]);
    assert_eq!(r.clock_ms(50), Some(1020));
}

/// P3-1 of the #462 review: a timestamp from the helper near the end of `u64` neither wraps nor panics in the clock.
#[test]
fn a_timestamp_near_the_end_of_u64_saturates_the_clock() {
    let mut r = ring(1000, 1);
    r.push(u64::MAX - 1, &[0.1; 500]);
    r.fill(&mut [0.0f32; 500]);
    assert_eq!(r.clock_ms(0), Some(u64::MAX));
    assert_eq!(r.clock_ms(1000), Some(u64::MAX - 1));
}

/// P3-3 of the #462 review: a flush (a seek, a stop) zeroes the whole ring, the samples already played included; a
/// dropped ring is zeroed the same way (`Samples::drop`). The ring keeps working after it.
#[test]
fn a_flush_zeroes_every_slot_of_the_ring() {
    let mut r = ring(100, 1);
    // 150 samples through a ring of 200 slots: 120 played, 30 still queued, all of them in its memory.
    r.push(0, &[0.5; 150]);
    r.fill(&mut [0.0f32; 120]);
    assert_eq!(r.buf.data.iter().filter(|s| **s == 0.5).count(), 150);
    r.flush();
    assert!(r.buf.is_empty());
    assert!(
        r.buf.data.iter().all(|s| *s == 0.0),
        "no sample of the file is left in the ring"
    );
    assert_eq!(r.buf.data.len(), 200, "the ring keeps its size");
    // Round and round: a push past the end of the buffer comes back out in order.
    r.push(0, &[0.25; 190]);
    r.fill(&mut [0.0f32; 180]);
    let wrapped: Vec<f32> = (0..30).map(|i| i as f32 / 100.0).collect();
    assert_eq!(r.push(1900, &wrapped), 30);
    let mut out = [0.0f32; 40];
    assert_eq!(r.fill(&mut out), 40);
    assert_eq!(out[..10], [0.25; 10]);
    assert_eq!(out[10..], wrapped[..]);
}

#[cfg(not(any(windows, target_os = "macos")))]
#[test]
fn null_sink_clock_advances_with_time() {
    let mut out = AudioOut::start(44_100).expect("the null sink always opens");
    let (rate, channels) = out.format();
    assert_eq!(out.clock_ms(), None, "nothing played yet");
    // One second of sound at pts 1000.
    let one_second = vec![0.0f32; rate as usize * channels as usize];
    assert_eq!(out.push(1000, &one_second), one_second.len());
    std::thread::sleep(std::time::Duration::from_millis(150));
    let at = out.clock_ms().expect("the timer has played some");
    assert!((1140..=2000).contains(&at), "clock {at} ms after 150 ms of wall time");
    // Paused, the clock stands still.
    out.pause();
    let held = out.clock_ms();
    std::thread::sleep(std::time::Duration::from_millis(60));
    assert_eq!(out.clock_ms(), held);
}

#[cfg(not(any(windows, target_os = "macos")))]
#[test]
fn the_player_sees_a_lost_device() {
    // The null sink has no device to lose, so the test sets the same flag `keep_alive` sets when the rebuild fails.
    let mut out = AudioOut::start(44_100).expect("the null sink always opens");
    {
        let sink: &dyn AudioSink = &out;
        assert!(!sink.failed(), "a fresh output has not failed");
    }
    out.failed = true;
    let sink: &dyn AudioSink = &out;
    assert!(AudioOut::failed(&out));
    assert!(
        sink.failed(),
        "the player asks through the trait and must hear the failure"
    );
}

// The callback's fill step (M4-1'): no cpal stream, so it runs on Linux with the two formats the callback supports.
#[cfg(not(any(windows, target_os = "macos")))]
impl DeviceSample for f32 {
    fn from_f32(s: f32) -> f32 {
        s
    }
}
#[cfg(not(any(windows, target_os = "macos")))]
impl DeviceSample for i16 {
    fn from_f32(s: f32) -> i16 {
        (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
    }
}

#[test]
fn device_fill_f32_writes_the_queued_samples_then_silence() {
    let shared = Shared::new(1000, 1);
    shared.ring().push(0, &[0.25, -0.5, 0.75, -1.0]);
    let mut scratch = Zeroizing::new(Vec::new());
    let mut data = [9.0f32; 6];
    fill_device(&shared, &mut scratch, &mut data);
    assert_eq!(
        data,
        [0.25, -0.5, 0.75, -1.0, 0.0, 0.0],
        "the sound, in order, then silence only past it"
    );
}

#[test]
fn device_fill_i16_writes_the_converted_samples_then_silence() {
    let shared = Shared::new(1000, 1);
    shared.ring().push(0, &[0.5, -0.5, 0.25]);
    let mut scratch = Zeroizing::new(Vec::new());
    let mut data = [7i16; 5];
    fill_device(&shared, &mut scratch, &mut data);
    assert!(
        data[0] > 0 && data[1] < 0 && data[2] > 0,
        "signs follow the samples: {data:?}"
    );
    assert!(data[0] > data[2], "0.5 is louder than 0.25: {data:?}");
    assert_eq!(data[0], -data[1], "symmetric: {data:?}");
    assert_eq!(&data[3..], &[0, 0], "silence only past the sound");
}

#[test]
fn device_fill_empty_ring_is_silence() {
    let shared = Shared::new(1000, 2);
    let mut scratch = Zeroizing::new(Vec::new());
    let mut f = [9.0f32; 8];
    fill_device(&shared, &mut scratch, &mut f);
    assert!(f.iter().all(|s| *s == 0.0));
    let mut i = [9i16; 8];
    fill_device(&shared, &mut scratch, &mut i);
    assert!(i.iter().all(|s| *s == 0));
}

#[test]
fn device_fill_applies_the_gain() {
    let shared = Shared::new(1000, 1);
    shared.ring().push(0, &[0.5, -1.0, 0.25, 1.0]);
    shared.set_gain(0.5);
    let mut scratch = Zeroizing::new(Vec::new());
    let mut data = [9.0f32; 2];
    fill_device(&shared, &mut scratch, &mut data);
    assert_eq!(data, [0.25, -0.5], "half the volume, from this buffer on");
    // A change is heard from the next buffer, not after what is queued.
    shared.set_gain(0.0);
    fill_device(&shared, &mut scratch, &mut data);
    assert_eq!(data, [0.0, 0.0], "muted: silence");
    let mut i = [7i16; 2];
    shared.ring().push(0, &[1.0, -1.0]);
    shared.set_gain(1.0);
    fill_device(&shared, &mut scratch, &mut i);
    assert!(i[0] > 0 && i[1] < 0, "full volume again: {i:?}");
}

#[cfg(not(any(windows, target_os = "macos")))]
#[test]
fn audio_out_gain_is_clamped() {
    let mut out = AudioOut::start(48_000).expect("the null sink");
    out.set_gain(2.0);
    assert_eq!(out.shared.gain(), 1.0);
    out.set_gain(-1.0);
    assert_eq!(out.shared.gain(), 0.0);
    out.set_gain(f32::NAN);
    assert_eq!(out.shared.gain(), 0.0);
    out.set_gain(0.3);
    assert_eq!(out.shared.gain(), 0.3);
}

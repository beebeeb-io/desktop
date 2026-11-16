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
    let mut scratch = Vec::new();
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
    let mut scratch = Vec::new();
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
    let mut scratch = Vec::new();
    let mut f = [9.0f32; 8];
    fill_device(&shared, &mut scratch, &mut f);
    assert!(f.iter().all(|s| *s == 0.0));
    let mut i = [9i16; 8];
    fill_device(&shared, &mut scratch, &mut i);
    assert!(i.iter().all(|s| *s == 0));
}

//! w12-video-start: what reaches the window, and when. On the rig of `player_tests.rs`.

use super::*;

/// w12-video-start: the first frame of a run is in no scene yet (the view puts it on the stage only when it is run),
/// so the toolkit wakes nobody for it; before this the stage stayed black until a press ran the view. The player now
/// wakes the window for everything a view draws: here the missing sound device, the first frame, a refusal. The waker
/// is called with the player free (it reads the player itself here).
#[test]
fn the_window_is_woken_for_the_first_frame_no_sound_and_a_refusal() {
    type Seen = Arc<Mutex<Vec<(bool, Option<NoSound>, Option<Refusal>)>>>;
    fn woken(r: &Rig) -> Seen {
        let seen: Seen = Arc::default();
        let (handle, wakes) = (r.handle.clone(), Arc::clone(&seen));
        r.handle.set_waker(Arc::new(move || {
            let state = handle.state();
            wakes.lock().unwrap().push((
                handle.frame().is_some(),
                state.as_ref().and_then(|state| state.no_sound),
                state.and_then(|state| state.refused),
            ));
        }));
        seen
    }
    // An H.264 + MP3 file on a PC without a sound device (the Windows test machine).
    let r = rig(1_000, || plays_video(None));
    r.sound.no_device.store(true, SeqCst);
    let seen = woken(&r);
    r.handle.set_view_size(2, 2);
    r.open("v", KindHint::Video);
    wait_until("a wake that finds the first frame", || {
        seen.lock().unwrap().iter().any(|(framed, _, _)| *framed)
    });
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(_, no_sound, _)| *no_sound == Some(NoSound::Device)),
        "the window was woken for 'Sound isn't available on this PC'"
    );
    r.handle.stop(StopReason::Closed);

    let refused = rig(1_000, no_system_aac);
    let seen = woken(&refused);
    refused.open("a", KindHint::Audio);
    wait_until("a wake that finds the refusal", || {
        seen.lock().unwrap().iter().any(|(_, _, refusal)| refusal.is_some())
    });
}

/// w12-video-start: without sound the clock was a wall clock that ran on whether frames came or not. On a PC that
/// decodes slower than the file plays (the Windows test machine: 12 frames a second of a 30 frames a second file) every frame was late,
/// 4 a second were shown, and the place said ran ahead of the picture to the end of the file. Now a late frame is
/// shown and the clock goes on from its time: every frame is shown, and the place is the picture's.
#[test]
fn without_sound_a_slow_helper_shows_every_frame_and_the_place_is_the_pictures() {
    let (go, frames) = std::sync::mpsc::channel::<()>();
    let frames = Arc::new(Mutex::new(Some(frames)));
    let r = rig(1_000, move || {
        // One helper is started: it answers a want for a frame only when the test lets it.
        let frames = frames.lock().unwrap().take().expect("one helper");
        let mut index = 0u64;
        script(move |msg, out| match msg {
            ToHelper::Open { .. } => out.send(FromHelper::Opened(video_only(2, 2))),
            // Each want waits here until the test lets one frame go.
            ToHelper::WantVideo { w, h } if frames.recv().is_ok() => {
                out.send(frame(index * 1000 / 30, *w, *h));
                index += 1;
            }
            _ => {}
        })
    });
    r.handle.set_view_size(2, 2);
    r.open("v", KindHint::Video);
    wait_until("opened", || r.handle.phase() == Phase::Playing);
    for index in 0..12u64 {
        // The helper takes 80 ms for each frame; the file's frames are 33 ms apart.
        r.wall.0.fetch_add(80, SeqCst);
        go.send(()).expect("the helper waits");
        let pts = index * 1000 / 30;
        wait_until("the frame is shown and the place is its time", || {
            r.shown().last() == Some(&pts) && r.handle.state().is_some_and(|state| state.at_ms == pts)
        });
    }
    let expected: Vec<u64> = (0..12).map(|index| index * 1000 / 30).collect();
    assert_eq!(r.shown(), expected, "every frame was shown, none dropped as late");
    assert_eq!(r.received(), expected);
    r.handle.stop(StopReason::Closed);
}

//! A video's thumbnail comes from the confined media helper's first frame, and from nothing else: the engine starts
//! no program found by a name.

use super::*;
use crate::video_frame_source::{FirstFrameSource, Frame, FrameRequest, Refused, checked, first_frame_from};

fn frame(w: u32, h: u32, bytes: usize) -> Frame {
    Frame {
        rgba: Zeroizing::new(vec![9u8; bytes]),
        width: w,
        height: h,
    }
}

struct Fixed(Result<(u32, u32, usize), Refused>);

impl FirstFrameSource for Fixed {
    fn first_frame(&self, _path: &Path, _request: FrameRequest) -> Result<Frame, Refused> {
        self.0.map(|(w, h, n)| frame(w, h, n))
    }
}

#[test]
fn a_helper_frame_is_the_videos_thumbnail_source() {
    let source = Fixed(Ok((4, 2, 4 * 2 * 4)));
    let got = video_source_from(first_frame_from(Some(&source), Path::new("a.mp4"))).expect("a source");
    assert_eq!((got.width, got.height, got.kind), (4, 2, MediaKind::Video));
    assert_eq!(got.rgba.len(), 32);
}

#[test]
fn a_refusing_helper_or_none_is_no_thumbnail() {
    for refused in [Refused::NoHelper, Refused::NoFrame, Refused::BadFrame] {
        let source = Fixed(Err(refused));
        let got = video_source_from(first_frame_from(Some(&source), Path::new("a.mp4")));
        assert!(matches!(got, Err(MediaError::NoVideoFrame)), "{refused:?}");
    }
    let none = video_source_from(first_frame_from(None, Path::new("a.mp4")));
    assert!(matches!(none, Err(MediaError::NoVideoFrame)));
}

#[test]
fn a_frame_that_does_not_fit_what_was_asked_is_refused() {
    let ask = FrameRequest { max_w: 8, max_h: 8 };
    assert!(checked(frame(8, 8, 256), ask).is_ok());
    for (w, h, n) in [
        (9, 8, 288),
        (8, 9, 288),
        (0, 8, 0),
        (8, 0, 0),
        (8, 8, 255),
        (8, 8, 257),
        (u32::MAX, u32::MAX, 4),
    ] {
        assert_eq!(
            checked(frame(w, h, n), ask).err(),
            Some(Refused::BadFrame),
            "{w}x{h} with {n} bytes"
        );
    }
}

/// The base started `ffmpeg` by search name (and the `BEEBEEB_FFMPEG_PATH` override). A fake one first on the path,
/// and named by the override, must never run: the child process (this test binary) asks for a video's thumbnail with
/// no helper registered, and the fake leaves no marker.
#[cfg(unix)]
#[test]
fn a_program_named_ffmpeg_is_never_started() {
    use std::os::unix::fs::PermissionsExt;
    const CHILD: &str = "BB_VIDEO_FRAME_CHILD";
    let path = std::env::var_os(CHILD).map(PathBuf::from);
    if let Some(dir) = path {
        let video = dir.join("clip.mp4");
        let key = beebeeb_core::kdf::FileKey::from_bytes([7; 32]);
        let got = prepare_thumbnail_uploads_for_plaintext_media(&video, Some("video/mp4"), &key);
        assert!(
            matches!(got, Err(MediaError::NoVideoFrame)),
            "no helper means no thumbnail"
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("started");
    let fake = dir.path().join("ffmpeg");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\necho started > '{}'\nexit 1\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.path().join("clip.mp4"), b"not a video").unwrap();
    let out = std::process::Command::new(
        #[cfg(target_os = "linux")]
        "/proc/self/exe",
        #[cfg(not(target_os = "linux"))]
        std::env::current_exe().unwrap(),
    )
        .args([
            "--exact",
            "engine_bridge::tests::video_frame::a_program_named_ffmpeg_is_never_started",
            "--test-threads=1",
            "--nocapture",
        ])
        .env(CHILD, dir.path())
        .env(
            "PATH",
            format!("{}:{}", dir.path().display(), std::env::var("PATH").unwrap_or_default()),
        )
        .env("BEEBEEB_FFMPEG_PATH", &fake)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "the child failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("test result: ok. 1 passed;"),
        "the child ran no test:\n{stdout}"
    );
    assert!(!marker.exists(), "the fake ffmpeg was started");
}

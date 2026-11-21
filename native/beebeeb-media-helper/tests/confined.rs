//! The `System` route inside the sandbox (Windows): the helper binary started as the app starts it, confined in the
//! system-decoder profile without HEVC, plays AAC through Windows' decoder. That profile holds `lpacCom` (Guus,
//! 2026-11-21); without it COM does not start there and a video plays silent (ux-57). Needs a desktop session: in
//! session 0 the system's libraries do not initialise inside an AppContainer.
#![cfg(windows)]

use std::fs::File;
use std::path::{Path, PathBuf};

use beebeeb_media_protocol::{Codec, FromHelper, KindHint, ToHelper, read_to_app, write_msg};
use beebeeb_office_sandbox::{Limits, Profile, Sandbox};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|_| panic!("fixture {name} is missing: run scripts/media-fixtures.sh"))
}

/// The app's side over the confined helper's two pipes: answers `ReadAt` from the file.
struct App {
    to_helper: File,
    from_helper: File,
    file: Vec<u8>,
}

impl App {
    fn send(&mut self, msg: &ToHelper) {
        write_msg(&mut self.to_helper, msg).expect("the helper is listening");
    }

    fn ask(&mut self, msg: ToHelper) -> FromHelper {
        self.send(&msg);
        loop {
            match read_to_app(&mut self.from_helper).expect("a message from the helper") {
                FromHelper::ReadAt { id, offset, len } => {
                    let end = offset + u64::from(len);
                    if end <= self.file.len() as u64 {
                        let bytes = self.file[offset as usize..end as usize].to_vec();
                        self.send(&ToHelper::Data { id, bytes });
                    } else {
                        self.send(&ToHelper::DataFailed { id });
                    }
                }
                other => return other,
            }
        }
    }
}

#[test]
fn confined_system_profile_plays_aac_with_sound() {
    let profile = Profile::SystemDecoder { hevc: false };
    let sandbox = Sandbox::with_profile(Limits::default(), profile).expect("a sandbox");
    let mut child = sandbox
        .spawn(Path::new(env!("CARGO_BIN_EXE_beebeeb-media-helper")))
        .expect("the helper starts confined");
    let pipes = child.take_pipes().expect("the pipes");
    let mut app = App {
        to_helper: pipes.to_helper,
        from_helper: pipes.from_helper,
        file: fixture("aac-lc.m4a"),
    };
    // As the app tells a helper started in the system profile: no further restart follows.
    app.send(&ToHelper::LastHelper);
    let len = app.file.len() as u64;
    let opened = match app.ask(ToHelper::Open {
        len,
        hint: KindHint::Audio,
    }) {
        FromHelper::Opened(opened) => opened,
        other => panic!("{profile:?}: expected Opened, got {other:?} (COM or Media Foundation did not start)"),
    };
    assert_eq!(
        opened.audio.map(|audio| audio.codec),
        Some(Codec::Aac),
        "the AAC track plays"
    );
    let mut samples = Vec::new();
    for _ in 0..5 {
        match app.ask(ToHelper::WantAudio {
            rate: 48_000,
            channels: 2,
            frames: 4_800,
        }) {
            FromHelper::Audio { samples: got, .. } => samples.extend(got),
            other => panic!("expected Audio, got {other:?}"),
        }
    }
    let power = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum::<f64>() / samples.len().max(1) as f64;
    assert!(power.sqrt() > 0.01, "sound, not silence: rms {}", power.sqrt());
    app.send(&ToHelper::Close);
    let _ = child.kill();
}

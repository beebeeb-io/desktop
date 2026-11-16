//! What the link zeroes of the helper's answers (P3-3 of the #462 review): a kill lets go of the inbox and of the
//! batches still queued, and each answer in them is zeroed first by [`wipe_answer`].

use beebeeb_media_protocol::{ErrorCode, FromHelper, Tags};

use super::wipe_answer;

#[test]
fn a_wiped_answer_holds_nothing_of_the_file() {
    let mut frame = FromHelper::Frame {
        pts_ms: 40,
        w: 2,
        h: 2,
        rgba: vec![200; 16],
    };
    wipe_answer(&mut frame);
    let FromHelper::Frame { rgba, .. } = &frame else {
        panic!("still a frame");
    };
    assert!(rgba.iter().all(|b| *b == 0), "the pixels are zeroed");

    let mut sound = FromHelper::Audio {
        pts_ms: 0,
        channels: 2,
        samples: vec![0.5; 8],
    };
    wipe_answer(&mut sound);
    let FromHelper::Audio { samples, .. } = &sound else {
        panic!("still sound");
    };
    assert!(samples.iter().all(|s| *s == 0.0), "the sound is zeroed");

    let mut tags = FromHelper::Tags(Tags {
        title: "A title".into(),
        artist: "An artist".into(),
        art: Some((1, 1, vec![9; 4])),
    });
    wipe_answer(&mut tags);
    let FromHelper::Tags(Tags { title, artist, art }) = &tags else {
        panic!("still tags");
    };
    assert!(title.is_empty() && artist.is_empty(), "the words are zeroed");
    assert!(
        art.as_ref().is_some_and(|(_, _, rgba)| rgba.iter().all(|b| *b == 0)),
        "the picture is zeroed"
    );

    // An answer that holds nothing of the file is left as it is.
    let mut error = FromHelper::Error(ErrorCode::Corrupt);
    wipe_answer(&mut error);
    assert!(matches!(error, FromHelper::Error(ErrorCode::Corrupt)));
}

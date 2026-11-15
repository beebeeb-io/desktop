//! Thumbnails and previews (DW-0744) drawn on `EngineDriveSettings` with its preview host on a double of the engine:
//! wired, drawing the engine's figures; never the board's fallback, grid-thumbnail or "On this PC" rows the engine
//! has no figure for (the board's fixture still draws them). Every name is invented.

use beebeeb_engine::photo_previews::PreviewFormat as Wire;
use beebeeb_engine::photo_previews::runner::PreviewRun;

use super::*;
use crate::engine_host::drive_previews::tests::{FakePreviews, fake as previews_fake};
use crate::engine_host::drive_previews::{HEIC_NOTE, PreviewsHost};

/// DW-0744 on the engine source, once the pane's read is drawn.
fn on_engine(previews: Arc<FakePreviews>) -> Runtime<State> {
    let drive = EngineDriveSettings::new(account(), Arc::new(FakePlatform::default()))
        .with_previews(PreviewsHost::new(previews, PreviewRun::new()));
    let mut rt = drawn("DW-0744", Box::new(drive));
    let began = Instant::now();
    while !has(&rt, "item-0-0") {
        assert!(
            began.elapsed() < Duration::from_secs(10),
            "the pane never drew its read"
        );
        std::thread::sleep(Duration::from_millis(5));
        rt.frame(0.016);
        rt.settle_all();
    }
    rt
}

#[test]
fn previews_pane_is_wired_on_the_engine() {
    let rt = on_engine(previews_fake());
    assert!(rt.state.settings.wired(Pane::Previews));
    assert!(!has(&rt, "not-built"), "the shell's frame");
    let texts = texts(&rt);
    // The engine's figures (the double's): 2 previews, 2.4 GB, 400 KB each at 1080p JPEG.
    assert!(texts.iter().any(|t| t == "2.4 GB"), "{texts:?}");
    assert!(
        texts
            .iter()
            .any(|t| t.contains("All 2 photos at 1080p JPEG; average 400 KB per preview, measured on 2")),
        "{texts:?}"
    );
    // None of the board's.
    for sample in ["2,431", "3.1 GB", "30.9 MB", "420 MB", "300 MB"] {
        assert!(!texts.iter().any(|t| t.contains(sample)), "draws {sample}");
    }
    // Without the engine's preview host the pane stays the shell's frame (the check can fail).
    let off = drawn(
        "DW-0744",
        Box::new(EngineDriveSettings::new(account(), Arc::new(FakePlatform::default()))),
    );
    assert!(!off.state.settings.wired(Pane::Previews) && has(&off, "not-built"));
}

#[test]
fn engine_pane_draws_no_fallback_grid_or_on_pc_rows() {
    let fake = previews_fake();
    fake.setting.lock().unwrap().format = Wire::Heic;
    let rt = on_engine(fake);
    let texts = texts(&rt);
    // "What it uses": the previews row only; no "On this PC" list; no fallback preview row or sentence.
    assert!(has(&rt, "item-3-0"));
    for key in ["item-3-1", "item-3-2", "item-4-0", "item-5-1"] {
        assert!(!has(&rt, key), "draws {key}");
    }
    for words in [
        "Fallback previews",
        "Fallback preview, with HEIC",
        "720p WebP",
        "On this PC",
        "ON THIS PC",
    ] {
        assert!(!texts.iter().any(|t| t.contains(words)), "draws {words}");
    }
    // With HEIC stored, the Format caption says this PC makes none.
    assert!(texts.iter().any(|t| t.contains(HEIC_NOTE)), "{texts:?}");
    // The board's fixture draws every one of them (the checks can fail).
    let board = drawn("DW-0744", Box::new(BoardDrive::new()));
    for key in ["item-3-1", "item-3-2", "item-4-0", "item-5-1"] {
        assert!(has(&board, key), "the board draws {key}");
    }
    assert!(!super::texts(&board).iter().any(|t| t.contains(HEIC_NOTE)));
}

//! First-run step 4 on the real engine (`setup_previews.rs`), against the setup double: the step follows the read of
//! the account's setting (loading, failed with Try again, answered), starts on the stored choice, writes once on
//! Continue and never on Not now, draws no "Space on this PC", and says the bundled picture only when it is one.
//! Every name is invented.

use beebeeb_engine::photo_previews::{PreviewFormat as Wire, PreviewsSummary};

use super::*;
use crate::frames::signin::setup::Ui;

/// The setup double's step-4 read: what the test set, after a hold the test may keep.
pub(super) fn fake_read(fake: &Fake) -> Result<PreviewsFacts, PreviewsRefused> {
    drop(fake.previews_held.lock().unwrap());
    fake.previews
        .lock()
        .unwrap()
        .clone()
        .unwrap_or(Err(PreviewsRefused::NotSignedIn))
}

/// The setup double's write, as the engine's: only for the session in memory.
pub(super) fn fake_set(
    fake: &Fake,
    session: SessionGeneration,
    setting: PreviewSetting,
) -> Result<(), PreviewsRefused> {
    if session != fake.stamp().0 {
        return Err(PreviewsRefused::NotSignedIn);
    }
    fake.previews_sets.lock().unwrap().push((session, setting));
    Ok(())
}

/// The setup double's sample: an invented photo of the account when the test says there is one.
pub(super) fn fake_sample(fake: &Fake, session: SessionGeneration) -> Result<SampleFacts, PreviewsRefused> {
    let photo = fake.previews_photo.load(Ordering::SeqCst).then(|| {
        let png = |n: u8| zeroize::Zeroizing::new(vec![0x89, b'P', b'N', b'G', n]);
        beebeeb_engine::app::photo_previews_api::SamplePhoto {
            original_bytes: 4_000_000,
            weights: vec![(1280, 200_000), (1920, 410_000), (2560, 700_000), (3840, 1_200_000)],
            original_png: png(0),
            previews_png: vec![(1280, png(1)), (1920, png(2)), (2560, png(3)), (3840, png(4))],
        }
    });
    Ok(SampleFacts { session, photo })
}

fn facts(enabled: bool, format: Wire, px: u32, count: u64) -> PreviewsFacts {
    PreviewsFacts {
        session: session(1),
        setting: PreviewSetting { enabled, format, px },
        summary: PreviewsSummary {
            count,
            total_bytes: count * 400_000,
        },
        averages: if count > 0 {
            vec![((Wire::Heic, 2560), 300_000, count)]
        } else {
            Vec::new()
        },
    }
}

/// A first run whose step-4 read answers `read`; its answers taken in.
fn step_four(read: Result<PreviewsFacts, PreviewsRefused>, photo: bool) -> Set {
    let engine = fake();
    *engine.previews.lock().unwrap() = Some(read);
    engine.previews_photo.store(photo, Ordering::SeqCst);
    let mut set = set_on(engine, FakePlatform::default());
    // The read, and the sample measured after a read that worked.
    let began = Instant::now();
    while set.source.loading(Step::Previews) || set.source.previews.measuring {
        assert!(began.elapsed() < Duration::from_secs(10), "step 4 never answered");
        std::thread::sleep(Duration::from_millis(5));
        set.source.poll();
    }
    set
}

#[test]
fn setup_step4_follows_the_read() {
    // Out: a loading state, never "not in this version", and Continue stores nothing.
    let engine = fake();
    *engine.previews.lock().unwrap() = Some(Ok(facts(false, Wire::Heic, 2560, 0)));
    let held = engine.previews_held.lock().unwrap();
    let set = set_on(engine.clone(), FakePlatform::default());
    let mut ui = Ui::new(Box::new(set.source));
    ui.step = Step::Previews;
    assert!(ui.available(Step::Previews) && ui.loading(Step::Previews) && !ui.failed(Step::Previews));
    ui.go_on();
    assert_eq!(ui.step, Step::Previews, "Continue waits for the read");
    drop(held);

    // Off: offered.
    let off = step_four(Ok(facts(false, Wire::Heic, 2560, 0)), false);
    assert!(off.source.available(Step::Previews) && !off.source.loading(Step::Previews));
    assert!(!off.source.failed(Step::Previews));
    // Already on (another device): still the step, on the account's own choice (never "not in this version").
    let on = step_four(Ok(facts(true, Wire::Jpeg, 3840, 12)), false);
    assert!(on.source.available(Step::Previews) && !on.source.failed(Step::Previews));
    assert_eq!(
        on.source.stored_previews(),
        Some((PreviewSize::K4, PreviewFormat::Jpeg))
    );
    // Failed: said as a failure (Try again), never read as off or on.
    let failed = step_four(Err(PreviewsRefused::Offline), false);
    assert!(failed.source.available(Step::Previews) && failed.source.failed(Step::Previews));
    assert_eq!(failed.source.stored_previews(), None);
    // A double that binds no previews: the step is not offered (the check can fail).
    let unbound = super::set();
    assert!(!unbound.source.available(Step::Previews));
}

#[test]
fn setup_starts_on_the_stored_choice() {
    let set = step_four(Ok(facts(false, Wire::Jpeg, 1920, 0)), false);
    let engine = set.engine.clone();
    let mut ui = Ui::new(Box::new(set.source));
    assert_eq!(
        (ui.size, ui.format),
        (PreviewSize::P1080, PreviewFormat::Jpeg),
        "not the board's 4K HEIC"
    );
    ui.step = Step::Previews;
    ui.go_on();
    let began = Instant::now();
    while engine.previews_sets.lock().unwrap().is_empty() {
        assert!(began.elapsed() < Duration::from_secs(10), "nothing was stored");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        *engine.previews_sets.lock().unwrap(),
        [(
            session(1),
            PreviewSetting {
                enabled: true,
                format: Wire::Jpeg,
                px: 1920
            }
        )]
    );
}

#[test]
fn setup_continue_writes_once_not_now_writes_nothing() {
    // Not now: nothing written, the row says it was not set here.
    let set = step_four(Ok(facts(false, Wire::Heic, 2560, 0)), false);
    let engine = set.engine.clone();
    let mut ui = Ui::new(Box::new(set.source));
    ui.step = Step::Previews;
    ui.not_now();
    assert_eq!(ui.step, Step::Explorer);
    std::thread::sleep(Duration::from_millis(50));
    assert!(engine.previews_sets.lock().unwrap().is_empty());
    assert_eq!(
        ui.answered(Switch::Previews),
        Some(crate::frames::signin::setup::Answered::NotAsked)
    );

    // Continue: one write, answered as the Previews switch.
    let set = step_four(Ok(facts(false, Wire::Heic, 2560, 0)), false);
    let engine = set.engine.clone();
    let mut ui = Ui::new(Box::new(set.source));
    ui.step = Step::Previews;
    ui.go_on();
    assert_eq!(
        ui.answered(Switch::Previews),
        Some(crate::frames::signin::setup::Answered::Waiting)
    );
    let began = Instant::now();
    while ui.answered(Switch::Previews) == Some(crate::frames::signin::setup::Answered::Waiting) {
        assert!(began.elapsed() < Duration::from_secs(10), "no answer");
        std::thread::sleep(Duration::from_millis(5));
        ui.pump();
    }
    assert_eq!(ui.answered(Switch::Previews), None, "made");
    assert_eq!(engine.previews_sets.lock().unwrap().len(), 1);
}

/// Step 4 drawn in the app's window.
fn drawn(ui: Ui) -> Runtime<State> {
    let mut state = crate::init();
    crate::shell::host::dispatch(&mut state, crate::shell::app::Event::Gate(ShellGate::FirstRun));
    state.setup = Some(ui);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (958.0, 1100.0), env);
    rt.frame(0.0);
    rt.frame(0.016);
    rt
}

fn texts(rt: &Runtime<State>) -> Vec<String> {
    rt.elements().into_iter().filter_map(|e| e.text).collect()
}

#[test]
fn setup_space_row_hidden_when_no_limit() {
    let set = step_four(Ok(facts(false, Wire::Heic, 2560, 3)), false);
    let mut ui = Ui::new(Box::new(set.source));
    ui.step = Step::Previews;
    let engine = texts(&drawn(ui));
    assert!(engine.iter().any(|t| t == "Thumbnails and previews"), "step 4 is drawn");
    assert!(!engine.iter().any(|t| t.contains("Space on this PC")), "{engine:?}");
    assert!(!engine.iter().any(|t| t.contains("in this version")));
    // The line under the choices: the account's own figures at the choice (3 previews at 2K HEIC, 300 KB each).
    assert!(
        engine
            .iter()
            .any(|t| t == "About 900 KB in your Beebeeb storage for 3 photos."),
        "{engine:?}"
    );
    // The board's fixture draws the row (the check can fail).
    let mut board = Ui::new(Box::new(crate::frames::signin::setup::source::FixtureSetupSource::new(
        crate::frames::signin::setup::source::Script::default(),
    )));
    board.step = Step::Previews;
    let board = texts(&drawn(board));
    assert!(board.iter().any(|t| t.contains("Space on this PC")), "{board:?}");
}

#[test]
fn sample_is_bundled_only_when_said() {
    let none = step_four(Ok(facts(false, Wire::Jpeg, 1920, 0)), false);
    assert!(none.source.sample().bundled);
    assert_eq!(
        none.source.original_weight(),
        crate::frames::settings::previews::NOT_MEASURED
    );
    let own = step_four(Ok(facts(false, Wire::Jpeg, 1920, 0)), true);
    let sample = own.source.sample();
    assert!(!sample.bundled);
    assert_eq!(&sample.original_png[..], &[0x89, b'P', b'N', b'G', 0]);
    assert_eq!(
        &sample.preview_png[..],
        &[0x89, b'P', b'N', b'G', 2],
        "the JPEG preview at the stored 1920"
    );
    assert_eq!(own.source.original_weight(), "4 MB");
    assert_eq!(
        own.source.weight(PreviewSize::P1080, PreviewFormat::Jpeg).as_deref(),
        Some("410 KB")
    );
    // HEIC is not made here: no weight is measured for it, and its picture is the original.
    assert_eq!(own.source.weight(PreviewSize::P1080, PreviewFormat::Heic), None);
    let heic = step_four(Ok(facts(false, Wire::Heic, 2560, 0)), true);
    let sample = heic.source.sample();
    assert_eq!(sample.preview_png, sample.original_png);
}

#[test]
fn setup_space_limit_is_the_engines() {
    use crate::frames::signin::setup::source::SetupSource;
    // The engine reports 35 GB: step 4 says "35 GB", not the board's constant.
    let engine = fake();
    *engine.previews.lock().unwrap() = Some(Ok(facts(false, Wire::Heic, 2560, 3)));
    engine.space_limit.store(35_000_000_000, Ordering::SeqCst);
    let set = set_on(engine, FakePlatform::default());
    assert_eq!(set.source.space_limit(), "35 GB");
    // No limit known: empty (the row is not drawn), never "20 GB".
    let none = step_four(Ok(facts(false, Wire::Heic, 2560, 3)), false);
    assert_eq!(none.source.space_limit(), "");
}

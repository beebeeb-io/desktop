//! Thumbnails and previews on the real engine (`PreviewsHost`): against a double of the engine (what each read
//! answers, every change it was sent, a read held until the test lets it go), and once against a real handle on the
//! fake API with the preview routes. Every name is invented.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use beebeeb_engine::photo_previews::PreviewsSummary;

use super::*;

pub(crate) const ONE: &str = "one@example.eu";
pub(crate) const TWO: &str = "two@example.eu";

/// A double of the engine: a test sets what it answers and reads what it was sent.
pub(crate) struct FakePreviews {
    pub(crate) session: Mutex<SessionGeneration>,
    pub(crate) ready: AtomicBool,
    pub(crate) account: Mutex<Option<String>>,
    /// The setting the account stores, and what a read of it answers when it fails.
    pub(crate) setting: Mutex<PreviewSetting>,
    pub(crate) read_fails: Mutex<Option<PreviewsRefused>>,
    pub(crate) set_fails: AtomicBool,
    /// Every setting written, with its session.
    pub(crate) sets: Mutex<Vec<(SessionGeneration, PreviewSetting)>>,
    /// Every run's work that ran, with its session and whether it was a rebuild.
    pub(crate) makes: Mutex<Vec<(SessionGeneration, bool)>>,
    /// Every facts read waits for a permit while `gated` (the test hands them out one by one).
    pub(crate) gated: AtomicBool,
    permits: Mutex<u32>,
    moved: Condvar,
    pub(crate) wakes: AtomicUsize,
    /// What a sample measure answers (`true`: one photo, measured).
    pub(crate) has_photo: AtomicBool,
    /// How many whole reads (with the per-file list) and summary reads were asked.
    pub(crate) facts_reads: AtomicUsize,
    pub(crate) summary_reads: AtomicUsize,
    /// Added to the previews' count (more were made).
    pub(crate) extra: AtomicU64,
    /// A run's work waits while this is set (at most 10 s), or until its stop flag is set.
    pub(crate) hold_make: AtomicBool,
    /// A held run's work saw its stop flag set.
    pub(crate) saw_stop: AtomicBool,
    /// A setting's write waits while this is set (at most 10 s).
    pub(crate) hold_set: AtomicBool,
    /// A run's work counts nothing (as one waiting for sync's first pass).
    pub(crate) quiet_make: AtomicBool,
}

pub(crate) fn fake() -> Arc<FakePreviews> {
    Arc::new(FakePreviews {
        session: Mutex::new(SessionGeneration::for_test(1)),
        ready: AtomicBool::new(true),
        account: Mutex::new(Some(ONE.into())),
        setting: Mutex::new(PreviewSetting {
            enabled: true,
            format: Wire::Jpeg,
            px: 1920,
        }),
        read_fails: Mutex::new(None),
        set_fails: AtomicBool::new(false),
        sets: Mutex::default(),
        makes: Mutex::default(),
        gated: AtomicBool::new(false),
        permits: Mutex::new(0),
        moved: Condvar::new(),
        wakes: AtomicUsize::new(0),
        has_photo: AtomicBool::new(false),
        facts_reads: AtomicUsize::new(0),
        summary_reads: AtomicUsize::new(0),
        extra: AtomicU64::new(0),
        hold_make: AtomicBool::new(false),
        saw_stop: AtomicBool::new(false),
        hold_set: AtomicBool::new(false),
        quiet_make: AtomicBool::new(false),
    })
}

impl FakePreviews {
    pub(crate) fn permit(&self) {
        *self.permits.lock().unwrap() += 1;
        self.moved.notify_all();
    }

    fn wait_for_permit(&self) {
        if !self.gated.load(Ordering::SeqCst) {
            return;
        }
        let permits = self.permits.lock().unwrap();
        let (mut permits, timed_out) = self
            .moved
            .wait_timeout_while(permits, Duration::from_secs(30), |n| *n == 0)
            .unwrap();
        assert!(!timed_out.timed_out(), "the test never let the read go");
        *permits -= 1;
    }

    /// Two previews at the stored setting, of 300 and 500 bytes, for ONE; 7 previews for any other account.
    fn facts_now(&self, session: SessionGeneration) -> PreviewsFacts {
        let setting = *self.setting.lock().unwrap();
        let count = if self.account.lock().unwrap().as_deref() == Some(ONE) {
            2
        } else {
            7
        } + self.extra.load(Ordering::SeqCst);
        PreviewsFacts {
            session,
            setting,
            summary: PreviewsSummary {
                count,
                total_bytes: 2_400_000_000,
            },
            averages: vec![((Wire::Jpeg, 1920), 400_000, count)],
        }
    }
}

impl PreviewsEngine for FakePreviews {
    fn session(&self) -> SessionGeneration {
        *self.session.lock().unwrap()
    }
    fn ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }
    fn account(&self) -> Option<String> {
        self.account.lock().unwrap().clone()
    }
    fn wake(&self) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
    }
    fn facts(&self, session: SessionGeneration) -> Result<PreviewsFacts, PreviewsRefused> {
        self.facts_reads.fetch_add(1, Ordering::SeqCst);
        // The answer is the account's as the read began.
        let facts = self.facts_now(session);
        self.wait_for_permit();
        match *self.read_fails.lock().unwrap() {
            Some(why) => Err(why),
            None => Ok(facts),
        }
    }
    fn summary(&self, session: SessionGeneration) -> Result<PreviewsSummary, PreviewsRefused> {
        self.summary_reads.fetch_add(1, Ordering::SeqCst);
        match *self.read_fails.lock().unwrap() {
            Some(why) => Err(why),
            None => Ok(self.facts_now(session).summary),
        }
    }
    fn setting(&self, _session: SessionGeneration) -> Result<PreviewSetting, PreviewsRefused> {
        match *self.read_fails.lock().unwrap() {
            Some(why) => Err(why),
            None => Ok(*self.setting.lock().unwrap()),
        }
    }
    fn set_setting(&self, session: SessionGeneration, setting: PreviewSetting) -> Result<(), PreviewsRefused> {
        if self.set_fails.load(Ordering::SeqCst) {
            return Err(PreviewsRefused::Offline);
        }
        let began = Instant::now();
        while self.hold_set.load(Ordering::SeqCst) && began.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.sets.lock().unwrap().push((session, setting));
        *self.setting.lock().unwrap() = setting;
        Ok(())
    }
    fn make(&self, session: SessionGeneration, force: bool, stop: &AtomicBool, progress: &dyn Fn(u32, u32)) -> RunEnd {
        self.makes.lock().unwrap().push((session, force));
        if !self.quiet_make.load(Ordering::SeqCst) {
            progress(1, 1);
        }
        let began = Instant::now();
        while self.hold_make.load(Ordering::SeqCst) && began.elapsed() < Duration::from_secs(10) {
            if stop.load(Ordering::SeqCst) {
                self.saw_stop.store(true, Ordering::SeqCst);
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        RunEnd {
            made: 1,
            ..RunEnd::default()
        }
    }
    fn measure(&self, session: SessionGeneration, _picked: Option<PathBuf>) -> Result<SampleFacts, PreviewsRefused> {
        if !self.has_photo.load(Ordering::SeqCst) {
            return Ok(SampleFacts { session, photo: None });
        }
        let png = |n: u8| zeroize::Zeroizing::new(vec![0x89, b'P', b'N', b'G', n]);
        Ok(SampleFacts {
            session,
            photo: Some(beebeeb_engine::app::photo_previews_api::SamplePhoto {
                original_bytes: 30_900_000,
                weights: vec![(1280, 200_000), (1920, 410_000), (2560, 700_000), (3840, 1_200_000)],
                original_png: png(0),
                previews_png: vec![(1280, png(1)), (1920, png(2)), (2560, png(3)), (3840, png(4))],
            }),
        })
    }
}

/// Pump until `done` (at most 10 s).
pub(crate) fn pump_until(host: &mut PreviewsHost, what: &str, done: impl Fn(&PreviewsHost) -> bool) {
    let began = Instant::now();
    loop {
        host.pump();
        if done(host) {
            return;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn ready(host: &PreviewsHost) -> Option<Previews> {
    match host.photo_previews() {
        PhotoPreviews::Ready(previews) => Some(previews),
        _ => None,
    }
}

/// Wait until `n` runs' work ran.
fn runs(fake: &FakePreviews, n: usize) {
    let began = Instant::now();
    while fake.makes.lock().unwrap().len() < n {
        assert!(began.elapsed() < Duration::from_secs(10), "{n} runs did not run");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn opened(fake: &Arc<FakePreviews>) -> PreviewsHost {
    let mut host = PreviewsHost::new(fake.clone(), PreviewRun::new());
    host.pump();
    host.opened();
    pump_until(&mut host, "the pane read", |host| ready(host).is_some());
    host
}

#[test]
fn the_pane_draws_the_accounts_setting_and_figures_and_nothing_made_up() {
    let fake = fake();
    let host = opened(&fake);
    let p = ready(&host).unwrap();
    assert!(p.on);
    assert_eq!((p.size, p.format), (PreviewSize::P1080, PreviewFormat::Jpeg));
    assert_eq!((p.photos.as_str(), p.total.as_str()), ("2", "2.4 GB"));
    assert_eq!(p.uses, [Some("2.4 GB".to_string()), None, None]);
    assert_eq!(p.on_pc, None);
    assert!(!p.fallback);
    assert_eq!(p.format_note, None);
    assert_eq!(
        p.weight(PreviewSize::P1080, PreviewFormat::Jpeg).as_deref(),
        Some("400 KB")
    );
    assert_eq!(p.measured_on, 2);
    // No photo measured: no weight of it, and the bundled picture.
    assert_eq!((p.original.as_str(), p.sample.is_none()), (NOT_MEASURED, true));
}

#[test]
fn with_heic_stored_the_caption_says_this_pc_makes_none() {
    let fake = fake();
    fake.setting.lock().unwrap().format = Wire::Heic;
    let host = opened(&fake);
    let p = ready(&host).unwrap();
    assert_eq!(p.format_note, Some(HEIC_NOTE));
    assert_eq!(p.format, PreviewFormat::Heic);
}

#[test]
fn earlier_generation_answer_is_dropped() {
    let fake = fake();
    fake.gated.store(true, Ordering::SeqCst);
    let mut host = PreviewsHost::new(fake.clone(), PreviewRun::new());
    host.pump();
    host.opened();
    // The read of ONE is on its way; then TWO signs in (the session turns too).
    *fake.account.lock().unwrap() = Some(TWO.into());
    *fake.session.lock().unwrap() = SessionGeneration::for_test(2);
    assert_eq!(
        host.photo_previews(),
        PhotoPreviews::Unavailable,
        "nothing of ONE before the pump"
    );
    host.pump();
    assert_eq!(host.photo_previews(), PhotoPreviews::Loading);
    // ONE's answer comes in: dropped.
    let woken = fake.wakes.load(Ordering::SeqCst);
    fake.permit();
    let began = Instant::now();
    while fake.wakes.load(Ordering::SeqCst) == woken {
        assert!(began.elapsed() < Duration::from_secs(10), "the read never answered");
        std::thread::sleep(Duration::from_millis(5));
    }
    host.pump();
    assert_eq!(
        host.photo_previews(),
        PhotoPreviews::Loading,
        "ONE's answer is never drawn"
    );
    // TWO's own read is answered and drawn (the check can pass).
    fake.permit();
    pump_until(&mut host, "TWO's read", |host| ready(host).is_some());
    assert_eq!(ready(&host).unwrap().photos, "7");
}

#[test]
fn a_failed_read_is_said_never_drawn_as_off() {
    let fake = fake();
    *fake.read_fails.lock().unwrap() = Some(PreviewsRefused::Failed);
    let mut host = PreviewsHost::new(fake.clone(), PreviewRun::new());
    host.pump();
    host.opened();
    pump_until(&mut host, "the read failed", |host| {
        host.photo_previews() != PhotoPreviews::Loading
    });
    assert_eq!(host.photo_previews(), PhotoPreviews::Failed(READ_FAILED));
    // A change cannot be built on a setting that was not read.
    host.set_on(false);
    assert_eq!(host.take_toast(), Some(CHANGE_LOCKED));
    assert!(fake.sets.lock().unwrap().is_empty());
    // Try again reads again (the check can pass).
    *fake.read_fails.lock().unwrap() = None;
    host.reload();
    pump_until(&mut host, "the read again", |host| ready(host).is_some());
}

#[test]
fn off_writes_enabled_false_keeps_the_choice_stops_the_run_and_deletes_nothing() {
    let fake = fake();
    let mut host = opened(&fake);
    // The session's own run (the setting is on, JPEG) ran as the session began.
    runs(&fake, 1);
    host.set_on(false);
    let began = Instant::now();
    while fake.sets.lock().unwrap().is_empty() {
        assert!(began.elapsed() < Duration::from_secs(10), "nothing was written");
        std::thread::sleep(Duration::from_millis(5));
    }
    let sets = fake.sets.lock().unwrap().clone();
    assert_eq!(
        sets,
        [(
            SessionGeneration::for_test(1),
            PreviewSetting {
                enabled: false,
                format: Wire::Jpeg,
                px: 1920
            }
        )]
    );
    // Off starts no run (the engine double has no delete at all: nothing else can be sent).
    pump_until(&mut host, "the switch reads off", |host| {
        ready(host).is_some_and(|p| !p.on)
    });
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(fake.makes.lock().unwrap().len(), 1);
}

#[test]
fn apply_stores_the_choice_then_runs_and_never_draws_it_first() {
    let fake = fake();
    fake.gated.store(true, Ordering::SeqCst);
    let mut host = PreviewsHost::new(fake.clone(), PreviewRun::new());
    host.pump();
    host.opened();
    fake.permit();
    pump_until(&mut host, "the pane read", |host| ready(host).is_some());
    // The session's own run has begun its work: a start that still waits is replaced by the next one
    // (`PreviewRun::start`), and Apply's would then be the only run there ever is.
    runs(&fake, 1);
    host.apply(PreviewSize::K4, PreviewFormat::Jpeg);
    // A second press while the first is on its way sends nothing, and says so.
    host.apply(PreviewSize::P720, PreviewFormat::Jpeg);
    assert_eq!(host.take_toast(), Some(CHANGE_BUSY));
    // Until the server answered and was read again, the stored choice is what is drawn.
    assert_eq!(ready(&host).unwrap().size, PreviewSize::P1080);
    fake.permit();
    pump_until(&mut host, "the new choice", |host| {
        ready(host).is_some_and(|p| p.size == PreviewSize::K4)
    });
    assert_eq!(fake.sets.lock().unwrap().len(), 1);
    // The session's own run, then the one Apply started.
    runs(&fake, 2);
    assert_eq!(fake.makes.lock().unwrap()[1], (SessionGeneration::for_test(1), false));
}

#[test]
fn a_refused_change_is_said_and_the_pane_shows_what_the_server_holds() {
    let fake = fake();
    let mut host = opened(&fake);
    runs(&fake, 1);
    fake.set_fails.store(true, Ordering::SeqCst);
    host.apply(PreviewSize::K4, PreviewFormat::Jpeg);
    pump_until(&mut host, "the refusal", |host| host.toast.is_some());
    assert_eq!(host.take_toast(), Some(CHANGE_FAILED));
    pump_until(&mut host, "the read after it", |host| !host.reading);
    assert_eq!(ready(&host).unwrap().size, PreviewSize::P1080);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(fake.makes.lock().unwrap().len(), 1, "a refused change starts no run");
}

#[test]
fn rebuild_is_a_forced_run_and_a_new_session_runs_once_when_on_and_jpeg() {
    let fake = fake();
    let mut host = opened(&fake);
    // The session's own run (on, JPEG) ran once as the session began.
    let began = Instant::now();
    while fake.makes.lock().unwrap().is_empty() {
        assert!(began.elapsed() < Duration::from_secs(10), "no session run");
        std::thread::sleep(Duration::from_millis(5));
    }
    host.rebuild();
    while fake.makes.lock().unwrap().len() < 2 {
        assert!(began.elapsed() < Duration::from_secs(10), "no rebuild");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(fake.makes.lock().unwrap()[1], (SessionGeneration::for_test(1), true));
    // HEIC stored: a new session starts no run.
    fake.setting.lock().unwrap().format = Wire::Heic;
    *fake.session.lock().unwrap() = SessionGeneration::for_test(5);
    host.pump();
    std::thread::sleep(Duration::from_millis(100));
    host.pump();
    assert_eq!(fake.makes.lock().unwrap().len(), 2);
}

#[test]
fn the_cost_is_the_accounts_average_or_the_measured_jpeg_and_never_a_table() {
    let fake = fake();
    fake.has_photo.store(true, Ordering::SeqCst);
    let mut host = opened(&fake);
    pump_until(&mut host, "the sample", |host| host.sample.is_some());
    // The account's own average (400 KB) times its 2 previews.
    assert_eq!(
        host.cost(PreviewSize::P1080, PreviewFormat::Jpeg).as_deref(),
        Some("800 KB")
    );
    // No average at 4K JPEG: the sample's measured weight (1.2 MB) times 2.
    assert_eq!(
        host.cost(PreviewSize::K4, PreviewFormat::Jpeg).as_deref(),
        Some("2.4 MB")
    );
    // HEIC with no previews of the account at that size: nothing known, no figure.
    assert_eq!(host.cost(PreviewSize::K2, PreviewFormat::Heic), None);
    // The sample is the person's own, measured here, and said as that.
    let p = ready(&host).unwrap();
    assert_eq!(p.original, "30.9 MB");
    let sample = p.sample.expect("the measured photo");
    assert!(!sample.bundled);
    assert_eq!(
        &sample.preview_png[..],
        &[0x89, b'P', b'N', b'G', 2],
        "the preview at the stored 1920"
    );
}

#[test]
fn locked_or_signed_out_hands_out_nothing() {
    let fake = fake();
    let mut host = opened(&fake);
    fake.ready.store(false, Ordering::SeqCst);
    assert_eq!(host.photo_previews(), PhotoPreviews::Unavailable);
    assert_eq!(host.cost(PreviewSize::P1080, PreviewFormat::Jpeg), None);
    host.pump();
    assert!(
        host.facts.is_none() && host.sample.is_none(),
        "what was read is dropped"
    );
}

/// The glue, once, on a real handle against the fake API with the preview routes: the setting, the facts, Off.
#[test]
fn the_engine_glue_reads_and_writes_the_accounts_setting() {
    use beebeeb_engine::app::EngineConfig;
    use beebeeb_engine::host::ChannelHost;
    use beebeeb_engine::identity::TEST;
    use beebeeb_engine::signin::memory_store::MemoryCredentials;
    use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
    use beebeeb_engine::test_api::fake_previews::PreviewRoutes;
    use beebeeb_engine::test_api::{FakeApi, Request};

    const PASSWORD: &str = "an invented password for previews";
    let (account, phrase) = FakeAccount::register("glue-previews@example.eu", PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let routes = Arc::new(PreviewRoutes::new(account.clone()));
    let (previews, own) = (routes.clone(), account_handler(account.clone()));
    let api = FakeApi::start(Arc::new(move |req: &Request| {
        previews.answer(req).unwrap_or_else(|| own(req))
    }));
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "previews-glue-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: api.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("opens"));
    engine.restore();
    engine
        .sign_in_password("glue-previews@example.eu", zeroize::Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(zeroize::Zeroizing::new(phrase))
        .expect("the phrase unlocks");
    let session = PreviewsEngine::session(&*engine);
    assert!(PreviewsEngine::ready(&*engine));
    // Unset: the server's default.
    assert_eq!(
        PreviewsEngine::setting(&*engine, session).unwrap(),
        PreviewSetting::default()
    );
    routes.store("e0000000-0000-4000-8000-000000000001", "jpeg", 1920, vec![0; 300]);
    let on = PreviewSetting {
        enabled: true,
        format: Wire::Jpeg,
        px: 1920,
    };
    PreviewsEngine::set_setting(&*engine, session, on).unwrap();
    let facts = PreviewsEngine::facts(&*engine, session).unwrap();
    assert_eq!(
        (facts.setting, facts.summary.count, facts.summary.total_bytes),
        (on, 1, 300)
    );
    PreviewsEngine::set_setting(&*engine, session, PreviewSetting { enabled: false, ..on }).unwrap();
    assert_eq!(routes.preference_puts().len(), 2);
    assert_eq!(routes.deletes(), 0, "off deletes no preview");
    // Another session's question answers nothing.
    let other = SessionGeneration::for_test(9_999);
    assert_eq!(
        PreviewsEngine::facts(&*engine, other).unwrap_err(),
        PreviewsRefused::NotSignedIn
    );
    assert!(routes.puts() == 0);
}

#[test]
fn while_making_only_the_summary_is_read_again() {
    let fake = fake();
    fake.hold_make.store(true, Ordering::SeqCst);
    let mut host = opened(&fake);
    pump_until(&mut host, "the session's run makes previews", |host| {
        matches!(host.run_state(), RunState::Making { .. })
    });
    let whole_reads = fake.facts_reads.load(Ordering::SeqCst);
    fake.extra.store(1, Ordering::SeqCst);
    // The read is due again.
    host.read_at = Instant::now().checked_sub(READ_EVERY_WHILE_MAKING);
    pump_until(&mut host, "the count follows the summary", |host| {
        ready(host).is_some_and(|previews| previews.photos == "3")
    });
    assert!(fake.summary_reads.load(Ordering::SeqCst) >= 1);
    assert_eq!(
        fake.facts_reads.load(Ordering::SeqCst),
        whole_reads,
        "the whole read (the per-file list) is not asked while the run makes previews"
    );
    fake.hold_make.store(false, Ordering::SeqCst);
}

#[test]
fn a_picture_is_wiped_by_whoever_lets_go_of_it_last() {
    let mut held: Arc<[u8]> = Arc::from(&[1u8, 2, 3][..]);
    let other = held.clone();
    wipe_if_last(&mut held);
    assert_eq!(&held[..], &[1, 2, 3], "another holder still draws it");
    drop(other);
    wipe_if_last(&mut held);
    assert_eq!(&held[..], &[0, 0, 0]);
    // The picture that ships inside the app is never wiped.
    let bundled = Sample::bundled();
    drop(Sample::bundled());
    assert_ne!(bundled.original_png.iter().filter(|b| **b != 0).count(), 0);
}

/// Wait until a held run's work saw its stop flag (at most 5 s).
fn stopped_in_time(fake: &FakePreviews) -> bool {
    let began = Instant::now();
    while !fake.saw_stop.load(Ordering::SeqCst) {
        if began.elapsed() > Duration::from_secs(5) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

/// A lock (or a sign-out, another account) while a run makes previews: the run is told to stop.
#[test]
fn a_lock_stops_the_run_of_the_session_before_it() {
    let fake = fake();
    fake.hold_make.store(true, Ordering::SeqCst);
    let mut host = opened(&fake);
    pump_until(&mut host, "the session's run makes previews", |host| {
        matches!(host.run_state(), RunState::Making { .. })
    });
    assert!(!fake.saw_stop.load(Ordering::SeqCst));
    fake.ready.store(false, Ordering::SeqCst);
    host.pump();
    assert!(
        stopped_in_time(&fake),
        "the run of the locked session was not told to stop"
    );
    fake.hold_make.store(false, Ordering::SeqCst);
}

/// Off pressed while a Rebuild is still queued behind other work: once Off is written, the forced run that started
/// for the Rebuild is stopped.
#[test]
fn off_stops_a_rebuild_that_was_still_queued() {
    let fake = fake();
    let mut host = opened(&fake);
    runs(&fake, 1);
    // The worker is busy with a read the test holds, so the Rebuild and the Off queue behind it and the switch's own
    // stop comes before the Rebuild's start. The Off's write is held until the forced run has taken its work.
    fake.gated.store(true, Ordering::SeqCst);
    host.reload();
    fake.hold_make.store(true, Ordering::SeqCst);
    fake.hold_set.store(true, Ordering::SeqCst);
    host.rebuild();
    host.set_on(false);
    fake.permit();
    runs(&fake, 2);
    assert_eq!(
        fake.makes.lock().unwrap()[1].1,
        true,
        "the Rebuild's forced run started"
    );
    assert!(
        !fake.saw_stop.load(Ordering::SeqCst),
        "nothing stopped the forced run before the Off's write"
    );
    fake.hold_set.store(false, Ordering::SeqCst);
    assert!(stopped_in_time(&fake), "Off did not stop the forced run");
    assert!(fake.sets.lock().unwrap().iter().any(|(_, setting)| !setting.enabled));
    fake.gated.store(false, Ordering::SeqCst);
    fake.hold_make.store(false, Ordering::SeqCst);
}

/// The pane says previews are being made only once the run counted what it makes: a run that has counted nothing
/// (one waiting for sync's first pass) says nothing; the next run, counting, says it.
#[test]
fn the_making_notice_needs_a_count() {
    let fake = fake();
    fake.quiet_make.store(true, Ordering::SeqCst);
    fake.hold_make.store(true, Ordering::SeqCst);
    let mut host = opened(&fake);
    runs(&fake, 1);
    assert_eq!(ready(&host).unwrap().notice, None, "a run that counted nothing");
    fake.hold_make.store(false, Ordering::SeqCst);
    fake.quiet_make.store(false, Ordering::SeqCst);
    pump_until(&mut host, "the first run ended", |host| {
        ready(host).is_some_and(|p| p.notice.is_none())
    });
    std::thread::sleep(Duration::from_millis(50));
    fake.hold_make.store(true, Ordering::SeqCst);
    host.rebuild();
    runs(&fake, 2);
    pump_until(&mut host, "the counting run says it", |host| {
        ready(host).is_some_and(|p| p.notice == Some(REBUILDING))
    });
    fake.hold_make.store(false, Ordering::SeqCst);
}

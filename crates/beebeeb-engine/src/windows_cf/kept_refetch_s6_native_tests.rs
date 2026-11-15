//! S6, the Windows-native half on a QA sync root (a child of the LD-0105 rungs: their QA root is reused): the kept
//! files' batch with the PRODUCTION fetch (`hydrate_placeholder`) brings a kept placeholder this PC does not hold
//! down, and a place with nothing there is "not there", not a failure.

use super::*;

/// A provider that serves every request whole (the engine's paced callback stands in).
unsafe extern "system" fn s6_serve_whole(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    if info.is_null() || params.is_null() {
        return;
    }
    // SAFETY: valid for the duration of the callback.
    let r = unsafe { request_of(info, params) };
    let data = content(r.file_size as usize);
    // SAFETY: the keys of this live request.
    unsafe {
        let _ = transfer_range(r.connection, r.transfer, r.request, &data, || false, r.offset, r.length);
    }
}

/// S6 (R4): the batch's production fetch hydrates a kept (in the database), dehydrated placeholder (one file at a time, blocking until
/// the bytes are on disk); a path with nothing there reads as not there. Red on the mutant "the production fetch
/// returns `Done` without fetching".
#[test]
fn s6_kept_native_the_production_fetch_brings_a_kept_placeholder_down() {
    use crate::engine_bridge::{KeptFetch, KeptPlace, KeptRefetch};
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(s6_serve_whole);
    let len = 300_000;
    assert!(placeholders::create_placeholder(&qa.root, "qa-s6-kept", "kept.bin", len, 1_700_000_000, false).unwrap());
    // No OS pin: pinning a cloud-only placeholder on a connected root makes Windows fetch it itself (measured,
    // `ld0105_w1_native_…`), so the batch would meet it on disk. "Kept" is the database's state, which the plan reads.
    let kept = qa.path("kept.bin");
    let before = placeholders::fully_on_disk(&kept);
    let dir = tempfile::tempdir().unwrap();
    let db = std::sync::Arc::new(crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap());
    let batch = vec![
        KeptPlace {
            file_id: "qa-s6-kept".into(),
            place: kept.clone(),
        },
        KeptPlace {
            file_id: "qa-s6-none".into(),
            place: qa.path("nothing-here.bin"),
        },
    ];
    // Bounded: a fetch the provider never serves would block the test for ever.
    let (sent, outcome) = std::sync::mpsc::channel();
    let batch_db = db.clone();
    std::thread::spawn(move || {
        let fetched = Mutex::new(Vec::new());
        let fetch = |place: &Path| {
            let one = crate::engine_bridge::fetch_kept_placeholder(place);
            fetched.lock().unwrap().push(one);
            one
        };
        let done = KeptRefetch::default().run_batch(&batch_db, batch, &fetch, &|| true);
        let _ = sent.send((done, fetched.into_inner().unwrap()));
    });
    let (done, fetched) = outcome
        .recv_timeout(Duration::from_secs(60))
        .expect("the batch ended within 60 s");
    let on_disk = placeholders::fully_on_disk(&kept);
    println!("s6 kept native: before={before} on_disk={on_disk} fetched={fetched:?} batch={done:?}");
    assert!(!before, "precondition: the kept placeholder is not on disk");
    assert_eq!(fetched, [KeptFetch::Done, KeptFetch::NotThere]);
    assert!(on_disk, "the production fetch brought it down");
    assert_eq!(std::fs::read(&kept).unwrap(), content(len as usize), "its bytes");
}

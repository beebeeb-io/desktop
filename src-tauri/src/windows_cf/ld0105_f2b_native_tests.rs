//! LD-0105 #225 F2b, the Windows-native rungs for robocopy ERROR 379: a diagnosis matrix (ignored, run
//! alone) and the fix's rung. Each test registers its OWN sync root ([`QaRoot`], the QA provider);
//! nothing here talks to a server or touches the person's root.

use super::*;

/// F2b (desktop #225 QA rung, robocopy ERROR 379): what the provider does with a FETCH.
/// 0: the 16 MiB chunk that covers it (as a range session transfers); 1: only the required range;
/// 2: the whole file; 3: as 0, but from a worker thread after the callback returned (as a session);
/// 4: as 3, each chunk in ONE CfExecute (as a session's StreamSink: 16 MiB per call, not 1 MiB);
/// 5: as 0, each chunk in one CfExecute.
/// Nothing is served before [`F2B_DELAY_MS`] after the first FETCH (a session's preparation).
static F2B_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static F2B_DELAY_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static F2B_READY_AT: Mutex<Option<Instant>> = Mutex::new(None);
static F2B_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
static F2B_SERVED: Mutex<Vec<(i64, i64)>> = Mutex::new(Vec::new());
static F2B_T0: Mutex<Option<Instant>> = Mutex::new(None);
static F2B_DATA: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
/// The QA rung's clip-b.mp4.
const F2B_SIZE: usize = 344_661_074;

fn f2b_note(line: String) {
    let ms = F2B_T0.lock().unwrap().get_or_insert_with(Instant::now).elapsed().as_secs_f64() * 1000.0;
    F2B_LOG.lock().unwrap().push(format!("{ms:10.1} ms {line}"));
}

fn f2b_serve(r: Request) {
    let ready = *F2B_READY_AT
        .lock()
        .unwrap()
        .get_or_insert_with(|| Instant::now() + Duration::from_millis(F2B_DELAY_MS.load(std::sync::atomic::Ordering::SeqCst)));
    std::thread::sleep(ready.saturating_duration_since(Instant::now()));
    let data = F2B_DATA.get().expect("initialised before robocopy starts");
    let size = data.len() as i64;
    let mut served = F2B_SERVED.lock().unwrap();
    if served.iter().any(|&(s, e)| s <= r.offset && r.offset + r.length <= e) {
        f2b_note(format!("  [{}, +{}) already served: no transfer", r.offset, r.length));
        return;
    }
    const CHUNK: i64 = 16 << 20;
    let mode = F2B_MODE.load(std::sync::atomic::Ordering::SeqCst);
    let (start, end) = match mode {
        1 => (r.offset, r.offset + r.length),
        2 => (0, size),
        _ => (r.offset / CHUNK * CHUNK, ((r.offset + r.length + CHUNK - 1) / CHUNK * CHUNK).min(size)),
    };
    // SAFETY: the keys of a request Windows has not completed or cancelled (a refusal is logged).
    let result = if matches!(mode, 4 | 5) {
        // SAFETY: as above; the buffer outlives the call (CfExecute copies it).
        unsafe {
            crate::windows_cf::callbacks::transfer_one(
                r.connection,
                r.transfer,
                r.request,
                data.as_ptr().add(start as usize) as *const core::ffi::c_void,
                start,
                end - start,
                windows::Win32::Foundation::STATUS_SUCCESS,
            )
        }
    } else {
        unsafe { transfer_range(r.connection, r.transfer, r.request, data, || false, start, end - start) }
    };
    f2b_note(format!("  TRANSFER [{start}, {end}) -> {:?}", result.as_ref().map_err(|e| format!("0x{:08X}", e.code().0 as u32))));
    if result.is_ok() {
        served.push((start, end));
    }
}

unsafe extern "system" fn f2b_fetch(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    if info.is_null() || params.is_null() {
        return;
    }
    // SAFETY: valid for the duration of the callback.
    let r = unsafe { request_of(info, params) };
    // SAFETY: the fetch arm of a FETCH_DATA callback.
    let fetch = unsafe { (*params).Anonymous.FetchData };
    // SAFETY: the info pointer is live; ProcessInfo is filled (REQUIRE_PROCESS_INFO).
    let pid = unsafe { if (*info).ProcessInfo.is_null() { 0 } else { (*(*info).ProcessInfo).ProcessId } };
    f2b_note(format!(
        "FETCH req [{}, +{}) opt [{}, +{}) pid {pid} rkey {}",
        r.offset, r.length, fetch.OptionalFileOffset, fetch.OptionalLength, r.request
    ));
    if matches!(F2B_MODE.load(std::sync::atomic::Ordering::SeqCst), 3 | 4) {
        std::thread::spawn(move || f2b_serve(r));
    } else {
        f2b_serve(r);
    }
}

unsafe extern "system" fn f2b_cancel(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    if info.is_null() || params.is_null() {
        return;
    }
    // SAFETY: the cancel arm of a CANCEL_FETCH_DATA callback.
    let cancel = unsafe { (*params).Anonymous.Cancel.Anonymous.FetchData };
    f2b_note(format!("CANCEL [{}, +{})", cancel.FileOffset, cancel.Length));
}

/// One robocopy of `name` from the QA root: its exit code and its ERROR lines.
fn f2b_robocopy(qa: &QaRoot, name: &str, dst: &Path, flags: &[&str]) -> (Option<i32>, String) {
    let out = std::process::Command::new("robocopy")
        .arg(&qa.root)
        .arg(dst)
        .arg(name)
        .args(flags)
        .args(["/R:0", "/W:0", "/NP", "/NJH", "/NJS"])
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            let errors: Vec<&str> = text.lines().filter(|l| l.contains("ERROR")).map(str::trim).collect();
            (o.status.code(), errors.join(" | "))
        }
        Err(e) => (None, format!("spawn failed: {e}")),
    }
}

/// F2b diagnosis matrix (desktop #225, lead 2026-11-06): which provider behaviour, delay and robocopy
/// option decide ERROR 379. Each case: a fresh placeholder of the QA clip's size, robocopy twice (the
/// second run is robocopy's retry on whatever the first left on disk). A measurement: it prints, it
/// asserts nothing about robocopy. Run alone:
/// `cargo test --lib ld0105_f2b_native_robocopy_matrix -- --ignored --nocapture`.
#[test]
#[ignore = "F2b diagnosis matrix (robocopy runs of a 329 MiB file): run alone with --ignored"]
fn ld0105_f2b_native_robocopy_matrix() {
    let progressive = crate::windows_cf::HYDRATION_POLICY;
    let data = F2B_DATA.get_or_init(|| content(F2B_SIZE));
    let all_off: &'static [&'static str] = &["/NOOFFLOAD", "/NOCLONE", "/SPARSE:N"];
    let cases: Vec<(&str, CF_HYDRATION_POLICY_PRIMARY, u8, u64, &[&str])> = vec![
        ("progressive", progressive, 3, 1700, &[]),
        ("progressive", progressive, 4, 1700, &[]),
        ("progressive", progressive, 4, 1700, &["/J"]),
        ("progressive", progressive, 4, 1700, all_off),
        ("progressive", progressive, 5, 0, &[]),
        ("progressive", progressive, 5, 0, &["/J"]),
        ("partial", CF_HYDRATION_POLICY_PARTIAL, 4, 1700, &[]),
    ];
    let mut table = Vec::new();
    for (i, (policy_name, policy, mode, delay, flags)) in cases.into_iter().enumerate() {
        let mut qa = QaRoot::new(policy);
        qa.connect_with(f2b_fetch, Some(f2b_cancel));
        F2B_MODE.store(mode, std::sync::atomic::Ordering::SeqCst);
        F2B_DELAY_MS.store(delay, std::sync::atomic::Ordering::SeqCst);
        *F2B_READY_AT.lock().unwrap() = None;
        *F2B_T0.lock().unwrap() = None;
        F2B_SERVED.lock().unwrap().clear();
        F2B_LOG.lock().unwrap().clear();
        let name = format!("f2b-{i}.mp4");
        assert!(placeholders::create_placeholder(&qa.root, &format!("qa-f2b-{i}"), &name, F2B_SIZE as i64, 1_700_000_000, false).unwrap());
        let dst = tempfile::tempdir().unwrap();
        let first = f2b_robocopy(&qa, &name, dst.path(), flags);
        let after_first = census_of(&qa.path(&name)).0;
        let second = f2b_robocopy(&qa, &name, dst.path(), flags);
        let copied = std::fs::read(dst.path().join(&name)).ok();
        let whole = copied.as_deref() == Some(data.as_slice());
        // A worker thread of mode 3 may still be serving: let it finish before the root goes.
        std::thread::sleep(Duration::from_millis(300));
        let log = F2B_LOG.lock().unwrap().clone();
        let cancels = log.iter().filter(|l| l.contains("CANCEL")).count();
        let row = format!(
            "f2b case {i}: policy {policy_name} mode {mode} delay {delay} ms flags {flags:?}: run1 exit {:?} [{}] on disk {after_first:?}; run2 exit {:?} [{}]; copy byte-equal {whole}; fetches {} cancels {cancels}",
            first.0,
            first.1,
            second.0,
            second.1,
            log.iter().filter(|l| l.contains("FETCH")).count()
        );
        println!("{row}");
        for line in log.iter().take(60) {
            println!("    {line}");
        }
        table.push(row);
        drop(qa);
    }
    println!("ld0105-f2b-native: {} cases", table.len());
    for row in &table {
        println!("{row}");
    }
}

static FIX_SERVED: Mutex<Vec<(i64, i64)>> = Mutex::new(Vec::new());

/// A provider that answers each FETCH as a range session does: the 16 MiB chunk that covers it, sent
/// through the session's own [`crate::windows_cf::streaming::transfer_piece`] (nothing twice).
unsafe extern "system" fn session_piece_fetch(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    if info.is_null() || params.is_null() {
        return;
    }
    // SAFETY: valid for the duration of the callback.
    let r = unsafe { request_of(info, params) };
    let data = F2B_DATA.get().expect("initialised before robocopy starts");
    let mut served = FIX_SERVED.lock().unwrap();
    if served.iter().any(|&(s, e)| s <= r.offset && r.offset + r.length <= e) {
        return;
    }
    const CHUNK: i64 = 16 << 20;
    let start = r.offset / CHUNK * CHUNK;
    let end = ((r.offset + r.length + CHUNK - 1) / CHUNK * CHUNK).min(data.len() as i64);
    let keys = crate::cf_range::RequestKeys { transfer_key: r.transfer, request_key: r.request };
    // SAFETY: the keys of this live request.
    if unsafe { crate::windows_cf::streaming::transfer_piece(r.connection, keys, start as u64, &data[start as usize..end as usize]) }.is_ok() {
        served.push((start, end));
    }
}

/// The fix's rung (desktop #225 F2b, lead 2026-11-06: "robocopy buffered x3 -> exit 0"): a provider that
/// serves 16 MiB session pieces through the session's own `transfer_piece`; three buffered robocopies
/// of fresh placeholders of the QA clip's size each copy it, byte for byte. Red on the mutant
/// `CF_TRANSFER_MAX = 16 MiB` (one CfExecute per piece, as at fa0c6fc): ERROR 379, exit 8.
#[test]
fn ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces() {
    let data = F2B_DATA.get_or_init(|| content(F2B_SIZE));
    let mut runs = Vec::new();
    for i in 0..3 {
        let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
        qa.connect(session_piece_fetch);
        FIX_SERVED.lock().unwrap().clear();
        let name = format!("robo-{i}.mp4");
        assert!(placeholders::create_placeholder(&qa.root, &format!("qa-robo-{i}"), &name, F2B_SIZE as i64, 1_700_000_000, false).unwrap());
        let dst = tempfile::tempdir().unwrap();
        let (exit, errors) = f2b_robocopy(&qa, &name, dst.path(), &[]);
        let whole = std::fs::read(dst.path().join(&name)).ok().as_deref() == Some(data.as_slice());
        runs.push((exit, errors, whole));
    }
    println!("ld0105-f2b-native: buffered robocopy x3 of session pieces (CF_TRANSFER_MAX {}): {runs:?}", crate::windows_cf::streaming::CF_TRANSFER_MAX);
    for (exit, errors, whole) in &runs {
        assert_eq!(*exit, Some(1), "robocopy copied the file (exit 1), errors: [{errors}]");
        assert!(*whole, "the copy is byte for byte the file");
    }
}

// The confinement's hardening after two security reviews, a child of `mod windows` in `tests/sandbox.rs` (a file of
// its own so that one stays under the 1,500-line limit for tests): the process mitigations, the token's
// less-privileged flag and integrity level, the memory watch, and the helper's handle count. Every claim shows its
// instrument able to fail first, on a plain probe.

use windows_sys::Win32::System::ProcessStatus::{K32GetPerformanceInfo, PERFORMANCE_INFORMATION};

use super::*;

const MIB: u64 = 1024 * 1024;

/// What a probe did with one request: its answer, or `None` when it ended instead of answering.
fn answer_or_end(probe: &mut Probe, request: &str) -> Option<String> {
    assert_eq!(probe.ask("ping"), "pong", "the probe answers before the attempt");
    probe.send(request);
    let mut line = String::new();
    let read = probe
        .from
        .as_mut()
        .expect("reader")
        .read_line(&mut line)
        .expect("the answer is readable");
    (read > 0).then(|| line.trim().to_string())
}

/// The number after `ok `, or a failed test that shows the answer.
fn ok_number(answer: &str) -> u64 {
    answer
        .strip_prefix("ok ")
        .and_then(|number| number.parse().ok())
        .unwrap_or_else(|| panic!("an answer with a number: {answer}"))
}

/// Grants every section to the probe read-only and returns the `hold-grants` request for them.
fn hold_request(probe: &Probe, sections: &[Section]) -> String {
    let pairs: Vec<String> = sections
        .iter()
        .map(|section| {
            let granted = probe.child().grant(section, Access::Read).expect("a grant");
            format!("{} {}", granted.wire, granted.len)
        })
        .collect();
    format!("hold-grants {}", pairs.join(" "))
}

/// The system's committed memory, in bytes.
fn system_commit() -> u64 {
    let mut info = PERFORMANCE_INFORMATION {
        cb: size_of::<PERFORMANCE_INFORMATION>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is writable for `cb` bytes.
    assert_ne!(
        unsafe { K32GetPerformanceInfo(&mut info, info.cb) },
        0,
        "the system's counters"
    );
    (info.CommitTotal as u64) * (info.PageSize as u64)
}

/// A window, executable memory, and a handle the process does not hold: each works in a plain probe, and each is
/// refused (an error answer, or the process ended) in a confined one; the handle, by ending the process with
/// 0xC0000008 (strict handle checks).
#[test]
fn the_confined_helper_runs_under_the_mitigations() {
    let _serial = serial();
    let mut plain = Probe::plain();
    assert_eq!(
        plain.attempt("window"),
        "ok",
        "the instrument can fail: a plain probe makes a window and reads the screen"
    );
    assert_eq!(plain.attempt("jit"), "ok", "a plain probe makes executable memory");
    assert_eq!(
        plain.attempt("bad-handle"),
        "ok refused 6",
        "a plain probe holds no such handle, and survives using it (the call fails: invalid handle)"
    );
    drop(plain);

    let mut refused = 0;
    for verb in ["window", "jit", "bad-handle"] {
        let mut confined = Probe::confined();
        let answer = answer_or_end(&mut confined, verb);
        let ended = match answer {
            Some(_) => None,
            None => Some(
                confined
                    .child()
                    .wait_timeout(BOUND)
                    .unwrap()
                    .map(|status| status.code())
                    .expect("a probe that did not answer has ended"),
            ),
        };
        println!("mitigations: {verb}: answer {answer:?}, ended with {ended:x?}");
        if verb == "bad-handle" {
            assert_eq!(
                (answer.as_deref(), ended),
                (None, Some(ENDED_FOR_INVALID_HANDLE)),
                "bad-handle: the confined probe is ended for using a handle it does not hold"
            );
        }
        assert!(
            answer.as_deref().is_none_or(|line| !line.starts_with("ok")),
            "{verb}: the confined probe answered {answer:?}"
        );
        refused += 1;
    }
    assert_eq!(refused, 3);
}

/// The confined probe reads its own strict-handle-check policy as on and permanent, in both profiles; a plain
/// probe's is off (the instrument can fail).
#[test]
fn the_confined_helper_has_strict_handle_checks_on() {
    let _serial = serial();
    let mut plain = Probe::plain();
    assert_eq!(plain.attempt("strict-handles"), "ok raise=0 permanent=0");
    drop(plain);
    for profile in [Profile::NoCapabilities, Profile::SystemDecoder { hevc: false }] {
        let mut confined = Probe::confined_as(profile);
        assert_eq!(
            confined.attempt("strict-handles"),
            "ok raise=1 permanent=1",
            "{profile:?}"
        );
    }
}

/// The token of a confined probe is a less-privileged container at low integrity, in both profiles; a plain probe's
/// token is neither (the instrument can fail).
#[test]
fn the_token_reads_back_lpac_and_low_integrity() {
    let _serial = serial();
    let mut plain = Probe::plain();
    let answer = plain.attempt("token");
    let rid = answer
        .split("rid=0x")
        .nth(1)
        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
        .unwrap_or_else(|| panic!("the plain probe read its level: {answer}"));
    assert!(
        !answer.contains("lpac=1"),
        "the instrument can fail: a plain probe is no container: {answer}"
    );
    assert!(rid >= 0x2000, "a plain probe runs at medium or higher: {rid:#x}");
    drop(plain);

    for profile in [Profile::NoCapabilities, Profile::SystemDecoder { hevc: false }] {
        let mut confined = Probe::confined_as(profile);
        assert_eq!(confined.attempt("token"), "token lpac=1 rid=0x1000", "{profile:?}");
    }
}

/// A section the helper makes itself is not charged to the job's cap; the memory watch counts it and ends the helper.
#[test]
fn a_section_the_helper_makes_is_counted_and_ended() {
    let _serial = serial();
    const ASKED: u64 = 1536;
    let mut plain = Probe::plain();
    let shared = ok_number(&plain.attempt(&format!("section-commit {ASKED}")));
    assert!(
        shared >= ASKED * MIB,
        "the instrument counts: the plain probe's shared commit is {shared}"
    );
    drop(plain);

    // The default limit: 1 GiB.
    let mut confined = Probe::confined();
    assert!(!confined.child().killed_for_memory(), "nothing to end it for yet");
    let started = Instant::now();
    let answer = answer_or_end(&mut confined, &format!("section-commit {ASKED}"));
    let status = confined.child().wait_timeout(BOUND).unwrap();
    let took = started.elapsed();
    println!(
        "memory watch: answer {answer:?}, ended {:x?} after {took:?}",
        status.map(|status| status.code())
    );
    let status = status.expect("the helper ended within the bound");
    assert!(status.was_killed(), "ended by a kill, code {:#x}", status.code());
    assert!(confined.child().killed_for_memory(), "ended by the memory watch");
    assert!(took <= BOUND, "within the bound: {took:?}");
}

/// A measurement for the checkpoint's issues list, not a claim: what the job, the helper and the system count of
/// sections the app granted and the helper keeps, while the app holds them and after it dropped them. It asserts only
/// that the instruments move, on a plain probe's own section.
#[test]
#[ignore = "O-CP3: F4/R2 measurement"]
fn what_the_job_counts_of_held_grants() {
    let _serial = serial();
    const SECTIONS: usize = 8;
    const BYTES: usize = 64 << 20;

    let before = system_commit();
    let mut plain = Probe::plain();
    let shared = ok_number(&plain.attempt("section-commit 512"));
    let after = system_commit();
    assert!(shared >= 512 * MIB, "the probe's shared commit moved: {shared}");
    assert!(
        after.saturating_sub(before) >= 256 * MIB,
        "the system's commit moved: {before} -> {after}"
    );
    drop(plain);

    // The instrument can fail: a plain probe reads its own counters.
    let mut plain = Probe::plain();
    let own = plain.attempt("memory");
    assert!(
        own.starts_with("memory private="),
        "the plain probe read its counters: {own}"
    );
    drop(plain);

    let mut confined = Probe::confined();
    let commit_before = system_commit();
    let memory_before = confined.attempt("memory");
    let sections: Vec<Section> = (0..SECTIONS)
        .map(|_| {
            let mut section = Section::new(BYTES).expect("a section");
            section.as_mut_slice().fill(0x42);
            section
        })
        .collect();
    let request = hold_request(&confined, &sections);
    assert_eq!(confined.attempt(&request), format!("ok {SECTIONS}"));
    let commit_held = system_commit();
    let memory_held = confined.attempt("memory");
    drop(sections);
    let commit_dropped = system_commit();
    let memory_dropped = confined.attempt("memory");
    println!(
        "measurement held-grants: {SECTIONS} x {BYTES} bytes granted read-only and kept | before: {memory_before} \
         commit={commit_before} | held, app holds them: {memory_held} commit={commit_held} (delta {}) | app dropped \
         them: {memory_dropped} commit={commit_dropped} (delta {}) | killed_for_memory={}",
        commit_held as i128 - commit_before as i128,
        commit_dropped as i128 - commit_before as i128,
        confined.child().killed_for_memory()
    );
}

/// The helper's handle count grows with every grant it keeps.
#[test]
fn handle_count_reads_the_helper() {
    let _serial = serial();
    // The instrument can fail: `hold-grants` maps and keeps a section in a plain probe, which inherits the handle.
    let control = Section::new(65_536).expect("a section");
    make_inheritable(control.as_raw_handle());
    let mut plain = Probe::plain();
    let wire = control.as_raw_handle() as usize as u64;
    assert_eq!(plain.attempt(&format!("hold-grants {wire} 65536")), "ok 1");
    drop(plain);
    drop(control);

    let mut confined = Probe::confined();
    assert_eq!(confined.ask("ping"), "pong");
    let baseline = confined.child().handle_count().expect("a count");
    assert!(
        baseline > 0,
        "the instrument reads something: a running process holds handles"
    );
    let sections: Vec<Section> = (0..8).map(|_| Section::new(65_536).expect("a section")).collect();
    let request = hold_request(&confined, &sections);
    assert_eq!(confined.attempt(&request), "ok 8");
    let held = confined.child().handle_count().expect("a count");
    println!("handles: baseline {baseline}, holding 8 grants {held}");
    assert!(
        held >= baseline + 8,
        "{held} handles after 8 kept grants, {baseline} before"
    );
}

/// A section is copied out whole, byte for byte, through the raw view (its length is not a multiple of 8, so the
/// word loop and the tail both run), and a buffer of another length is refused with nothing copied.
#[test]
fn a_section_is_copied_out_whole_and_only_into_its_own_length() {
    const LEN: usize = 3 * 4096 + 5;
    let mut section = Section::new(LEN).expect("a section");
    for (index, byte) in section.as_mut_slice().iter_mut().enumerate() {
        *byte = pattern(index);
    }
    let mut out = vec![0u8; LEN];
    assert_eq!(section.copy_to(&mut out), Ok(()));
    let same = out
        .iter()
        .enumerate()
        .filter(|&(index, &byte)| byte == pattern(index))
        .count();
    assert_eq!(same, LEN, "every byte arrived in its place");

    let mut copied = 0;
    for wrong in [LEN - 1, LEN + 1] {
        let mut out = vec![0u8; wrong];
        assert_eq!(
            section.copy_to(&mut out),
            Err(SandboxError::Limits("a copy whose length is not the section's")),
            "a buffer of {wrong} bytes"
        );
        assert_eq!(out.iter().filter(|&&byte| byte != 0).count(), 0, "nothing was copied");
        copied += 1;
    }
    assert_eq!(copied, 2);
}

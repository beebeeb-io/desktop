//! Task 2530: a parked change on This PC's problems list, through the adapter: the row carries the ways out the engine
//! lists for it and no other; a way out is asked once, for the session of the press, against the version the row was
//! read at, and says what it did; a late press after a session move asks nothing and its answer is dropped; Try again
//! on a park whose way out it is makes the change due. Every name is invented.

use super::*;

/// A parked viewer save of `file_id` (version 1, a write id, a staged copy at `staged`), parked as `class`.
fn parked(db: &StateDb, op: &str, file_id: &str, class: &str, staged: &str) {
    file(
        db,
        file_id,
        &format!("Invented/{file_id}.txt"),
        FileStatus::Error,
        10,
        false,
    );
    db.test_execute(&format!(
        "UPDATE files SET current_version = 1 WHERE file_id = '{file_id}'"
    ))
    .unwrap();
    db.test_execute(&format!(
        "INSERT INTO operation_queue (op_id, kind, file_id, target_path, metadata_json, payload_path, base_version,
                                      attempts, max_attempts, next_retry_at, last_error, last_error_class,
                                      created_at, updated_at, write_id, write_origin)
         VALUES ('{op}', 'upload_version', '{file_id}', 'Invented/{file_id}.txt', '{{}}', '{staged}', 1,
                 25, 25, 0, '{class}', '{class}', {t}, {t}, 'w-{op}', 'minted')",
        t = now() - 60
    ))
    .unwrap();
}

/// A PC with one parked change, refused by the server (403): its way out is Keep the other.
fn refused_pc() -> (Arc<Fake>, tempfile::TempDir) {
    let fake = Fake::new();
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    std::fs::write(&staged, b"my edit").unwrap();
    parked(
        &fake.db(),
        "op-p",
        "f-p",
        "permission_denied",
        &staged.to_string_lossy(),
    );
    (fake, dir)
}

/// [`settle`], keeping every `Pumped` the source answered.
fn settle_all<E: ThisPcEngine>(source: &mut EngineThisPcSource<E>) -> Vec<Pumped> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut pumped = Vec::new();
    loop {
        pumped.push(source.pump());
        if source.idle() {
            pumped.push(source.pump());
            return pumped;
        }
        assert!(Instant::now() < deadline, "the worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn exits_of(source: &EngineThisPcSource<Fake>, problem: &str) -> Vec<Exit> {
    source
        .problems()
        .into_iter()
        .find(|c| c.problem() == Some(problem))
        .map(|c| c.exits().to_vec())
        .unwrap_or_default()
}

#[test]
fn a_parked_change_offers_its_ways_out_and_settles_once_for_the_press() {
    let (fake, _dir) = refused_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    assert_eq!(exits_of(&source, "op-p"), vec![Exit::KeepOther]);
    assert!(
        !source.problems()[0].can_retry(),
        "no Try again the engine did not list"
    );

    // A way out the row does not list asks nothing.
    source.settle("op-p", Exit::KeepThisPcs);
    source.settle("not-in-the-list", Exit::KeepOther);
    settle(&mut source);
    assert!(fake.settled.lock().unwrap().is_empty());

    source.settle("op-p", Exit::KeepOther);
    let pumped = settle_all(&mut source);
    assert_eq!(
        *fake.settled.lock().unwrap(),
        vec![("op-p".to_string(), ParkedPick::KeepOther, Some(1))],
        "asked once, against the version read"
    );
    assert!(
        pumped.iter().any(|p| p.done == Some(SETTLED_KEPT_OTHER)),
        "said what it did"
    );
    assert!(pumped.iter().all(|p| p.refusal.is_none()));
    assert!(source.problems().is_empty(), "the read after it no longer finds it");
}

/// R6: a press stamped with a session that moved on asks the engine nothing, its answer is dropped (no words), and
/// the list is read again for the session now.
#[test]
fn a_late_settle_answer_is_dropped() {
    let (fake, _dir) = refused_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    assert_eq!(exits_of(&source, "op-p"), vec![Exit::KeepOther]);

    // Another session (a lock and an unlock, a sign-in) before the window's next pump saw it: the press carries the
    // old one.
    fake.session.fetch_add(1, Ordering::SeqCst);
    source.settle("op-p", Exit::KeepOther);
    let pumped = settle_all(&mut source);
    assert!(
        fake.settled.lock().unwrap().is_empty(),
        "nothing was asked of the engine"
    );
    assert!(
        pumped.iter().all(|p| p.done.is_none() && p.refusal.is_none()),
        "no words for it"
    );
    assert!(
        fake.db().get_operation("op-p").unwrap().is_some(),
        "the parked change is as it was"
    );
    settle(&mut source);
    assert_eq!(
        exits_of(&source, "op-p"),
        vec![Exit::KeepOther],
        "read again for the session now: still there"
    );

    // A job stamped with the press's session that reaches the worker after the session moved: the engine is asked
    // nothing, for the old session or the new one.
    let pressed_under = Whose {
        account: "invented@beebeeb.io".into(),
        session: SessionGeneration::for_test(fake.session.load(Ordering::SeqCst)),
    };
    fake.session.fetch_add(1, Ordering::SeqCst);
    assert_eq!(
        super::super::settle(&*fake, &pressed_under, "op-p", ParkedPick::KeepOther, Some(1)),
        None,
        "a press of an earlier session is not this session's to answer"
    );
    assert!(
        fake.settled.lock().unwrap().is_empty(),
        "nothing was asked of the engine"
    );

    // An answer of the earlier generation that arrives after the move says nothing.
    source.pump();
    let late = Answer {
        generation: source.generation.wrapping_sub(1),
        whose: Some(pressed_under),
        refused: None,
        done: Some(SETTLED_KEPT_OTHER),
        read: Err(NoRead::NotNow),
    };
    source.inbox_for_test(late);
    assert_eq!(source.pump().done, None, "a late answer is dropped");
    assert!(fake.db().get_operation("op-p").unwrap().is_some());
}

/// A way out that moved since the row was read changes nothing and says so.
#[test]
fn a_change_that_moved_says_so_and_nothing_changes() {
    let (fake, _dir) = refused_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    // Another device's version reached this PC after the read: the row's version is not the file's.
    fake.db()
        .test_execute("UPDATE files SET current_version = 2 WHERE file_id = 'f-p'")
        .unwrap();
    source.settle("op-p", Exit::KeepOther);
    let pumped = settle_all(&mut source);
    assert!(pumped.iter().any(|p| p.refusal == Some(SETTLE_MOVED)));
    assert!(fake.db().get_operation("op-p").unwrap().is_some());
}

/// Try again on a `rekey_failed` park, the one park whose way out it is: made due through its own transaction, and
/// the row leaves the list.
#[test]
fn try_again_on_a_rekey_park_makes_it_due() {
    let fake = Fake::new();
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    std::fs::write(&staged, b"my copy").unwrap();
    parked(&fake.db(), "op-r", "f-r", "rekey_failed", &staged.to_string_lossy());
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    assert_eq!(exits_of(&source, "op-r"), vec![Exit::TryAgain]);
    source.retry("op-r");
    let pumped = settle_all(&mut source);
    assert!(pumped.iter().all(|p| p.refusal.is_none()), "tried, not refused");
    let op = fake.db().get_operation("op-r").unwrap().expect("still queued, now due");
    assert_eq!(op.attempts, 0);
    assert_eq!(op.base_version, Some(1), "the base is as it was");
    assert!(source.problems().is_empty());
}

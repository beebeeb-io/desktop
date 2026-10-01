use super::storage_tests::{owner, owner_body};
use super::*;
use std::sync::{
    Barrier,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

#[test]
fn slice1_r3_recovery_ledger_reopens_twice_without_refill() {
    let h = Harness::new().unwrap();
    let ledger = Ledger::open(&h).unwrap();
    let mut stores = Vec::new();
    for _ in 0..5 {
        let s = Store::new(&h).unwrap();
        let o = owner(&s, "Snapshot");
        s.allocate(o, "Capture", "Full", 1, 0, &mut Fault::default()).unwrap();
        stores.push(s);
    }
    drop(ledger);
    let mut reserve = Reserve::existing(h.path()).unwrap();
    reserve.release_terminal(&h).unwrap();
    let mut consumed = fs::File::create(h.path().join("terminal-consumption")).unwrap();
    consumed.write_all(&vec![0x7a; 4 * MIB as usize]).unwrap();
    consumed.sync_all().unwrap();
    h.volume.lock().unwrap().capacity_ceiling = Some(allocated_tree(h.path()).unwrap() + 12 * MIB);
    for _ in 0..2 {
        let ledger = Ledger::open(&h).expect("recovery open must succeed without refill headroom");
        assert_eq!(
            file_len(&h.path().join("reserve")),
            368 * MIB,
            "recovery open must not grow the reserve using terminal headroom"
        );
        let admission = h.volume.lock().unwrap();
        assert!(
            admission.space_admitted(h.path(), 0, false).is_err(),
            "non-terminal admission must wait for full refill"
        );
        admission.space_admitted(h.path(), 8 * MIB, true).unwrap();
        drop(admission);
        drop(ledger);
    }
    h.volume.lock().unwrap().capacity_ceiling = None;
    reserve.refill(&h).unwrap();
    h.volume.lock().unwrap().space_admitted(h.path(), 0, false).unwrap();
}

#[test]
fn slice1_r3_terminal_only_six_databases_grow_before_admission() {
    let h = Harness::new().unwrap();
    let _ledger = Ledger::open(&h).unwrap();
    let mut stores = Vec::new();
    for _ in 0..5 {
        let s = Store::new(&h).unwrap();
        owner(&s, "RootRetirement");
        assert!(
            file_len(&h.path().join("reserve")) >= required_emergency(h.path(), None).unwrap(),
            "terminal participant must physically grow reserve before owner commit"
        );
        stores.push(s);
    }
    assert_eq!(
        required_emergency(h.path(), None).unwrap(),
        384 * MIB,
        "terminal-only six databases need the 384 MiB reserve"
    );
    assert_eq!(
        file_len(&h.path().join("reserve")),
        384 * MIB,
        "terminal participant must physically grow reserve before owner commit"
    );
    assert!(allocated_len(&h.path().join("reserve")).unwrap() >= 384 * MIB);
    let s = &stores[0];
    let o: Vec<u8> =
        s.db.query_row("SELECT owner_id FROM v2_owners", [], |r| r.get(0))
            .unwrap();
    let disposition = id();
    s.db.execute(
        "INSERT INTO v2_dispositions VALUES(?1,?2,'OwnershipTransfer',1,x'01')",
        params![disposition.as_slice(), &o],
    )
    .unwrap();
    s.db.execute("UPDATE v2_owners SET terminal_disposition=?1", [disposition.as_slice()])
        .unwrap();
    s.db.execute(
        "INSERT INTO v2_transfers VALUES(?1,?2,?3,'Intent',NULL,NULL)",
        params![id().as_slice(), o, id().as_slice()],
    )
    .unwrap();
    assert_eq!(
        required_emergency(h.path(), None).unwrap(),
        384 * MIB,
        "unacknowledged transfer must retain its database terminal budget"
    );
    s.db.execute(
        "UPDATE v2_transfers SET phase='Acknowledged',ledger_record_id=zeroblob(32),ledger_receipt=x'01'",
        [],
    )
    .unwrap();
    assert_eq!(required_emergency(h.path(), None).unwrap(), 320 * MIB);
    let sixth = Store::new(&h).unwrap();
    h.volume.lock().unwrap().capacity_ceiling = Some(allocated_tree(h.path()).unwrap());
    assert!(
        sixth
            .owner(id(), "RootRetirement", &owner_body(&sixth, "RootRetirement"))
            .is_err()
    );
    assert_eq!(
        sixth
            .db
            .query_row("SELECT count(*) FROM v2_owners", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn slice1_r3_two_simultaneous_refills_do_not_overallocate() {
    let h = Harness::new().unwrap();
    let mut r = Reserve::existing(h.path()).unwrap();
    r.release_terminal(&h).unwrap();
    let entry = Barrier::new(2);
    let unlocked_read = Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..2)
            .map(|_| {
                let mut r = Reserve::existing(h.path()).unwrap();
                let entry = &entry;
                let unlocked_read = &unlocked_read;
                let volume = &h.volume;
                scope.spawn(move || {
                    r.refill_with_hook(volume, &mut |stage| {
                        if stage == "before lock" {
                            entry.wait();
                        }
                        if stage == "after observation" && volume.try_lock().is_ok() {
                            unlocked_read.wait();
                        }
                        Ok(())
                    })
                })
            })
            .collect();
        jobs.into_iter().map(|job| job.join().unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(
        file_len(&h.path().join("reserve")),
        256 * MIB,
        "simultaneous refill must not append the same deficit twice"
    );
    assert_eq!(results.len(), 2);
    for result in results {
        result.expect("serialized refill must succeed");
    }
    assert!(
        (256 * MIB..256 * MIB + MIB).contains(&allocated_len(&h.path().join("reserve")).unwrap()),
        "physical reserve overhead must stay below one MiB"
    );
}

#[test]
fn slice1_r3_refill_waits_for_terminal_release_and_consumption() {
    let h = Harness::new().unwrap();
    let active = AtomicBool::new(true);
    let (send, recv) = mpsc::channel();
    let terminal = h.volume.lock().unwrap();
    let mut r = Reserve::existing(h.path()).unwrap();
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            r.refill_with_hook(&h.volume, &mut |stage| {
                if stage == "blocked" {
                    send.send(()).unwrap();
                }
                if stage == "after observation" && active.load(Ordering::SeqCst) {
                    send.send(()).unwrap();
                    bail!("refill entered while terminal worker owns released space");
                }
                Ok(())
            })
        });
        recv.recv().unwrap();
        let mut released = Reserve::existing(h.path()).unwrap();
        released.release_terminal_locked().unwrap();
        let mut output = fs::File::create(h.path().join("terminal-output")).unwrap();
        output.write_all(&vec![0x6b; 8 * MIB as usize]).unwrap();
        output.sync_all().unwrap();
        assert_eq!(file_len(&h.path().join("reserve")), 240 * MIB);
        assert_eq!(file_len(&h.path().join("terminal-output")), 8 * MIB);
        active.store(false, Ordering::SeqCst);
        drop(terminal);
        worker
            .join()
            .unwrap()
            .expect("refill must wait until terminal bytes are consumed");
    });
    assert_eq!(r.remaining, 256 * MIB);
    assert_eq!(file_len(&r.path), 256 * MIB);
}

#[test]
fn slice1_r3_default_campaign_is_bounded_and_acceptance_is_named() {
    let source = include_str!("storage_tests.rs");
    assert!(
        source.contains("#[ignore = \"mandatory acceptance: 20 x 3 GiB plus final 3 GiB validation\"]"),
        "full campaign must be an explicitly named ignored acceptance job"
    );
    assert!(source.contains("fn slice1_acceptance_twenty_three_gib_cycles_and_three_gib_manifest()"));
    assert!(source.contains("fn slice1_g4_bounded_cycles_and_g7_manifest()"));
    assert!(
        source.contains("run_storage_campaign(20, 1024 * MIB, 3 * 1024 * MIB)"),
        "acceptance must retain twenty full-size cycles and final validation"
    );
}

#[test]
fn slice1_r3_refill_write_failure_preserves_cache_and_retries_physical_deficit() {
    let h = Harness::new().unwrap();
    let mut r = Reserve::existing(h.path()).unwrap();
    r.release_terminal(&h).unwrap();
    let cached = r.remaining;
    let mut cuts = 0;
    let result = r.refill_with_hook(&h.volume, &mut |stage| {
        if stage == "after write" {
            cuts += 1;
            bail!("injected append failure");
        }
        Ok(())
    });
    assert!(result.is_err(), "refill append cut must execute");
    assert_eq!(cuts, 1);
    assert_eq!(r.remaining, cached, "failed refill must not publish cached state");
    assert_eq!(file_len(&r.path), cached + CHUNK as u64);
    r.refill(&h).unwrap();
    assert_eq!(r.remaining, 256 * MIB);
    assert_eq!(
        file_len(&r.path),
        256 * MIB,
        "retry must reread the partial physical append"
    );
}

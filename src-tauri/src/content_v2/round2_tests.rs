use super::storage_tests::{captured, discard, envelope, owner};
use super::*;

#[test]
fn slice1_r2_extent_pinned_reader_stops_nonterminal_at_256_mib() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let o = owner(&s, "Migration");
    let a = s
        .allocate(o, "LegacyRecovery", "LegacyPartial", 64 * MIB, 0, &mut Fault::default())
        .unwrap();
    s.chunk(&a, 0, &vec![0x71; CHUNK]).unwrap();
    checkpoint(&s.db, &s.path, true).unwrap();
    let reader = Connection::open(&s.path).unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM v2_artifacts;")
        .unwrap();
    let mut admitted = 0;
    let error = loop {
        let result = s.extent(
            &a,
            admitted,
            Extent {
                offset: admitted * 2,
                packed: admitted,
                len: 1,
            },
        );
        if let Err(error) = result {
            break error;
        }
        admitted += 1;
        assert!(
            file_len(&wal_path(&s.path)) < WAL_LIMIT,
            "extent writes crossed the 320 MiB hard WAL budget without rejection"
        );
    };
    let wal = file_len(&wal_path(&s.path));
    assert!(admitted > 1000, "stress must execute real fragmented extents");
    assert!(
        wal >= 256 * MIB && wal <= 256 * MIB + DIRTY_LIMIT,
        "metadata must reach and stop at 256 MiB: {wal}, {error}"
    );
    assert!(
        error.to_string().contains("non-terminal metadata backpressure"),
        "{error}"
    );
    let rows: u64 =
        s.db.query_row("SELECT count(*) FROM v2_extents", [], |r| r.get(0))
            .unwrap();
    assert_eq!(rows, admitted);
    let consumed: u64 =
        s.db.query_row("SELECT consumed_bytes FROM v2_reservations", [], |r| r.get(0))
            .unwrap();
    assert!(consumed > CHUNK as u64, "extent metadata must charge its reservation");
    metadata_admitted(&s.db, &s.path, true).unwrap();
    assert!(
        s.extent(
            &a,
            admitted,
            Extent {
                offset: admitted * 2,
                packed: admitted,
                len: 1
            }
        )
        .is_err()
    );
    assert_eq!(file_len(&wal_path(&s.path)), wal, "rejection cannot append WAL");
    reader.execute_batch("ROLLBACK").unwrap();
    checkpoint(&s.db, &s.path, true).unwrap();
    s.extent(
        &a,
        admitted,
        Extent {
            offset: admitted * 2,
            packed: admitted,
            len: 1,
        },
    )
    .unwrap();
    println!(
        "fragmented_extents={admitted} pinned_wal={wal} consumed={consumed} settled_wal={}",
        file_len(&wal_path(&s.path))
    );
}

#[test]
fn slice1_r2_extent_amplification_is_reserved_before_commit() {
    let h = Harness::new().unwrap();
    let s = Store::new(&h).unwrap();
    let o = owner(&s, "Migration");
    let a = s
        .allocate(o, "LegacyRecovery", "LegacyPartial", 4096, 0, &mut Fault::default())
        .unwrap();
    s.extent(
        &a,
        0,
        Extent {
            offset: 0,
            packed: 0,
            len: 1,
        },
    )
    .unwrap();
    let consumed: u64 =
        s.db.query_row("SELECT consumed_bytes FROM v2_reservations", [], |r| r.get(0))
            .unwrap();
    assert!(consumed > 0, "extent metadata was not charged");
    s.db.execute("UPDATE v2_reservations SET payload_bytes=consumed_bytes", [])
        .unwrap();
    assert!(
        s.extent(
            &a,
            1,
            Extent {
                offset: 2,
                packed: 1,
                len: 1
            }
        )
        .is_err(),
        "extent amplification exceeded its reservation"
    );
    assert_eq!(
        s.db.query_row("SELECT count(*) FROM v2_extents", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1,
        "failed admission must roll back extent"
    );
}

fn key_publication_cut(cut: usize) {
    let h = Harness::new().unwrap();
    let l = Ledger::open(&h).unwrap();
    let record = id();
    let token = id();
    let body = envelope(id());
    let mut fault = Fault {
        cut: Some(cut),
        seen: 0,
    };
    assert!(
        l.put(record, "Envelope", &body, Some(30), Some(token), &mut fault)
            .is_err()
    );
    assert_eq!(fault.seen, cut, "key I/O injection must execute");
    assert_eq!(
        l.db.query_row("SELECT count(*) FROM protected_records", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert!(
        !h.path().join("keyslots").join(hex(&record)).exists(),
        "incomplete key was published at cut={cut}"
    );
    drop(l);
    for retry in 1..=2 {
        let l = Ledger::open(&h).unwrap();
        l.put(record, "Envelope", &body, Some(30), Some(token), &mut Fault::default())
            .unwrap_or_else(|e| panic!("key publication cut={cut} retry={retry} failed: {e}"));
        assert_eq!(l.read(record).unwrap(), body);
        assert_eq!(l.keys_count().unwrap(), 1, "orphan temporary keys must be reconciled");
    }
}
#[test]
fn slice1_r2_key_create_retry_twice() {
    key_publication_cut(2);
}
#[test]
fn slice1_r2_key_write_retry_twice() {
    key_publication_cut(3);
}
#[test]
fn slice1_r2_key_flush_retry_twice() {
    key_publication_cut(4);
}

#[test]
fn slice1_r2_unreferenced_poisoned_key_repaired_referenced_key_preserved() {
    let h = Harness::new().unwrap();
    let l = Ledger::open(&h).unwrap();
    let record = id();
    let path = h.path().join("keyslots").join(hex(&record));
    fs::write(&path, b"truncated orphan").unwrap();
    let body = envelope(id());
    l.put(record, "Envelope", &body, Some(30), Some(id()), &mut Fault::default())
        .expect("unreferenced orphan must permit retry");
    fs::write(&path, b"damaged referenced key").unwrap();
    drop(l);
    let l = Ledger::open(&h).unwrap();
    assert!(
        l.put(record, "Envelope", &body, Some(30), Some(id()), &mut Fault::default())
            .is_err()
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        b"damaged referenced key",
        "committed ciphertext forbids orphan repair"
    );
}

#[test]
fn slice1_r2_capacity_rejects_missing_and_sparse_reserve() {
    let h = Harness::new().unwrap();
    let s = Store::new(&h).unwrap();
    let o = owner(&s, "Snapshot");
    let path = h.path().join("reserve");
    if path.exists() {
        fs::remove_file(&path).unwrap();
    }
    assert!(
        s.allocate(o, "Capture", "Full", 1, 0, &mut Fault::default()).is_err(),
        "missing physical emergency reserve admitted work"
    );
    let f = fs::File::create(&path).unwrap();
    f.set_len(256 * MIB).unwrap();
    f.sync_all().unwrap();
    // On Windows set_len can allocate non-sparse space; the missing-reserve control
    // is portable and Unix explicitly constructs a sparse negative fixture.
    #[cfg(unix)]
    assert!(
        s.allocate(o, "Capture", "Full", 1, 0, &mut Fault::default()).is_err(),
        "sparse reserve admitted work"
    );
}

#[test]
fn slice1_r2_capacity_uses_measured_available_filesystem_bytes() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let _reserve = if h.path().join("reserve").exists() {
        None
    } else {
        Some(Reserve::create(&h).unwrap())
    };
    let o = owner(&s, "Snapshot");
    let available = super::storage_tests::available_bytes(h.path());
    s.quota = available * 2 + 1024 * MIB;
    assert!(
        s.allocate(o, "Capture", "Full", available, 0, &mut Fault::default())
            .is_err(),
        "allocation exceeded measured available filesystem space"
    );
}

#[test]
fn slice1_r2_cipher_zeroization_features_resolved() {
    let manifest: toml::Value = toml::from_str(include_str!("../../Cargo.toml")).unwrap();
    for dependency in ["aes", "aes-gcm", "ghash", "polyval"] {
        let features = manifest["dependencies"]
            .get(dependency)
            .and_then(|v| v.get("features"))
            .and_then(|v| v.as_array());
        assert!(
            features.is_some_and(|v| v.iter().any(|f| f.as_str() == Some("zeroize"))),
            "{dependency} zeroize feature missing"
        );
    }
    let lock: toml::Value = toml::from_str(include_str!("../../Cargo.lock")).unwrap();
    for dependency in ["aes", "aes-gcm", "ghash", "polyval"] {
        let package = lock["package"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"].as_str() == Some(dependency))
            .unwrap();
        assert!(
            package["dependencies"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p.as_str() == Some("zeroize")),
            "{dependency} lock lacks zeroize dependency"
        );
    }
}

#[test]
fn slice1_r2_restore_test_has_no_constant_predicate_or_unused_variant() {
    let source = include_str!("storage_tests.rs");
    assert!(
        !source.contains("for _variant in"),
        "restore scenarios must construct distinct persisted states"
    );
    let source = include_str!("mod.rs");
    assert!(
        !source.contains("fn recovered_can_submit"),
        "constant recovery predicate is not a submission boundary"
    );
}

#[test]
fn slice1_r2_capacity_accounts_for_installation_ledger() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let o = owner(&s, "Snapshot");
    let before = s.budget_from_disk().unwrap();
    let l = Ledger::open(&h).unwrap();
    l.put(
        id(),
        "Envelope",
        &envelope(id()),
        Some(30),
        Some(id()),
        &mut Fault::default(),
    )
    .unwrap();
    let after = s.budget_from_disk().unwrap();
    assert!(
        after.0 + after.1 >= before.0 + before.1 + WAL_LIMIT + 256 * MIB,
        "InstallationLedger WAL and terminal headroom missing from shared budget"
    );
    s.quota = after.0 + after.1 + 1024;
    assert!(
        s.allocate(o, "Capture", "Full", 1, 0, &mut Fault::default()).is_err(),
        "ledger consumed shared quota but allocation was admitted"
    );
}

#[test]
fn slice1_r2_terminal_completion_requires_actual_reserve_release() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let a = captured(&mut s, b"terminal capacity model");
    s.settle().unwrap();
    let new_owner = owner(&s, "Snapshot");
    let before = allocated_tree(h.path()).unwrap();
    h.volume.lock().unwrap().capacity_ceiling = Some(before + 4096);
    // This ceiling measures real file allocation while leaving the host volume alone.
    // dispose() must release real reserve storage before its FULL terminal commit.
    let result = s.dispose(&a, &discard(&a), &mut Fault::default());
    assert!(
        result.is_ok(),
        "terminal disposition must complete using physically released reserve: {result:?}"
    );
    let after = allocated_tree(h.path()).unwrap();
    assert!(
        after + 8 * MIB < before,
        "terminal completion consumed no physical reserve"
    );
    assert_eq!(
        s.db.query_row("SELECT count(*) FROM v2_dispositions", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1
    );
    h.volume.lock().unwrap().capacity_ceiling = None;
    s.gc(&a, &mut Fault::default()).unwrap();
    assert!(
        s.allocate(new_owner, "Capture", "Full", 1, 0, &mut Fault::default())
            .is_err(),
        "new payload must wait for reserve refill"
    );
    let mut reserve = Reserve::existing(h.path()).unwrap();
    reserve.refill().unwrap();
    s.allocate(new_owner, "Capture", "Full", 1, 0, &mut Fault::default())
        .unwrap();
}

#[test]
fn slice1_r2_key_publication_never_clobbers_existing_slot() {
    let h = Harness::new().unwrap();
    let temporary = h.path().join("wrapped-key.tmp");
    let destination = h.path().join("wrapped-key");
    fs::write(&temporary, b"new wrapped key").unwrap();
    fs::write(&destination, b"existing wrapped key").unwrap();
    assert!(
        ledger::publish_key(&temporary, &destination).is_err(),
        "no-clobber publication replaced an existing key"
    );
    assert_eq!(fs::read(&destination).unwrap(), b"existing wrapped key");
}

#[test]
fn slice1_r2_extent_writes_schedule_real_checkpoint_progress() {
    let h = Harness::new().unwrap();
    let s = Store::new(&h).unwrap();
    let o = owner(&s, "Migration");
    let a = s
        .allocate(o, "LegacyRecovery", "LegacyPartial", 64 * MIB, 0, &mut Fault::default())
        .unwrap();
    checkpoint(&s.db, &s.path, true).unwrap();
    let before = allocated_len(&s.path).unwrap();
    let mut peak = 0;
    for ordinal in 0..10_000 {
        s.extent(
            &a,
            ordinal,
            Extent {
                offset: ordinal * 2,
                packed: ordinal,
                len: 1,
            },
        )
        .unwrap();
        peak = peak.max(file_len(&wal_path(&s.path)));
        assert!(
            peak <= 64 * MIB + DIRTY_LIMIT,
            "extent scheduler did not checkpoint/reuse WAL at 64 MiB: {peak}"
        );
    }
    assert!(
        allocated_len(&s.path).unwrap() > before,
        "checkpoint did not copy any actual extent pages to the DB"
    );
    assert_eq!(
        s.db.query_row("SELECT count(*) FROM v2_extents", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        10_000
    );
    println!(
        "extent_checkpoint_rows=10000 peak_wal={peak} db_allocation_growth={}",
        allocated_len(&s.path).unwrap() - before
    );
}

#[test]
fn slice1_r2_reserve_grows_for_all_participating_databases() {
    let h = Harness::new().unwrap();
    let _ledger = Ledger::open(&h).unwrap();
    let mut stores = Vec::new();
    for _ in 0..5 {
        let s = Store::new(&h).unwrap();
        let o = owner(&s, "Snapshot");
        let artifact = s.allocate(o, "Capture", "Full", 1, 0, &mut Fault::default()).unwrap();
        stores.push((s, artifact));
    }
    let reserve = h.path().join("reserve");
    assert_eq!(
        file_len(&reserve),
        6 * 64 * MIB,
        "six participating databases need six terminal budgets"
    );
    assert!(allocated_len(&reserve).unwrap() >= 6 * 64 * MIB);
    let mut physical = Reserve::existing(h.path()).unwrap();
    physical.release_terminal().unwrap();
    assert_eq!(file_len(&reserve), 6 * 64 * MIB - 16 * MIB);
    let first_path = stores[0].0.path.clone();
    let first_artifact = stores[0].1.clone();
    assert!(
        stores[0].0.chunk(&first_artifact, 0, b"x").is_err(),
        "chunk spent released terminal reserve"
    );
    for _restart in 0..2 {
        h.volume.lock().unwrap().emergency_required = 0;
        let mut restored = Store::reopen(&h, &first_path).unwrap();
        assert!(
            restored.chunk(&first_artifact, 0, b"x").is_err(),
            "reopen lost the participating reserve requirement"
        );
    }
    physical.refill().unwrap();
    assert_eq!(
        file_len(&reserve),
        6 * 64 * MIB,
        "refill must restore every participating database's terminal budget"
    );
    assert!(allocated_len(&reserve).unwrap() >= 6 * 64 * MIB);
    stores[0].0.chunk(&first_artifact, 0, b"x").unwrap();
}

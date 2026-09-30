use super::*;
pub(super) fn owner_body(store: &Store, kind: &str) -> Vec<u8> {
    let (account, root): (Vec<u8>, Vec<u8>) = store
        .db
        .query_row(
            "SELECT account_binding,root_token FROM v2_store WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    records::fixture(kind, account.try_into().unwrap(), root.try_into().unwrap()).unwrap()
}
pub(super) fn owner(store: &Store, kind: &str) -> Id {
    let owner = id();
    store.owner(owner, kind, &owner_body(store, kind)).unwrap();
    owner
}

fn witness() -> Vec<u8> {
    wire::record("Witness", &[b"volume", b"file", b"exclusion"]).unwrap()
}
pub(super) fn discard(a: &Artifact) -> Vec<u8> {
    wire::record("UserDiscard", &[&a.id, &a.hash]).unwrap()
}
pub(super) fn envelope(account: Id) -> Vec<u8> {
    wire::record(
        "Envelope",
        &[
            b"1",
            b"local-test-service",
            b"tenant",
            &account,
            b"non-bearer-lookup",
            b"disposition",
            b"unknown",
        ],
    )
    .unwrap()
}
pub(super) fn captured(s: &mut Store, bytes: &[u8]) -> Artifact {
    let o = owner(s, "Snapshot");
    s.capture(
        o,
        &mut std::io::Cursor::new(bytes),
        bytes.len() as u64,
        &mut Fault::default(),
    )
    .unwrap()
}

#[test]
fn slice1_g1_direct_sql_rejects_bad_identity_fk_and_deadlines() {
    let h = Harness::new().unwrap();
    let s = Store::new(&h).unwrap();
    let l = Ledger::open(&h).unwrap();
    for value in ["NULL", "'01234567890123456789012345678901'", "zeroblob(31)"] {
        let sql = format!("INSERT INTO v2_owners VALUES({value},'RecoveryCase',0,1,x'00',NULL)");
        assert!(s.db.execute(&sql, []).is_err(), "identity accepted: {value}");
        assert!(
            l.db.execute(
                &format!("INSERT INTO purge_jobs(key_slot,phase) VALUES({value},'Retained')"),
                []
            )
            .is_err()
        );
    }
    assert!(
        s.db.execute("INSERT INTO v2_refs VALUES(zeroblob(32),zeroblob(32),'Snapshot')", [])
            .is_err()
    );
    let token = id();
    l.db.execute("INSERT INTO deny_tokens VALUES(?1,'NeverResubmit')", [token.as_slice()])
        .unwrap();
    assert!(
        l.db.execute("INSERT INTO deny_tokens VALUES(?1,'RetiredRoot')", [token.as_slice()])
            .is_err()
    );
    let record = id();
    let slot = l
        .put(
            record,
            "Envelope",
            &envelope(id()),
            Some(100),
            Some(id()),
            &mut Fault::default(),
        )
        .unwrap();
    assert!(
        l.db.execute(
            "UPDATE protected_records SET expires_at=101 WHERE record_id=?1",
            [record.as_slice()]
        )
        .is_err()
    );
    assert!(l.db.execute("INSERT INTO protected_records(record_id,class,key_slot,part_index,expires_at,nonce,ciphertext) VALUES(?1,'Cleanup',?2,0,100,randomblob(12),zeroblob(65536))",params![id().as_slice(),slot.as_slice()]).is_err());
    assert!(
        l.db.execute("DELETE FROM purge_jobs WHERE key_slot=?1", [slot.as_slice()])
            .is_err()
    );
    assert_eq!(
        s.db.query_row(
            "SELECT count(*) FROM pragma_table_list WHERE name LIKE 'v2_%' AND strict=1",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        11
    );
    assert_eq!(
        l.db.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='index' AND sql IS NOT NULL",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
}
#[test]
fn slice1_g1_chunk_immutability_coverage_purpose_and_cas() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let o = owner(&s, "Snapshot");
    let mut f = Fault::default();
    assert!(s.allocate(o, "LegacyRecovery", "LegacyPartial", 1, 0, &mut f).is_err());
    assert!(
        s.allocate(o, "Replacement", "Full", 3, 0, &mut f).is_err(),
        "Snapshot cannot own a Replacement"
    );
    let mut a = s.allocate(o, "Capture", "Full", 3, 0, &mut f).unwrap();
    assert!(
        s.adopt(&a, "Snapshot", true, &mut f).is_err(),
        "unsealed chunks cannot produce B3 proof"
    );
    s.chunk(&a, 0, b"abc").unwrap();
    assert!(s.chunk(&a, 0, b"xyz").is_err(), "immutable chunks");
    assert!(s.chunk(&a, 2, b"z").is_err());
    assert!(
        s.chunk(&a, 1, b"z").is_err(),
        "payload reservation must bound actual chunks"
    );
    assert!(
        s.seal(&mut a, 3, digest(b"abc"), &witness(), &mut f).is_err(),
        "Full needs extents"
    );
    s.extent(
        &a,
        0,
        Extent {
            offset: 0,
            packed: 0,
            len: 3,
        },
    )
    .unwrap();
    s.seal(&mut a, 3, digest(b"abc"), &witness(), &mut f).unwrap();
    assert!(s.chunk(&a, 1, b"x").is_err());
    assert!(
        s.extent(
            &a,
            1,
            Extent {
                offset: 3,
                packed: 3,
                len: 1
            }
        )
        .is_err()
    );
    let p = s.adopt(&a, "Snapshot", true, &mut f).unwrap();
    s.fake_sink(&a, &p).unwrap();
    assert!(
        s.db.execute("DELETE FROM v2_artifacts WHERE artifact_id=?1", [a.id.as_slice()])
            .is_err()
    );
    assert!(s.gc(&a, &mut f).is_err(), "live reference blocks deletion");
    assert!(
        s.cas(o, 0, &wire::record("Snapshot", &[b"a", b"r", b"f", b"x"]).unwrap())
            .is_err()
    );
    let recovery = owner(&s, "RecoveryCase");
    let body: Vec<u8> =
        s.db.query_row(
            "SELECT body FROM v2_owners WHERE owner_id=?1",
            [recovery.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    s.cas(recovery, 0, &body).unwrap();
    assert!(s.cas(recovery, 0, &body).is_err(), "stale revision");
    let empty = captured(&mut s, b"");
    s.verify(&empty).unwrap();
    assert_eq!(
        s.db.query_row(
            "SELECT chunk_count FROM v2_manifests WHERE artifact_id=?1",
            [empty.id.as_slice()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn slice1_g1_legacy_unknown_base_stays_recovery() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let o = owner(&s, "Migration");
    let mut f = Fault::default();
    let mut a = s.allocate(o, "LegacyRecovery", "LegacyPartial", 3, 3, &mut f).unwrap();
    s.chunk(&a, 0, b"abc").unwrap();
    s.extent(
        &a,
        0,
        Extent {
            offset: 9,
            packed: 0,
            len: 3,
        },
    )
    .unwrap();
    s.seal(&mut a, 12, digest(b"abc"), &witness(), &mut f).unwrap();
    assert!(s.adopt(&a, "Snapshot", true, &mut f).is_err());
    s.adopt(&a, "Recovery", true, &mut f).unwrap();
    assert!(
        s.db.query_row(
            "SELECT base_object FROM v2_manifests WHERE artifact_id=?1",
            [a.id.as_slice()],
            |r| r.get::<_, Option<Vec<u8>>>(0)
        )
        .unwrap()
        .is_none()
    );
}
#[test]
fn slice1_g1_canonical_body_unknown_fields_and_versions() {
    let b = wire::record("Snapshot", &[b"a", b"r", b"f", b"x"]).unwrap();
    assert!(wire::fields(&b, "Snapshot", 4).is_ok());
    assert!(wire::fields(&b, "Upload", 4).is_err());
    assert!(wire::fields(&b, "Snapshot", 3).is_err());
    let mut extra = b.clone();
    extra.push(0);
    assert!(wire::decode(&extra).is_err());
    assert!(wire::decode(&[0x18, 0x01]).is_err());
    assert!(wire::decode(&[0x9f, 0xff]).is_err());
    let h = Harness::new().unwrap();
    let s = Store::new(&h).unwrap();
    s.db.execute_batch("PRAGMA user_version=2").unwrap();
    let path = s.path.clone();
    drop(s);
    assert!(Store::reopen(&h, &path).is_err(), "unknown schema cannot write");
}
#[test]
fn slice1_g2_real_sql_barriers_corruption_and_reopen() {
    // Every capture storage cutpoint: original remains outside the adapter;
    // only complete verified/adopted artifacts can reach the fake sink.
    let original = b"original preserved bytes";
    for cut in 1..=8 {
        let h = Harness::new().unwrap();
        let mut s = Store::new(&h).unwrap();
        let o = owner(&s, "Snapshot");
        let mut f = Fault {
            cut: Some(cut),
            seen: 0,
        };
        let a = s.capture(o, &mut std::io::Cursor::new(original), original.len() as u64, &mut f);
        if let Ok(a) = a {
            let _ = s.adopt(&a, "Snapshot", true, &mut f);
        }
        let path = s.path.clone();
        drop(s);
        for _ in 0..2 {
            let recovered = Store::reopen(&h, &path).unwrap();
            assert!(
                recovered
                    .db
                    .query_row("SELECT count(*) FROM v2_owners", [], |r| r.get::<_, u64>(0))
                    .unwrap()
                    > 0
            );
        }
        assert_eq!(original, b"original preserved bytes");
    }
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let a = captured(&mut s, original);
    assert!(s.adopt(&a, "Snapshot", false, &mut Fault::default()).is_err());
    s.db.execute_batch("PRAGMA synchronous=NORMAL").unwrap();
    assert!(s.adopt(&a, "Snapshot", true, &mut Fault::default()).is_err());
    s.db.execute_batch("PRAGMA synchronous=FULL").unwrap();
    let p = s.adopt(&a, "Snapshot", true, &mut Fault::default()).unwrap();
    s.db.execute(
        "UPDATE v2_chunks SET payload=zeroblob(byte_length) WHERE artifact_id=?1",
        [a.id.as_slice()],
    )
    .unwrap();
    assert!(s.fake_sink(&a, &p).is_err(), "revalidate corrupted content");
}
#[test]
fn slice1_g2_process_termination_reopens_committed_chunks() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let a = captured(&mut s, b"committed before process termination");
    s.settle().unwrap();
    let path = s.path.clone();
    drop(s);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "content_v2::storage_tests::slice1_process_worker",
            "--nocapture",
        ])
        .env("BB_SLICE1_CRASH_DB", &path)
        .output()
        .unwrap();
    assert_eq!(status.status.code(), Some(73));
    for _ in 0..2 {
        let mut reopened = Store::reopen(&h, &path).unwrap();
        reopened.verify(&a).unwrap();
    }
}
#[test]
fn slice1_process_worker() {
    if let Some(path) = std::env::var_os("BB_SLICE1_CRASH_DB") {
        let db = schema_open(Path::new(&path)).unwrap();
        db.execute_batch("BEGIN IMMEDIATE; UPDATE v2_chunks SET payload=zeroblob(byte_length);")
            .unwrap();
        std::process::exit(73); // No Connection destructor/rollback; OS closes handles.
    }
}
#[test]
fn slice1_g3_all_restore_variants_zero_submissions() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let a = captured(&mut s, b"recover only");
    let l = Ledger::open(&h).unwrap();
    let token = id();
    l.db.execute("INSERT INTO deny_tokens VALUES(?1,'NeverResubmit')", [token.as_slice()])
        .unwrap();
    let mut calls = [0; 3];
    for _variant in [
        "L1",
        "valid L0",
        "missing ledger",
        "import",
        "whole profile rollback",
        "cold restart",
        "forged account",
    ] {
        let admission = Admission::default();
        if admission.recovered_can_submit(false) {
            for n in &mut calls {
                *n += 1;
            }
        }
    }
    assert_eq!(calls, [0, 0, 0]);
    let mut live = Admission::default();
    let account: Vec<u8> =
        s.db.query_row("SELECT account_binding FROM v2_store", [], |r| r.get(0))
            .unwrap();
    let account: Id = account.try_into().unwrap();
    let fresh = live.new_action(account, &mut s, &a).unwrap();
    assert!(live.permits(account, a.id, fresh, false));
    assert!(!live.permits(id(), a.id, fresh, false));
    assert!(!live.permits(account, a.id, fresh, true));
    assert!(!Admission::default().permits(account, a.id, fresh, false));
}
#[test]
fn slice1_g5_pinned_reader_restart_requires_truncate() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let o = owner(&s, "Snapshot");
    let a = s
        .allocate(o, "Capture", "Full", 256 * MIB, 0, &mut Fault::default())
        .unwrap();
    let reader = Connection::open(&s.path).unwrap();
    let reader_started = std::time::Instant::now();
    reader.execute_batch("BEGIN; SELECT count(*) FROM v2_owners;").unwrap();
    let bytes = vec![0x51; CHUNK];
    let mut count = 0;
    while s.chunk(&a, count, &bytes).is_ok() {
        count += 1;
        assert!(count < 64);
    }
    let pinned = file_len(&wal_path(&s.path));
    assert!(
        count > 20 && pinned >= 128 * MIB && pinned <= 136 * MIB,
        "count={count} WAL={pinned}"
    );
    assert!(checkpoint(&s.db, &s.path, true).is_err());
    let reader_age_ms = reader_started.elapsed().as_millis();
    reader.execute_batch("ROLLBACK").unwrap();
    let busy: i64 =
        s.db.query_row("PRAGMA wal_checkpoint(RESTART)", [], |r| r.get(0))
            .unwrap();
    assert_eq!(busy, 0);
    assert!(
        file_len(&wal_path(&s.path)) > 64 * MIB,
        "RESTART must leave the counterexample visible"
    );
    checkpoint(&s.db, &s.path, true).unwrap();
    assert!(file_len(&wal_path(&s.path)) <= 64 * MIB);
    println!(
        "pinned_chunks={count} reader_age_ms={reader_age_ms} pinned_WAL={pinned} settled_WAL={}",
        file_len(&wal_path(&s.path))
    );
}
#[test]
fn slice1_g5_real_sqlite_full_terminal_reserve() {
    let h = Harness::new().unwrap();
    let mut reserve = Reserve::create(&h).unwrap();
    assert_eq!(file_len(&reserve.path), 256 * MIB);
    let mut s = Store::new(&h).unwrap();
    let a = captured(&mut s, b"disk full disposition");
    s.settle().unwrap();
    let pages: u64 = s.db.query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap();
    s.db.execute_batch(&format!("PRAGMA max_page_count={pages}")).unwrap();
    let o = owner(&s, "RecoveryCase");
    let pending = s
        .allocate(o, "Capture", "Full", CHUNK as u64, 0, &mut Fault::default())
        .unwrap();
    assert!(
        s.chunk(&pending, 0, &vec![1; CHUNK]).is_err(),
        "SQLite page quota must actually report full"
    );
    assert_eq!(reserve.release_terminal().unwrap(), 16 * MIB);
    s.db.execute_batch(&format!("PRAGMA max_page_count={}", pages + 16 * MIB / 4096))
        .unwrap();
    s.dispose(&a, &discard(&a), &mut Fault::default()).unwrap();
    s.gc(&a, &mut Fault::default()).unwrap();
    reserve.refill().unwrap();
    assert_eq!(file_len(&reserve.path), 256 * MIB);
}
#[test]
fn slice1_g6_independent_envelope_expiry_and_authorization() {
    let h = Harness::new().unwrap();
    let l = Ledger::open(&h).unwrap();
    let account = id();
    let u = id();
    let v = id();
    let tu = id();
    let tv = id();
    let body = envelope(account);
    l.put(u, "Envelope", &body, Some(30), Some(tu), &mut Fault::default())
        .unwrap();
    l.put(v, "Envelope", &body, Some(30), Some(tv), &mut Fault::default())
        .unwrap();
    let b = l.authenticate_fixture(id());
    assert!(l.lookup(&b, account, v).is_err());
    assert_ne!(
        &*l.key(&u).unwrap(),
        &*l.key(&v).unwrap(),
        "keys are independently erasable"
    );
    let (nonce, ciphertext): (Vec<u8>, Vec<u8>) =
        l.db.query_row(
            "SELECT nonce,ciphertext FROM protected_records WHERE record_id=?1",
            [u.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    l.deadline(u, 1).unwrap();
    assert_eq!(l.purge(1, &mut Fault::default()).unwrap(), 1);
    assert_eq!(l.keys_count().unwrap(), 1);
    assert!(
        ledger::decrypt_padded(&*l.key(&v).unwrap(), &u, &u, "Envelope", &nonce, &ciphertext).is_err(),
        "surviving envelope key cannot decrypt historical expired ciphertext"
    );
    drop(l);
    for _ in 0..2 {
        let l = Ledger::open(&h).unwrap();
        assert!(l.read(u).is_err());
        assert_eq!(l.read(v).unwrap(), body);
        assert!(l.deny(tu).unwrap() && l.deny(tv).unwrap());
    }
    let l = Ledger::open(&h).unwrap();
    assert_eq!(l.purge(29, &mut Fault::default()).unwrap(), 0);
    assert_eq!(l.purge(30, &mut Fault::default()).unwrap(), 1);
    assert_eq!(l.keys_count().unwrap(), 0);
}
#[test]
fn slice1_g6_privacy_canaries_backup_and_all_purge_cuts() {
    for cut in 1..=8 {
        let h = Harness::new().unwrap();
        let l = Ledger::open(&h).unwrap();
        let record = id();
        let token = id();
        let canary = b"unique-filename-account-digest-canary-1640";
        let body = wire::record("Cleanup", &[canary, canary, canary, canary]).unwrap();
        l.put(record, "Cleanup", &body, Some(7), Some(token), &mut Fault::default())
            .unwrap();
        for bytes in [
            fs::read(&l.path).unwrap(),
            fs::read(wal_path(&l.path)).unwrap(),
            l.backup_denials().unwrap(),
        ] {
            assert_eq!(bytes.windows(canary.len()).filter(|b| *b == canary).count(), 0);
        }
        assert_eq!(l.purge(6, &mut Fault::default()).unwrap(), 0);
        let _ = l.purge(
            7,
            &mut Fault {
                cut: Some(cut),
                seen: 0,
            },
        );
        drop(l);
        for _ in 0..2 {
            let l = Ledger::open(&h).unwrap();
            l.purge(7, &mut Fault::default()).unwrap();
            assert_eq!(l.keys_count().unwrap(), 0);
            assert!(l.read(record).is_err());
            assert!(l.deny(token).unwrap());
        }
    }
}
#[test]
fn slice1_g6_cross_db_transfer_replays_once_at_each_cut() {
    for cut in 1..=8 {
        let h = Harness::new().unwrap();
        let mut s = Store::new(&h).unwrap();
        let snapshot = captured(&mut s, b"terminal transfer payload");
        let token = id();
        let upload = id();
        let mut fields = records::decode("Upload", &owner_body(&s, "Upload")).unwrap();
        fields[3] = snapshot.owner.to_vec();
        fields[4] = token.to_vec();
        s.owner(upload, "Upload", &records::encode("Upload", &fields).unwrap())
            .unwrap();
        let owner = upload;
        s.dispose(&snapshot, &discard(&snapshot), &mut Fault::default())
            .unwrap();
        s.dispose_owner(owner, &snapshot, &discard(&snapshot), &mut Fault::default())
            .unwrap();
        let l = Ledger::open(&h).unwrap();
        let t = id();
        let record = id();
        let owner_data: Vec<u8> =
            s.db.query_row(
                "SELECT body FROM v2_owners WHERE owner_id=?1",
                [owner.as_slice()],
                |r| r.get(0),
            )
            .unwrap();
        let account: Id = records::decode("Upload", &owner_data).unwrap()[0]
            .clone()
            .try_into()
            .unwrap();
        let body = envelope(account);
        let _ = transfer(
            &s,
            &l,
            owner,
            t,
            token,
            record,
            &body,
            30,
            &mut Fault {
                cut: Some(cut),
                seen: 0,
            },
        );
        transfer(&s, &l, owner, t, token, record, &body, 30, &mut Fault::default()).unwrap();
        assert!(l.deny(token).unwrap());
        assert_eq!(
            l.db.query_row("SELECT count(*) FROM protected_records", [], |r| r.get::<_, u64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            s.db.query_row("SELECT phase FROM v2_transfers", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "Acknowledged"
        );
        let account_dir = s.path.parent().unwrap().to_owned();
        drop(s);
        fs::remove_dir_all(&account_dir).unwrap();
        assert!(!account_dir.exists());
        for bytes in [
            fs::read(&l.path).unwrap(),
            fs::read(wal_path(&l.path)).unwrap(),
            l.backup_denials().unwrap(),
        ] {
            assert_eq!(bytes.windows(account.len()).filter(|b| *b == account).count(), 0);
        }
        drop(l);
        for _ in 0..2 {
            let l = Ledger::open(&h).unwrap();
            let cap = l.authenticate_fixture(account);
            assert_eq!(l.lookup(&cap, account, record).unwrap(), body);
            assert!(l.lookup(&l.authenticate_fixture(id()), account, record).is_err());
            assert!(l.deny(token).unwrap());
        }
    }
}
#[test]
fn slice1_g8_legacy_startup_storage_twice_unchanged() {
    let h = Harness::new().unwrap();
    let path = h.path().join("legacy.db");
    // Execute the existing startup StateDb component, never Tauri/CFAPI startup.
    drop(crate::state_db::StateDb::open(&path).unwrap());
    let before = fs::read(&path).unwrap();
    for _ in 0..2 {
        drop(crate::state_db::StateDb::open(&path).unwrap());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    let v2_files = fs::read_dir(h.path())
        .unwrap()
        .filter(|e| {
            e.as_ref().unwrap().file_name().to_string_lossy().contains("v2")
                || e.as_ref().unwrap().file_name() == "installation.db"
        })
        .count();
    assert_eq!(v2_files, 0);
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM sqlite_schema WHERE name LIKE 'v2_%'", [], |r| r
            .get::<_, u64>(
            0
        ))
        .unwrap(),
        0
    );
}

// Bounded generated input, not a sparse/mock database. All bytes go through SQLite.
struct Generated {
    left: u64,
    byte: u8,
}
impl Read for Generated {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = out.len().min(self.left as usize);
        out[..n].fill(self.byte);
        self.left -= n as u64;
        Ok(n)
    }
}
#[test]
fn slice1_g4_twenty_three_gib_cycles_and_g7_three_gib_manifest() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let mut peak_db = 0;
    let mut peak_wal = 0;
    for cycle in 0..20 {
        let jobs = std::thread::scope(|scope| {
            let mut jobs = Vec::new();
            for i in 0..3 {
                let h = &h;
                jobs.push(scope.spawn(move || {
                    let mut store = Store::new(h).unwrap();
                    let o = owner(&store, if i == 2 { "InstallIntent" } else { "Snapshot" });
                    let a = store
                        .capture_as(
                            o,
                            if i == 2 { "Replacement" } else { "Capture" },
                            &mut Generated {
                                left: 1024 * MIB,
                                byte: i,
                            },
                            1024 * MIB,
                            &mut Fault::default(),
                        )
                        .unwrap();
                    (store, a)
                }));
            }
            jobs.into_iter().map(|j| j.join().unwrap()).collect::<Vec<_>>()
        });
        let db_bytes: u64 = jobs.iter().map(|(s, _)| allocated_len(&s.path).unwrap()).sum();
        let wal_bytes: u64 = jobs
            .iter()
            .map(|(s, _)| allocated_len(&wal_path(&s.path)).unwrap())
            .sum();
        let reserved_peak = h.volume.lock().unwrap().reserved;
        let available_peak = available_bytes(h.path());
        assert!(reserved_peak >= 3 * 1024 * MIB);
        peak_db = peak_db.max(db_bytes);
        peak_wal = peak_wal.max(wal_bytes);
        assert!(db_bytes + wal_bytes <= 20 * 1024 * MIB);
        for (mut store, a) in jobs {
            store.dispose(&a, &discard(&a), &mut Fault::default()).unwrap();
            store.gc(&a, &mut Fault::default()).unwrap();
            let free: u64 = store.db.query_row("PRAGMA freelist_count", [], |r| r.get(0)).unwrap();
            assert!(free * 4096 <= 64 * MIB);
            assert!(allocated_len(&wal_path(&store.path)).unwrap() <= 64 * MIB);
            assert!(allocated_len(&store.path).unwrap() <= 64 * MIB);
            s.max_buffer = s.max_buffer.max(store.max_buffer);
            s.max_dirty = s.max_dirty.max(store.max_dirty);
            s.max_rows = s.max_rows.max(store.max_rows);
        }
        assert_eq!(h.volume.lock().unwrap().reserved, 0);
        let settled_total: u64 = fs::read_dir(h.path().join("accounts"))
            .unwrap()
            .map(|e| {
                let p = e.unwrap().path().join("state-v2.db");
                allocated_len(&p).unwrap() + allocated_len(&wal_path(&p)).unwrap()
            })
            .sum();
        assert!(
            settled_total <= 64 * MIB,
            "all retired account DBs remain bounded over 20 cycles"
        );
        println!(
            "cycle={} allocated_db_peak={db_bytes} allocated_wal_peak={wal_bytes} reserved_peak={reserved_peak} available_peak={available_peak} settled_all_databases={settled_total}",
            cycle + 1
        );
    }
    let o = owner(&s, "Snapshot");
    let a = s
        .capture(
            o,
            &mut Generated {
                left: 3 * 1024 * MIB,
                byte: 0x72,
            },
            3 * 1024 * MIB,
            &mut Fault::default(),
        )
        .unwrap();
    s.verify(&a).unwrap();
    assert!(s.max_buffer <= 16 * MIB as usize && s.max_dirty <= 8 * MIB && s.max_rows <= 256);
    println!(
        "cycles=20 peak_db={peak_db} peak_wal={peak_wal} max_buffer={} max_dirty={} max_rows={}",
        s.max_buffer, s.max_dirty, s.max_rows
    );
}

#[test]
fn slice1_g1_full_gap_and_live_reference_gc_gate() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let o = owner(&s, "Snapshot");
    let mut a = s.allocate(o, "Capture", "Full", 3, 0, &mut Fault::default()).unwrap();
    s.chunk(&a, 0, b"abc").unwrap();
    s.extent(
        &a,
        0,
        Extent {
            offset: 1,
            packed: 0,
            len: 3,
        },
    )
    .unwrap();
    assert!(
        s.seal(&mut a, 4, digest(b"abc"), &witness(), &mut Fault::default())
            .is_err(),
        "Full coverage gap must fail"
    );
    let a = captured(&mut s, b"live reference");
    s.adopt(&a, "Snapshot", true, &mut Fault::default()).unwrap();
    let d = id();
    s.db.execute(
        "INSERT INTO v2_dispositions VALUES(?1,?2,'UserDiscard',1,?3)",
        params![d.as_slice(), a.owner.as_slice(), discard(&a)],
    )
    .unwrap();
    s.db.execute(
        "INSERT INTO v2_gc VALUES(?1,?2,0,'Pending',NULL)",
        params![a.id.as_slice(), d.as_slice()],
    )
    .unwrap();
    assert!(
        s.gc(&a, &mut Fault::default()).is_err(),
        "live reference must block GC even with a disposition"
    );
    s.verify(&a).unwrap();
}

#[test]
fn slice1_g4_interrupted_duplicate_import_rebuilds_reservations() {
    let h = Harness::new().unwrap();
    let s = Store::new(&h).unwrap();
    let o = owner(&s, "Migration");
    let a = s
        .allocate(
            o,
            "LegacyRecovery",
            "LegacyPartial",
            1024 * MIB,
            1024 * MIB,
            &mut Fault::default(),
        )
        .unwrap();
    let expected = Store::reserve_cost(1024 * MIB, 1024 * MIB, 1).unwrap();
    assert_eq!(h.volume.lock().unwrap().reserved, expected);
    let path = s.path.clone();
    drop(s);
    for _ in 0..2 {
        h.volume.lock().unwrap().reserved = 0;
        let mut restored = Store::reopen(&h, &path).unwrap();
        restored.reconstruct_reservations().unwrap();
        assert_eq!(h.volume.lock().unwrap().reserved, expected);
        let o = owner(&restored, "Snapshot");
        restored.quota = expected + 1024;
        assert!(
            restored
                .allocate(o, "Capture", "Full", 1024 * MIB, 0, &mut Fault::default())
                .is_err()
        );
        assert_eq!(
            restored
                .db
                .query_row(
                    "SELECT expected_bytes FROM v2_artifacts WHERE artifact_id=?1",
                    [a.id.as_slice()],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            1024 * MIB
        );
    }
}
#[test]
fn slice1_g1_malformed_body_import_and_missing_wal_refuse_write() {
    let h = Harness::new().unwrap();
    let s = Store::new(&h).unwrap();
    let o = owner(&s, "RecoveryCase");
    s.db.execute("UPDATE v2_owners SET body=x'00' WHERE owner_id=?1", [o.as_slice()])
        .unwrap();
    let path = s.path.clone();
    drop(s);
    assert!(Store::reopen(&h, &path).is_err());
    let s = Store::new(&h).unwrap();
    let path = s.path.clone();
    drop(s);
    fs::remove_file(wal_path(&path)).unwrap();
    assert!(Store::reopen(&h, &path).is_err());
}

#[test]
fn slice1_g7_vacuum_step_drains_its_bounded_result_rows() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let a = captured(&mut s, &vec![3; CHUNK]);
    s.db.execute("DELETE FROM v2_chunks WHERE artifact_id=?1", [a.id.as_slice()])
        .unwrap();
    let before: u64 = s.db.query_row("PRAGMA freelist_count", [], |r| r.get(0)).unwrap();
    assert!(before > 256);
    s.reclaim_step().unwrap();
    let after: u64 = s.db.query_row("PRAGMA freelist_count", [], |r| r.get(0)).unwrap();
    assert_eq!(before - after, 256, "one vacuum step must consume all 256 result rows");
}

#[test]
fn slice1_g2_native_vfs_flush_aware_cutpoints() {
    fn scenario(cut: usize) -> (usize, usize) {
        let h = Harness::new().unwrap();
        let vfs = fault_vfs::FaultVfs::new().unwrap();
        let mut s = Store::new_with_vfs(&h, Some(vfs.name())).unwrap();
        let baseline = captured(&mut s, b"baseline preserved before injection");
        s.adopt(&baseline, "Snapshot", true, &mut Fault::default()).unwrap();
        let baseline_relative = s.path.strip_prefix(h.path()).unwrap().to_owned();
        let next_owner = owner(&s, "Snapshot");
        vfs.arm(cut);
        let next = s.capture(
            next_owner,
            &mut std::io::Cursor::new(vec![0x37; 65536]),
            65536,
            &mut Fault::default(),
        );
        let committed = match next {
            Ok(a) => {
                let p = s.adopt(&a, "Snapshot", true, &mut Fault::default());
                if let Ok(p) = p {
                    s.fake_sink(&a, &p).unwrap();
                    Some(a)
                } else {
                    None
                }
            }
            Err(_) => None,
        };
        let events = vfs.event_count();
        let syncs = vfs.sync_count();
        drop(s);
        let restart = Harness::new().unwrap();
        vfs.reboot(h.path(), restart.path()).unwrap();
        let path = restart.path().join(baseline_relative);
        for _ in 0..2 {
            let mut restored = Store::reopen(&restart, &path).unwrap();
            restored.verify(&baseline).unwrap();
            if let Some(a) = &committed {
                restored.verify(a).unwrap();
            }
        }
        (events, syncs)
    }
    let (events, syncs) = scenario(usize::MAX);
    assert!(events > 10 && syncs > 0, "VFS must observe writes and flushes");
    // The initial owner-insert is itself a fault boundary: create the owner before
    // arming in all cut runs so each failure is handled by the storage operation.
    for cut in 1..=events {
        scenario(cut);
    }
    println!(
        "vfs_cutpoints={events} completed_control_syncs={syncs} restarts={}",
        events * 2
    );
}

#[test]
fn slice1_g2_uncertain_capture_and_adoption_deduplicate() {
    for cut in 1..=8 {
        let h = Harness::new().unwrap();
        let mut s = Store::new(&h).unwrap();
        let o = owner(&s, "Snapshot");
        let bytes = b"same held-source fixture";
        let result = s.capture(
            o,
            &mut std::io::Cursor::new(bytes),
            bytes.len() as u64,
            &mut Fault {
                cut: Some(cut),
                seen: 0,
            },
        );
        if let Ok(a) = result {
            let _ = s.adopt(&a, "Snapshot", true, &mut Fault { cut: Some(2), seen: 0 });
        }
        let a = s
            .capture(
                o,
                &mut std::io::Cursor::new(bytes),
                bytes.len() as u64,
                &mut Fault::default(),
            )
            .unwrap();
        s.adopt(&a, "Snapshot", true, &mut Fault::default()).unwrap();
        s.adopt(&a, "Snapshot", true, &mut Fault::default()).unwrap();
        assert_eq!(
            s.db.query_row(
                "SELECT count(*) FROM v2_artifacts WHERE allocation_owner=?1",
                [o.as_slice()],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            s.db.query_row(
                "SELECT count(*) FROM v2_refs WHERE artifact_id=?1",
                [a.id.as_slice()],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
    }
}
#[test]
fn slice1_g1_receipts_handoff_and_atomic_ownership_transfer() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let a = captured(&mut s, b"verified receipt bytes");
    let other = owner(&s, "RecoveryCase");
    s.db.execute(
        "INSERT INTO v2_refs VALUES(?1,?2,'Recovery')",
        params![other.as_slice(), a.id.as_slice()],
    )
    .unwrap();
    let proof = wire::record("OwnershipTransfer", &[&a.id, &a.hash, &other, b"Recovery"]).unwrap();
    let d = s
        .dispose_named(a.owner, &a, "OwnershipTransfer", &proof, &mut Fault::default())
        .unwrap();
    assert!(
        s.db.execute(
            "UPDATE v2_owners SET terminal_disposition=?2 WHERE owner_id=?1",
            params![other.as_slice(), d.as_slice()]
        )
        .is_err(),
        "disposition belongs to its owner"
    );
    assert!(s.gc(&a, &mut Fault::default()).is_err());
    s.verify(&a).unwrap();
    let unqualified = wire::record(
        "UserHandoff",
        &[
            &a.id,
            &a.hash,
            &a.bytes.to_be_bytes(),
            b"chosen-destination",
            b"Unqualified",
            b"Acknowledged",
        ],
    )
    .unwrap();
    assert!(
        s.dispose_named(other, &a, "UserHandoff", &unqualified, &mut Fault::default())
            .is_err()
    );
    let exported = tempfile::tempdir().unwrap();
    let destination = exported.path().join("chosen-file");
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&destination)
        .unwrap();
    file.write_all(b"verified receipt bytes").unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert_eq!(digest(&fs::read(&destination).unwrap()), a.hash);
    let handoff = wire::record(
        "UserHandoff",
        &[
            &a.id,
            &a.hash,
            &a.bytes.to_be_bytes(),
            b"chosen-destination",
            b"QualifiedFixture",
            b"Acknowledged",
        ],
    )
    .unwrap();
    let d = s
        .dispose_named(other, &a, "UserHandoff", &handoff, &mut Fault::default())
        .unwrap();
    assert_eq!(
        s.dispose_named(other, &a, "UserHandoff", &handoff, &mut Fault::default())
            .unwrap(),
        d
    );
    s.gc(&a, &mut Fault::default()).unwrap();
    assert_eq!(fs::read(&destination).unwrap(), b"verified receipt bytes");
    let a = captured(&mut s, b"server receipt");
    let receipt = wire::record(
        "ServerReceipt",
        &[
            &a.id,
            &a.hash,
            &a.bytes.to_be_bytes(),
            b"immutable-server-object",
            b"v7",
        ],
    )
    .unwrap();
    s.dispose_named(a.owner, &a, "ServerReceipt", &receipt, &mut Fault::default())
        .unwrap();
    s.gc(&a, &mut Fault::default()).unwrap();
}
#[test]
fn slice1_g7_many_small_records_and_unknown_phase_rejection() {
    let h = Harness::new().unwrap();
    let s = Store::new(&h).unwrap();
    for _ in 0..1024 {
        owner(&s, "RecoveryCase");
    }
    assert_eq!(
        s.db.query_row("SELECT count(*) FROM v2_owners", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1024
    );
    let mut fields = records::decode("InstallIntent", &owner_body(&s, "InstallIntent")).unwrap();
    fields[3] = b"PartialMaterialization".to_vec();
    assert!(records::encode("InstallIntent", &fields).is_err());
    let mut fields = records::decode("FileControl", &owner_body(&s, "FileControl")).unwrap();
    fields[6] = b"S4".to_vec();
    assert!(records::encode("FileControl", &fields).is_err());
    let mut fields = records::decode("Migration", &owner_body(&s, "Migration")).unwrap();
    fields[7] = b"UnknownPhase".to_vec();
    assert!(records::encode("Migration", &fields).is_err());
}

#[test]
fn slice1_g7_measured_buffers_and_gc_batches() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let o = owner(&s, "Snapshot");
    let a = s
        .capture(
            o,
            &mut Generated {
                left: 32 * MIB,
                byte: 0x32,
            },
            32 * MIB,
            &mut Fault::default(),
        )
        .unwrap();
    s.verify(&a).unwrap();
    assert!(s.max_buffer <= 16 * MIB as usize, "application working buffer bound");
    let o = owner(&s, "Migration");
    let mut a = s
        .allocate(o, "LegacyRecovery", "LegacyPartial", 4096, 4096, &mut Fault::default())
        .unwrap();
    s.chunk(&a, 0, &vec![0x73; 4096]).unwrap();
    for i in 0..4096 {
        s.extent(
            &a,
            i,
            Extent {
                offset: 2 * i,
                packed: i,
                len: 1,
            },
        )
        .unwrap();
    }
    s.seal(
        &mut a,
        8192,
        digest(&vec![0x73; 4096]),
        &witness(),
        &mut Fault::default(),
    )
    .unwrap();
    s.dispose(&a, &discard(&a), &mut Fault::default()).unwrap();
    s.gc(&a, &mut Fault::default()).unwrap();
    assert!(s.max_rows <= 256, "actual metadata deletion batch bound");
    assert_eq!(s.max_rows, 256);
}

#[test]
fn slice1_g2_connection_environment_evidence() {
    let h = Harness::new().unwrap();
    let s = Store::new(&h).unwrap();
    let version: String = s.db.query_row("SELECT sqlite_version()", [], |r| r.get(0)).unwrap();
    let options: Vec<String> =
        s.db.prepare("PRAGMA compile_options")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
    assert!(options.len() > 10);
    verify_pragmas(&s.db).unwrap();
    let vfs = unsafe {
        let v = rusqlite::ffi::sqlite3_vfs_find(std::ptr::null());
        std::ffi::CStr::from_ptr((*v).zName).to_string_lossy().into_owned()
    };
    println!(
        "sqlite={version} native_vfs={vfs} compile_option_count={} options={options:?} synchronous=2 foreign_keys=1 page_size=4096 auto_vacuum=2 wal_autocheckpoint=0 persistent_wal=1 allocated_db={} allocated_wal={}",
        options.len(),
        allocated_len(&s.path).unwrap(),
        allocated_len(&wal_path(&s.path)).unwrap()
    );
}
#[cfg(windows)]
#[test]
fn slice1_g5_windows_sharing_blocks_key_purge_safely() {
    use std::os::windows::fs::OpenOptionsExt;
    let h = Harness::new().unwrap();
    let l = Ledger::open(&h).unwrap();
    let record = id();
    let token = id();
    l.put(
        record,
        "Envelope",
        &envelope(id()),
        Some(30),
        Some(token),
        &mut Fault::default(),
    )
    .unwrap();
    let key = l.key(&record).unwrap();
    let path = h.path().join("keyslots").join(hex(&record));
    let wrapped = fs::read(&path).unwrap();
    assert!(wrapped.len() > 32);
    assert_eq!(
        wrapped.windows(32).filter(|b| *b == key.as_slice()).count(),
        0,
        "DPAPI key slot has no plaintext key"
    );
    let held = fs::OpenOptions::new().read(true).share_mode(1).open(&path).unwrap();
    l.deadline(record, 0).unwrap();
    assert!(l.purge(0, &mut Fault::default()).is_err());
    assert!(path.exists());
    assert!(l.read(record).is_err());
    assert!(l.deny(token).unwrap());
    drop(held);
    assert_eq!(l.purge(0, &mut Fault::default()).unwrap(), 1);
    assert!(!path.exists());
    assert!(l.deny(token).unwrap());
}

/// OS available-to-caller bytes, captured alongside logical reservations.
pub(super) fn available_bytes(path: &Path) -> u64 {
    #[cfg(unix)]
    unsafe {
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        assert_eq!(libc::statvfs(name.as_ptr(), info.as_mut_ptr()), 0);
        let info = info.assume_init();
        info.f_bavail as u64 * info.f_frsize as u64
    }
    #[cfg(windows)]
    unsafe {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "Kernel32")]
        unsafe extern "system" {
            fn GetDiskFreeSpaceExW(path: *const u16, available: *mut u64, total: *mut u64, free: *mut u64) -> i32;
        }
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0;
        assert_ne!(
            GetDiskFreeSpaceExW(
                name.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ),
            0
        );
        available
    }
}

#[test]
fn slice1_g2_disposition_gc_cuts_reclaim_after_terminal_commit() {
    for cut in 1..=8 {
        let h = Harness::new().unwrap();
        let mut s = Store::new(&h).unwrap();
        let a = captured(&mut s, &vec![0x29; CHUNK * 2]);
        let path = s.path.clone();
        let mut fault = Fault {
            cut: Some(cut),
            seen: 0,
        };
        let attempt = (|| -> Result<()> {
            s.dispose(&a, &discard(&a), &mut fault)?;
            s.gc(&a, &mut fault)
        })();
        assert!(attempt.is_err(), "cut {cut} must execute");
        assert_eq!(fault.seen, cut);
        drop(s);
        for restart in 1..=2 {
            let mut s = Store::reopen(&h, &path).unwrap();
            s.dispose(&a, &discard(&a), &mut Fault::default()).unwrap();
            s.gc(&a, &mut Fault::default()).unwrap();
            for table in ["v2_chunks", "v2_manifests", "v2_refs"] {
                let count: u64 =
                    s.db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                        .unwrap();
                assert_eq!(count, 0, "{table} cut={cut} restart={restart}");
            }
            let dispositions: u64 =
                s.db.query_row("SELECT count(*) FROM v2_dispositions", [], |r| r.get(0))
                    .unwrap();
            assert_eq!(dispositions, 1);
            assert_eq!(h.volume.lock().unwrap().reserved, 0);
            let free: u64 = s.db.query_row("PRAGMA freelist_count", [], |r| r.get(0)).unwrap();
            assert_eq!(
                free, 0,
                "GC terminal commit must resume physical reclamation: cut={cut} restart={restart}"
            );
            assert!(allocated_len(&wal_path(&s.path)).unwrap() <= 64 * MIB);
        }
    }
    println!("gc_disposition_cutpoints=8 recovery_opens=16");
}

#[test]
fn slice1_g4_owner_admission_waits_for_account_bootstrap() {
    let h = Harness::new().unwrap();
    let s = Store::new(&h).unwrap();
    let body = owner_body(&s, "RecoveryCase");
    let construction = h.volume.lock().unwrap();
    let partial = h.path().join("accounts").join(hex(&id())).join("state-v2.db");
    fs::create_dir_all(partial.parent().unwrap()).unwrap();
    fs::File::create(&partial).unwrap();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            entered_tx.send(()).unwrap();
            done_tx.send(s.owner(id(), "RecoveryCase", &body)).unwrap();
        });
        entered_rx.recv().unwrap();
        assert!(
            matches!(
                done_rx.recv_timeout(std::time::Duration::from_millis(500)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ),
            "owner admission must wait while another account schema is being established"
        );
        fs::remove_file(&partial).unwrap(); // finish this fixture's paused empty-file bootstrap
        drop(schema_open(&partial).unwrap());
        drop(construction);
        done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
    });
}

#[test]
fn slice1_g7_dirty_pages_count_wal_reuse() {
    let h = Harness::new().unwrap();
    let mut s = Store::new(&h).unwrap();
    let o = owner(&s, "Snapshot");
    let a = s
        .allocate(o, "Capture", "Full", 8 * MIB, 0, &mut Fault::default())
        .unwrap();
    s.chunk(&a, 0, &vec![1; CHUNK]).unwrap();
    let busy: i64 =
        s.db.query_row("PRAGMA wal_checkpoint(RESTART)", [], |r| r.get(0))
            .unwrap();
    assert_eq!(busy, 0);
    let before = file_len(&wal_path(&s.path));
    s.max_dirty = 0;
    s.chunk(&a, 1, &vec![2; CHUNK]).unwrap();
    assert!(
        file_len(&wal_path(&s.path)) <= before,
        "fixture must actually reuse WAL capacity"
    );
    assert!(
        s.max_dirty >= CHUNK as u64 && s.max_dirty <= DIRTY_LIMIT,
        "dirty pages must be counted even without WAL growth: {}",
        s.max_dirty
    );
}

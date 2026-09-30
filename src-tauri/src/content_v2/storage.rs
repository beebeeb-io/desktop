use super::*;

pub(super) struct Store {
    pub(super) db: Connection,
    pub(super) path: PathBuf,
    volume: Arc<Mutex<Budget>>,
    pub(super) quota: u64,
    pub(super) max_buffer: usize,
    pub(super) max_dirty: u64,
    pub(super) max_rows: usize,
}
#[derive(Clone, Debug)]
pub(super) struct Artifact {
    pub id: Id,
    pub owner: Id,
    pub reservation: Id,
    pub bytes: u64,
    pub hash: Id,
}
#[derive(Clone, Copy)]
pub(super) struct Extent {
    pub offset: u64,
    pub packed: u64,
    pub len: u64,
}
/// A fixture proof is deliberately not accepted by any CFAPI or network API.
pub(super) struct Preservation {
    owner: Id,
    artifact: Id,
    revision: i64,
}
impl Store {
    pub(super) fn new(h: &Harness) -> Result<Self> {
        Self::new_with_vfs(h, None)
    }
    pub(super) fn new_with_vfs(h: &Harness, vfs: Option<&str>) -> Result<Self> {
        let _creation = h.volume.lock().map_err(|_| anyhow::anyhow!("allocator poisoned"))?;
        let store_id = id();
        let dir = h.path().join("accounts").join(hex(&store_id));
        fs::create_dir_all(&dir)?;
        let path = dir.join("state-v2.db");
        let db = open_db_vfs(&path, include_str!("account.sql"), vfs)?;
        db.execute(
            "INSERT INTO v2_store VALUES(1,1,?1,?2,?3,NULL,NULL,'Fixture',?4)",
            params![store_id.as_slice(), id().as_slice(), id().as_slice(), 20 * 1024 * MIB],
        )?;
        Ok(Self {
            db,
            path,
            volume: h.volume.clone(),
            quota: 20 * 1024 * MIB,
            max_buffer: 0,
            max_dirty: 0,
            max_rows: 0,
        })
    }
    pub(super) fn reopen(h: &Harness, path: &Path) -> Result<Self> {
        let db = schema_open(path)?;
        let quota = db.query_row("SELECT quota_bytes FROM v2_store WHERE singleton=1", [], |r| r.get(0))?;
        // Recovered ownership is data only. No submission or destructive grant is reconstructed.
        Ok(Self {
            db,
            path: path.to_owned(),
            volume: h.volume.clone(),
            quota,
            max_buffer: 0,
            max_dirty: 0,
            max_rows: 0,
        })
    }
    pub(super) fn owner(&self, owner: Id, kind: &str, body: &[u8]) -> Result<()> {
        metadata_admitted(&self.db, &self.path, false)?;
        let (reserved, excess) = self.budget_from_disk()?;
        ensure!(reserved + excess + DIRTY_LIMIT <= self.quota, "metadata quota");
        let fields = records::decode(kind, body)?;
        let (account, root): (Vec<u8>, Vec<u8>) = self.db.query_row(
            "SELECT account_binding,root_token FROM v2_store WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(fields[0] == account && fields[1] == root, "owner scope mismatch");
        if kind == "Upload" {
            let snapshot: &[u8] = &fields[3];
            let complete:i64=self.db.query_row("SELECT count(*) FROM v2_owners o JOIN v2_artifacts a ON a.allocation_owner=o.owner_id JOIN v2_manifests m USING(artifact_id) WHERE o.owner_id=?1 AND o.kind='Snapshot' AND a.format='Full' AND a.phase IN ('Ready','Referenced')",[snapshot],|r|r.get(0))?;
            ensure!(complete == 1, "Upload requires one complete Full Snapshot");
        }
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO v2_owners(owner_id,kind,revision,body_version,body) VALUES(?1,?2,0,1,?3)",
            params![owner.as_slice(), kind, body],
        )?;
        if kind == "Upload" {
            tx.execute("INSERT INTO v2_refs SELECT ?1,artifact_id,'Snapshot' FROM v2_artifacts WHERE allocation_owner=?2 AND phase IN ('Ready','Referenced')",params![owner.as_slice(),&fields[3]])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub(super) fn cas(&self, owner: Id, revision: i64, body: &[u8]) -> Result<()> {
        metadata_admitted(&self.db, &self.path, false)?;
        let kind: String = self.db.query_row(
            "SELECT kind FROM v2_owners WHERE owner_id=?1",
            [owner.as_slice()],
            |r| r.get(0),
        )?;
        let fields = records::decode(&kind, body)?;
        let old: Vec<u8> = self.db.query_row(
            "SELECT body FROM v2_owners WHERE owner_id=?1",
            [owner.as_slice()],
            |r| r.get(0),
        )?;
        let previous = records::decode(&kind, &old)?;
        ensure!(fields[..3] == previous[..3], "immutable owner identity");
        if kind == "Upload" {
            ensure!(fields[3..6] == previous[3..6], "immutable upload intent");
        }
        ensure!(kind != "Snapshot", "immutable snapshot");
        ensure!(
            self.db.execute(
                "UPDATE v2_owners SET revision=revision+1,body=?3 WHERE owner_id=?1 AND revision=?2",
                params![owner.as_slice(), revision, body]
            )? == 1,
            "stale owner revision"
        );
        Ok(())
    }
    pub(super) fn reserve_cost(payload: u64, duplicates: u64, n: u64) -> Result<u64> {
        payload
            .checked_add(payload.div_ceil(4))
            .and_then(|x| x.checked_add(2 * MIB * n))
            .and_then(|x| x.checked_add(duplicates))
            .and_then(|x| x.checked_add(WAL_LIMIT + 256 * MIB))
            .context("reservation overflow")
    }
    fn budget_from_disk(&self) -> Result<(u64, u64)> {
        let accounts = self
            .path
            .parent()
            .context("store parent")?
            .parent()
            .context("accounts parent")?;
        let mut reserved = 0u64;
        let mut excess = 0u64;
        for entry in fs::read_dir(accounts)? {
            let path = entry?.path().join("state-v2.db");
            if !path.exists() {
                continue;
            }
            let conn = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let (payload,count):(u64,u64)=conn.query_row("SELECT coalesce(sum(payload_bytes+duplicate_bytes),0),count(*) FROM v2_reservations WHERE phase<>'Released'",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
            let cost = payload + if count > 0 { WAL_LIMIT } else { 0 };
            let allocated = allocated_len(&path)? + allocated_len(&wal_path(&path))?;
            reserved = reserved.checked_add(cost).context("volume reservation overflow")?;
            excess = excess
                .checked_add(allocated.saturating_sub(cost))
                .context("volume allocation overflow")?;
        }
        if reserved > 0 {
            reserved = reserved.checked_add(256 * MIB).context("terminal reserve overflow")?;
        }
        Ok((reserved, excess))
    }
    pub(super) fn reconstruct_reservations(&self) -> Result<()> {
        let mut global = self.volume.lock().map_err(|_| anyhow::anyhow!("allocator poisoned"))?;
        global.reserved = self.budget_from_disk()?.0;
        Ok(())
    }
    pub(super) fn allocate(
        &self,
        owner: Id,
        purpose: &str,
        format: &str,
        size: u64,
        duplicates: u64,
        fault: &mut Fault,
    ) -> Result<Artifact> {
        fault.point("before B0")?;
        verify_pragmas(&self.db)?;
        let kind: String = self.db.query_row(
            "SELECT kind FROM v2_owners WHERE owner_id=?1",
            [owner.as_slice()],
            |r| r.get(0),
        )?;
        let purpose_ok = match kind.as_str() {
            "Snapshot" => purpose == "Capture",
            "InstallIntent" => matches!(purpose, "Preimage" | "Replacement"),
            "Migration" => purpose == "LegacyRecovery",
            "RecoveryCase" => matches!(purpose, "Capture" | "LegacyRecovery" | "ExportCache"),
            _ => false,
        };
        ensure!(purpose_ok, "wrong allocation purpose");
        if format != "Full" {
            ensure!(
                matches!(kind.as_str(), "Migration" | "RecoveryCase") && purpose == "LegacyRecovery",
                "wrong-purpose partial"
            );
        }
        let reservation = id();
        let artifact = id();
        let mut cost = Self::reserve_cost(size, duplicates, 1)?;
        let mut global = self.volume.lock().map_err(|_| anyhow::anyhow!("allocator poisoned"))?;
        let existing:Option<(Vec<u8>,Vec<u8>,String,u64,Vec<u8>)>=self.db.query_row("SELECT a.artifact_id,a.reservation_id,a.format,a.expected_bytes,coalesce(m.content_digest,zeroblob(32)) FROM v2_artifacts a LEFT JOIN v2_manifests m USING(artifact_id) WHERE a.allocation_owner=?1 AND a.purpose=?2 AND a.phase<>'Removed'",params![owner.as_slice(),purpose],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        if let Some((artifact, reservation, old_format, old_size, hash)) = existing {
            ensure!(
                old_format == format && old_size == size,
                "allocation identity collision"
            );
            return Ok(Artifact {
                id: artifact.try_into().map_err(|_| anyhow::anyhow!("artifact identity"))?,
                owner,
                reservation: reservation
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("reservation identity"))?,
                bytes: size,
                hash: hash.try_into().map_err(|_| anyhow::anyhow!("digest identity"))?,
            });
        }
        let (reserved, excess) = self.budget_from_disk()?;
        global.reserved = reserved;
        let active: u64 = self.db.query_row(
            "SELECT count(*) FROM v2_reservations WHERE phase<>'Released'",
            [],
            |r| r.get(0),
        )?;
        let wal = if active == 0 { WAL_LIMIT } else { 0 };
        let terminal = if reserved == 0 { 256 * MIB } else { 0 };
        cost = cost - WAL_LIMIT - 256 * MIB + wal + terminal;
        ensure!(
            global.reserve(cost, self.quota.saturating_sub(excess)),
            "shared volume quota"
        );
        let commit = (|| -> Result<()> {
            let tx = self.db.unchecked_transaction()?;
            tx.execute(
                "INSERT INTO v2_reservations VALUES(?1,?2,?3,?4,?5,?6,?7,0,'Reserved')",
                params![
                    reservation.as_slice(),
                    owner.as_slice(),
                    b"fixture-volume".as_slice(),
                    size + size.div_ceil(4) + 2 * MIB,
                    duplicates,
                    wal,
                    terminal
                ],
            )?;
            tx.execute(
                "INSERT INTO v2_artifacts VALUES(?1,?2,?3,'Allocated',?4,?5,?6)",
                params![
                    artifact.as_slice(),
                    owner.as_slice(),
                    purpose,
                    format,
                    size,
                    reservation.as_slice()
                ],
            )?;
            let role = match purpose {
                "Capture" => "Snapshot",
                "Preimage" => "Preimage",
                "Replacement" => "Replacement",
                "LegacyRecovery" => "Recovery",
                "ExportCache" => "Export",
                _ => bail!("purpose"),
            };
            tx.execute(
                "INSERT INTO v2_refs VALUES(?1,?2,?3)",
                params![owner.as_slice(), artifact.as_slice(), role],
            )?;
            tx.commit()?;
            Ok(())
        })();
        if commit.is_err() {
            global.release(cost)?;
        }
        commit?;
        drop(global);
        fault.point("after B0")?;
        Ok(Artifact {
            id: artifact,
            owner,
            reservation,
            bytes: size,
            hash: [0; 32],
        })
    }
    fn before_write(&self, terminal: bool) -> Result<()> {
        verify_pragmas(&self.db)?;
        let len = file_len(&wal_path(&self.path));
        if !terminal {
            ensure!(payload_admitted(len), "pinned reader: payload backpressure");
        }
        ensure!(
            len.checked_add(DIRTY_LIMIT).is_some_and(|x| x <= WAL_LIMIT),
            "terminal WAL budget exhausted"
        );
        Ok(())
    }
    pub(super) fn chunk(&mut self, a: &Artifact, index: u64, bytes: &[u8]) -> Result<()> {
        ensure!(!bytes.is_empty() && bytes.len() <= CHUNK, "chunk bound");
        self.before_write(false)?;
        let phase: String = self.db.query_row(
            "SELECT phase FROM v2_artifacts WHERE artifact_id=?1",
            [a.id.as_slice()],
            |r| r.get(0),
        )?;
        ensure!(phase == "Allocated", "immutable sealed artifact");
        let hash = digest(bytes);
        let previous: Option<(Vec<u8>, Vec<u8>)> = self
            .db
            .query_row(
                "SELECT digest,payload FROM v2_chunks WHERE artifact_id=?1 AND chunk_index=?2",
                params![a.id.as_slice(), index],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((h, p)) = previous {
            ensure!(h == hash && p == bytes, "immutable chunk");
            return Ok(());
        }
        let count: u64 = self.db.query_row(
            "SELECT count(*) FROM v2_chunks WHERE artifact_id=?1",
            [a.id.as_slice()],
            |r| r.get(0),
        )?;
        ensure!(index == count, "non-contiguous chunk index");
        let (written,expected):(u64,u64)=self.db.query_row("SELECT (SELECT coalesce(sum(byte_length),0) FROM v2_chunks WHERE artifact_id=?1),expected_bytes FROM v2_artifacts WHERE artifact_id=?1",[a.id.as_slice()],|r|Ok((r.get(0)?,r.get(1)?)))?;
        ensure!(
            written.checked_add(bytes.len() as u64).is_some_and(|n| n <= expected),
            "payload exceeds reservation"
        );
        let before = file_len(&wal_path(&self.path));
        let tx = self.db.unchecked_transaction()?;
        let phase: String = tx.query_row(
            "SELECT phase FROM v2_artifacts WHERE artifact_id=?1",
            [a.id.as_slice()],
            |r| r.get(0),
        )?;
        ensure!(phase == "Allocated", "capture ownership changed");
        let live_before: u64 = tx.query_row(
            "SELECT (SELECT page_count FROM pragma_page_count)-(SELECT freelist_count FROM pragma_freelist_count)",
            [],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO v2_chunks VALUES(?1,?2,?3,?4,?5)",
            params![a.id.as_slice(), index, bytes.len(), hash.as_slice(), bytes],
        )?;
        let live_after: u64 = tx.query_row(
            "SELECT (SELECT page_count FROM pragma_page_count)-(SELECT freelist_count FROM pragma_freelist_count)",
            [],
            |r| r.get(0),
        )?;
        let growth = live_after.saturating_sub(live_before) * 4096;
        let (used, allowance): (u64, u64) = tx.query_row(
            "SELECT consumed_bytes,payload_bytes FROM v2_reservations WHERE reservation_id=?1",
            [a.reservation.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(used + growth <= allowance, "page amplification exceeds reservation");
        tx.execute(
            "UPDATE v2_reservations SET consumed_bytes=consumed_bytes+?2 WHERE reservation_id=?1",
            params![a.reservation.as_slice(), growth],
        )?;
        tx.commit()?;
        let dirty = file_len(&wal_path(&self.path)).saturating_sub(before);
        self.max_dirty = self.max_dirty.max(dirty);
        ensure!(dirty <= DIRTY_LIMIT, "transaction dirty page bound");
        self.max_buffer = self.max_buffer.max(bytes.len() * 2);
        if file_len(&wal_path(&self.path)) >= 64 * MIB {
            checkpoint(&self.db, &self.path, false)?;
        }
        Ok(())
    }
    pub(super) fn extent(&self, a: &Artifact, ordinal: u64, e: Extent) -> Result<()> {
        let phase: String = self.db.query_row(
            "SELECT phase FROM v2_artifacts WHERE artifact_id=?1",
            [a.id.as_slice()],
            |r| r.get(0),
        )?;
        ensure!(phase == "Allocated", "immutable extents");
        let old: Option<(u64, u64, u64)> = self
            .db
            .query_row(
                "SELECT logical_offset,packed_offset,byte_length FROM v2_extents WHERE artifact_id=?1 AND ordinal=?2",
                params![a.id.as_slice(), ordinal],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some(old) = old {
            ensure!(old == (e.offset, e.packed, e.len), "immutable extent");
            return Ok(());
        }
        self.db.execute(
            "INSERT INTO v2_extents VALUES(?1,?2,?3,?4,?5)",
            params![a.id.as_slice(), ordinal, e.offset, e.packed, e.len],
        )?;
        Ok(())
    }
    fn scan(
        &mut self,
        a: &Artifact,
        eof: u64,
        format: &str,
        witness: &[u8],
        base: Option<&[u8]>,
    ) -> Result<(Id, Id, u64, u64, u64)> {
        let mut content = Sha256::new();
        let mut manifest = Sha256::new();
        let (owner,purpose,kind,body):(Vec<u8>,String,String,Vec<u8>)=self.db.query_row("SELECT a.allocation_owner,a.purpose,o.kind,o.body FROM v2_artifacts a JOIN v2_owners o ON o.owner_id=a.allocation_owner WHERE a.artifact_id=?1",[a.id.as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        ensure!(owner == a.owner, "artifact owner mismatch");
        let fields = records::decode(&kind, &body)?;
        manifest.update(wire::record(
            "Manifest",
            &[
                format.as_bytes(),
                &eof.to_be_bytes(),
                &a.id,
                &owner,
                purpose.as_bytes(),
                &fields[0],
                &fields[1],
                &fields[2],
                witness,
                base.unwrap_or_default(),
            ],
        )?);
        let mut index: u64 = 0;
        let mut packed = 0;
        loop {
            // Each query drops its read transaction before hashing or requesting another row.
            let row: Option<(u64, Vec<u8>, Vec<u8>)> = self
                .db
                .query_row(
                    "SELECT byte_length,digest,payload FROM v2_chunks WHERE artifact_id=?1 AND chunk_index=?2",
                    params![a.id.as_slice(), index],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((len, hash, payload)) = row else { break };
            ensure!(
                len == payload.len() as u64 && digest(&payload).as_slice() == hash,
                "corrupt chunk"
            );
            self.max_buffer = self.max_buffer.max(payload.len() + CHUNK);
            content.update(&payload);
            manifest.update(index.to_be_bytes());
            manifest.update(len.to_be_bytes());
            manifest.update(&hash);
            packed += len;
            index += 1;
        }
        let count: u64 = self.db.query_row(
            "SELECT count(*) FROM v2_chunks WHERE artifact_id=?1",
            [a.id.as_slice()],
            |r| r.get(0),
        )?;
        ensure!(count == index, "missing/reordered chunk");
        let mut ordinal = 0;
        let mut packed_end = 0;
        let mut logical_end = 0;
        loop {
            let e:Option<(u64,u64,u64)>=self.db.query_row("SELECT logical_offset,packed_offset,byte_length FROM v2_extents WHERE artifact_id=?1 AND ordinal=?2",params![a.id.as_slice(),ordinal],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            let Some((offset, p, len)) = e else { break };
            ensure!(
                p == packed_end && offset >= logical_end && offset.checked_add(len).is_some_and(|x| x <= eof),
                "invalid extent coverage"
            );
            if format == "Full" {
                ensure!(offset == logical_end, "Full coverage gap");
            }
            logical_end = offset + len;
            packed_end += len;
            for n in [ordinal, offset, p, len] {
                manifest.update(n.to_be_bytes());
            }
            ordinal += 1;
        }
        let count: u64 = self.db.query_row(
            "SELECT count(*) FROM v2_extents WHERE artifact_id=?1",
            [a.id.as_slice()],
            |r| r.get(0),
        )?;
        ensure!(
            count == ordinal && packed_end == packed,
            "extent cardinality/packed coverage"
        );
        if format == "Full" {
            ensure!(packed == eof && logical_end == eof, "Full EOF coverage");
        }
        ensure!(format != "Unknown", "unknown representation cannot be Ready");
        Ok((
            content.finalize().into(),
            manifest.finalize().into(),
            index,
            ordinal,
            packed,
        ))
    }
    pub(super) fn seal(
        &mut self,
        a: &mut Artifact,
        eof: u64,
        expected: Id,
        witness: &[u8],
        fault: &mut Fault,
    ) -> Result<()> {
        fault.point("before B2")?;
        let format: String = self.db.query_row(
            "SELECT format FROM v2_artifacts WHERE artifact_id=?1",
            [a.id.as_slice()],
            |r| r.get(0),
        )?;
        wire::fields(witness, "Witness", 3)?;
        let (hash, manifest, chunks, extents, packed) = self.scan(a, eof, &format, witness, None)?;
        ensure!(
            hash == expected && packed == a.bytes,
            "digest/expected byte count mismatch"
        );
        self.before_write(false)?;
        let tx = self.db.unchecked_transaction()?;
        let (phase,actual_chunks,actual_extents):(String,u64,u64)=tx.query_row("SELECT phase,(SELECT count(*) FROM v2_chunks WHERE artifact_id=?1),(SELECT count(*) FROM v2_extents WHERE artifact_id=?1) FROM v2_artifacts WHERE artifact_id=?1",[a.id.as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        ensure!(
            phase == "Allocated" && actual_chunks == chunks && actual_extents == extents,
            "manifest changed during verification"
        );
        tx.execute(
            "INSERT INTO v2_manifests VALUES(?1,1,?2,?3,?4,?5,?6,?7,NULL,?8)",
            params![
                a.id.as_slice(),
                eof,
                packed,
                chunks,
                extents,
                hash.as_slice(),
                manifest.as_slice(),
                witness
            ],
        )?;
        let (kind, body): (String, Vec<u8>) = tx.query_row(
            "SELECT kind,body FROM v2_owners WHERE owner_id=?1",
            [a.owner.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if kind == "Snapshot" {
            let mut fields = records::decode(&kind, &body)?;
            fields[3] = a.id.to_vec();
            fields[7] = hash.to_vec();
            fields[8] = eof.to_be_bytes().to_vec();
            fields[9] = witness.to_vec();
            tx.execute(
                "UPDATE v2_owners SET body=?2 WHERE owner_id=?1",
                params![a.owner.as_slice(), records::encode(&kind, &fields)?],
            )?;
        }
        tx.execute(
            "UPDATE v2_artifacts SET phase='Ready' WHERE artifact_id=?1 AND phase='Allocated'",
            [a.id.as_slice()],
        )?;
        tx.commit()?;
        a.hash = hash;
        fault.point("after B2")?;
        Ok(())
    }
    pub(super) fn verify(&mut self, a: &Artifact) -> Result<()> {
        let (format,eof,hash,manifest,witness,base):(String,u64,Vec<u8>,Vec<u8>,Vec<u8>,Option<Vec<u8>>)=self.db.query_row("SELECT a.format,m.logical_eof,m.content_digest,m.manifest_digest,m.source_witness,m.base_object FROM v2_artifacts a JOIN v2_manifests m USING(artifact_id) WHERE artifact_id=?1",[a.id.as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
        let (actual, md, _, _, _) = self.scan(a, eof, &format, &witness, base.as_deref())?;
        let (kind, body): (String, Vec<u8>) = self.db.query_row(
            "SELECT kind,body FROM v2_owners WHERE owner_id=?1",
            [a.owner.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if kind == "Snapshot" {
            let fields = records::decode(&kind, &body)?;
            ensure!(
                fields[3] == a.id && fields[7] == hash && fields[8] == eof.to_be_bytes() && fields[9] == witness,
                "Snapshot/manifest mismatch"
            );
        }
        ensure!(
            actual.as_slice() == hash && md.as_slice() == manifest,
            "corrupt manifest"
        );
        Ok(())
    }
    pub(super) fn adopt(
        &mut self,
        a: &Artifact,
        role: &str,
        qualified_fixture: bool,
        fault: &mut Fault,
    ) -> Result<Preservation> {
        fault.point("before B3")?;
        verify_pragmas(&self.db)?;
        self.verify(a)?;
        let format: String = self.db.query_row(
            "SELECT format FROM v2_artifacts WHERE artifact_id=?1",
            [a.id.as_slice()],
            |r| r.get(0),
        )?;
        ensure!(format == "Full" || role == "Recovery", "partial snapshot/upload");
        ensure!(qualified_fixture, "unqualified bootstrap: copy-only");
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO v2_refs VALUES(?1,?2,?3) ON CONFLICT DO NOTHING",
            params![a.owner.as_slice(), a.id.as_slice(), role],
        )?;
        tx.execute(
            "UPDATE v2_artifacts SET phase='Referenced' WHERE artifact_id=?1 AND phase='Ready'",
            [a.id.as_slice()],
        )?;
        tx.commit()?;
        fault.point("after B3")?;
        let revision = self.db.query_row(
            "SELECT revision FROM v2_owners WHERE owner_id=?1",
            [a.owner.as_slice()],
            |r| r.get(0),
        )?;
        Ok(Preservation {
            owner: a.owner,
            artifact: a.id,
            revision,
        })
    }
    pub(super) fn fake_sink(&mut self, a: &Artifact, p: &Preservation) -> Result<()> {
        let revision = self.db.query_row(
            "SELECT revision FROM v2_owners WHERE owner_id=?1",
            [p.owner.as_slice()],
            |r| r.get::<_, i64>(0),
        )?;
        let refs: i64 = self.db.query_row(
            "SELECT count(*) FROM v2_refs WHERE owner_id=?1 AND artifact_id=?2",
            params![a.owner.as_slice(), a.id.as_slice()],
            |r| r.get(0),
        )?;
        let manifest = self.verify(a).is_ok();
        ensure!(
            p.artifact == a.id
                && p.owner == a.owner
                && revision == p.revision
                && storage_proof(verify_pragmas(&self.db).is_ok(), manifest, refs > 0, true),
            "invalid B3 fixture proof"
        );
        Ok(())
    }
    pub(super) fn capture(
        &mut self,
        owner: Id,
        reader: &mut impl Read,
        size: u64,
        fault: &mut Fault,
    ) -> Result<Artifact> {
        self.capture_as(owner, "Capture", reader, size, fault)
    }
    pub(super) fn capture_as(
        &mut self,
        owner: Id,
        purpose: &str,
        reader: &mut impl Read,
        size: u64,
        fault: &mut Fault,
    ) -> Result<Artifact> {
        let mut a = self.allocate(owner, purpose, "Full", size, 0, fault)?;
        let phase: String = self.db.query_row(
            "SELECT phase FROM v2_artifacts WHERE artifact_id=?1",
            [a.id.as_slice()],
            |r| r.get(0),
        )?;
        if matches!(phase.as_str(), "Ready" | "Referenced") {
            self.verify(&a)?;
            return Ok(a);
        }
        let mut buffer = vec![0; CHUNK];
        self.max_buffer = self.max_buffer.max(buffer.capacity() + CHUNK);
        let mut hash = Sha256::new();
        let mut index: u64 = 0;
        let mut total = 0;
        loop {
            let mut used = 0;
            while used < CHUNK {
                let n = reader.read(&mut buffer[used..CHUNK])?;
                if n == 0 {
                    break;
                }
                used += n;
            }
            if used == 0 {
                break;
            }
            ensure!(total + used as u64 <= size, "source grew beyond reservation");
            fault.point("before B1 chunk")?;
            self.chunk(&a, index, &buffer[..used])?;
            fault.point("after B1 chunk")?;
            hash.update(&buffer[..used]);
            total += used as u64;
            index += 1;
        }
        ensure!(total == size, "source truncated");
        if size > 0 {
            self.extent(
                &a,
                0,
                Extent {
                    offset: 0,
                    packed: 0,
                    len: size,
                },
            )?;
        }
        self.seal(
            &mut a,
            size,
            hash.finalize().into(),
            &wire::record("Witness", &[b"fixture", b"held-handle", b"v1"])?,
            fault,
        )?;
        Ok(a)
    }
    pub(super) fn dispose(&self, a: &Artifact, proof: &[u8], fault: &mut Fault) -> Result<Id> {
        self.dispose_owner(a.owner, a, proof, fault)
    }
    pub(super) fn dispose_owner(&self, owner: Id, a: &Artifact, proof: &[u8], fault: &mut Fault) -> Result<Id> {
        self.dispose_named(owner, a, "UserDiscard", proof, fault)
    }
    pub(super) fn dispose_named(
        &self,
        owner: Id,
        a: &Artifact,
        kind: &str,
        proof: &[u8],
        fault: &mut Fault,
    ) -> Result<Id> {
        let arity = match kind {
            "UserDiscard" => 2,
            "ServerReceipt" => 5,
            "UserHandoff" => 6,
            "OwnershipTransfer" => 4,
            _ => bail!("disposition kind"),
        };
        let fields = wire::fields(proof, kind, arity)?;
        ensure!(fields[0] == a.id && fields[1] == a.hash, "disposition scope mismatch");
        if matches!(kind, "ServerReceipt" | "UserHandoff") {
            ensure!(fields[2] == a.bytes.to_be_bytes(), "receipt EOF mismatch");
        }
        if kind == "UserHandoff" {
            ensure!(
                fields[4] == b"QualifiedFixture" && fields[5] == b"Acknowledged",
                "copy-only handoff"
            );
        }
        if kind == "ServerReceipt" {
            ensure!(
                !fields[3].is_empty() && !fields[4].is_empty(),
                "missing exact server identity"
            );
        }
        metadata_admitted(&self.db, &self.path, true)?;
        fault.point("before B5 disposition")?;
        let tx = self.db.unchecked_transaction()?;
        let existing: Option<Vec<u8>> = tx
            .query_row(
                "SELECT disposition_id FROM v2_dispositions WHERE owner_id=?1 AND kind=?2 AND proof=?3",
                params![owner.as_slice(), kind, proof],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            return Ok(existing
                .try_into()
                .map_err(|_| anyhow::anyhow!("disposition identity"))?);
        }
        let relation: i64 = tx.query_row(
            "SELECT count(*) FROM v2_refs WHERE owner_id=?1 AND artifact_id=?2",
            params![owner.as_slice(), a.id.as_slice()],
            |r| r.get(0),
        )?;
        ensure!(relation > 0, "unrelated disposition owner");
        let stored: Option<(Vec<u8>, u64)> = tx
            .query_row(
                "SELECT content_digest,packed_bytes FROM v2_manifests WHERE artifact_id=?1",
                [a.id.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((hash, bytes)) = stored {
            ensure!(hash == a.hash && bytes == a.bytes, "disposition content mismatch");
        } else {
            ensure!(kind == "UserDiscard" && a.hash == [0; 32], "unverified receipt");
        }
        if kind == "OwnershipTransfer" {
            ensure!(fields[2] != owner, "self transfer");
            let refs:i64=tx.query_row("SELECT count(*) FROM v2_refs r JOIN v2_owners o USING(owner_id) WHERE r.owner_id=?1 AND r.artifact_id=?2 AND r.role=?3 AND o.terminal_disposition IS NULL",params![&fields[2],a.id.as_slice(),std::str::from_utf8(&fields[3])?],|r|r.get(0))?;
            ensure!(refs == 1, "missing surviving owner reference");
        }
        let disposition = id();
        tx.execute(
            "INSERT INTO v2_dispositions VALUES(?1,?2,?3,1,?4)",
            params![disposition.as_slice(), owner.as_slice(), kind, proof],
        )?;
        tx.execute(
            "DELETE FROM v2_refs WHERE owner_id=?1 AND artifact_id=?2",
            params![owner.as_slice(), a.id.as_slice()],
        )?;
        let remaining: i64 = tx.query_row(
            "SELECT count(*) FROM v2_refs WHERE owner_id=?1",
            [owner.as_slice()],
            |r| r.get(0),
        )?;
        if remaining == 0 {
            tx.execute(
                "UPDATE v2_owners SET terminal_disposition=?2 WHERE owner_id=?1",
                params![owner.as_slice(), disposition.as_slice()],
            )?;
        }
        let refs: i64 = tx.query_row(
            "SELECT count(*) FROM v2_refs WHERE artifact_id=?1",
            [a.id.as_slice()],
            |r| r.get(0),
        )?;
        if refs == 0 {
            tx.execute(
                "UPDATE v2_artifacts SET phase='RetirePending' WHERE artifact_id=?1",
                [a.id.as_slice()],
            )?;
            tx.execute(
                "INSERT INTO v2_gc VALUES(?1,?2,0,'Pending',NULL)",
                params![a.id.as_slice(), disposition.as_slice()],
            )?;
        }
        tx.commit()?;
        fault.point("after B5 disposition")?;
        Ok(disposition)
    }
    pub(super) fn gc(&mut self, a: &Artifact, fault: &mut Fault) -> Result<()> {
        loop {
            fault.point("before GC batch")?;
            self.before_write(true)?;
            let tx = self.db.unchecked_transaction()?;
            let refs: i64 = tx.query_row(
                "SELECT count(*) FROM v2_refs WHERE artifact_id=?1",
                [a.id.as_slice()],
                |r| r.get(0),
            )?;
            ensure!(refs == 0, "live refs block GC");
            let (cursor, phase): (u64, String) = tx.query_row(
                "SELECT next_chunk,phase FROM v2_gc JOIN v2_dispositions USING(disposition_id) WHERE artifact_id=?1",
                [a.id.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if phase == "Removed" {
                return Ok(());
            }
            let n = tx.execute(
                "DELETE FROM v2_chunks WHERE artifact_id=?1 AND chunk_index=?2",
                params![a.id.as_slice(), cursor],
            )?;
            if n == 0 {
                let removed=tx.execute("DELETE FROM v2_extents WHERE artifact_id=?1 AND ordinal IN (SELECT ordinal FROM v2_extents WHERE artifact_id=?1 LIMIT ?2)",params![a.id.as_slice(),batch_limit(4096)])?;
                self.max_rows = self.max_rows.max(removed);
                let remain: i64 = tx.query_row(
                    "SELECT count(*) FROM v2_extents WHERE artifact_id=?1",
                    [a.id.as_slice()],
                    |r| r.get(0),
                )?;
                if remain == 0 {
                    tx.execute("DELETE FROM v2_manifests WHERE artifact_id=?1", [a.id.as_slice()])?;
                    tx.execute(
                        "UPDATE v2_artifacts SET phase='Removed' WHERE artifact_id=?1",
                        [a.id.as_slice()],
                    )?;
                    tx.execute(
                        "UPDATE v2_gc SET phase='Removed' WHERE artifact_id=?1",
                        [a.id.as_slice()],
                    )?;
                    let cost:u64=tx.query_row("SELECT payload_bytes+duplicate_bytes+wal_bytes+terminal_bytes FROM v2_reservations WHERE reservation_id=?1 AND phase<>'Released'",[a.reservation.as_slice()],|r|r.get(0))?;
                    tx.execute(
                        "UPDATE v2_reservations SET phase='Released' WHERE reservation_id=?1",
                        [a.reservation.as_slice()],
                    )?;
                    tx.commit()?;
                    let _ = cost; // durable reservations, not a second persisted refcount
                    self.reconstruct_reservations()?;
                    fault.point("after GC terminal")?;
                    break;
                }
            } else {
                tx.execute(
                    "UPDATE v2_gc SET phase='Deleting',next_chunk=?2 WHERE artifact_id=?1",
                    params![a.id.as_slice(), cursor + 1],
                )?;
            }
            tx.commit()?;
            self.max_rows = self.max_rows.max(n);
            fault.point("after GC batch")?;
            checkpoint(&self.db, &self.path, false)?;
        }
        self.settle()?;
        Ok(())
    }
    pub(super) fn reclaim_step(&self) -> Result<()> {
        let mut statement = self.db.prepare("PRAGMA incremental_vacuum(256)")?;
        let mut rows = statement.query([])?;
        let mut count = 0;
        while rows.next()?.is_some() {
            count += 1;
            ensure!(count <= 256, "vacuum batch bound");
        }
        Ok(())
    }
    pub(super) fn settle(&self) -> Result<()> {
        loop {
            let free: u64 = self.db.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
            if free == 0 {
                break;
            }
            self.reclaim_step()?;
            checkpoint(&self.db, &self.path, true)?;
        }
        checkpoint(&self.db, &self.path, true)?;
        Ok(())
    }
}

/// Physically written reserve, never sparse allocation. Fixture callers own its directory.
pub(super) struct Reserve {
    pub(super) path: PathBuf,
    remaining: u64,
}
impl Reserve {
    pub(super) fn create(h: &Harness) -> Result<Self> {
        let path = h.path().join("reserve");
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        let bytes = vec![0xA5; CHUNK];
        for _ in 0..64 {
            f.write_all(&bytes)?;
        }
        f.sync_all()?;
        Ok(Self {
            path,
            remaining: 256 * MIB,
        })
    }
    pub(super) fn release_terminal(&mut self) -> Result<u64> {
        ensure!(self.remaining >= 16 * MIB, "emergency reserve exhausted");
        self.remaining -= 16 * MIB;
        let f = fs::OpenOptions::new().write(true).open(&self.path)?;
        f.set_len(self.remaining)?;
        f.sync_all()?;
        Ok(16 * MIB)
    }
    pub(super) fn refill(&mut self) -> Result<()> {
        let mut f = fs::OpenOptions::new().append(true).open(&self.path)?;
        let bytes = vec![0xA5; CHUNK];
        while self.remaining < 256 * MIB {
            f.write_all(&bytes)?;
            self.remaining += CHUNK as u64;
        }
        f.sync_all()?;
        Ok(())
    }
}

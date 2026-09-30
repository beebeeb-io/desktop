use super::*;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
#[cfg(windows)]
use zeroize::Zeroize;
use zeroize::Zeroizing;

fn aad(record: &Id, slot: &Id, class: &str) -> Result<Vec<u8>> {
    wire::record("ProtectedV1", &[class.as_bytes(), record, slot])
}
pub(super) fn encrypt_padded(
    key: &Id,
    record: &Id,
    slot: &Id,
    class: &str,
    body: &[u8],
) -> Result<([u8; 12], Vec<u8>)> {
    let mut nonce = [0; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let encoded = wire::record(
        "Fragment",
        &[
            class.as_bytes(),
            record,
            &0u64.to_be_bytes(),
            &1u64.to_be_bytes(),
            &digest(body),
            body,
        ],
    )?;
    ensure!(encoded.len() + 4 <= 65520, "protected body exceeds one fragment");
    let mut padded = Zeroizing::new(vec![0; 65520]);
    padded[..4].copy_from_slice(&(encoded.len() as u32).to_be_bytes());
    padded[4..4 + encoded.len()].copy_from_slice(&encoded);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| anyhow::anyhow!("key size"))?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &padded,
                aad: &aad(record, slot, class)?,
            },
        )
        .map_err(|_| anyhow::anyhow!("encrypt"))?;
    Ok((nonce, ciphertext))
}
pub(super) fn decrypt_padded(
    key: &Id,
    record: &Id,
    slot: &Id,
    class: &str,
    nonce: &[u8],
    bytes: &[u8],
) -> Result<Vec<u8>> {
    ensure!(nonce.len() == 12 && bytes.len() == 65536, "protected size");
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| anyhow::anyhow!("key size"))?;
    let plain = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: bytes,
                    aad: &aad(record, slot, class)?,
                },
            )
            .map_err(|_| anyhow::anyhow!("authentication failed"))?,
    );
    let n = u32::from_be_bytes(plain[..4].try_into()?) as usize;
    ensure!(
        n <= plain.len() - 4 && plain[4 + n..].iter().all(|x| *x == 0),
        "invalid padding"
    );
    let fields = wire::fields(&plain[4..4 + n], "Fragment", 6)?;
    ensure!(
        fields[0] == class.as_bytes()
            && fields[1] == record
            && fields[2] == 0u64.to_be_bytes()
            && fields[3] == 1u64.to_be_bytes()
            && fields[4] == digest(&fields[5]),
        "fragment identity"
    );
    Ok(fields[5].clone())
}

// Real current-user DPAPI on Windows. Unix is an explicitly test-only 0600 key
// fixture; no Linux OS-key-protection claim and no production constructor exists.
#[cfg(windows)]
fn wrap(input: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    #[repr(C)]
    struct Blob {
        len: u32,
        data: *mut u8,
    }
    #[link(name = "Crypt32")]
    unsafe extern "system" {
        fn CryptProtectData(
            input: *const Blob,
            description: *const u16,
            entropy: *const Blob,
            reserved: *mut std::ffi::c_void,
            prompt: *mut std::ffi::c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            input: *const Blob,
            description: *mut *mut u16,
            entropy: *const Blob,
            reserved: *mut std::ffi::c_void,
            prompt: *mut std::ffi::c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
    }
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn LocalFree(ptr: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }
    let input = Blob {
        len: u32::try_from(input.len())?,
        data: input.as_ptr().cast_mut(),
    };
    let mut out = Blob {
        len: 0,
        data: std::ptr::null_mut(),
    };
    unsafe {
        let ok = if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1,
                &mut out,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1,
                &mut out,
            )
        };
        ensure!(ok != 0, "DPAPI failed: {}", std::io::Error::last_os_error());
        let bytes = std::slice::from_raw_parts_mut(out.data, out.len as usize);
        let result = bytes.to_vec();
        bytes.zeroize();
        LocalFree(out.data.cast());
        Ok(result)
    }
}
#[cfg(not(windows))]
fn wrap(input: &[u8], _encrypt: bool) -> Result<Vec<u8>> {
    Ok(input.to_vec())
}

pub(super) struct Ledger {
    pub db: Connection,
    pub path: PathBuf,
    keys: PathBuf,
    run: Id,
    volume: Arc<Mutex<Budget>>,
}
pub(super) struct ReadCapability {
    account: Id,
    run: Id,
}
impl Ledger {
    pub(super) fn open(h: &Harness) -> Result<Self> {
        let mut admission = h.volume.lock().map_err(|_| anyhow::anyhow!("allocator poisoned"))?;
        let path = h.path().join("installation.db");
        let db = open_db(&path, include_str!("ledger.sql"))?;
        let keys = h.path().join("keyslots");
        fs::create_dir_all(&keys)?;
        // Recovery opens remain possible after an interrupted terminal release.
        // New payload/metadata admission still requires the full reserve refill.
        if file_len(&h.path().join("reserve")) >= 256 * MIB {
            grow_emergency(h.path(), None)?;
        }
        admission.emergency_required = required_emergency(h.path(), None)?;
        db.execute(
            "INSERT OR IGNORE INTO installation_format VALUES(1,1,?1,NULL)",
            [id().as_slice()],
        )?;
        let ledger = Self {
            db,
            path,
            keys,
            run: id(),
            volume: h.volume.clone(),
        };
        ledger.reconcile_keys()?;
        Ok(ledger)
    }
    fn key_path(&self, slot: &Id) -> PathBuf {
        self.keys.join(hex(slot))
    }
    fn slot_referenced(&self, slot: &Id) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT count(*) FROM protected_records WHERE key_slot=?1",
            [slot.as_slice()],
            |r| r.get::<_, u64>(0),
        )? > 0)
    }
    fn reconcile_keys(&self) -> Result<()> {
        for entry in fs::read_dir(&self.keys)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().context("invalid key filename")?;
            let slot_name = name.split('.').next().context("key slot")?;
            ensure!(
                slot_name.len() == 64 && slot_name.bytes().all(|b| b.is_ascii_hexdigit()),
                "unknown key-slot name"
            );
            let mut slot = [0; 32];
            for (i, byte) in slot.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&slot_name[2 * i..2 * i + 2], 16)?;
            }
            // Open and put hold the same volume allocator as the ledger writer.
            // Never repair a damaged slot still referenced by committed ciphertext.
            if !self.slot_referenced(&slot)? {
                fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }
    fn save_key(&self, slot: &Id, key: &Id, fault: &mut Fault) -> Result<()> {
        let wrapped = Zeroizing::new(wrap(key, true)?);
        let temporary = self.keys.join(format!("{}.tmp.{}", hex(slot), hex(&id())));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut f = options.open(&temporary)?;
        fault.point("key create")?;
        let half = wrapped.len() / 2;
        f.write_all(&wrapped[..half])?;
        fault.point("key write")?;
        f.write_all(&wrapped[half..])?;
        f.flush()?;
        fault.point("key flush")?;
        f.sync_all()?;
        drop(f);
        publish_key(&temporary, &self.key_path(slot))?;
        Ok(())
    }
    pub(super) fn key(&self, slot: &Id) -> Result<Zeroizing<Id>> {
        let wrapped = Zeroizing::new(fs::read(self.key_path(slot))?);
        let raw = Zeroizing::new(wrap(&wrapped, false)?);
        ensure!(raw.len() == 32, "key size");
        let mut key = Zeroizing::new([0; 32]);
        key.copy_from_slice(&raw);
        Ok(key)
    }
    pub(super) fn put(
        &self,
        record: Id,
        class: &str,
        body: &[u8],
        expires: Option<i64>,
        deny: Option<Id>,
        fault: &mut Fault,
    ) -> Result<Id> {
        let admission = self.volume.lock().map_err(|_| anyhow::anyhow!("allocator poisoned"))?;
        let terminal = class != "Activation";
        admission.space_admitted(self.path.parent().context("ledger volume")?, DIRTY_LIMIT, terminal)?;
        metadata_admitted(&self.db, &self.path, terminal)?;
        let existing: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT key_slot FROM protected_records WHERE record_id=?1",
                [record.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(slot) = existing {
            let slot: Id = slot.try_into().map_err(|_| anyhow::anyhow!("key identity"))?;
            ensure!(self.read(record)? == body, "record identity collision");
            return Ok(slot);
        }
        match class {
            "Envelope" => {
                wire::fields(body, "Envelope", 7)?;
                ensure!(
                    expires.is_some() && deny.is_some(),
                    "envelope needs deadline and denial"
                );
            }
            "Cleanup" | "Activation" => {
                wire::fields(body, class, 4)?;
            }
            _ => bail!("class"),
        }
        ensure!(
            file_len(&self.path) + file_len(&wal_path(&self.path)) + 2 * MIB <= 1024 * MIB,
            "ledger quota"
        );
        self.reconcile_keys()?;
        fault.point("before key publication")?;
        // A stable opaque slot survives an uncertain ledger commit. Its independently
        // random key is reused only for retries of this exact record, never another record.
        let slot = record;
        let key = if self.key_path(&slot).exists() {
            self.key(&slot)?
        } else {
            let key = Zeroizing::new(id());
            self.save_key(&slot, &key, fault)?;
            key
        };
        fault.point("after key publication")?;
        let (nonce, ciphertext) = encrypt_padded(&key, &record, &slot, class, body)?;
        verify_pragmas(&self.db)?;
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO purge_jobs(key_slot,due_at,phase) VALUES(?1,?2,'Retained')",
            params![slot.as_slice(), expires],
        )?;
        tx.execute("INSERT INTO protected_records(record_id,class,key_slot,part_index,expires_at,nonce,ciphertext) VALUES(?1,?2,?3,0,?4,?5,?6)",params![record.as_slice(),class,slot.as_slice(),expires,nonce.as_slice(),ciphertext])?;
        if let Some(token) = deny {
            tx.execute(
                "INSERT OR IGNORE INTO deny_tokens VALUES(?1,'NeverResubmit')",
                [token.as_slice()],
            )?;
        }
        fault.point("before ledger commit")?;
        tx.commit()?;
        if file_len(&wal_path(&self.path)) >= 64 * MIB {
            checkpoint(&self.db, &self.path, false)?;
        }
        fault.point("after ledger commit")?;
        Ok(slot)
    }
    pub(super) fn read(&self, record: Id) -> Result<Vec<u8>> {
        let (class,slot,nonce,bytes,phase):(String,Vec<u8>,Vec<u8>,Vec<u8>,String)=self.db.query_row("SELECT r.class,r.key_slot,r.nonce,r.ciphertext,j.phase FROM protected_records r JOIN purge_jobs j USING(key_slot) WHERE record_id=?1",[record.as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        ensure!(phase == "Retained", "decryptor fenced");
        let slot: Id = slot.try_into().map_err(|_| anyhow::anyhow!("key slot identity"))?;
        decrypt_padded(&*self.key(&slot)?, &record, &slot, &class, &nonce, &bytes)
    }
    pub(super) fn authenticate_fixture(&self, account: Id) -> ReadCapability {
        ReadCapability { account, run: self.run }
    }
    pub(super) fn lookup(&self, cap: &ReadCapability, caller_account: Id, record: Id) -> Result<Vec<u8>> {
        ensure!(
            cap.run == self.run && cap.account == caller_account,
            "caller account is not authorization"
        );
        let body = self.read(record)?;
        let fields = wire::fields(&body, "Envelope", 7)?;
        ensure!(fields[3] == cap.account, "cross-account read");
        Ok(body)
    }
    pub(super) fn deadline(&self, record: Id, now: i64) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
        tx.execute("UPDATE purge_jobs SET due_at=?2 WHERE key_slot=(SELECT key_slot FROM protected_records WHERE record_id=?1)",params![record.as_slice(),now])?;
        tx.execute(
            "UPDATE protected_records SET expires_at=?2 WHERE record_id=?1",
            params![record.as_slice(), now],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub(super) fn purge(&self, now: i64, fault: &mut Fault) -> Result<usize> {
        let admission = self.volume.lock().map_err(|_| anyhow::anyhow!("allocator poisoned"))?;
        metadata_admitted(&self.db, &self.path, true)?;
        admission.space_admitted(self.path.parent().context("ledger volume")?, DIRTY_LIMIT, true)?;
        let mut count = 0;
        loop {
            let slot:Option<Vec<u8>>=self.db.query_row("SELECT key_slot FROM purge_jobs WHERE (due_at IS NOT NULL AND due_at<=?1) OR phase<>'Retained' ORDER BY key_slot LIMIT 1",[now],|r|r.get(0)).optional()?;
            let Some(slot) = slot else { break };
            let slot: Id = slot.try_into().map_err(|_| anyhow::anyhow!("slot identity"))?;
            fault.point("before PurgeIntent")?;
            self.db.execute(
                "UPDATE purge_jobs SET phase='PurgeIntent' WHERE key_slot=?1 AND phase='Retained'",
                [slot.as_slice()],
            )?;
            fault.point("after PurgeIntent")?;
            // Sole-owner, synchronous decryptor: &mut key buffers never escape read().
            match fs::remove_file(self.key_path(&slot)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            ensure!(!self.key_path(&slot).exists(), "key remains reachable");
            fault.point("after key removal")?;
            self.db.execute(
                "UPDATE purge_jobs SET phase='KeyRemoved' WHERE key_slot=?1",
                [slot.as_slice()],
            )?;
            fault.point("after KeyRemoved")?;
            let tx = self.db.unchecked_transaction()?;
            tx.execute("DELETE FROM protected_records WHERE key_slot=?1", [slot.as_slice()])?;
            tx.execute(
                "UPDATE purge_jobs SET phase='RowsRemoved' WHERE key_slot=?1",
                [slot.as_slice()],
            )?;
            tx.commit()?;
            fault.point("after RowsRemoved")?;
            checkpoint(&self.db, &self.path, true)?;
            fault.point("after purge checkpoint")?;
            self.db.execute(
                "UPDATE purge_jobs SET phase='Purged' WHERE key_slot=?1",
                [slot.as_slice()],
            )?;
            fault.point("after Purged")?;
            checkpoint(&self.db, &self.path, true)?;
            self.db
                .execute("DELETE FROM purge_jobs WHERE key_slot=?1", [slot.as_slice()])?;
            count += 1;
            fault.point("after job retirement")?;
        }
        Ok(count)
    }
    pub(super) fn deny(&self, token: Id) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT count(*) FROM deny_tokens WHERE token=?1",
            [token.as_slice()],
            |r| r.get::<_, i64>(0),
        )? > 0)
    }
    pub(super) fn backup_denials(&self) -> Result<Vec<u8>> {
        // Streaming callers can consume pages; this fixture helper rejects >256 rows.
        let mut stmt = self
            .db
            .prepare("SELECT token,kind FROM deny_tokens ORDER BY token LIMIT 257")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for (n, row) in rows.enumerate() {
            ensure!(n < 256, "backup batch bound");
            let (token, kind) = row?;
            out.extend(wire::record("Denial", &[&token, kind.as_bytes()])?);
        }
        Ok(out)
    }
    pub(super) fn keys_count(&self) -> Result<usize> {
        Ok(fs::read_dir(&self.keys)?.count())
    }
}

pub(super) fn transfer(
    store: &Store,
    ledger: &Ledger,
    owner: Id,
    transfer: Id,
    token: Id,
    record: Id,
    body: &[u8],
    expires: i64,
    fault: &mut Fault,
) -> Result<()> {
    let (kind, owner_body, terminal): (String, Vec<u8>, Option<Vec<u8>>) = store.db.query_row(
        "SELECT kind,body,terminal_disposition FROM v2_owners WHERE owner_id=?1",
        [owner.as_slice()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    ensure!(kind == "Upload" && terminal.is_some(), "transfer needs disposed Upload");
    let fields = records::decode(&kind, &owner_body)?;
    ensure!(fields[4] == token, "immutable deny token mismatch");
    let envelope_fields = wire::fields(body, "Envelope", 7)?;
    ensure!(envelope_fields[3] == fields[0], "envelope account mismatch");
    let disposed: i64 = store.db.query_row(
        "SELECT count(*) FROM v2_owners WHERE owner_id=?1 AND kind='Snapshot' AND terminal_disposition IS NOT NULL",
        [&fields[3]],
        |r| r.get(0),
    )?;
    let live: i64 = store.db.query_row(
        "SELECT count(*) FROM v2_refs WHERE owner_id=?1",
        [owner.as_slice()],
        |r| r.get(0),
    )?;
    ensure!(disposed == 1 && live == 0, "local byte disposition incomplete");
    metadata_admitted(&store.db, &store.path, true)?;
    fault.point("before transfer intent")?;
    store.db.execute(
        "INSERT OR IGNORE INTO v2_transfers VALUES(?1,?2,?3,'Intent',?4,NULL)",
        params![
            transfer.as_slice(),
            owner.as_slice(),
            token.as_slice(),
            record.as_slice()
        ],
    )?;
    let (o, t, r): (Vec<u8>, Vec<u8>, Vec<u8>) = store.db.query_row(
        "SELECT owner_id,deny_token,ledger_record_id FROM v2_transfers WHERE transfer_id=?1",
        [transfer.as_slice()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    ensure!(o == owner && t == token && r == record, "transfer collision");
    fault.point("after transfer intent")?;
    ledger.put(record, "Envelope", body, Some(expires), Some(token), fault)?;
    ensure!(
        ledger.deny(token)? && ledger.read(record)? == body,
        "ledger acknowledgement mismatch"
    );
    fault.point("before transfer ack")?;
    store.db.execute(
        "UPDATE v2_transfers SET phase='Acknowledged',ledger_record_id=?2,ledger_receipt=?3 WHERE transfer_id=?1",
        params![transfer.as_slice(), record.as_slice(), digest(body).as_slice()],
    )?;
    fault.point("after transfer ack")?;
    Ok(())
}

pub(super) fn publish_key(temporary: &Path, destination: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "Kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
        }
        let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = destination.as_os_str().encode_wide().chain(Some(0)).collect();
        // WRITE_THROUGH, deliberately without REPLACE_EXISTING or COPY_ALLOWED.
        ensure!(
            unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 8) } != 0,
            "key publication failed: {}",
            std::io::Error::last_os_error()
        );
    }
    #[cfg(unix)]
    {
        fs::hard_link(temporary, destination)?; // atomic no-clobber publication
        fs::remove_file(temporary)?;
        fs::File::open(destination.parent().context("key parent")?)?.sync_all()?;
    }
    Ok(())
}

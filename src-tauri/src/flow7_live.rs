//! QA-only headless harness (flow-7, 2026-09-25). NOT for merge.
//! Drives the real desktop EngineBridge (the daemon's queue + transfer loop +
//! sync_tick + hydrate) against a LOCAL beebeeb-api. Gated on FLOW7_API.
#![allow(dead_code)]

use crate::api_client::ApiClient;
use crate::engine_bridge::{sync_tick, EngineBridge, FinderWriteItemKind, FinderWriteOutcome, FinderWriteTarget};
use crate::state_db::StateDb;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}
fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}
fn sha(b: &[u8]) -> String {
    hex_lower(&Sha256::digest(b))
}
fn hex_lower(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

struct Report {
    lines: Vec<String>,
    fails: usize,
}
impl Report {
    fn rec(&mut self, step: &str, ok: bool, ev: String) {
        let l = format!("STEP {step}: {} :: {ev}", if ok { "PASS" } else { "FAIL" });
        eprintln!("{l}");
        if !ok {
            self.fails += 1;
        }
        self.lines.push(l);
    }
}

async fn register_and_login(base: &str, email: &str, pw: &[u8]) -> Result<String, String> {
    use beebeeb_core::opaque_protocol;
    let c = reqwest::Client::new();
    let rs = opaque_protocol::client_registration_start(pw).map_err(|e| e.to_string())?;
    let r = c
        .post(format!("{base}/api/v1/opaque/register-start"))
        .json(&serde_json::json!({"email": email, "client_message": b64().encode(&rs.message)}))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let st = r.status();
    let body: serde_json::Value = r.json().await.unwrap_or_default();
    if !st.is_success() {
        return Err(format!("register-start {st}: {body}"));
    }
    let sm = b64().decode(body["server_message"].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
    let up = opaque_protocol::client_registration_finish(&rs.state, pw, &sm).map_err(|e| e.to_string())?;
    let x_priv = [0x05u8; 32];
    let x_pub = beebeeb_core::opaque::derive_x25519_public(&x_priv);
    let r = c
        .post(format!("{base}/api/v1/opaque/register-finish"))
        .json(&serde_json::json!({
            "email": email,
            "client_message": b64().encode(&up),
            "recovery_check": b64().encode([0xEEu8; 32]),
            "x25519_public_key": b64().encode(x_pub),
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let st = r.status();
    let body: serde_json::Value = r.json().await.unwrap_or_default();
    if !st.is_success() {
        return Err(format!("register-finish {st}: {body}"));
    }
    // Now log in exactly like desktop_login does (OPAQUE login-start/finish).
    let ls = opaque_protocol::client_login_start(pw).map_err(|e| e.to_string())?;
    let r = c
        .post(format!("{base}/api/v1/opaque/login-start"))
        .header("X-Beebeeb-Client", "desktop")
        .json(&serde_json::json!({"email": email, "client_message": b64().encode(&ls.message)}))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let st = r.status();
    let sb: serde_json::Value = r.json().await.unwrap_or_default();
    if !st.is_success() {
        return Err(format!("login-start {st}: {sb}"));
    }
    let smsg = b64().decode(sb["server_message"].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
    let ksf = sb["ksf_version"].as_u64().unwrap_or(0) as u32;
    let lf = opaque_protocol::client_login_finish(&ls.state, pw, &smsg, ksf).map_err(|e| e.to_string())?;
    let r = c
        .post(format!("{base}/api/v1/opaque/login-finish"))
        .header("X-Beebeeb-Client", "desktop")
        .json(&serde_json::json!({"email": email, "client_message": b64().encode(&lf.message), "server_state": sb["server_state"]}))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let st = r.status();
    let fb: serde_json::Value = r.json().await.unwrap_or_default();
    if !st.is_success() {
        return Err(format!("login-finish {st}: {fb}"));
    }
    fb["session_token"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("no session_token: {fb}"))
}

struct Dev {
    bridge: EngineBridge,
    root: PathBuf,
    _dir: tempfile::TempDir,
}
fn device(base: &str, token: &str, mk: [u8; 32]) -> Dev {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&root).unwrap();
    let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
    let api = Arc::new(ApiClient::new(base.to_string(), token.to_string(), mk));
    Dev {
        bridge: EngineBridge::new(db, api),
        root,
        _dir: dir,
    }
}

async fn drain(d: &Dev, max_rounds: usize) -> (usize, usize, usize, Vec<String>) {
    let (mut c, mut r, mut p) = (0, 0, 0);
    let mut errs = vec![];
    for _ in 0..max_rounds {
        match d.bridge.process_due_operations(&d.root, now() + 3600).await {
            Ok(o) => {
                c += o.completed_op_ids.len();
                r += o.retried_op_ids.len();
                p += o.paused_op_ids.len();
                if o.completed_op_ids.is_empty() && o.retried_op_ids.is_empty() {
                    break;
                }
            }
            Err(e) => {
                errs.push(e.to_string());
                break;
            }
        }
    }
    // record any lingering ops' last_error
    if let Ok(ops) = d.bridge.db().list_due_operations(now() + 10_000_000) {
        for o in ops {
            errs.push(format!("left:{:?} attempts={} err={:?}", o.kind, o.attempts, o.last_error));
        }
    }
    (c, r, p, errs)
}

fn write_src(dir: &Path, name: &str, data: &[u8]) -> String {
    let p = dir.join(format!("src-{name}"));
    std::fs::write(&p, data).unwrap();
    p.to_string_lossy().into_owned()
}

fn vid(d: &Dev, id: &str) -> Option<String> {
    let c = d.bridge.db().get_file_contract_state(id).ok()??;
    Some(format!("{}:0:0", c.current_version))
}

fn sid(d: &Dev, path: &str, fallback: &str) -> String {
    d.bridge.db().get_file_by_path(path).ok().flatten().map(|e| e.file_id).unwrap_or_else(|| fallback.to_string())
}

async fn fetch(d: &Dev, id: &str) -> Vec<u8> {
    let dir = d.root.parent().unwrap().join("fetch");
    let _ = std::fs::create_dir_all(&dir);
    let dest = dir.join(uuid::Uuid::new_v4().to_string());
    match d.bridge.hydrate_file(id, &dest, &[dir.as_path()]).await {
        Ok(()) => std::fs::read(&dest).unwrap_or_default(),
        Err(e) => format!("<fetch error: {e}>").into_bytes(),
    }
}

async fn server_names(d: &Dev, parent: Option<&str>) -> Vec<(String, String)> {
    let mk = beebeeb_core::kdf::MasterKey::from_bytes(*d.bridge.api().master_key());
    let files = d.bridge.api().list_files(parent).await.unwrap_or_default();
    files
        .iter()
        .map(|f| {
            let id = f["id"].as_str().unwrap_or("").to_string();
            let n = f["name_encrypted"].as_str().unwrap_or("");
            let name = beebeeb_core::encrypt::decrypt_name(&mk, &id, n).unwrap_or_else(|_| "<undecryptable>".into());
            (id, name)
        })
        .collect()
}

#[test]
#[ignore]
fn flow7_desktop_engine_against_local_server() {
    let Ok(base) = std::env::var("FLOW7_API") else {
        eprintln!("FLOW7_API unset; skipping");
        return;
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let mut rep = Report { lines: vec![], fails: 0 };
        let email = format!("flow7-desktop-{}@beebeeb.io", uuid::Uuid::new_v4().simple());
        let token = match register_and_login(&base, &email, b"flow7-correct-horse-battery").await {
            Ok(t) => {
                rep.rec("1 signup+opaque-login", true, format!("{email} token_len={}", t.len()));
                t
            }
            Err(e) => {
                rep.rec("1 signup+opaque-login", false, e);
                panic!("cannot continue");
            }
        };
        let mut mk = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut mk);
        let a = device(&base, &token, mk);
        let b = device(&base, &token, mk);
        let scratch = tempfile::tempdir().unwrap();

        // 2. small upload from A
        let small = b"hello from flow7 desktop harness\n".repeat(10);
        let out = a
            .bridge
            .queue_finder_create(FinderWriteTarget {
                file_id: None,
                parent_id: None,
                filename: "notes.txt".into(),
                rel_path: None,
                kind: FinderWriteItemKind::File,
                contents_path: Some(write_src(scratch.path(), "notes.txt", &small)),
                content_type: Some("text/plain".into()),
                base_version_identifier: None,
            })
            .unwrap();
        let FinderWriteOutcome::Queued { file_id: Some(small_id), .. } = out else { panic!("not queued") };
        let (c, r, p, errs) = drain(&a, 5).await;
        let client_small = small_id.clone();
        let small_id = sid(&a, "notes.txt", &small_id);
        eprintln!("small: client_id={client_small} server_id={small_id}");
        let st = a.bridge.db().get_file(&small_id).unwrap().map(|e| format!("{:?}", e.status));
        let names = server_names(&a, None).await;
        rep.rec(
            "2 upload small (A)",
            c >= 1 && errs.is_empty() && names.iter().any(|(i, n)| i == &small_id && n == "notes.txt"),
            format!("completed={c} retried={r} paused={p} errs={errs:?} local_status={st:?} server={names:?}"),
        );

        // 3. B sees it via sync_tick and downloads it (round trip)
        let conf = sync_tick(&b.bridge, &b.root).await;
        let brow = b.bridge.db().get_file(&small_id).unwrap();
        rep.rec(
            "3a sync_tick on B lists new file",
            brow.as_ref().map(|e| e.path == "notes.txt").unwrap_or(false),
            format!("tick={:?} row={:?}", conf.as_ref().map(|v| v.len()), brow.as_ref().map(|e| (&e.path, &e.status, e.size_bytes))),
        );
        let dest = b.root.join("notes.txt");
        let h = b.bridge.hydrate_file(&small_id, &dest, &[b.root.as_path()]).await;
        let got = std::fs::read(&dest).unwrap_or_default();
        rep.rec(
            "3b hydrate on B byte-identical",
            h.is_ok() && got == small,
            format!("hydrate={:?} sha_src={} sha_got={} len={}", h.err().map(|e| e.to_string()), sha(&small), sha(&got), got.len()),
        );

        // 4. large file (multi-chunk) round trip
        let size: usize = std::env::var("FLOW7_LARGE_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(64) * 1024 * 1024 + 12345;
        let mut big = vec![0u8; size];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut big);
        let src = write_src(scratch.path(), "big.bin", &big);
        let out = a
            .bridge
            .queue_finder_create(FinderWriteTarget {
                file_id: None,
                parent_id: None,
                filename: "big.bin".into(),
                rel_path: None,
                kind: FinderWriteItemKind::File,
                contents_path: Some(src.clone()),
                content_type: None,
                base_version_identifier: None,
            })
            .unwrap();
        let FinderWriteOutcome::Queued { file_id: Some(big_id), .. } = out else { panic!() };
        let t0 = Instant::now();
        let (c, r, p, errs) = drain(&a, 5).await;
        let up_s = t0.elapsed().as_secs_f64();
        let big_id = sid(&a, "big.bin", &big_id);
        let meta = a.bridge.api().get_file(&big_id).await.ok();
        rep.rec(
            "4a upload large (A)",
            c >= 1 && errs.is_empty(),
            format!(
                "bytes={size} secs={up_s:.1} completed={c} retried={r} paused={p} errs={errs:?} server size={:?} chunks={:?}",
                meta.as_ref().map(|m| m["size_bytes"].clone()),
                meta.as_ref().map(|m| m["chunk_count"].clone())
            ),
        );
        let _ = sync_tick(&b.bridge, &b.root).await;
        let dest = b.root.join("big.bin");
        let t0 = Instant::now();
        let h = b.bridge.hydrate_file(&big_id, &dest, &[b.root.as_path()]).await;
        let got = std::fs::read(&dest).unwrap_or_default();
        rep.rec(
            "4b hydrate large on B byte-identical",
            h.is_ok() && sha(&got) == sha(&big),
            format!("secs={:.1} err={:?} sha_src={} sha_got={} len={}", t0.elapsed().as_secs_f64(), h.err().map(|e| e.to_string()), sha(&big), sha(&got), got.len()),
        );
        drop(big);

        // 5. rename on A → B sees new name
        let v = vid(&a, &small_id);
        let _ = a.bridge.queue_finder_modify(FinderWriteTarget {
            file_id: Some(small_id.clone()),
            parent_id: None,
            filename: "notes-renamed.txt".into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: None,
            content_type: Some("text/plain".into()),
            base_version_identifier: v.clone(),
        });
        let (c, _r, _p, errs) = drain(&a, 5).await;
        let names = server_names(&a, None).await;
        let _ = sync_tick(&b.bridge, &b.root).await;
        let brow = b.bridge.db().get_file(&small_id).unwrap();
        rep.rec(
            "5 rename (A→server→B)",
            names.iter().any(|(i, n)| i == &small_id && n == "notes-renamed.txt")
                && brow.as_ref().map(|e| e.path == "notes-renamed.txt").unwrap_or(false),
            format!("base={v:?} completed={c} errs={errs:?} server={names:?} B.path={:?}", brow.map(|e| e.path)),
        );

        // 6. create folder + move file into it
        let out = a
            .bridge
            .queue_finder_create(FinderWriteTarget {
                file_id: None,
                parent_id: None,
                filename: "Docs".into(),
                rel_path: None,
                kind: FinderWriteItemKind::Folder,
                contents_path: None,
                content_type: None,
                base_version_identifier: None,
            })
            .unwrap();
        let FinderWriteOutcome::Queued { file_id: Some(folder_id), .. } = out else { panic!() };
        let (c1, _, _, e1) = drain(&a, 5).await;
        let folder_id = sid(&a, "Docs", &folder_id);
        let v = vid(&a, &small_id);
        let _ = a.bridge.queue_finder_modify(FinderWriteTarget {
            file_id: Some(small_id.clone()),
            parent_id: Some(folder_id.clone()),
            filename: "notes-renamed.txt".into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: None,
            content_type: Some("text/plain".into()),
            base_version_identifier: v,
        });
        let (c2, _, _, e2) = drain(&a, 5).await;
        let in_folder = server_names(&a, Some(&folder_id)).await;
        let _ = sync_tick(&b.bridge, &b.root).await;
        let brow = b.bridge.db().get_file(&small_id).unwrap();
        if let Ok(ops) = b.bridge.api().sync_ops(0).await {
            for o in &ops.ops {
                eprintln!("DIAG6 op seq={} type={} payload={}", o.seq_id, o.op_type, o.payload.to_string().chars().take(200).collect::<String>());
            }
        }
        eprintln!("DIAG6 B rows: {:?}", b.bridge.db().list_files().unwrap_or_default().iter().map(|e| (e.file_id.clone(), e.path.clone(), format!("{:?}", e.item_kind))).collect::<Vec<_>>());
        eprintln!("DIAG6 B cursor: {:?}", b.bridge.db().get_sync_cursor());
        rep.rec(
            "6 folder create + move into folder",
            in_folder.iter().any(|(i, _)| i == &small_id) && brow.as_ref().map(|e| e.path == "Docs/notes-renamed.txt").unwrap_or(false),
            format!("folder_c={c1} e1={e1:?} move_c={c2} e2={e2:?} server_in_folder={in_folder:?} B.path={:?}", brow.map(|e| e.path)),
        );

        // 7. conflict: A and B both edit same base version of big? use a fresh small file
        let base_bytes = b"version one\n".to_vec();
        let out = a
            .bridge
            .queue_finder_create(FinderWriteTarget {
                file_id: None,
                parent_id: None,
                filename: "shared.txt".into(),
                rel_path: None,
                kind: FinderWriteItemKind::File,
                contents_path: Some(write_src(scratch.path(), "shared1", &base_bytes)),
                content_type: Some("text/plain".into()),
                base_version_identifier: None,
            })
            .unwrap();
        let FinderWriteOutcome::Queued { file_id: Some(sh_id), .. } = out else { panic!() };
        drain(&a, 5).await;
        let sh_id = sid(&a, "shared.txt", &sh_id);
        let _ = sync_tick(&b.bridge, &b.root).await;
        let _ = b.bridge.hydrate_file(&sh_id, &b.root.join("shared.txt"), &[b.root.as_path()]).await;
        let va = vid(&a, &sh_id);
        let vb = vid(&b, &sh_id);
        let a_edit = b"edited on A\n".to_vec();
        let b_edit = b"edited on B\n".to_vec();
        let _ = a.bridge.queue_finder_modify(FinderWriteTarget {
            file_id: Some(sh_id.clone()),
            parent_id: None,
            filename: "shared.txt".into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: Some(write_src(scratch.path(), "sharedA", &a_edit)),
            content_type: Some("text/plain".into()),
            base_version_identifier: va.clone(),
        });
        let (ca, _, pa, ea) = drain(&a, 5).await;
        let _ = b.bridge.queue_finder_modify(FinderWriteTarget {
            file_id: Some(sh_id.clone()),
            parent_id: None,
            filename: "shared.txt".into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: Some(write_src(scratch.path(), "sharedB", &b_edit)),
            content_type: Some("text/plain".into()),
            base_version_identifier: vb.clone(),
        });
        let (cb, rb, pb, eb) = drain(&b, 5).await;
        let conflicts = sync_tick(&b.bridge, &b.root).await;
        let _ = sync_tick(&a.bridge, &a.root).await;
        // what does the server hold now? decrypt current content + all names
        let root_names = server_names(&a, None).await;
        let versions = a.bridge.api().list_versions(&sh_id).await.ok();
        let cur = fetch(&a, &sh_id).await;
        let mut contents = vec![String::from_utf8_lossy(&cur).to_string()];
        for (id, n) in &root_names {
            if n.starts_with("shared") && id != &sh_id {
                let x = fetch(&a, id).await;
                contents.push(format!("{n}={}", String::from_utf8_lossy(&x)));
            }
        }
        let both = contents.iter().any(|s| s.contains("edited on A")) && contents.iter().any(|s| s.contains("edited on B"));
        let vfeed = b.bridge.version_conflict_feed().map(|v| v.len()).ok();
        eprintln!("DIAG7 feed: {:?}", b.bridge.version_conflict_feed().map(|v| v.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>()));
        let (c3, r3, p3, e3) = drain(&b, 6).await;
        eprintln!("DIAG7 after 6 more B rounds: c={c3} r={r3} p={p3} e={e3:?}");
        eprintln!("DIAG7 review ops: {:?}", b.bridge.db().list_review_operations().map(|v| v.iter().map(|o| (format!("{:?}", o.kind), o.attempts, o.last_error.clone())).collect::<Vec<_>>()));
        eprintln!("DIAG7 B row: {:?}", b.bridge.db().get_file(&sh_id).ok().flatten().map(|e| (e.path, e.status)));
        let after = server_names(&a, None).await;
        eprintln!("DIAG7 server names after: {after:?}");
        rep.rec(
            "7 concurrent edit: no silent loss (both edits survive somewhere)",
            both,
            format!(
                "va={va:?} vb={vb:?} A:c={ca} p={pa} e={ea:?} | B:c={cb} r={rb} p={pb} e={eb:?} | tick_conflicts={:?} version_conflict_feed={vfeed:?} | server_contents={contents:?} | versions={}",
                conflicts.as_ref().map(|v| v.iter().map(|c| c.file_name.clone()).collect::<Vec<_>>()),
                versions.map(|v| v.to_string().chars().take(600).collect::<String>()).unwrap_or_default()
            ),
        );

        // 8. delete on A → server trash, B drops row
        let v = vid(&a, &small_id);
        let _ = a.bridge.queue_finder_delete(&small_id, v.clone());
        let (c, _, p, errs) = drain(&a, 5).await;
        let trashed = a.bridge.api().list_trashed_files_all().await.unwrap_or_default();
        let _ = sync_tick(&b.bridge, &b.root).await;
        let brow = b.bridge.db().get_file(&small_id).unwrap();
        rep.rec(
            "8 delete → trash (A) and removal on B",
            trashed.iter().any(|f| f["id"].as_str() == Some(small_id.as_str())) && brow.is_none(),
            format!("base={v:?} completed={c} paused={p} errs={errs:?} trashed_n={} B.row={:?}", trashed.len(), brow.map(|e| (e.path, e.status))),
        );

        // 9. resume: cancel an in-flight large upload mid-way, then re-run
        let size2 = 48 * 1024 * 1024 + 7;
        let mut big2 = vec![0u8; size2];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut big2);
        let src2 = write_src(scratch.path(), "resume.bin", &big2);
        let out = a
            .bridge
            .queue_finder_create(FinderWriteTarget {
                file_id: None,
                parent_id: None,
                filename: "resume.bin".into(),
                rel_path: None,
                kind: FinderWriteItemKind::File,
                contents_path: Some(src2),
                content_type: None,
                base_version_identifier: None,
            })
            .unwrap();
        let FinderWriteOutcome::Queued { file_id: Some(rid), .. } = out else { panic!() };
        let cut = tokio::time::timeout(
            std::time::Duration::from_millis(std::env::var("FLOW7_CUT_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500)),
            a.bridge.process_due_operations(&a.root, now() + 3600),
        )
        .await;
        let was_cut = cut.is_err();
        let (c, r, p, errs) = drain(&a, 8).await;
        let rid = sid(&a, "resume.bin", &rid);
        let names = server_names(&a, None).await;
        let dup = names.iter().filter(|(_, n)| n == "resume.bin").count();
        let raw = a.bridge.api().list_files(None).await.unwrap_or_default();
        for f in &raw {
            let id = f["id"].as_str().unwrap_or("");
            if names.iter().any(|(i, n)| i == id && n == "resume.bin") {
                eprintln!("DIAG9 raw={}", f);
                let body = fetch(&a, id).await;
                eprintln!("resume copy id={id} local_row={} size={} chunks={:?} status={:?} fetch_len={} fetch_head={:?}", id == rid, f["size_bytes"], f["chunk_count"], f.get("status").or(f.get("upload_status")), body.len(), String::from_utf8_lossy(&body[..body.len().min(120)]));
            }
        }
        let got = fetch(&a, &rid).await;
        rep.rec(
            "9 interrupted upload resumes to a correct single file",
            was_cut && dup == 1 && sha(&got) == sha(&big2),
            format!("cut_mid_flight={was_cut} then completed={c} retried={r} paused={p} errs={errs:?} copies_on_server={dup} sha_ok={}", sha(&got) == sha(&big2)),
        );

        // 10. empty (0-byte) file
        let out = a.bridge.queue_finder_create(FinderWriteTarget {
            file_id: None, parent_id: None, filename: "empty.txt".into(), rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: Some(write_src(scratch.path(), "empty.txt", b"")),
            content_type: Some("text/plain".into()), base_version_identifier: None,
        });
        let (c, r, p, errs) = drain(&a, 3).await;
        let names = server_names(&a, None).await;
        let row = a.bridge.db().get_file_by_path("empty.txt").ok().flatten().map(|e| format!("{:?}", e.status));
        rep.rec("10 empty 0-byte file syncs", names.iter().any(|(_, n)| n == "empty.txt"),
            format!("queued={:?} completed={c} retried={r} paused={p} errs={errs:?} local_row_status={row:?}", out.as_ref().map(|_| "ok").map_err(|e| e.to_string())));

        eprintln!("==== FLOW7 SUMMARY: {} steps, {} fail ====", rep.lines.len(), rep.fails);
        for l in &rep.lines {
            eprintln!("{l}");
        }
    });
}

async fn mk_file(d: &Dev, scratch: &Path, name: &str, data: &[u8]) -> String {
    let out = d
        .bridge
        .queue_finder_create(FinderWriteTarget {
            file_id: None,
            parent_id: None,
            filename: name.into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: Some(write_src(scratch, name, data)),
            content_type: Some("text/plain".into()),
            base_version_identifier: None,
        })
        .unwrap();
    let FinderWriteOutcome::Queued { file_id: Some(id), .. } = out else { panic!() };
    drain(d, 3).await;
    sid(d, name, &id)
}

#[test]
#[ignore]
fn flow7_move_matrix() {
    let Ok(base) = std::env::var("FLOW7_API") else { return };
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let email = format!("flow7-move-{}@beebeeb.io", uuid::Uuid::new_v4().simple());
        let token = register_and_login(&base, &email, b"flow7-correct-horse-battery").await.unwrap();
        let mut mk = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut mk);
        let a = device(&base, &token, mk);
        let b = device(&base, &token, mk);
        let scratch = tempfile::tempdir().unwrap();
        // folder
        let out = a.bridge.queue_finder_create(FinderWriteTarget { file_id: None, parent_id: None, filename: "Dest".into(), rel_path: None, kind: FinderWriteItemKind::Folder, contents_path: None, content_type: None, base_version_identifier: None }).unwrap();
        let FinderWriteOutcome::Queued { file_id: Some(fid), .. } = out else { panic!() };
        drain(&a, 3).await;
        let fid = sid(&a, "Dest", &fid);
        let f_local_web = mk_file(&a, scratch.path(), "local-web.txt", b"x1").await;
        let f_cloud_web = mk_file(&a, scratch.path(), "cloud-web.txt", b"x2").await;
        let f_local_desk = mk_file(&a, scratch.path(), "local-desk.txt", b"x3").await;
        let f_cloud_desk = mk_file(&a, scratch.path(), "cloud-desk.txt", b"x4").await;
        let _ = sync_tick(&b.bridge, &b.root).await; // bootstrap
        let _ = sync_tick(&b.bridge, &b.root).await;
        for id in [&f_local_web, &f_local_desk] {
            let p = b.bridge.db().get_file(id).unwrap().unwrap().path;
            b.bridge.hydrate_file(id, &b.root.join(&p), &[b.root.as_path()]).await.unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        // web-style: parent only
        for id in [&f_local_web, &f_cloud_web] {
            a.bridge.api().update_metadata(id, None, Some(&fid)).await.unwrap();
        }
        // desktop-style: MoveFile op (name + parent)
        for (id, name) in [(&f_local_desk, "local-desk.txt"), (&f_cloud_desk, "cloud-desk.txt")] {
            let _ = a.bridge.queue_finder_modify(FinderWriteTarget { file_id: Some(id.clone()), parent_id: Some(fid.clone()), filename: name.into(), rel_path: None, kind: FinderWriteItemKind::File, contents_path: None, content_type: Some("text/plain".into()), base_version_identifier: vid(&a, id) });
        }
        drain(&a, 3).await;
        let _ = sync_tick(&b.bridge, &b.root).await;
        for (label, id) in [("web-move/B-hydrated", &f_local_web), ("web-move/B-cloudonly", &f_cloud_web), ("desktop-move/B-hydrated", &f_local_desk), ("desktop-move/B-cloudonly", &f_cloud_desk)] {
            let e = b.bridge.db().get_file(id).unwrap().unwrap();
            let ok = e.path.starts_with("Dest/");
            eprintln!("MOVE {label}: {} path={} status={:?}", if ok { "PASS" } else { "FAIL" }, e.path, e.status);
        }
        // does a later tick / restart heal it?
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let _ = sync_tick(&b.bridge, &b.root).await;
        let c = device(&base, &token, mk);
        let _ = sync_tick(&c.bridge, &c.root).await;
        for (label, id) in [("web-move/B-hydrated", &f_local_web), ("desktop-move/B-hydrated", &f_local_desk)] {
            let e = b.bridge.db().get_file(id).unwrap().unwrap();
            let fresh = c.bridge.db().get_file(id).unwrap().map(|e| e.path);
            eprintln!("MOVE-AFTER-2ND-TICK {label}: B.path={} fresh-device.path={fresh:?}", e.path);
        }
    });
}

#[test]
#[ignore]
fn flow7_move_matrix_late_folder() {
    let Ok(base) = std::env::var("FLOW7_API") else { return };
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let email = format!("flow7-move-{}@beebeeb.io", uuid::Uuid::new_v4().simple());
        let token = register_and_login(&base, &email, b"flow7-correct-horse-battery").await.unwrap();
        let mut mk = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut mk);
        let a = device(&base, &token, mk);
        let b = device(&base, &token, mk);
        let scratch = tempfile::tempdir().unwrap();
        // folder
        let f_local_web = mk_file(&a, scratch.path(), "local-web.txt", b"x1").await;
        let f_cloud_web = mk_file(&a, scratch.path(), "cloud-web.txt", b"x2").await;
        let f_local_desk = mk_file(&a, scratch.path(), "local-desk.txt", b"x3").await;
        let f_cloud_desk = mk_file(&a, scratch.path(), "cloud-desk.txt", b"x4").await;
        let _ = sync_tick(&b.bridge, &b.root).await; // bootstrap
        let _ = sync_tick(&b.bridge, &b.root).await;
        for id in [&f_local_web, &f_local_desk] {
            let p = b.bridge.db().get_file(id).unwrap().unwrap().path;
            b.bridge.hydrate_file(id, &b.root.join(&p), &[b.root.as_path()]).await.unwrap();
        }
        let out = a.bridge.queue_finder_create(FinderWriteTarget { file_id: None, parent_id: None, filename: "Dest".into(), rel_path: None, kind: FinderWriteItemKind::Folder, contents_path: None, content_type: None, base_version_identifier: None }).unwrap();
        let FinderWriteOutcome::Queued { file_id: Some(fid), .. } = out else { panic!() };
        drain(&a, 3).await;
        let fid = sid(&a, "Dest", &fid);
        if std::env::var("FLOW7_SLEEP").is_ok() { tokio::time::sleep(std::time::Duration::from_millis(1100)).await; }
        // web-style: parent only
        for id in [&f_local_web, &f_cloud_web] {
            a.bridge.api().update_metadata(id, None, Some(&fid)).await.unwrap();
        }
        // desktop-style: MoveFile op (name + parent)
        for (id, name) in [(&f_local_desk, "local-desk.txt"), (&f_cloud_desk, "cloud-desk.txt")] {
            let _ = a.bridge.queue_finder_modify(FinderWriteTarget { file_id: Some(id.clone()), parent_id: Some(fid.clone()), filename: name.into(), rel_path: None, kind: FinderWriteItemKind::File, contents_path: None, content_type: Some("text/plain".into()), base_version_identifier: vid(&a, id) });
        }
        drain(&a, 3).await;
        let _ = sync_tick(&b.bridge, &b.root).await;
        for (label, id) in [("web-move/B-hydrated", &f_local_web), ("web-move/B-cloudonly", &f_cloud_web), ("desktop-move/B-hydrated", &f_local_desk), ("desktop-move/B-cloudonly", &f_cloud_desk)] {
            let e = b.bridge.db().get_file(id).unwrap().unwrap();
            let ok = e.path.starts_with("Dest/");
            eprintln!("MOVE {label}: {} path={} status={:?}", if ok { "PASS" } else { "FAIL" }, e.path, e.status);
        }
        // does a later tick / restart heal it?
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let _ = sync_tick(&b.bridge, &b.root).await;
        let c = device(&base, &token, mk);
        let _ = sync_tick(&c.bridge, &c.root).await;
        for (label, id) in [("web-move/B-hydrated", &f_local_web), ("desktop-move/B-hydrated", &f_local_desk)] {
            let e = b.bridge.db().get_file(id).unwrap().unwrap();
            let fresh = c.bridge.db().get_file(id).unwrap().map(|e| e.path);
            eprintln!("MOVE-AFTER-2ND-TICK {label}: B.path={} fresh-device.path={fresh:?}", e.path);
        }
    });
}

#[test]
#[ignore]
fn flow7_double_rename_between_ticks() {
    let Ok(base) = std::env::var("FLOW7_API") else { return };
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let email = format!("flow7-dbl-{}@beebeeb.io", uuid::Uuid::new_v4().simple());
        let token = register_and_login(&base, &email, b"flow7-correct-horse-battery").await.unwrap();
        let mut mk = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut mk);
        let a = device(&base, &token, mk);
        let b = device(&base, &token, mk);
        let scratch = tempfile::tempdir().unwrap();
        let hyd = mk_file(&a, scratch.path(), "draft.txt", b"d1").await;
        let cld = mk_file(&a, scratch.path(), "cloud.txt", b"d2").await;
        let _ = sync_tick(&b.bridge, &b.root).await;
        let _ = sync_tick(&b.bridge, &b.root).await;
        b.bridge.hydrate_file(&hyd, &b.root.join("draft.txt"), &[b.root.as_path()]).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        for (id, n1, n2) in [(&hyd, "draft-v2.txt", "final.txt"), (&cld, "cloud-v2.txt", "cloud-final.txt")] {
            for n in [n1, n2] {
                let _ = a.bridge.queue_finder_modify(FinderWriteTarget { file_id: Some(id.clone()), parent_id: None, filename: n.into(), rel_path: None, kind: FinderWriteItemKind::File, contents_path: None, content_type: Some("text/plain".into()), base_version_identifier: vid(&a, id) });
                drain(&a, 3).await;
            }
        }
        let server = server_names(&a, None).await;
        let _ = sync_tick(&b.bridge, &b.root).await;
        for (label, id, want) in [("B-hydrated(Local)", &hyd, "final.txt"), ("B-cloudonly", &cld, "cloud-final.txt")] {
            let e = b.bridge.db().get_file(id).unwrap().unwrap();
            eprintln!("DBL {label}: {} B.path={} want={want} status={:?}", if e.path == want { "PASS" } else { "FAIL" }, e.path, e.status);
        }
        eprintln!("DBL server={server:?}");
        for i in 0..3 {
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            let _ = sync_tick(&b.bridge, &b.root).await;
            let e = b.bridge.db().get_file(&hyd).unwrap().unwrap();
            eprintln!("DBL later tick {i}: B-hydrated path={}", e.path);
        }
    });
}

#[test]
#[ignore]
fn flow7_rename_then_edit_between_ticks() {
    let Ok(base) = std::env::var("FLOW7_API") else { return };
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let email = format!("flow7-re-{}@beebeeb.io", uuid::Uuid::new_v4().simple());
        let token = register_and_login(&base, &email, b"flow7-correct-horse-battery").await.unwrap();
        let mut mk = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut mk);
        let a = device(&base, &token, mk);
        let b = device(&base, &token, mk);
        let scratch = tempfile::tempdir().unwrap();
        let id = mk_file(&a, scratch.path(), "report.txt", b"v1 short").await;
        let _ = sync_tick(&b.bridge, &b.root).await;
        let _ = sync_tick(&b.bridge, &b.root).await;
        b.bridge.hydrate_file(&id, &b.root.join("report.txt"), &[b.root.as_path()]).await.unwrap();
        let before = b.bridge.db().get_file_contract_state(&id).unwrap().unwrap().current_version;
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        let no_rename = std::env::var("FLOW7_NO_RENAME").is_ok();
        let fname = if no_rename { "report.txt" } else { "report-final.txt" };
        if !no_rename {
        let _ = a.bridge.queue_finder_modify(FinderWriteTarget { file_id: Some(id.clone()), parent_id: None, filename: "report-final.txt".into(), rel_path: None, kind: FinderWriteItemKind::File, contents_path: None, content_type: Some("text/plain".into()), base_version_identifier: vid(&a, &id) });
        drain(&a, 3).await;
        }
        let _ = a.bridge.queue_finder_modify(FinderWriteTarget { file_id: Some(id.clone()), parent_id: None, filename: fname.into(), rel_path: None, kind: FinderWriteItemKind::File, contents_path: Some(write_src(scratch.path(), "v2", b"v2 is a much longer body than v1")), content_type: Some("text/plain".into()), base_version_identifier: vid(&a, &id) });
        let (c, _, _, e) = drain(&a, 3).await;
        let sv = a.bridge.api().get_file(&id).await.unwrap();
        if std::env::var("FLOW7_ONLY_UPDATE").is_ok() {
            // control: advance B's cursor past every op except the final file_update
            let ops = b.bridge.api().sync_ops(0).await.unwrap();
            let upd = ops.ops.iter().filter(|o| o.op_type == "file_update" && o.payload["id"].as_str() == Some(id.as_str())).map(|o| o.seq_id).max().unwrap();
            b.bridge.db().set_sync_cursor(upd - 1).unwrap();
            eprintln!("RE control: cursor set to {} so only file_update {upd} applies", upd - 1);
        }
        let _ = sync_tick(&b.bridge, &b.root).await;
        let e2 = b.bridge.db().get_file(&id).unwrap().unwrap();
        let cst = b.bridge.db().get_file_contract_state(&id).unwrap().unwrap();
        eprintln!("RE A edit completed={c} errs={e:?} server size={} version={}", sv["size_bytes"], sv["version_number"]);
        eprintln!("RE B after tick: path={} status={:?} size={} contract.current_version={} (before {before}) local_base={}", e2.path, e2.status, e2.size_bytes, cst.current_version, cst.local_base_version);
        let ok = e2.path == fname && e2.size_bytes == sv["size_bytes"].as_i64().unwrap_or(-1);
        if let Ok(ops) = b.bridge.api().sync_ops(0).await { for o in &ops.ops { if o.payload["id"].as_str()==Some(id.as_str()) { eprintln!("RE op {} {}", o.seq_id, o.op_type); } } }
        eprintln!("RE verdict: {}", if ok { "PASS" } else { "FAIL" });
    });
}

#[test]
#[ignore]
fn flow7_wrong_phrase_unlock_poisons_uploads() {
    let Ok(base) = std::env::var("FLOW7_API") else { return };
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let (p_right, _) = beebeeb_core::recovery::generate_recovery_phrase().unwrap();
        let (p_wrong, _) = beebeeb_core::recovery::generate_recovery_phrase().unwrap();
        // what desktop_unlock_with_recovery_phrase does: normalize (12 words) + recover_from_phrase, nothing else
        let mk_right = beebeeb_core::recovery::recover_from_phrase(&p_right).unwrap().to_bytes();
        let mk_wrong = beebeeb_core::recovery::recover_from_phrase(&p_wrong);
        eprintln!("WRONG-PHRASE: recover_from_phrase(other valid phrase) ok={} (desktop accepts it: no recovery_check comparison)", mk_wrong.is_ok());
        let mk_wrong = mk_wrong.unwrap().to_bytes();
        let email = format!("flow7-wp-{}@beebeeb.io", uuid::Uuid::new_v4().simple());
        let token = register_and_login(&base, &email, b"flow7-correct-horse-battery").await.unwrap();
        let right = device(&base, &token, mk_right);
        let wrong = device(&base, &token, mk_wrong);
        let scratch = tempfile::tempdir().unwrap();
        let id = mk_file(&wrong, scratch.path(), "tax-return.pdf", b"precious bytes").await;
        let names = server_names(&right, None).await;
        let body = fetch(&right, &id).await;
        eprintln!("WRONG-PHRASE: server accepted upload from wrong-key device id={id}; right-key device sees names={names:?}; content fetch={:?}", String::from_utf8_lossy(&body));
    });
}

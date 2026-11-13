//! THROWAWAY (task 2477 editor spike): WordCraft's own commands, undo and redo, with the font store compiled out.
//!   probe <host:port> | roundtrip <file> <outdir> | keystroke <file> <n> | frames
use serde_json::{Value, json};
use std::time::Instant;
use wordcraft_doc::StoryRef;
use wordcraft_engine::Session;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
fn res<T, E: std::fmt::Debug>(r: &Result<T, E>) -> String {
    match r {
        Ok(_) => "ok".into(),
        Err(e) => format!("ERR {e:?}").chars().take(160).collect(),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("probe") => {
            use std::net::ToSocketAddrs;
            let ok = a[2]
                .to_socket_addrs()
                .ok()
                .and_then(|mut x| x.next())
                .map(|x| std::net::TcpStream::connect_timeout(&x, std::time::Duration::from_secs(8)).is_ok())
                .unwrap_or(false);
            println!("probe {}: {}", a[2], if ok { "CONNECTED" } else { "NO CONNECTION" });
            std::process::exit(if ok { 0 } else { 1 });
        }
        Some("frames") => frames(),
        Some("keystroke") => keystroke(&a[2], a[3].parse().unwrap()),
        Some("roundtrip") => roundtrip(&a[2], &a[3]),
        _ => std::process::exit(2),
    }
}

/// One typed character through the engine's own command (which also records the undo step).
fn keystroke(path: &str, n: usize) {
    let doc = wordcraft_docx::read(&std::fs::read(path).expect("read")).expect("parse");
    let blocks = doc.body.len();
    let mut s = Session::new(doc);
    let mut v = Vec::new();
    for _ in 0..n {
        let t = Instant::now();
        s.run("text.insert", &json!({"text": "x", "raw": true}))
            .expect("insert");
        v.push(ms(t));
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let t = Instant::now();
    let undone = s.undo();
    let undo_ms = ms(t);
    println!(
        "{{\"engine\":\"wordcraft-engine (patched)\",\"test\":\"text.insert of one character, {n} times\",\"body_blocks\":{blocks},\"ms_p50\":{:.3},\"ms_p95\":{:.3},\"ms_max\":{:.3},\"undo_depth\":{},\"one_undo_ms\":{undo_ms:.3},\"undone\":{undone}}}",
        v[n / 2],
        v[n * 95 / 100],
        v[n - 1],
        s.undo_depth()
    );
}

/// DW-1550 (Table, Image is not tried: it needs a file, Link, Page break), DW-1551 (Heading 1 to 3, Bold),
/// DW-1558 (a new, blank document).
fn frames() {
    let mut s = Session::new(wordcraft_doc::Document::default());
    let mut out = Vec::new();
    let mut step = |s: &mut Session, id: &str, p: Value| {
        let r = s.run(id, &p);
        out.push(format!("{id} {p} = {}", res(&r)));
    };
    step(&mut s, "file.new", json!({}));
    step(&mut s, "text.insert", json!({"text": "Heading text", "raw": true}));
    step(&mut s, "para.heading1", json!({}));
    step(&mut s, "para.heading2", json!({}));
    step(&mut s, "para.heading3", json!({}));
    step(&mut s, "format.bold", json!({}));
    step(&mut s, "insert.table", json!({"rows": 2, "cols": 3}));
    step(
        &mut s,
        "insert.link",
        json!({"url": "https://example.com", "text": "a link"}),
    );
    step(&mut s, "insert.pageBreak", json!({}));
    let bytes = wordcraft_docx::write(&s.doc);
    let back = bytes.as_ref().ok().and_then(|b| wordcraft_docx::read(b).ok());
    println!(
        "{{\"engine\":\"wordcraft-engine (patched)\",\"test\":\"frames\",\"results\":{out:?},\"saves\":{:?},\"reopened_blocks\":{:?},\"reopened_text\":{:?}}}",
        res(&bytes),
        back.as_ref().map(|d| d.body.len()),
        back.as_ref().map(|d| d.plain_text(StoryRef::Body))
    );
}

fn roundtrip(path: &str, outdir: &str) {
    let name = std::path::Path::new(path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let (stem, ext) = name
        .rsplit_once('.')
        .map(|x| (x.0.to_string(), x.1.to_string()))
        .unwrap_or((name.clone(), "docx".into()));
    let doc = wordcraft_docx::read(&std::fs::read(path).expect("read")).expect("parse");
    let before = doc.plain_text(StoryRef::Body);
    let mut s = Session::new(doc);
    let cmds = [
        ("text.insert", json!({"text": "BBEDIT-ENGINE ", "raw": true})),
        ("select.text", json!({"text": "BBEDIT-ENGINE", "occurrence": 1})),
        ("format.bold", json!({})),
    ];
    let mut ok = Vec::new();
    for (id, p) in &cmds {
        ok.push(format!("{id}={}", res(&s.run(id, p))));
    }
    let after = s.doc.plain_text(StoryRef::Body);
    let depth = s.undo_depth();
    let mut undone = 0;
    while s.undo() {
        undone += 1;
    }
    let undo_restores = s.doc.plain_text(StoryRef::Body) == before;
    let mut redone = 0;
    while s.redo() {
        redone += 1;
    }
    let redo_restores = s.doc.plain_text(StoryRef::Body) == after;
    let bytes = wordcraft_docx::write(&s.doc).expect("write");
    std::fs::write(format!("{outdir}/{stem}.engine-edited.{ext}"), &bytes).unwrap();
    let back = wordcraft_docx::read(&bytes).expect("reopen");
    println!(
        "{{\"engine\":\"wordcraft-engine (patched, no font store)\",\"file\":\"{name}\",\"commands\":{:?},\"text_changed\":{},\"contains_marker\":{},\"undo_depth\":{depth},\"undo_steps\":{undone},\"undo_restores\":{undo_restores},\"redo_steps\":{redone},\"redo_restores\":{redo_restores},\"reopened_same_text\":{}}}",
        ok,
        after != before,
        after.contains("BBEDIT-ENGINE"),
        back.plain_text(StoryRef::Body) == after
    );
}

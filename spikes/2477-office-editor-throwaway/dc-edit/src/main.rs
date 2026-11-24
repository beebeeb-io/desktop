//! THROWAWAY (task 2477 editor spike): DeckCraft through deckcraft-engine's public commands.
//!   probe <host:port> | roundtrip <file> <outdir> | frames
use deckcraft_engine::Session;
use serde_json::{Value, json};
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
fn slides(s: &Session) -> usize {
    s.doc().map(|d| d.doc.slides.len()).unwrap_or(0)
}
fn undo_len(s: &Session) -> usize {
    s.doc().map(|d| d.history.undo.len()).unwrap_or(0)
}
fn res(r: &Result<Value, impl std::fmt::Debug>) -> String {
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
        Some("roundtrip") => roundtrip(&a[2], &a[3]),
        _ => std::process::exit(2),
    }
}

/// DW-1560 (the five slide layouts) and DW-1558 (a new, blank presentation).
fn frames() {
    let mut s = Session::new();
    let mut out = Vec::new();
    out.push(format!("file.new={}", res(&s.execute("file.new", &json!({})))));
    out.push(format!(
        "file.templates={}",
        match s.execute("file.templates", &json!({})) {
            Ok(v) => v.to_string().chars().take(300).collect(),
            Err(e) => format!("ERR {e:?}"),
        }
    ));
    let names: Vec<String> = s
        .doc()
        .map(|d| {
            d.doc
                .masters
                .iter()
                .flat_map(|m| m.layouts.iter().map(|l| l.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    for wanted in ["Title Slide", "Title and Content", "Two Content", "Title Only", "Blank"] {
        let r = s.execute("slide.new", &json!({"layout": wanted}));
        let r2 = s.execute("slide.layout", &json!({"layout": wanted}));
        out.push(format!(
            "layout {wanted:?}: slide.new={} slide.layout={}",
            res(&r),
            res(&r2)
        ));
    }
    let n = slides(&s);
    let bytes = deckcraft_engine::cmd::file::save_bytes(&s.doc().unwrap().doc, "pptx").map(|b| b.len());
    println!(
        "{{\"engine\":\"deckcraft\",\"test\":\"frames\",\"layouts_in_a_new_deck\":{names:?},\"results\":{out:?},\"slides\":{n},\"saves\":{:?}}}",
        bytes.map_err(|e| format!("{e:?}"))
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
        .unwrap_or((name.clone(), "pptx".into()));
    let mut s = Session::new();
    let t = Instant::now();
    if let Err(e) = s.execute("file.open", &json!({"path": path})) {
        println!(
            "{{\"engine\":\"deckcraft\",\"file\":\"{name}\",\"error\":{:?}}}",
            format!("{e:?}")
        );
        return;
    }
    let load_ms = ms(t);
    let n0 = slides(&s);
    let had_marker = format!("{:?}", s.doc().unwrap().doc.slides).contains("SLIDE 1");
    let t = Instant::now();
    let resaved = deckcraft_engine::cmd::file::save_bytes(&s.doc().unwrap().doc, "pptx");
    let save_ms = ms(t);
    if let Ok(b) = &resaved {
        std::fs::write(format!("{outdir}/{stem}.resaved.{ext}"), b).unwrap();
    }
    let u0 = undo_len(&s);
    let mut ok = Vec::new();
    let mut step = |s: &mut Session, id: &str, p: Value| {
        let t = Instant::now();
        let r = s.execute(id, &p);
        ok.push(format!("{id}={} ({:.1} ms)", res(&r), ms(t)));
    };
    step(&mut s, "text.replace", json!({"text": "SLIDE 1", "with": "BBEDIT 1"}));
    step(&mut s, "slide.new", json!({"title": "BBEDIT NEW SLIDE"}));
    step(&mut s, "slide.go", json!({"index": 0}));
    step(&mut s, "slide.duplicate", json!({}));
    let n1 = slides(&s);
    let steps = undo_len(&s) - u0;
    let after = format!("{:?}", s.doc().unwrap().doc.slides);
    let t = Instant::now();
    let (png, w, h) = deckcraft_engine::cmd::file::render_png_with(&s.doc().unwrap().doc, 0, 1.0, false, false);
    let render_ms = ms(t);
    std::fs::write(format!("{outdir}/{stem}.edited.slide1.png"), &png).unwrap();
    for _ in 0..steps {
        let _ = s.execute("edit.undo", &json!({}));
    }
    let n_undo = slides(&s);
    let undo_restores = n_undo == n0 && format!("{:?}", s.doc().unwrap().doc.slides).contains("SLIDE 1") == had_marker;
    for _ in 0..steps {
        let _ = s.execute("edit.redo", &json!({}));
    }
    let redo_restores = format!("{:?}", s.doc().unwrap().doc.slides) == after;
    let edited = deckcraft_engine::cmd::file::save_bytes(&s.doc().unwrap().doc, "pptx");
    let (mut n_back, mut back) = (0, String::new());
    if let Ok(b) = &edited {
        std::fs::write(format!("{outdir}/{stem}.edited.{ext}"), b).unwrap();
        if let Ok(p) = deckcraft_engine::cmd::file::open_presentation("edited.pptx", b) {
            n_back = p.slides.len();
            back = format!("{:?}", p.slides);
        }
    }
    println!(
        "{{\"engine\":\"deckcraft\",\"file\":\"{name}\",\"load_ms\":{load_ms:.1},\"save_ms\":{save_ms:.1},\"resave_ok\":{},\"commands\":{:?},\"had_SLIDE_1\":{had_marker},\"slides_before\":{n0},\"slides_after\":{n1},\"undo_steps\":{steps},\"undo_restores\":{undo_restores},\"redo_restores\":{redo_restores},\"save_ok\":{},\"slides_reopened\":{n_back},\"reopened_has_replaced_text\":{},\"reopened_has_new_slide_title\":{},\"reopened_still_has_old_text\":{},\"render_ms\":{render_ms:.1},\"render_px\":\"{w}x{h}\"}}",
        resaved.is_ok(),
        ok,
        edited.is_ok(),
        back.contains("BBEDIT 1"),
        back.contains("BBEDIT NEW SLIDE"),
        back.contains("SLIDE 1")
    );
}

//! THROWAWAY (task 2477 editor spike): WordCraft edited WITHOUT wordcraft-engine.
//!   probe <host:port>                 exit 0 = a TCP connection worked, 1 = it did not
//!   roundtrip <file> <outdir>         load, save unchanged, apply 3 edits, save, reopen, verify
//!   keystroke <file> <n>              per keystroke: edit, incremental re-layout, caret, hit-test, strip render
//!   latency <file> <n>                the same through a helper PROCESS (pipes): round trip per keystroke
//!   helper <file>                     (started by `latency`)
use std::io::{Read, Write};
use std::time::Instant;
use wordcraft_doc::{CharProps, Document, Pos, StoryRef};
use wordcraft_layout::{LayoutCache, LayoutOptions};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn load(path: &str) -> Document {
    let bytes = std::fs::read(path).expect("read file");
    if path.to_ascii_lowercase().ends_with(".doc") {
        wordcraft_docbin::read(&bytes).expect("read .doc")
    } else {
        wordcraft_docx::read(&bytes).expect("read .docx")
    }
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() as f64 - 1.0) * p).round() as usize]
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
        Some("roundtrip") => roundtrip(&a[2], &a[3]),
        Some("keystroke") => keystroke(&a[2], a[3].parse().unwrap()),
        Some("latency") => latency(&a[2], a[3].parse().unwrap()),
        Some("helper") => helper(&a[2]),
        _ => std::process::exit(2),
    }
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
    let ext = if ext == "doc" { "docx".to_string() } else { ext };
    let t = Instant::now();
    let doc = load(path);
    let load_ms = ms(t);
    let before = doc.plain_text(StoryRef::Body);
    let pages_before = wordcraft_layout::layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default())
        .pages
        .len();
    // A: save with no edit at all: what does the writer change by itself?
    let t = Instant::now();
    let resaved = wordcraft_docx::write(&doc).expect("write unchanged");
    let save_ms = ms(t);
    std::fs::write(format!("{outdir}/{stem}.resaved.{ext}"), &resaved).unwrap();

    // B: three edits through the public API of wordcraft-doc
    let mut d = doc.clone();
    let mut edits = Vec::new();
    let snap0 = d.clone(); // "undo" below the engine is ours: a snapshot
    let e1 = d.insert_text(&Pos::body(0, 0), "BBEDIT-INSERT ", &CharProps::default());
    edits.push(("insert_text", e1.is_ok()));
    let e2 = d.format_range(&Pos::body(0, 0), &Pos::body(0, 13), &|p: &mut CharProps| {
        p.bold = Some(true)
    });
    edits.push(("format_range bold", e2.is_ok()));
    let e3 = d.delete_range(&Pos::body(1, 0), &Pos::body(1, 5));
    edits.push(("delete_range", e3.is_ok()));
    let after = d.plain_text(StoryRef::Body);
    // expected text, computed independently of the engine
    let mut paras: Vec<String> = before.split('\n').map(String::from).collect();
    paras[0] = format!("BBEDIT-INSERT {}", paras[0]);
    if e3.is_ok() && paras.len() > 1 && paras[1].len() >= 5 && paras[1].is_char_boundary(5) {
        paras[1] = paras[1][5..].to_string();
    }
    let text_as_expected = after == paras.join("\n");
    // undo / redo by snapshot
    let redo = d.clone();
    let undone = snap0.plain_text(StoryRef::Body) == before;
    let redone = redo.plain_text(StoryRef::Body) == after;

    let edited = wordcraft_docx::write(&d).expect("write edited");
    std::fs::write(format!("{outdir}/{stem}.edited.{ext}"), &edited).unwrap();
    let t = Instant::now();
    let back = wordcraft_docx::read(&edited).expect("reopen edited");
    let reopen_ms = ms(t);
    let reopened_same_text = back.plain_text(StoryRef::Body) == after;
    let pages_after = wordcraft_layout::layout(&back, &mut LayoutCache::new(), &LayoutOptions::default())
        .pages
        .len();
    let back0 = wordcraft_docx::read(&resaved).expect("reopen resaved");
    println!(
        "{{\"engine\":\"wordcraft\",\"file\":\"{name}\",\"load_ms\":{load_ms:.1},\"save_ms\":{save_ms:.1},\"reopen_ms\":{reopen_ms:.1},\"edits_ok\":{},\"edits\":{:?},\"text_as_expected\":{text_as_expected},\"reopened_same_text\":{reopened_same_text},\"resaved_same_text\":{},\"undo_restores\":{undone},\"redo_restores\":{redone},\"pages_before\":{pages_before},\"pages_after\":{pages_after},\"chars_before\":{},\"chars_after\":{}}}",
        edits.iter().filter(|e| e.1).count(),
        edits,
        back0.plain_text(StoryRef::Body) == before,
        before.len(),
        after.len()
    );
}

struct Ed {
    doc: Document,
    cache: LayoutCache,
    opts: LayoutOptions,
    pos: Pos,
}

impl Ed {
    fn open(path: &str) -> (Ed, f64, usize) {
        let doc = load(path);
        let blocks = doc.plain_text(StoryRef::Body).split('\n').count();
        let mut ed = Ed {
            doc,
            cache: LayoutCache::new(),
            opts: LayoutOptions::default(),
            pos: Pos::body(blocks / 2, 0),
        };
        let t = Instant::now();
        let l = wordcraft_layout::layout(&ed.doc, &mut ed.cache, &ed.opts);
        (ed, ms(t), l.pages.len())
    }
    /// One keystroke: edit, incremental layout, caret, then a strip of the page around the caret as pixels.
    fn key(&mut self) -> (f64, f64, f64, wordcraft_render::Rendered, bool, usize) {
        let t = Instant::now();
        self.pos = self
            .doc
            .insert_text(&self.pos, "x", &CharProps::default())
            .expect("insert");
        let edit = ms(t);
        let t = Instant::now();
        let l = wordcraft_layout::layout(&self.doc, &mut self.cache, &self.opts);
        let lay = ms(t);
        let c = l.caret(&self.pos).expect("caret");
        let hit = l.hit(c.page, c.x + 0.5, c.top + c.height / 2.0, StoryRef::Body);
        let hit_ok = hit
            .as_ref()
            .map(|p| p.path == self.pos.path && (p.off as i64 - self.pos.off as i64).abs() <= 1)
            .unwrap_or(false);
        let sel = l
            .selection_rects(
                &self.doc,
                &Pos::new(StoryRef::Body, self.pos.path.clone(), 0),
                &self.pos,
                c.page,
            )
            .len();
        let page = &l.pages[c.page];
        let t = Instant::now();
        let area = wordcraft_geom::Rect::new(0.0, (c.top - 40.0).max(0.0), page.w, 120.0);
        let r = wordcraft_render::render_area(
            &self.doc,
            page,
            area,
            96.0 / 72.0,
            &wordcraft_render::RenderOptions::default(),
        );
        (edit, lay, ms(t), r, hit_ok, sel)
    }
}

fn keystroke(path: &str, n: usize) {
    let (mut ed, full_ms, pages) = Ed::open(path);
    let t = Instant::now();
    let snapshot = ed.doc.clone();
    let clone_ms = ms(t);
    drop(snapshot);
    let (mut e, mut l, mut r) = (Vec::new(), Vec::new(), Vec::new());
    let (mut hits, mut sels, mut px) = (0, 0, (0, 0));
    for _ in 0..n {
        let (a, b, c, img, hit_ok, sel) = ed.key();
        e.push(a);
        l.push(b);
        r.push(c);
        hits += hit_ok as usize;
        sels += (sel > 0) as usize;
        px = (img.width, img.height);
    }
    let l0 = wordcraft_layout::layout(&ed.doc, &mut ed.cache, &ed.opts);
    let t = Instant::now();
    let full = wordcraft_render::render_page(
        &ed.doc,
        &l0.pages[pages / 2],
        96.0 / 72.0,
        &wordcraft_render::RenderOptions::default(),
    );
    let page_ms = ms(t);
    println!(
        "{{\"engine\":\"wordcraft\",\"test\":\"keystroke\",\"pages\":{pages},\"keystrokes\":{n},\"first_full_layout_ms\":{full_ms:.1},\"snapshot_clone_ms\":{clone_ms:.2},\"edit_ms_p50\":{:.3},\"relayout_ms_p50\":{:.2},\"relayout_ms_p95\":{:.2},\"relayout_ms_max\":{:.2},\"strip_render_ms_p50\":{:.2},\"strip_render_ms_p95\":{:.2},\"strip_px\":\"{}x{}\",\"full_page_render_ms\":{page_ms:.1},\"full_page_px\":\"{}x{}\",\"hit_test_round_trips_ok\":{hits},\"selection_rects_nonempty\":{sels}}}",
        pct(&mut e, 0.5),
        pct(&mut l, 0.5),
        pct(&mut l, 0.95),
        pct(&mut l, 1.0),
        pct(&mut r, 0.5),
        pct(&mut r, 0.95),
        px.0,
        px.1,
        full.width,
        full.height
    );
}

fn helper(path: &str) {
    let (mut ed, _, _) = Ed::open(path);
    let (mut i, mut o) = (std::io::stdin().lock(), std::io::stdout().lock());
    o.write_all(b"R").unwrap(); // ready
    o.flush().unwrap();
    let mut b = [0u8; 1];
    while i.read_exact(&mut b).is_ok() && b[0] == b'k' {
        let (_, _, _, img, _, _) = ed.key();
        o.write_all(&(img.pixels.len() as u32).to_le_bytes()).unwrap();
        o.write_all(&img.pixels).unwrap();
        o.flush().unwrap();
    }
}

fn latency(path: &str, n: usize) {
    let t = Instant::now();
    let mut c = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["helper", path])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn helper");
    let (mut to, mut from) = (c.stdin.take().unwrap(), c.stdout.take().unwrap());
    let mut ready = [0u8; 1];
    from.read_exact(&mut ready).unwrap();
    let start_ms = ms(t);
    let mut rt = Vec::new();
    let mut bytes = 0usize;
    let mut buf = Vec::new();
    for _ in 0..n {
        let t = Instant::now();
        to.write_all(b"k").unwrap();
        to.flush().unwrap();
        let mut len = [0u8; 4];
        from.read_exact(&mut len).unwrap();
        buf.resize(u32::from_le_bytes(len) as usize, 0);
        from.read_exact(&mut buf).unwrap();
        rt.push(ms(t));
        bytes = buf.len();
    }
    // a helper that dies mid-edit: kill it, and see what the host notices
    let pid = c.id();
    c.kill().unwrap();
    let dead = to.write_all(b"k").and_then(|_| to.flush()).is_err() || from.read_exact(&mut [0u8; 4]).is_err();
    let _ = c.wait();
    println!(
        "{{\"engine\":\"wordcraft\",\"test\":\"latency_through_a_helper_process\",\"keystrokes\":{n},\"helper_start_to_ready_ms\":{start_ms:.0},\"round_trip_ms_p50\":{:.2},\"round_trip_ms_p95\":{:.2},\"round_trip_ms_max\":{:.2},\"reply_bytes\":{bytes},\"killed_helper_pid\":{pid},\"host_saw_the_dead_helper\":{dead}}}",
        pct(&mut rt, 0.5),
        pct(&mut rt, 0.95),
        pct(&mut rt, 1.0)
    );
}

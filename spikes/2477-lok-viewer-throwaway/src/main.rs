//! THROWAWAY spike for task 2477: load LibreOfficeKit on Windows, open one document, write every
//! page / sheet / slide as a PNG, print one JSON line of counts and timings. Not product code.
//!
//!   lok-spike probe <host:port>                      exit 0 = TCP connect worked, 1 = it did not
//!   lok-spike render --install <program dir> --profile <dir> --file <doc> --out <dir>
//!                    [--label <text>] [--scale <f>] [--options <lok load options>] [--listen]
//!
//! `--listen` binds 127.0.0.1:47477 and counts connections: remote.docx links an image there, so the
//! count says whether the library tried to fetch linked content.
use std::ffi::{c_char, c_int, c_long, c_void, CStr, CString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

extern "C" {
    fn bb_lok_init(install: *const c_char, profile_url: *const c_char) -> *mut c_void;
    fn bb_lok_error(k: *mut c_void) -> *mut c_char;
    fn bb_lok_version(k: *mut c_void) -> *mut c_char;
    fn bb_lok_free(k: *mut c_void, s: *mut c_char);
    fn bb_lok_destroy(k: *mut c_void);
    fn bb_lok_load(k: *mut c_void, url: *const c_char, options: *const c_char) -> *mut c_void;
    fn bb_doc_type(d: *mut c_void) -> c_int;
    fn bb_doc_parts(d: *mut c_void) -> c_int;
    fn bb_doc_page_rects(d: *mut c_void) -> *mut c_char;
    fn bb_doc_part_name(d: *mut c_void, n: c_int) -> *mut c_char;
    fn bb_doc_set_part(d: *mut c_void, n: c_int);
    fn bb_doc_size(d: *mut c_void, w: *mut c_long, h: *mut c_long);
    fn bb_doc_init_render(d: *mut c_void, args: *const c_char);
    fn bb_doc_tile_mode(d: *mut c_void) -> c_int;
    fn bb_doc_paint(d: *mut c_void, buf: *mut u8, cw: c_int, ch: c_int, x: c_int, y: c_int, w: c_int, h: c_int);
    fn bb_doc_destroy(d: *mut c_void);
}

static HITS: AtomicUsize = AtomicUsize::new(0);

#[cfg(windows)]
fn peak_memory_mb() -> (f64, f64) {
    #[repr(C)]
    #[derive(Default)]
    struct Pmc {
        cb: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        q1: usize,
        q2: usize,
        q3: usize,
        q4: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(h: isize, p: *mut Pmc, cb: u32) -> i32;
    }
    let mut p = Pmc::default();
    p.cb = std::mem::size_of::<Pmc>() as u32;
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut p, p.cb) };
    (
        p.peak_working_set as f64 / 1048576.0,
        p.peak_pagefile as f64 / 1048576.0,
    )
}
#[cfg(not(windows))]
fn peak_memory_mb() -> (f64, f64) {
    (0.0, 0.0)
}

#[cfg(windows)]
fn pump_until(done: impl Fn() -> bool) {
    #[repr(C)]
    struct Msg {
        hwnd: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: [i32; 2],
        private: u32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn PeekMessageW(m: *mut Msg, hwnd: isize, min: u32, max: u32, remove: u32) -> i32;
        fn TranslateMessage(m: *const Msg) -> i32;
        fn DispatchMessageW(m: *const Msg) -> isize;
    }
    let mut m = Msg {
        hwnd: 0,
        message: 0,
        wparam: 0,
        lparam: 0,
        time: 0,
        pt: [0, 0],
        private: 0,
    };
    while !done() {
        unsafe {
            while PeekMessageW(&mut m, 0, 0, 0, 1) != 0 {
                TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
#[cfg(not(windows))]
fn pump_until(done: impl Fn() -> bool) {
    while !done() {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn file_url(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/").replace(' ', "%20");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

fn take(k: *mut c_void, s: *mut c_char) -> String {
    if s.is_null() {
        return String::new();
    }
    let out = unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned();
    unsafe { bb_lok_free(k, s) };
    out
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// Tile bytes (BGRA or RGBA, premultiplied) to opaque RGB over white, plus the distinct-colour count
/// (capped): a page that rendered as one flat colour is a failed render, not a page.
fn to_rgb(buf: &[u8], bgra: bool) -> (Vec<u8>, usize) {
    let mut out = Vec::with_capacity(buf.len() / 4 * 3);
    let mut seen = std::collections::HashSet::new();
    for px in buf.chunks_exact(4) {
        let a = px[3] as u16;
        let (c0, c1, c2) = if bgra {
            (px[2], px[1], px[0])
        } else {
            (px[0], px[1], px[2])
        };
        let over = |c: u8| (c as u16 + (255 - a)).min(255) as u8;
        let rgb = [over(c0), over(c1), over(c2)];
        if seen.len() < 4096 {
            seen.insert(rgb);
        }
        out.extend_from_slice(&rgb);
    }
    (out, seen.len())
}

fn write_png(path: &Path, w: u32, h: u32, rgb: &[u8]) {
    let f = std::io::BufWriter::new(std::fs::File::create(path).expect("create png"));
    let mut e = png::Encoder::new(f, w, h);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()
        .expect("png header")
        .write_image_data(rgb)
        .expect("png data");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("probe") => {
            let addr = args.get(2).expect("host:port");
            use std::net::ToSocketAddrs;
            let ok = addr
                .to_socket_addrs()
                .ok()
                .and_then(|mut a| a.next())
                .map(|a| std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_secs(8)).is_ok())
                .unwrap_or(false);
            println!("probe {addr}: {}", if ok { "CONNECTED" } else { "NO CONNECTION" });
            std::process::exit(if ok { 0 } else { 1 });
        }
        Some("render") if args.iter().any(|a| a == "--load-on-thread") => {
            // Diagnosis: every LibreOfficeKit call on a worker thread; the main thread pumps messages.
            let a = args.clone();
            let worker = std::thread::spawn(move || render(&a));
            pump_until(|| worker.is_finished());
        }
        Some("render") => render(&args),
        _ => {
            eprintln!("usage: lok-spike probe <host:port> | render --install … --profile … --file … --out …");
            std::process::exit(2);
        }
    }
}

fn render(args: &[String]) {
    let t_process = Instant::now();
    let install = arg(args, "--install").expect("--install");
    let profile = PathBuf::from(arg(args, "--profile").expect("--profile"));
    let file = PathBuf::from(arg(args, "--file").expect("--file"));
    let out = PathBuf::from(arg(args, "--out").expect("--out"));
    let label = arg(args, "--label").unwrap_or_default();
    let scale: f64 = arg(args, "--scale").map(|s| s.parse().expect("scale")).unwrap_or(1.0);
    let options = arg(args, "--options").unwrap_or_default();
    std::fs::create_dir_all(&out).expect("out dir");
    std::fs::create_dir_all(&profile).expect("profile dir");

    if args.iter().any(|a| a == "--listen") {
        let l = std::net::TcpListener::bind("127.0.0.1:47477").expect("bind 47477");
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                HITS.fetch_add(1, Ordering::SeqCst);
                let mut s = s;
                let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        });
    }

    let c = |s: &str| CString::new(s).expect("no NUL");
    let stage = |m: &str| eprintln!("stage {m} +{:.0}ms", t_process.elapsed().as_secs_f64() * 1000.0);
    stage("init");
    let t = Instant::now();
    let k = unsafe { bb_lok_init(c(&install).as_ptr(), c(&file_url(&profile)).as_ptr()) };
    let init_ms = t.elapsed().as_secs_f64() * 1000.0;
    if k.is_null() {
        println!(
            "{{\"label\":\"{label}\",\"file\":\"{}\",\"error\":\"lok_init_2 returned NULL\",\"init_ms\":{init_ms:.0}}}",
            file.display()
        );
        std::process::exit(3);
    }
    let version = take(k, unsafe { bb_lok_version(k) });
    stage("load");

    let t = Instant::now();
    let d = unsafe { bb_lok_load(k, c(&file_url(&file)).as_ptr(), c(&options).as_ptr()) };
    let load_ms = t.elapsed().as_secs_f64() * 1000.0;
    if d.is_null() {
        let e = take(k, unsafe { bb_lok_error(k) });
        println!(
            "{{\"label\":\"{label}\",\"file\":\"{}\",\"error\":\"load failed: {}\",\"init_ms\":{init_ms:.0}}}",
            file.display(),
            e.replace('"', "'")
        );
        std::process::exit(4);
    }
    stage("init_render");
    let t = Instant::now();
    unsafe { bb_doc_init_render(d, std::ptr::null()) };
    let init_render_ms = t.elapsed().as_secs_f64() * 1000.0;

    stage("parts");
    let doc_type = unsafe { bb_doc_type(d) }; // 0 text, 1 spreadsheet, 2 presentation, 3 drawing
    let parts = unsafe { bb_doc_parts(d) };
    let tile_mode = unsafe { bb_doc_tile_mode(d) }; // 0 RGBA, 1 BGRA
    let rects_raw = take(k, unsafe { bb_doc_page_rects(d) });
    let stem = file.file_stem().unwrap().to_string_lossy().into_owned();
    let ext = file.extension().unwrap().to_string_lossy().into_owned();

    // (name, part to select or -1, x, y, w, h) in twips
    let mut pages: Vec<(String, i32, i32, i32, i32, i32)> = Vec::new();
    if doc_type == 0 {
        for (i, r) in rects_raw.split(';').filter(|r| !r.trim().is_empty()).enumerate() {
            let v: Vec<i32> = r.split(',').map(|n| n.trim().parse().expect("rect number")).collect();
            pages.push((format!("page{}", i + 1), -1, v[0], v[1], v[2], v[3]));
        }
    } else {
        for p in 0..parts {
            stage("set_part/size");
            unsafe { bb_doc_set_part(d, p) };
            let (mut w, mut h): (c_long, c_long) = (0, 0);
            unsafe { bb_doc_size(d, &mut w, &mut h) };
            // A sheet's "size" is its whole used area; the spike renders its top-left window only.
            let (w, h) = if doc_type == 1 {
                (w.min(12000), h.min(6000))
            } else {
                (w, h)
            };
            let kind = if doc_type == 1 { "sheet" } else { "slide" };
            pages.push((format!("{kind}{}", p + 1), p, 0, 0, w as i32, h as i32));
        }
    }

    let mut page_ms = Vec::new();
    let mut distinct = Vec::new();
    let mut names = Vec::new();
    let mut dims = Vec::new();
    for (name, part, x, y, w, h) in &pages {
        if *part >= 0 {
            unsafe { bb_doc_set_part(d, *part) };
            names.push(take(k, unsafe { bb_doc_part_name(d, *part) }));
        }
        let (cw, ch) = (
            ((*w as f64) / 15.0 * scale).round() as i32,
            ((*h as f64) / 15.0 * scale).round() as i32,
        );
        let mut buf = vec![0u8; (cw as usize) * (ch as usize) * 4];
        stage("paint");
        let t = Instant::now();
        unsafe { bb_doc_paint(d, buf.as_mut_ptr(), cw, ch, *x, *y, *w, *h) };
        page_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        let (rgb, n) = to_rgb(&buf, tile_mode == 1);
        distinct.push(n);
        dims.push(format!("\"{cw}x{ch}\""));
        write_png(
            &out.join(format!("{label}-{stem}-{ext}-{name}.png")),
            cw as u32,
            ch as u32,
            &rgb,
        );
    }
    let (peak_ws, peak_commit) = peak_memory_mb();
    let rendered = distinct.iter().filter(|n| **n > 1).count();
    let f = |v: &[f64]| v.iter().map(|m| format!("{m:.0}")).collect::<Vec<_>>().join(",");
    println!(
        "{{\"label\":\"{label}\",\"file\":\"{stem}.{ext}\",\"doc_type\":{doc_type},\"parts\":{parts},\"pages_found\":{},\"pages_rendered_non_blank\":{rendered},\"tile_mode\":{tile_mode},\"scale\":{scale},\"dims\":[{}],\"distinct_colours\":{:?},\"part_names\":{:?},\"init_ms\":{init_ms:.0},\"load_ms\":{load_ms:.0},\"init_render_ms\":{init_render_ms:.0},\"page_ms\":[{}],\"total_to_last_page_ms\":{:.0},\"peak_working_set_mb\":{peak_ws:.0},\"peak_commit_mb\":{peak_commit:.0},\"listener_hits\":{},\"version\":{}}}",
        pages.len(),
        dims.join(","),
        distinct,
        names,
        f(&page_ms),
        t_process.elapsed().as_secs_f64() * 1000.0,
        HITS.load(Ordering::SeqCst),
        if version.is_empty() { "null".to_string() } else { version.replace('\n', " ") },
    );
    std::io::stdout().flush().ok();
    unsafe { bb_doc_destroy(d) };
    if args.iter().any(|a| a == "--destroy") {
        let t = Instant::now();
        unsafe { bb_lok_destroy(k) };
        println!("destroy_ms {:.0}", t.elapsed().as_secs_f64() * 1000.0);
    }
    // The office's own teardown is not what the spike measures: leave like a killed helper would.
    std::process::exit(if rendered == pages.len() && !pages.is_empty() {
        0
    } else {
        5
    });
}

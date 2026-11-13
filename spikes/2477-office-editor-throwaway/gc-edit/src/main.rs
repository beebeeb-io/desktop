//! THROWAWAY (task 2477 editor spike): GridCraft through gridcraft-engine's public commands.
//!   probe <host:port> | roundtrip <file> <outdir> | recalc <n> | frames
use gridcraft_engine::Session;
use serde_json::{Value, json};
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
fn run(s: &mut Session, id: &str, p: Value) -> Result<Value, String> {
    s.execute_headless(id, p).map_err(|e| format!("{e:?}"))
}
fn read(s: &mut Session, range: &str) -> String {
    match run(s, "sheet.read", json!({"range": range, "formatted": true})) {
        Ok(v) => v.to_string(),
        Err(e) => format!("ERR {e}"),
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
        Some("recalc") => recalc(a[2].parse().unwrap()),
        Some("frames") => frames(),
        Some("roundtrip") => roundtrip(&a[2], &a[3]),
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
        .unwrap_or((name.clone(), "xlsx".into()));
    let mut s = Session::new();
    let t = Instant::now();
    let opened = run(&mut s, "file.open", json!({"path": path}));
    let load_ms = ms(t);
    if let Err(e) = &opened {
        println!("{{\"engine\":\"gridcraft\",\"file\":\"{name}\",\"error\":{:?}}}", e);
        return;
    }
    let t = Instant::now();
    let resave = run(
        &mut s,
        "file.save",
        json!({"path": format!("{outdir}/{stem}.resaved.{ext}")}),
    );
    let save_ms = ms(t);
    let before = read(&mut s, "J1:J3");
    // three edits (the second one is two cells and a recalculation)
    let mut ok = Vec::new();
    let mut step = |s: &mut Session, id: &str, p: Value| {
        let r = run(s, id, p);
        ok.push(format!(
            "{id}={}",
            match &r {
                Ok(_) => "ok".into(),
                Err(e) => format!("ERR {e}"),
            }
        ));
    };
    step(&mut s, "cell.set", json!({"cell": "J1", "input": "BBEDIT-CELL"}));
    step(&mut s, "cell.set", json!({"cell": "J2", "input": "=1+2"}));
    step(&mut s, "cell.set", json!({"cell": "J3", "input": "=J2*2"}));
    let first = read(&mut s, "J1:J3");
    step(&mut s, "cell.set", json!({"cell": "J2", "input": "5"}));
    let after = read(&mut s, "J1:J3");
    step(&mut s, "home.bold", json!({"range": "J1:J3", "on": true}));
    let n = ok.len();
    let undo = run(&mut s, "edit.undo", json!({"steps": n}));
    let undone = read(&mut s, "J1:J3");
    let redo = run(&mut s, "edit.redo", json!({"steps": n}));
    let redone = read(&mut s, "J1:J3");
    let saved = run(
        &mut s,
        "file.save",
        json!({"path": format!("{outdir}/{stem}.edited.{ext}")}),
    );
    let mut s2 = Session::new();
    let re = run(
        &mut s2,
        "file.open",
        json!({"path": format!("{outdir}/{stem}.edited.{ext}")}),
    );
    let reopened = read(&mut s2, "J1:J3");
    let style = run(&mut s2, "cell.get", json!({"cell": "J1"}))
        .map(|v| v["style"].to_string())
        .unwrap_or_else(|e| format!("\"ERR {e}\""));
    println!(
        "{{\"engine\":\"gridcraft\",\"file\":\"{name}\",\"load_ms\":{load_ms:.1},\"save_ms\":{save_ms:.1},\"resave_ok\":{},\"commands\":{:?},\"cells_before\":{before},\"after_formulas\":{first},\"after_changing_the_input\":{after},\"recalculated\":{},\"undo_ok\":{},\"undo_restores\":{},\"redo_ok\":{},\"redo_restores\":{},\"save_ok\":{},\"reopen_ok\":{},\"reopened_same_cells\":{},\"reopened_J1_style\":{}}}",
        resave.is_ok(),
        ok,
        first.contains("\"6\"") && after.contains("\"10\""),
        undo.is_ok(),
        undone == before,
        redo.is_ok(),
        redone == after,
        saved.is_ok(),
        re.is_ok(),
        reopened == after,
        style
    );
}

/// One cell edit that makes `n` dependent formulas recalculate (a chain down column B).
fn recalc(n: usize) {
    let mut s = Session::new();
    s.new_workbook();
    let t = Instant::now();
    for i in 1..=n {
        run(
            &mut s,
            "cell.set",
            json!({"cell": format!("A{i}"), "input": format!("{i}")}),
        )
        .unwrap();
        let f = if i == 1 {
            "=A1*2".to_string()
        } else {
            format!("=A{i}*2+B{}", i - 1)
        };
        run(&mut s, "cell.set", json!({"cell": format!("B{i}"), "input": f})).unwrap();
    }
    let build_ms = ms(t);
    let mut v = Vec::new();
    for k in 0..50 {
        let t = Instant::now();
        run(
            &mut s,
            "cell.set",
            json!({"cell": "A1", "input": format!("{}", 100 + k)}),
        )
        .unwrap();
        v.push(ms(t));
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let last = read(&mut s, &format!("B{n}:B{n}"));
    println!(
        "{{\"engine\":\"gridcraft\",\"test\":\"cell edit with {n} dependent formulas\",\"build_ms\":{build_ms:.0},\"edit_and_recalc_ms_p50\":{:.2},\"edit_and_recalc_ms_p95\":{:.2},\"edit_and_recalc_ms_max\":{:.2},\"last_cell\":{last}}}",
        v[25], v[47], v[49]
    );
}

/// DW-1559 (Sort A to Z, Filter, Table, the go-to box and its error) and DW-1558 (a new, blank workbook).
fn frames() {
    let mut s = Session::new();
    let mut out = Vec::new();
    let mut step = |s: &mut Session, id: &str, p: Value| {
        let r = run(s, id, p.clone());
        out.push(format!(
            "{id} {p} = {}",
            match &r {
                Ok(v) => format!("ok {v}").chars().take(120).collect::<String>(),
                Err(e) => format!("ERR {e}").chars().take(160).collect(),
            }
        ));
    };
    step(&mut s, "file.new", json!({}));
    for (c, v) in [
        ("A1", "name"),
        ("B1", "n"),
        ("A2", "pear"),
        ("B2", "3"),
        ("A3", "apple"),
        ("B3", "1"),
        ("A4", "fig"),
        ("B4", "2"),
    ] {
        run(&mut s, "cell.set", json!({"cell": c, "input": v})).unwrap();
    }
    step(&mut s, "edit.goTo", json!({"reference": "A2"}));
    step(&mut s, "data.sortAscending", json!({"header": true, "column": "A"}));
    let sorted = read(&mut s, "A1:B4");
    step(&mut s, "data.filter", json!({}));
    step(&mut s, "insert.table", json!({"range": "A1:B4", "headers": true}));
    step(&mut s, "edit.goTo", json!({"reference": "B5"}));
    step(&mut s, "edit.goTo", json!({"reference": "not a cell !!"}));
    let saved = run(&mut s, "file.saveBytes", json!({"format": "xlsx"})).map(|v| v.to_string().len());
    println!(
        "{{\"engine\":\"gridcraft\",\"test\":\"frames\",\"results\":{out:?},\"after_sort\":{sorted},\"saves\":{:?}}}",
        saved
    );
}

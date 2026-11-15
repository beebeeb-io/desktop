//! media-mf-run: starts `media-mf-probe` once per (fixture, column, accel) and prints what each run said.
//!
//! Usage: `media-mf-run <fixtures-dir> [columns] [files] [hw]`
//! - `columns`: comma list of `plain` (a plain child process, the runner's environment), `plainenv` (a plain child
//!   process with exactly the sandbox's environment names and working directory), `confined` (the product's
//!   `beebeeb-office-sandbox`, unchanged). Default: all three. A row that works in `plainenv` and fails in `confined`
//!   is the AppContainer's doing; one that fails in `plainenv` already is the environment's.
//! - `files`: comma list of fixture names, or `all` (default).
//! - `hw`: `1` adds a run per video fixture with a D3D11 device manager (hardware decoding when the PC has it).
//!
//! The fixtures dir holds `manifest.tsv` (`file kind codec want frames ref_y0 note`, tab-separated, `#` comments).
//! Every probe run gets a timeout and is killed after it.
#![cfg(windows)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use beebeeb_office_sandbox::{Limits, Sandbox};

const TIMEOUT: Duration = Duration::from_secs(90);

struct Row {
    file: String,
    kind: String,
    codec: String,
    want: String,
    frames: String,
    ref_y0: String,
}

struct Outcome {
    lines: Vec<String>,
    exit: String,
    ms: u128,
}

fn read_all_in_thread<R: Read + Send + 'static>(mut from: R) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = from.read_to_end(&mut bytes);
        let _ = tx.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    rx
}

fn write_all_in_thread<W: Write + Send + 'static>(mut to: W, request: Vec<u8>) {
    thread::spawn(move || {
        let _ = to.write_all(&request);
        let _ = to.flush();
        // Dropping `to` closes the pipe: the probe sees the end of its input.
    });
}

fn run_plain(probe: &Path, request: &[u8], sandbox_env: bool, windir: &str) -> Outcome {
    let started = Instant::now();
    let mut command = Command::new(probe);
    if sandbox_env {
        // The sandbox's block (`crates/beebeeb-office-sandbox/src/block.rs`): LOCALAPPDATA and SystemRoot set to the
        // Windows directory; Windows then adds TEMP and TMP under a placeholder that does not exist. The working
        // directory is the system directory.
        let missing = format!("{windir}\\beebeeb-spike-none");
        command
            .env_clear()
            .env("LOCALAPPDATA", windir)
            .env("SystemRoot", windir)
            .env("TEMP", &missing)
            .env("TMP", &missing)
            .current_dir(format!("{windir}\\System32"));
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Outcome {
                lines: vec![format!("RUNNER spawn failed: {e}")],
                exit: "not-started".into(),
                ms: started.elapsed().as_millis(),
            };
        }
    };
    let stdout = read_all_in_thread(child.stdout.take().expect("stdout piped"));
    let stderr = read_all_in_thread(child.stderr.take().expect("stderr piped"));
    write_all_in_thread(child.stdin.take().expect("stdin piped"), request.to_vec());
    let exit = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break status
                    .code()
                    .map(|c| format!("{:#X}", c as u32))
                    .unwrap_or("none".into());
            }
            Ok(None) if started.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                break "timeout-killed".into();
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => break format!("wait-error {e}"),
        }
    };
    let mut lines: Vec<String> = stdout
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    let err = stderr.recv_timeout(Duration::from_secs(10)).unwrap_or_default();
    lines.extend(
        err.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| format!("STDERR {l}")),
    );
    Outcome {
        lines,
        exit,
        ms: started.elapsed().as_millis(),
    }
}

fn run_confined(probe: &Path, request: &[u8]) -> Outcome {
    let started = Instant::now();
    let fail = |what: String| Outcome {
        lines: vec![format!("RUNNER {what}")],
        exit: "not-started".into(),
        ms: started.elapsed().as_millis(),
    };
    let sandbox = match Sandbox::new(Limits::default()) {
        Ok(s) => s,
        Err(e) => return fail(format!("sandbox refused: {e}")),
    };
    let mut child = match sandbox.spawn(probe) {
        Ok(c) => c,
        Err(e) => return fail(format!("sandbox spawn failed: {e}")),
    };
    let Some(pipes) = child.take_pipes() else {
        return fail("no pipes".into());
    };
    let stdout = read_all_in_thread(pipes.from_helper);
    write_all_in_thread(pipes.to_helper, request.to_vec());
    let exit = match child.wait_timeout(TIMEOUT) {
        Ok(Some(status)) => format!("{:#X}", status.code()),
        Ok(None) => {
            let _ = child.kill();
            "timeout-killed".into()
        }
        Err(e) => format!("wait-error {e}"),
    };
    let lines = stdout
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    Outcome {
        lines,
        exit,
        ms: started.elapsed().as_millis(),
    }
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
}

/// The probe sees the bytes only; the sandbox gives it no arguments. The path must be `X:\…` (no `\\?\`).
fn probe_path() -> PathBuf {
    sibling("media-mf-probe.exe")
}

fn sibling(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("own path");
    let path = exe.with_file_name(name);
    let text = path.to_string_lossy().to_string();
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

/// `media-mf-run --load <columns>`: starts `media-mf-loadcheck` in each column and prints what it said.
fn load_check(columns: &str) {
    let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let exe = sibling("media-mf-loadcheck.exe");
    println!("RUN load exe-exists={}", exe.is_file());
    for column in columns.split(',') {
        let o = match column {
            "plain" => run_plain(&exe, b"", false, &windir),
            "plainenv" => run_plain(&exe, b"", true, &windir),
            "confined" => run_confined(&exe, b""),
            other => Outcome {
                lines: vec![format!("RUNNER unknown column {other}")],
                exit: "not-started".into(),
                ms: 0,
            },
        };
        println!("== load {column} exit={} ms={}", o.exit, o.ms);
        for l in &o.lines {
            println!("  {l}");
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--load") {
        load_check(args.get(2).map(String::as_str).unwrap_or("plain,plainenv,confined"));
        return;
    }
    let Some(dir) = args.get(1).map(PathBuf::from) else {
        eprintln!("usage: media-mf-run <fixtures-dir> [columns] [files] [hw]");
        std::process::exit(2);
    };
    let columns: Vec<String> = args
        .get(2)
        .map(|s| s.split(',').map(str::to_owned).collect())
        .unwrap_or_else(|| vec!["plain".into(), "plainenv".into(), "confined".into()]);
    let only: Option<Vec<String>> = args
        .get(3)
        .filter(|s| s.as_str() != "all")
        .map(|s| s.split(',').map(str::to_owned).collect());
    let with_hw = args.get(4).map(|s| s == "1").unwrap_or(false);
    let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let probe = probe_path();
    println!(
        "RUN probe-exists={} columns={} hw={}",
        probe.is_file(),
        columns.join(","),
        u8::from(with_hw)
    );

    let manifest = std::fs::read_to_string(dir.join("manifest.tsv")).expect("manifest.tsv");
    let rows: Vec<Row> = manifest
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            (c.len() >= 6).then(|| Row {
                file: c[0].into(),
                kind: c[1].into(),
                codec: c[2].into(),
                want: c[3].into(),
                frames: c[4].into(),
                ref_y0: c[5].into(),
            })
        })
        .filter(|r| only.as_ref().is_none_or(|o| o.contains(&r.file)))
        .collect();
    println!("RUN rows={}", rows.len());

    let start = |column: &str, request: &[u8]| -> Outcome {
        match column {
            "plain" => run_plain(&probe, request, false, &windir),
            "plainenv" => run_plain(&probe, request, true, &windir),
            "confined" => run_confined(&probe, request),
            other => Outcome {
                lines: vec![format!("RUNNER unknown column {other}")],
                exit: "not-started".into(),
                ms: 0,
            },
        }
    };

    // What Media Foundation can find, per column.
    if only.is_none() {
        for column in &columns {
            let o = start(column, b"E\n");
            println!("== decoders {column} exit={} ms={}", o.exit, o.ms);
            for l in &o.lines {
                println!("  {l}");
            }
        }
    }

    let mut summary: Vec<String> = Vec::new();
    for row in &rows {
        let data = match std::fs::read(dir.join(&row.file)) {
            Ok(d) => d,
            Err(e) => {
                summary.push(format!("{}\tmissing\t{e}", row.file));
                continue;
            }
        };
        let ext = Path::new(&row.file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin")
            .to_owned();
        let mut accels = vec!["sw"];
        if with_hw && row.kind != "a" {
            accels.push("hw");
        }
        let mut plain_hash: Option<String> = None;
        for column in &columns {
            for accel in &accels {
                let mut request = format!("F {ext} {accel} {} {}\n", row.want, data.len()).into_bytes();
                request.extend_from_slice(&data);
                let o = start(column, &request);
                println!(
                    "== {} [{}] {column} {accel} exit={} ms={}",
                    row.file, row.codec, o.exit, o.ms
                );
                for l in &o.lines {
                    println!("  {l}");
                }
                let token = o
                    .lines
                    .iter()
                    .find(|l| l.starts_with("T "))
                    .cloned()
                    .unwrap_or_else(|| "T none".into());
                let confined = field(&token, "appcontainer").unwrap_or("?").to_owned();
                let v = o
                    .lines
                    .iter()
                    .find(|l| l.starts_with("V "))
                    .cloned()
                    .unwrap_or_else(|| "V none-printed".into());
                let a = o
                    .lines
                    .iter()
                    .find(|l| l.starts_with("A "))
                    .cloned()
                    .unwrap_or_else(|| "A -".into());
                let store = o
                    .lines
                    .iter()
                    .find(|l| l.starts_with("W "))
                    .cloned()
                    .unwrap_or_else(|| "W -".into());
                // The checks that make "ok" able to fail: the frame count, the reference hash, and the same picture
                // in every column.
                let mut checks = Vec::new();
                if v.starts_with("V ok") {
                    let frames = field(&v, "frames").unwrap_or("?");
                    checks.push(if row.frames == "-" || frames == row.frames {
                        format!("frames:{frames}=={}", row.frames)
                    } else {
                        format!("frames:{frames}!={}", row.frames)
                    });
                    let hash = field(&v, "hash").unwrap_or("-").to_owned();
                    checks.push(if row.ref_y0 == "-" {
                        "ref:n/a".into()
                    } else if hash == row.ref_y0 {
                        "ref:match".into()
                    } else {
                        "ref:DIFFERS".into()
                    });
                    if *accel == "sw" {
                        match &plain_hash {
                            None => plain_hash = Some(hash),
                            Some(first) => checks.push(if *first == hash {
                                "same-as-first-column".into()
                            } else {
                                "DIFFERS-from-first-column".into()
                            }),
                        }
                    }
                }
                summary.push(format!(
                    "{}\t{column}\t{accel}\tappcontainer={confined}\texit={}\t{}\t[{}]\t{}\t{}",
                    row.file,
                    o.exit,
                    v,
                    checks.join(" "),
                    a,
                    store
                ));
            }
        }
    }
    println!("== SUMMARY");
    for s in &summary {
        println!("{s}");
    }
    println!("RUN done rows={} lines={}", rows.len(), summary.len());
}

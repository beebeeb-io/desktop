//! Every process the app starts holds the launch lock (`launch.rs`, #460 F11): a scan of the crate's own code for a
//! start outside [`launch::spawn`] / [`launch::locked`], and the lock seen held from inside a start.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::launch;

/// What starts a process: a `std::process::Command`, and the system calls that start one.
const STARTS: [&str; 5] = [
    "Command::new(",
    "ShellExecuteW(",
    "ShellExecuteExW(",
    "SHOpenWithDialog(",
    ".Invoke(",
];
/// The calls that hold the launch lock around what is inside their parentheses.
const UNDER_LOCK: [&str; 2] = ["launch::spawn(", "launch::locked("];
/// The one start the scan knows of outside this crate: the engine's ffmpeg (P0 2542, held for the owner).
const FFMPEG: &str = "../../crates/beebeeb-engine/src/engine_bridge/media.rs";

/// `src` with its comments and the insides of its string and char literals blanked out (same length, same lines),
/// and cut where an inline test module starts (`#[cfg(test)]` over `mod name {`).
fn code_only(src: &str) -> String {
    let src = cut_inline_tests(src);
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut out[from..to] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = src[i..].find('\n').map_or(b.len(), |n| i + n);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let (mut depth, mut j) = (1, i + 2);
                while j < b.len() && depth > 0 {
                    if b[j] == b'/' && b.get(j + 1) == Some(&b'*') {
                        depth += 1;
                        j += 2;
                    } else if b[j] == b'*' && b.get(j + 1) == Some(&b'/') {
                        depth -= 1;
                        j += 2;
                    } else {
                        j += 1;
                    }
                }
                blank(&mut out, i, j);
                i = j;
            }
            // A raw string: r"…", r#"…"#, br"…".
            b'r' if (i == 0 || !ident(b[i - 1]) || (b[i - 1] == b'b' && (i < 2 || !ident(b[i - 2]))))
                && matches!(b.get(i + 1), Some(b'"' | b'#')) =>
            {
                let hashes = b[i + 1..].iter().take_while(|&&c| c == b'#').count();
                if b.get(i + 1 + hashes) != Some(&b'"') {
                    i += 1;
                    continue;
                }
                let close = format!("\"{}", "#".repeat(hashes));
                let body = i + 2 + hashes;
                let end = src[body..].find(&close).map_or(b.len(), |n| body + n + close.len());
                blank(&mut out, body, end.saturating_sub(close.len()).max(body));
                i = end;
            }
            b'"' => {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'"' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                blank(&mut out, i + 1, j.min(b.len()));
                i = j + 1;
            }
            // A char literal ('(' or '\''), not a lifetime ('a).
            b'\'' if b.get(i + 1) == Some(&b'\\') => {
                let end = src[i + 2..].find('\'').map_or(b.len(), |n| i + 2 + n);
                blank(&mut out, i + 1, end);
                i = end + 1;
            }
            b'\'' if b.get(i + 2) == Some(&b'\'') => {
                blank(&mut out, i + 1, i + 2);
                i += 3;
            }
            _ => i += 1,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `src` up to its first inline test module (`#[cfg(test)]`, then attributes, then `mod name {`). A test module
/// declared as a file of its own (`mod name;`) is not cut: its file is not scanned.
fn cut_inline_tests(src: &str) -> &str {
    let lines: Vec<(usize, &str)> = src
        .split_inclusive('\n')
        .scan(0, |at, line| {
            let start = *at;
            *at += line.len();
            Some((start, line))
        })
        .collect();
    for (n, (start, line)) in lines.iter().enumerate() {
        if line.trim() != "#[cfg(test)]" {
            continue;
        }
        let next = lines[n + 1..]
            .iter()
            .map(|(_, l)| l.trim())
            .find(|l| !l.starts_with("#["));
        if next.is_some_and(|l| l.contains("mod ") && l.ends_with('{')) {
            return &src[..*start];
        }
    }
    src
}

/// The 1-based lines of every start in `src` that no [`UNDER_LOCK`] call holds.
fn bare_starts(src: &str) -> Vec<usize> {
    let code = code_only(src);
    let mut held: Vec<bool> = Vec::new();
    let mut bare = Vec::new();
    for (at, c) in code.char_indices() {
        let before = &code[..at];
        if c == '(' {
            held.push(UNDER_LOCK.iter().any(|call| before.ends_with(&call[..call.len() - 1])));
        }
        if c == '(' && STARTS.iter().any(|start| before.ends_with(&start[..start.len() - 1])) && !held.contains(&true) {
            bare.push(before.matches('\n').count() + 1);
        }
        if c == ')' {
            held.pop();
        }
    }
    bare
}

/// Every `.rs` file under `dir` that is not a test's: no `tests.rs`, no `*_tests.rs`, nothing in a `*_tests` or
/// `tests` folder.
fn non_test_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !(name == "tests" || name.ends_with("_tests")) {
                non_test_files(&path, out);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_launch_takes_the_launch_lock() {
    // The instrument first, on fixed text: it counts a bare start, and only that.
    assert_eq!(
        bare_starts("fn f() { std::process::Command::new(\"x\").spawn(); }").len(),
        1
    );
    assert_eq!(
        bare_starts(
            "fn f() {\n    launch::spawn(std::process::Command::new(\"x\").arg(p));\n    \
             launch::locked(|| unsafe { ShellExecuteW(a, b) });\n    launch::locked(|| unsafe { h.Invoke(&d) });\n}"
        ),
        Vec::<usize>::new()
    );
    assert_eq!(
        bare_starts("// Command::new(\"x\")\nlet s = \"ShellExecuteW(\"; let c = '(';\n"),
        Vec::<usize>::new(),
        "a comment, a string and a char are not starts"
    );
    assert_eq!(
        bare_starts("let a = (1, 2);\nh.Invoke(&d);\n"),
        [2],
        "the line of the start"
    );
    assert_eq!(
        bare_starts("fn f() {}\n#[cfg(test)]\nmod tests {\n    fn g() { Command::new(\"x\"); }\n}\n"),
        Vec::<usize>::new(),
        "an inline test module is not the app's code"
    );

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lock_file = root.join("src/engine_host/launch.rs");
    let mut files = Vec::new();
    non_test_files(&root.join("src"), &mut files);
    files.sort();
    assert!(files.len() > 100, "the scan reads the crate: {} files", files.len());
    assert!(
        files.iter().any(|f| f.ends_with("engine_host.rs")),
        "engine_host.rs is scanned"
    );
    let mut bare = Vec::new();
    for file in files.iter().filter(|f| **f != lock_file) {
        let src = std::fs::read_to_string(file).unwrap();
        for line in bare_starts(&src) {
            bare.push(format!("{}:{line}", file.strip_prefix(root).unwrap_or(file).display()));
        }
    }
    assert_eq!(
        bare,
        Vec::<String>::new(),
        "a start outside launch::spawn / launch::locked"
    );

    // The one named exception outside the crate: the engine's ffmpeg (P0 2542), still the only bare start there.
    let ffmpeg = std::fs::read_to_string(root.join(FFMPEG)).unwrap();
    let ffmpeg_bare = bare_starts(&ffmpeg);
    eprintln!(
        "launch scan: {} files, 0 bare; one named exception: {FFMPEG}:{ffmpeg_bare:?} (P0 2542)",
        files.len()
    );
    assert_eq!(
        ffmpeg_bare.len(),
        1,
        "the engine's ffmpeg start is the one named exception: {ffmpeg_bare:?}"
    );
}

#[test]
fn launch_spawn_holds_the_lock_while_spawning() {
    // The instrument first: outside a start the lock is seen free (another test's start may hold it a moment).
    assert!(
        (0..20).any(|_| launch::free_within(Duration::from_millis(250))),
        "the probe never saw the lock free"
    );
    let mut seen_free = None;
    let started = launch::spawn_by(&mut std::process::Command::new("never-started"), |_| {
        seen_free = Some(launch::free_within(Duration::from_millis(300)));
        Err::<(), _>(std::io::Error::other("not started"))
    });
    assert!(started.is_err());
    assert_eq!(seen_free, Some(false), "the lock is held while the process starts");
    let mut seen_free = None;
    launch::locked(|| seen_free = Some(launch::free_within(Duration::from_millis(300))));
    assert_eq!(
        seen_free,
        Some(false),
        "the lock is held around a system call that starts one"
    );
}

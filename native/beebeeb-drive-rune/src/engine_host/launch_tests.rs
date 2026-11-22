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
/// The updater crate's way of saying it: `VerifiedInstaller::start_nsis` takes the app's `launch::spawn` as a parameter
/// of this name and starts the installer through it (the crate cannot name the lock itself).
const UPDATER_UNDER_LOCK: [&str; 1] = ["under_launch_lock("];
/// The updater crate's one start: the passive NSIS installer.
const UPDATER: &str = "../../crates/beebeeb-updater/src/machine.rs";
/// The engine's media file: it once started a program by search name for a video's thumbnail; it starts none now.
const FFMPEG: &str = "../../crates/beebeeb-engine/src/engine_bridge/media.rs";

/// `src` with its comments and the insides of its string and char literals blanked out (same length, same lines).
pub(super) fn code_only(src: &str) -> String {
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

/// `code` (from [`code_only`]) with every inline test module blanked: `#[cfg(test)]`, the attributes after it, then
/// `mod name { … }` to its closing brace. The code after it is still scanned. A test module in a file of its own
/// (`mod name;`) is left: its file is not scanned.
pub(super) fn without_inline_tests(code: &str) -> String {
    let mut out = code.as_bytes().to_vec();
    let mut from = 0;
    while let Some(n) = code[from..].find("#[cfg(test)]") {
        let start = from + n;
        let mut i = start + "#[cfg(test)]".len();
        loop {
            let rest = &code[i..];
            let trimmed = rest.trim_start();
            i += rest.len() - trimmed.len();
            if !trimmed.starts_with("#[") {
                break;
            }
            i += trimmed.find(']').map_or(trimmed.len(), |e| e + 1);
        }
        let head = code[i..]
            .trim_start_matches("pub(crate) ")
            .trim_start_matches("pub(super) ")
            .trim_start_matches("pub ");
        let head_at = code.len() - head.len();
        let open = head
            .starts_with("mod ")
            .then(|| head.find(['{', ';']).map(|e| head_at + e))
            .flatten()
            .filter(|&open| code.as_bytes()[open] == b'{');
        let Some(open) = open else {
            from = start + 1;
            continue;
        };
        let mut depth = 0;
        let mut end = code.len();
        for (k, c) in code[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + k + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        for c in &mut out[start..end] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
        from = end;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The starts in `src`: the 1-based lines of those no [`UNDER_LOCK`] call holds, and how many one holds.
fn starts(src: &str) -> (Vec<usize>, usize) {
    starts_under(src, &UNDER_LOCK)
}

/// [`starts`] with the calls that hold the lock named: this crate's, or the updater crate's.
fn starts_under(src: &str, under_lock: &[&str]) -> (Vec<usize>, usize) {
    let code = without_inline_tests(&code_only(src));
    let mut held: Vec<bool> = Vec::new();
    let (mut bare, mut covered) = (Vec::new(), 0);
    for (at, c) in code.char_indices() {
        let before = &code[..at];
        if c == '(' {
            held.push(under_lock.iter().any(|call| before.ends_with(&call[..call.len() - 1])));
            if STARTS.iter().any(|start| before.ends_with(&start[..start.len() - 1])) {
                if held.contains(&true) {
                    covered += 1;
                } else {
                    bare.push(before.matches('\n').count() + 1);
                }
            }
        }
        if c == ')' {
            held.pop();
        }
    }
    (bare, covered)
}

fn bare_starts(src: &str) -> Vec<usize> {
    starts(src).0
}

/// Every `.rs` file under `dir` that is not a test's: no `tests.rs`, no `*_tests.rs`, nothing in a `*_tests` or
/// `tests` folder.
pub(super) fn non_test_files(dir: &Path, out: &mut Vec<PathBuf>) {
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
    assert_eq!(
        bare_starts(
            "fn f() {}\n#[cfg(test)]\n#[path = \"x\"]\nmod t {\n    fn g() { Command::new(\"x\"); }\n}\nfn h() { Command::new(\"y\").spawn(); }\n"
        ),
        [7],
        "the code after an inline test module is still scanned"
    );
    assert_eq!(
        starts("launch::spawn(Command::new(\"x\"));").1,
        1,
        "a start under the lock is counted as held"
    );

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lock_file = root.join("src/engine_host/launch.rs");
    let mut files = Vec::new();
    non_test_files(&root.join("src"), &mut files);
    files.sort();
    assert!(files.len() > 100, "the scan reads the crate: {} files", files.len());
    assert!(
        files.iter().any(|f| f.ends_with("engine_host/mod.rs")),
        "engine_host/mod.rs is scanned"
    );
    let (mut bare, mut held) = (Vec::new(), Vec::new());
    for file in files.iter().filter(|f| **f != lock_file) {
        let name = file
            .strip_prefix(root)
            .unwrap_or(file)
            .display()
            .to_string()
            .replace('\\', "/");
        let (lines, covered) = starts(&std::fs::read_to_string(file).unwrap());
        bare.extend(lines.into_iter().map(|line| format!("{name}:{line}")));
        if covered > 0 {
            held.push((name, covered));
        }
    }
    assert_eq!(
        bare,
        Vec::<String>::new(),
        "a start outside launch::spawn / launch::locked"
    );
    held.sort();
    // The scan saw the starts it is meant to see, each under the lock: open, the chooser and Show in File Explorer
    // (7, every system's), and Open With's handler.
    assert_eq!(
        held,
        [
            ("src/engine_host/mod.rs".to_string(), 7),
            ("src/engine_host/open_with_apps.rs".to_string(), 1)
        ],
        "the starts held by the lock"
    );

    // Outside the crate: the engine's media file starts no program at all (a video's thumbnail comes from the
    // confined helper).
    let ffmpeg = std::fs::read_to_string(root.join(FFMPEG)).unwrap();
    let ffmpeg_bare = bare_starts(&ffmpeg);
    eprintln!(
        "launch scan: {} files + the updater, 0 bare; {FFMPEG}: {ffmpeg_bare:?}",
        files.len()
    );
    assert!(
        ffmpeg_bare.is_empty(),
        "the engine's media file starts a program: {ffmpeg_bare:?}"
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

/// F11 on H1-S: the app's starts take the SAME lock as the helpers' start (`beebeeb_office_sandbox::launch_lock`, which
/// `Sandbox::spawn` takes). Held through the sandbox's own call, the app's lock is seen held; let go, it is free.
#[test]
fn launch_lock_is_the_sandboxs_own() {
    assert!(
        (0..20).any(|_| launch::free_within(Duration::from_millis(250))),
        "the probe never saw the lock free"
    );
    let (holding, held) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let helper_start = std::thread::spawn(move || {
        let _guard = beebeeb_office_sandbox::launch_lock();
        holding.send(()).unwrap();
        let _ = released.recv_timeout(Duration::from_secs(3));
    });
    held.recv().unwrap();
    assert!(
        !launch::free_within(Duration::from_millis(300)),
        "while the sandbox's lock is held, the app's starts wait for it"
    );
    release.send(()).unwrap();
    helper_start.join().unwrap();
}

/// P3-4: the updater crate's installer start is seen by the scan, and it is under the launch lock (the app's
/// `launch::spawn`, handed in as `under_launch_lock`). With ffmpeg's (P0 2542) the scan's named exceptions stay 1.
#[test]
fn the_updaters_installer_start_takes_the_launch_lock() {
    // The instrument first, on fixed text: the updater's own phrasing counts as held; a bare start does not.
    let held = "fn f(under_launch_lock: impl FnOnce(&mut Command) -> R) { under_launch_lock(Command::new(\"x\").args([\"/P\"])); }";
    assert_eq!(starts_under(held, &UPDATER_UNDER_LOCK), (Vec::new(), 1));
    let bare = "fn f() {\n    Command::new(\"x\").spawn();\n}";
    assert_eq!(starts_under(bare, &UPDATER_UNDER_LOCK), (vec![2], 0));
    assert_eq!(
        starts_under(held, &UNDER_LOCK).0.len(),
        1,
        "this crate's own phrasing is not the updater's: it must not pass for it"
    );

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let updater_dir = root.join(UPDATER).parent().unwrap().to_path_buf();
    let mut files = Vec::new();
    non_test_files(&updater_dir, &mut files);
    files.sort();
    assert!(
        files.iter().any(|f| f.ends_with("machine.rs")),
        "the scan reads the updater's machine: {files:?}"
    );
    let (mut bare, mut covered) = (Vec::new(), 0);
    for file in &files {
        let (lines, held) = starts_under(&std::fs::read_to_string(file).unwrap(), &UPDATER_UNDER_LOCK);
        bare.extend(lines.into_iter().map(|line| format!("{}:{line}", file.display())));
        covered += held;
    }
    assert_eq!(
        bare,
        Vec::<String>::new(),
        "a start in the updater outside under_launch_lock"
    );
    assert_eq!(
        covered, 1,
        "the updater's one start (the installer) is seen, under the lock"
    );

    // The app hands the lock in: its call site passes `launch::spawn`.
    let apply = std::fs::read_to_string(root.join("src/engine_host/settings/updates_apply.rs")).unwrap();
    assert!(
        code_only(&apply).contains("start_nsis(stopped, crate::engine_host::launch::spawn)"),
        "the app starts the installer through launch::spawn"
    );
}

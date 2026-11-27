//! Pins on the helper's source and graph: it writes nothing but frames, its panic hook writes nothing, and it links no
//! network crate. Forbidden words are built from pieces, so this file's own text never matches.

use std::path::Path;

/// Every `.rs` file in `src/`, read from disk so that a new module is pinned too, sorted by name.
fn sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("src/ can be listed") {
        let path = entry.expect("a directory entry").path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        if name.ends_with(".rs") {
            let text = std::fs::read_to_string(&path).expect("a source file can be read");
            files.push((name, text));
        }
    }
    files.sort();
    files
}

/// Rule 7: no print, no logger, no file, no socket, no environment, no process in `src/`. Standard output is the
/// protocol, standard error is not connected, and nothing else may carry a byte out.
#[test]
fn the_helper_writes_nothing_but_frames() {
    let files = sources();
    let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "edit_guard.rs",
            "edit_map.rs",
            "engine.rs",
            "fit.rs",
            "lib.rs",
            "main.rs",
            "memory.rs",
            "pdf.rs",
            "save.rs",
            "serve.rs",
            "start.rs",
            "xpc.rs"
        ],
        "every module was read; a new one joins this list"
    );
    let forbidden = [
        ["print", "ln!"].concat(),
        ["print", "!"].concat(),
        ["eprint", "ln!"].concat(),
        ["eprint", "!"].concat(),
        ["dbg", "!"].concat(),
        ["set_", "logger"].concat(),
        ["set_boxed", "_logger"].concat(),
        ["set_max", "_level"].concat(),
        ["std::", "fs"].concat(),
        ["File", "::"].concat(),
        ["std::", "net"].concat(),
        ["Tcp", "Stream"].concat(),
        ["Udp", "Socket"].concat(),
        ["env::", "var"].concat(),
        ["env::", "args"].concat(),
        ["Command", "::new"].concat(),
        ["std", "err"].concat(),
    ];
    let mut checked = 0;
    for (name, source) in &files {
        assert!(source.len() > 200, "src/{name} was read");
        for word in &forbidden {
            assert!(
                !source.contains(word.as_str()),
                "src/{name} holds {word}: the helper's only output is frames"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 12 * 17, "twelve files, seventeen words");
}

/// The panic hook exits and never looks at the panic: its message can quote the document.
#[test]
fn the_panic_hook_writes_nothing() {
    let start = include_str!("../src/start.rs");
    assert!(
        start.contains(&["set_", "hook(Box::new(|_|"].concat()),
        "the hook takes the panic and ignores it"
    );
    assert!(start.contains(&["process::", "exit("].concat()), "the hook exits");
    let forbidden = [
        ["{", "}"].concat(),
        ["{:", "?}"].concat(),
        ["{:#", "?}"].concat(),
        ["format", "!"].concat(),
        ["write", "!"].concat(),
        ["writeln", "!"].concat(),
        ["pay", "load"].concat(),
        ["loca", "tion"].concat(),
        ["to_", "string"].concat(),
    ];
    let mut checked = 0;
    for word in &forbidden {
        assert!(
            !start.contains(word.as_str()),
            "src/start.rs holds {word}: the hook formats the panic"
        );
        checked += 1;
    }
    assert_eq!(checked, 9);
}

/// The names in `deny.toml`'s `[bans] deny` list.
fn denied(policy: &str) -> Vec<String> {
    let bans = policy.split("[bans]").nth(1).expect("deny.toml has a [bans] table");
    // Up to the next table.
    let bans = bans.split("\n[").next().unwrap_or(bans);
    let list = bans.split("deny = [").nth(1).expect("[bans] has a deny list");
    let list = list.split(']').next().unwrap_or(list);
    list.split("crate = \"")
        .skip(1)
        .map(|rest| rest.split('"').next().expect("a quoted name").to_owned())
        .collect()
}

/// Rule 8: no network crate in the helper's graph, by construction (the viewer contract §5.2).
#[test]
fn the_helper_links_no_network_crate() {
    let lock = include_str!("../Cargo.lock");
    let packages: Vec<&str> = lock
        .split("[[package]]")
        .skip(1)
        .map(|stanza| {
            stanza
                .lines()
                .find_map(|line| line.trim().strip_prefix("name = \"")?.strip_suffix('"'))
                .expect("every package has a name")
        })
        .collect();
    assert!(packages.len() > 50, "the lock was read: {} packages", packages.len());

    let denied = denied(include_str!("../deny.toml"));
    assert_eq!(denied.len(), 19, "every name of the deny list was parsed: {denied:?}");
    let mut checked = 0;
    for name in &denied {
        assert!(
            !packages.contains(&name.as_str()),
            "the helper links {name}, a network crate"
        );
        checked += 1;
    }
    assert_eq!(checked, 19);
}

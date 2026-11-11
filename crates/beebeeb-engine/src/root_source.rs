//! Scanner input after the lib.rs split (LD-0057 PR 2, spec §4.8): the text the source-reading tests
//! used to `include_str!("lib.rs")`. The crate root keeps `mod <child>;` for each child split out of
//! it; here each such line is replaced by that child's items, so `lib.rs` reads back in its old order
//! (the 19 children are contiguous slices of the old file) and a scanner's anchors land where they did.
//!
//! A child is a file beside lib.rs whose first line is `` //! `<child>` items split out of `lib.rs`. ``;
//! its first three lines (that header, a blank line, `use super::*;`) and the `pub(super) ` the split gave
//! a private item are not part of the old text.
//! Must not: be used by production code (it reads the source tree at run time).
//! Tests: every scanner that reads the crate root goes through [`lib_rs`]; a scanner that walks the tree reads
//! `lib.rs` as [`lib_rs`] and skips the children ([`lib_rs_child`]), so it reads each line once, where it read it before.

/// How many children the split moved out of `lib.rs`. A child whose header no longer matches would silently drop its
/// lines from every scanner's input, so the splice counts them (the TypeScript twin holds the same number).
const CHILDREN: usize = 19;

/// The spliced text and the children's file names, read once per test binary.
fn spliced() -> &'static (String, Vec<String>) {
    static SPLICED: std::sync::OnceLock<(String, Vec<String>)> = std::sync::OnceLock::new();
    SPLICED.get_or_init(|| {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/src");
        let hub = std::fs::read_to_string(src.join("lib.rs")).expect("read src/lib.rs");
        let mut out = String::with_capacity(hub.len() * 40);
        let mut children = Vec::new();
        for line in hub.split_inclusive('\n') {
            let child = line
                .trim_end_matches(['\r', '\n'])
                .strip_prefix("mod ")
                .and_then(|rest| rest.strip_suffix(';'));
            let text = child.and_then(|name| {
                let path = src.join(format!("{name}.rs"));
                let first = std::io::BufRead::lines(std::io::BufReader::new(std::fs::File::open(&path).ok()?))
                    .next()?
                    .ok()?;
                (first.trim_end() == format!("//! `{name}` items split out of `lib.rs`."))
                    .then(|| std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())))
            });
            match (child, text) {
                (Some(name), Some(text)) => {
                    children.push(format!("{name}.rs"));
                    // The split's only edit inside a child is a `pub(super) ` it gave a private item, field or method (no
                    // item of the crate root could carry `pub(super)` before), so dropping it gives back the old line.
                    let body: String = text
                        .split_inclusive('\n')
                        .skip(3)
                        .map(|line| {
                            let indent = line.len() - line.trim_start().len();
                            match line[indent..].strip_prefix("pub(super) ") {
                                Some(rest) => format!("{}{rest}", &line[..indent]),
                                None => line.to_string(),
                            }
                        })
                        .collect();
                    out.push_str(
                        body.strip_prefix("\r\n")
                            .or_else(|| body.strip_prefix('\n'))
                            .unwrap_or(&body),
                    );
                }
                _ => out.push_str(line),
            }
        }
        assert_eq!(
            children.len(),
            CHILDREN,
            "lib.rs splices {CHILDREN} children back in, found {children:?}"
        );
        (out, children)
    })
}

/// Whether `name` (a path under `src`, `/`-separated) is a child that [`lib_rs`] splices back into `lib.rs`.
// `tests/windows_session_wiring.rs` includes this file through `#[path]` and reads only `lib_rs`.
#[allow(dead_code)]
pub fn lib_rs_child(name: &str) -> bool {
    spliced().1.iter().any(|child| child == name)
}

/// `src-tauri/src/lib.rs` with each split-out child spliced back in at its `mod <child>;` line.
pub fn lib_rs() -> String {
    spliced().0.clone()
}

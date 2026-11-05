//! Scanner input after the lib.rs split (LD-0057 PR 2, spec §4.8): the text the source-reading tests
//! used to `include_str!("lib.rs")`. The crate root keeps `mod <child>;` for each child split out of
//! it; here each such line is replaced by that child's items, so `lib.rs` reads back in its old order
//! (the 19 children are contiguous slices of the old file) and a scanner's anchors land where they did.
//!
//! A child is a file beside lib.rs whose first line is `` //! `<child>` items split out of `lib.rs`. ``;
//! its first three lines (that header, a blank line, `use super::*;`) and the `pub(super) ` the split gave
//! a private item are not part of the old text.
//! Must not: be used by production code (it reads the source tree at run time).
//! Tests: every scanner that reads the crate root goes through [`lib_rs`].

/// `src-tauri/src/lib.rs` with each split-out child spliced back in at its `mod <child>;` line.
pub(crate) fn lib_rs() -> String {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let hub = std::fs::read_to_string(src.join("lib.rs")).expect("read src/lib.rs");
    let mut out = String::with_capacity(hub.len() * 40);
    for line in hub.split_inclusive('\n') {
        let child = line
            .trim_end_matches(['\r', '\n'])
            .strip_prefix("mod ")
            .and_then(|rest| rest.strip_suffix(';'));
        let text = child.and_then(|name| {
            let text = std::fs::read_to_string(src.join(format!("{name}.rs"))).ok()?;
            let header = format!("//! `{name}` items split out of `lib.rs`.");
            text.starts_with(&header).then_some(text)
        });
        match text {
            Some(text) => {
                // The split's only edit inside a child is a `pub(super) ` it gave a private item, field or
                // method (no item of the crate root could carry `pub(super)` before), so dropping it gives
                // back the old line.
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
                out.push_str(body.strip_prefix("\r\n").or_else(|| body.strip_prefix('\n')).unwrap_or(&body));
            }
            None => out.push_str(line),
        }
    }
    out
}

//! Test-only helper for the tests that read this crate's own source text.
//!
//! Some tests pin a property of the code by looking for a snippet in the source ("this function calls X before Y").
//! rustfmt is the canonical layout of that source (`cargo fmt --check` is a gate), so a snippet written on one line
//! stops matching the day rustfmt wraps the call over several lines. [`squeeze`] makes both sides of such a
//! comparison independent of the layout: `f(a, b)` and `f(\n    a,\n    b,\n)` squeeze to the same text.
//!
//! It is applied to source text in tests only, never to anything a person sees.

/// `text` without any whitespace, and without the comma rustfmt puts before a closing bracket when it puts a list on
/// one line per item.
pub fn squeeze(text: &str) -> String {
    let mut squeezed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    for closer in [")", "]", "}"] {
        squeezed = squeezed.replace(&format!(",{closer}"), closer);
    }
    squeezed
}

/// The text from `start` to the first `end` after it.
pub fn body_between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let from = source.find(start).unwrap_or_else(|| panic!("{start} exists"));
    let to = from
        + source[from..]
            .find(end)
            .unwrap_or_else(|| panic!("{start} ends with {end:?}"));
    &source[from..to]
}

/// `source` without the items, blocks and statements whose `cfg` attribute stands alone on its
/// line and starts with one of `attrs` (at any indentation). What an attribute covers ends at
/// the first line that ends with `;`, or that opens and closes its braces on one line (`fn f() {}`), or, when a
/// line ending with `{` comes first, at the `}`
/// that closes it at the attribute's own indentation (`} else {` continues it). An attribute
/// on an argument or field (the next line ends with `,`) covers that one line.
pub fn without_items(source: &str, attrs: &[&str]) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut kept = String::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if attrs
            .iter()
            .any(|attr| trimmed.starts_with(attr) && trimmed.trim_end().ends_with(']'))
        {
            let indent = &lines[i][..lines[i].len() - trimmed.len()];
            let mut j = i + 1;
            while j < lines.len()
                && (lines[j].trim_start().starts_with("#[") || lines[j].trim_start().starts_with("///"))
            {
                j += 1;
            }
            if lines.get(j).is_some_and(|line| line.trim_end().ends_with(',')) {
                i = j + 1;
                continue;
            }
            let mut k = j;
            while k < lines.len() {
                let line = lines[k].trim_end();
                if line.ends_with(';') {
                    break;
                }
                // A one-line item (`fn f() {}`): its braces close on the line it starts on (fix round 1, m1).
                if line.ends_with('}') && line.contains('{') && line.matches('{').count() == line.matches('}').count() {
                    break;
                }
                if line.ends_with('{') {
                    let close = format!("{indent}}}");
                    while k < lines.len() && lines[k].trim_end() != close && lines[k].trim_end() != format!("{close};")
                    {
                        k += 1;
                    }
                    break;
                }
                k += 1;
            }
            i = k + 1;
            continue;
        }
        kept.push_str(lines[i]);
        kept.push('\n');
        i += 1;
    }
    kept
}

/// `source` as the macOS build compiles it: without the test modules and without what is
/// Windows, Linux or "not macOS" only, so a census of call sites sees only code a Mac can run.
pub fn macos_compiled(source: &str) -> String {
    without_items(
        source,
        &[
            "#[cfg(test)]",
            "#[cfg(all(test",
            "#[cfg(any(test, feature = \"test-support\"))]",
            "#[cfg(all(any(test, feature = \"test-support\")",
            "#[cfg(any(target_os = \"windows\", test))]",
            "#[cfg(not(target_os = \"macos\"))]",
            "#[cfg(target_os = \"windows\")]",
            "#[cfg(target_os = \"linux\")]",
            "#[cfg(not(unix))]",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::squeeze;

    #[test]
    fn a_call_wrapped_over_lines_squeezes_to_its_one_line_form() {
        assert_eq!(squeeze("f(a, b)"), squeeze("f(\n    a,\n    b,\n)"));
        assert_eq!(
            squeeze("x.lock().unwrap_or_else(|p| p.into_inner())"),
            squeeze("x\n    .lock()\n    .unwrap_or_else(|p| p.into_inner())")
        );
        assert_eq!(squeeze("[1, 2]"), squeeze("[\n    1,\n    2,\n]"));
    }

    #[test]
    fn calls_that_differ_stay_different() {
        assert_ne!(squeeze("f(a, b)"), squeeze("f(b, a)"));
        assert_ne!(squeeze("f(a)"), squeeze("g(a)"));
        assert_ne!(squeeze("f(a, b)"), squeeze("f(a)"));
    }
}

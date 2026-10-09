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
pub(crate) fn squeeze(text: &str) -> String {
    let mut squeezed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    for closer in [")", "]", "}"] {
        squeezed = squeezed.replace(&format!(",{closer}"), closer);
    }
    squeezed
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

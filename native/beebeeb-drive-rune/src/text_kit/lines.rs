//! Line starts and line lookup over a byte-indexed text.

/// Byte offset of each line's first byte. Always at least `[0]`; a trailing newline adds a final empty line.
pub fn line_starts(text: &str) -> Vec<usize> {
    let mut v = vec![0];
    v.extend(
        text.bytes()
            .enumerate()
            .filter(|(_, b)| *b == b'\n')
            .map(|(i, _)| i + 1),
    );
    v
}

/// The line (0-based) holding `byte`. A byte past the end is on the last line.
pub fn line_of(starts: &[usize], byte: usize) -> usize {
    starts.partition_point(|&s| s <= byte).saturating_sub(1)
}

/// Digits the line-number gutter needs: at least 2.
#[cfg(test)]
pub fn gutter_digits(line_count: usize) -> usize {
    line_count.max(1).to_string().len().max(2)
}

#[cfg(test)]
#[path = "lines_tests.rs"]
mod tests;

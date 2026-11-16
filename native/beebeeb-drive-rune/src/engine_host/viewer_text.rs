//! The text and Markdown a viewer shows: the decrypted bytes as the lines the view draws, moved out of `viewer.rs`
//! unchanged (that file is at its line limit).

use super::*;

/// The decrypted bytes as text the view draws, or `None` when they are not text (not UTF-8, or a NUL in them).
/// A text the reader cannot hold whole (over [`READ_WHOLE_MAX`], more than [`WHOLE_LINES_MAX`] lines, or a line
/// over [`LINE_BYTES_MAX`]) is its first lines, read-only: at most [`SHOWN_LINES`] and [`SHOWN_BYTES`].
pub(super) fn text_body(bytes: &[u8], markdown: bool) -> Option<Body> {
    let text = std::str::from_utf8(bytes).ok()?;
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    if text.contains('\0') {
        return None;
    }
    let (mut total, mut longest) = (0usize, 0usize);
    for line in text.lines() {
        total += 1;
        longest = longest.max(line.len());
    }
    let why = if bytes.len() > READ_WHOLE_MAX {
        Some(Cut::OverTwoMb)
    } else if total > WHOLE_LINES_MAX || longest > LINE_BYTES_MAX {
        Some(Cut::TooLong)
    } else {
        None
    };
    if let Some(why) = why {
        let lines = first_lines(text);
        return Some(Body::Text(TextDoc {
            truncated: Some(Truncated {
                shown: u32::try_from(lines.len()).unwrap_or(u32::MAX),
                total: u32::try_from(total).unwrap_or(u32::MAX),
                why,
            }),
            lines,
        }));
    }
    Some(if markdown && bytes.len() <= MARKDOWN_RENDER_MAX {
        Body::Markdown(MdDoc {
            source: text.to_string(),
        })
    } else {
        Body::Text(TextDoc {
            lines: plain_lines(text),
            truncated: None,
        })
    })
}

/// The first lines of a text shown in part: at most [`SHOWN_LINES`] of them, each cut at [`LINE_BYTES_MAX`]
/// (on a character), and at most [`SHOWN_BYTES`] in all. No line the reader is handed is longer than the limit
/// it is said to hold.
fn first_lines(text: &str) -> Vec<Vec<crate::frames::viewers::source::Run>> {
    let cut = |line: &str, at_most: usize| {
        let mut end = line.len().min(at_most);
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        end
    };
    // Sized once: a buffer that grew would leave its earlier copies of the text behind, not overwritten.
    let mut shown = String::with_capacity(SHOWN_BYTES);
    for (i, line) in text.lines().take(SHOWN_LINES).enumerate() {
        let room = SHOWN_BYTES.saturating_sub(shown.len() + usize::from(i > 0));
        if room == 0 && i > 0 {
            break;
        }
        if i > 0 {
            shown.push('\n');
        }
        shown.push_str(&line[..cut(line, LINE_BYTES_MAX.min(room))]);
    }
    let lines = plain_lines(&shown);
    shown.zeroize();
    lines
}

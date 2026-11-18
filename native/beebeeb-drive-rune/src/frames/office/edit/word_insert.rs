//! The Insert tab's tools of a text document (a later task fills this).
#![allow(dead_code)]

// What a table, a link and a picture are asked for, checked and turned into the editor's calls. The drawing and
// the presses are `viewers/office_over.rs` (its `Over` holds the dialogs); this file holds no window: the forms'
// state, the words a refusal says, the bounds, and the picture's way from the platform's file picker to bytes.
//
// Nothing here opens an address: a link is text in the document, never followed from this window. A picked
// picture's path is never logged and never kept; its bytes are read once, on the picker's worker, into memory that
// is wiped when dropped, and travel on to `ViewerSource::insert_image`.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use beebeeb_office_protocol::check;
use beebeeb_office_protocol::office::EditOp;
use rune::{El, TextEdit};
use zeroize::Zeroizing;

use crate::shell::platform::PlatformError;
use crate::theme::tokens::Theme;

/// The most rows a table is asked for (the protocol has no bound of its own).
pub const MAX_ROWS: u32 = 100;
/// The most columns a table is asked for.
pub const MAX_COLS: u32 = 20;
/// A table starts as this many rows and columns.
pub const START_DIM: &str = "3";

pub const NOT_DONE: &str = "That can't be done right now.";
pub const NOT_ADDED: &str = "Couldn't add that picture.";
pub const NO_PICKER: &str = "Couldn't open the file picker on this device.";
pub const NOT_AN_IMAGE: &str = "Only PNG and JPEG pictures can be added. Save it as one of those and try again.";
pub const EMPTY_FILE: &str = "That file is empty.";
pub const UNREADABLE: &str = "Couldn't read that file.";
pub const NO_ADDRESS: &str = "Enter an address.";
pub const BAD_ADDRESS: &str = "An address is one line, without control characters.";
pub const BAD_WORDS: &str = "The text is one line, without control characters.";
pub const TOO_LONG: &str = "That is too long. Text and address can be at most 32 KB each.";

/// The Insert tab's tools, in the board's order (DW-1550): the ones `office_over` draws and answers.
pub fn tools(t: &Theme) -> Vec<El> {
    crate::frames::viewers::office_over::insert_tools(t)
}

/// What Insert's tools do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Insert {
    Table,
    Image,
    Link,
    PageBreak,
}

/// Which field of a form an error belongs to (the first field is 0), and the words.
pub type Why = (usize, String);

/// Table...: rows and columns, as typed.
pub struct TableForm {
    pub rows: TextEdit,
    pub cols: TextEdit,
    pub error: Option<Why>,
}

impl TableForm {
    pub fn new() -> Self {
        let mut rows = TextEdit::new(START_DIM);
        rows.set_selection(0, START_DIM.len());
        rows.focused = true;
        TableForm {
            rows,
            cols: TextEdit::new(START_DIM),
            error: None,
        }
    }
}

/// Link...: the words shown, and the address.
pub struct LinkForm {
    pub text: TextEdit,
    pub url: TextEdit,
    pub error: Option<Why>,
}

impl LinkForm {
    pub fn new() -> Self {
        let mut text = TextEdit::new("");
        text.focused = true;
        LinkForm {
            text,
            url: TextEdit::new(""),
            error: None,
        }
    }
}

fn dim(typed: &str, most: u32, name: &str) -> Result<u32, String> {
    match typed.trim().parse::<u32>() {
        Ok(n) if (1..=most).contains(&n) => Ok(n),
        _ => Err(format!("{name}: a whole number from 1 to {most}.")),
    }
}

/// The table the two fields ask for, or the field and the words that say what is wrong.
pub fn table_op(rows: &str, cols: &str) -> Result<EditOp, Why> {
    let rows = dim(rows, MAX_ROWS, "Rows").map_err(|words| (0, words))?;
    let cols = dim(cols, MAX_COLS, "Columns").map_err(|words| (1, words))?;
    Ok(EditOp::Table { rows, cols })
}

/// The link the two fields ask for. No words: the address stands in for them. An address is only text, so none
/// is judged by what it points to; only an empty one, a long one, or one with line breaks is refused.
pub fn link_op(text: &str, url: &str) -> Result<EditOp, Why> {
    let url = url.trim();
    if url.is_empty() {
        return Err((1, NO_ADDRESS.into()));
    }
    if url.chars().any(char::is_control) {
        return Err((1, BAD_ADDRESS.into()));
    }
    let text = text.trim();
    if text.chars().any(char::is_control) {
        return Err((0, BAD_WORDS.into()));
    }
    let text = if text.is_empty() { url } else { text };
    if url.len() > check::MAX_OP_TEXT_BYTES {
        return Err((1, TOO_LONG.into()));
    }
    if text.len() > check::MAX_OP_TEXT_BYTES {
        return Err((0, TOO_LONG.into()));
    }
    Ok(EditOp::Link {
        text: text.to_string(),
        url: url.to_string(),
    })
}

/// A picture read from the picked file: PNG, else JPEG. The bytes are wiped when dropped.
pub struct Image {
    pub bytes: Zeroizing<Vec<u8>>,
    pub png: bool,
}

/// PNG (`true`) or JPEG (`false`) by the file's first bytes, never by its name. Anything else (WebP, GIF, BMP, a
/// renamed file): `None`.
pub fn sniff(head: &[u8]) -> Option<bool> {
    if head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(true)
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(false)
    } else {
        None
    }
}

/// The words for a picture over the protocol's input cap.
pub fn too_large(cap: u64) -> String {
    format!("That picture is too large to add: at most {} MB.", cap / (1024 * 1024))
}

/// Read the picture at `path`, refusing in words: over `cap`, empty, not PNG or JPEG, unreadable. The size is read
/// before the file is, and the file's first bytes before the rest.
pub fn read_image_capped(path: &Path, cap: u64) -> Result<Image, String> {
    let mut file = File::open(path).map_err(|_| UNREADABLE.to_string())?;
    let meta = file.metadata().map_err(|_| UNREADABLE.to_string())?;
    if !meta.is_file() {
        return Err(UNREADABLE.into());
    }
    let len = meta.len();
    if len == 0 {
        return Err(EMPTY_FILE.into());
    }
    if len > cap {
        return Err(too_large(cap));
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(len as usize));
    let mut head = [0u8; 8];
    let got = read_up_to(&mut file, &mut head).map_err(|_| UNREADABLE.to_string())?;
    let Some(png) = sniff(&head[..got]) else {
        return Err(NOT_AN_IMAGE.into());
    };
    bytes.extend_from_slice(&head[..got]);
    // The rest, never more than the cap (a file that grew meanwhile).
    file.take(cap + 1 - got as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| UNREADABLE.to_string())?;
    if bytes.len() as u64 > cap {
        return Err(too_large(cap));
    }
    Ok(Image { bytes, png })
}

/// [`read_image_capped`] at the protocol's input cap.
pub fn read_image(path: &Path) -> Result<Image, String> {
    read_image_capped(path, check::MAX_INPUT_BYTES)
}

fn read_up_to(file: &mut File, into: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < into.len() {
        match file.read(&mut into[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// What the picker's worker handed back.
pub enum Answer {
    /// The picker was closed without a file.
    Closed,
    Image(Image),
    /// Why there is no picture to add (words for the window).
    Refused(String),
}

/// A picker that is open: its worker answers once.
pub struct Picking {
    answer: mpsc::Receiver<Answer>,
}

/// What looking for the answer found.
pub enum Looked {
    Waiting,
    Got(Answer),
}

impl Picking {
    /// Open the picker on a worker of its own (it BLOCKS until the person closes it): `pick` gives the file, which
    /// the worker reads at once. `None`: the worker did not start.
    pub fn start(pick: impl FnOnce() -> Result<Option<PathBuf>, PlatformError> + Send + 'static) -> Option<Picking> {
        let (send, answer) = mpsc::channel();
        std::thread::Builder::new()
            .name("image-picker".into())
            .spawn(move || {
                let done = match pick() {
                    Ok(None) => Answer::Closed,
                    Ok(Some(path)) => match read_image(&path) {
                        Ok(image) => Answer::Image(image),
                        Err(words) => Answer::Refused(words),
                    },
                    Err(_) => Answer::Refused(NO_PICKER.into()),
                };
                let _ = send.send(done);
            })
            .ok()?;
        Some(Picking { answer })
    }

    /// A picker answered by hand (tests).
    #[cfg(test)]
    pub fn answered(answer: Answer) -> Picking {
        let (send, receive) = mpsc::channel();
        let _ = send.send(answer);
        Picking { answer: receive }
    }

    pub fn look(&self) -> Looked {
        match self.answer.try_recv() {
            Ok(answer) => Looked::Got(answer),
            Err(mpsc::TryRecvError::Empty) => Looked::Waiting,
            // The worker ended without an answer.
            Err(mpsc::TryRecvError::Disconnected) => Looked::Got(Answer::Refused(NO_PICKER.into())),
        }
    }
}

#[cfg(test)]
#[path = "word_insert_tests.rs"]
mod tests;

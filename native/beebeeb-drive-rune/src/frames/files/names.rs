//! The rules for a file's or a folder's name (DW-0410, DW-0410b), the same for Rename, New folder
//! and a move into a folder that already has the name.
//!
//! The board's rule is the floor on every PC and Mac: no `/`, no `:`, not empty, at most 255
//! bytes, not a name the folder has. A name must also fit on the disk it is typed at, or the
//! item would be renamed in Beebeeb and never arrive in the folder on this PC: on Windows that
//! adds what File Explorer refuses ([`Host::Windows`]). The engine asks core the same question
//! when it writes the file (`beebeeb_core::path_safety::check_component_for`, its `windows`
//! half): this is that list, said before the call is made, in words a person can act on.

/// Whose disk a typed name has to fit on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    /// Windows: also no `\ < > " | ? *`, no reserved device name (`CON`, `NUL`, `COM1`, …), and
    /// dots at the end are dropped, as File Explorer drops them.
    Windows,
    /// A Mac (or anything else): the board's rule alone.
    Other,
}

impl Host {
    /// The computer the app runs on.
    pub const fn this() -> Host {
        if cfg!(target_os = "windows") {
            Host::Windows
        } else {
            Host::Other
        }
    }
}

/// Why a name cannot be given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameError {
    /// Nothing but spaces (on Windows: nothing but spaces and dots).
    Empty,
    /// A `/` or a `:` in it; on Windows also `\ < > " | ? *`.
    Character(char),
    /// A character nobody sees: a control character (a tab, a line break, NUL) or one that
    /// turns the reading direction round, which can make a name read as another name.
    Hidden,
    /// `.` or `..`: the folder itself and the one above, on every disk.
    Dots,
    /// A name Windows keeps for a device (`CON`, `NUL`, `COM1`, …), with or without an extension.
    Reserved(String),
    /// More than 255 bytes.
    TooLong,
    /// Another item of the folder has it (compared without case). `folder`: the other one is a folder.
    Taken { name: String, folder: bool },
}

impl NameError {
    /// What the window says under the field. A name already used: DW-0410b's sentence.
    pub fn message(&self) -> String {
        match self {
            NameError::Empty => "A name can't be empty.".to_string(),
            NameError::Character(c) => format!("A name can't contain {c}."),
            NameError::Hidden => "A name can't contain a character that isn't shown.".to_string(),
            NameError::Dots => "A name can't be only dots.".to_string(),
            NameError::Reserved(name) => format!("Windows keeps the name {name} for itself. Choose another name."),
            NameError::TooLong => "That name is too long.".to_string(),
            NameError::Taken { name, folder } => format!(
                "There's already a {} called {name} here. Choose another name.",
                if *folder { "folder" } else { "file" }
            ),
        }
    }
}

/// The most bytes a name may have.
pub const MAX_BYTES: usize = 255;

/// The device names Windows keeps (core's `WINDOWS_RESERVED`), compared without ASCII case
/// against the part before the first dot.
const RESERVED: [&str; 32] = [
    "CON",
    "PRN",
    "AUX",
    "NUL",
    "CONIN$",
    "CONOUT$",
    "COM0",
    "COM1",
    "COM2",
    "COM3",
    "COM4",
    "COM5",
    "COM6",
    "COM7",
    "COM8",
    "COM9",
    "LPT0",
    "LPT1",
    "LPT2",
    "LPT3",
    "LPT4",
    "LPT5",
    "LPT6",
    "LPT7",
    "LPT8",
    "LPT9",
    // Windows also keeps the superscript spellings (U+00B9, U+00B2, U+00B3).
    "COM\u{b9}",
    "COM\u{b2}",
    "COM\u{b3}",
    "LPT\u{b9}",
    "LPT\u{b2}",
    "LPT\u{b3}",
];

/// The device name a typed name would be on Windows: `nul.txt` → `NUL`.
fn reserved(name: &str) -> Option<&'static str> {
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    RESERVED.iter().copied().find(|r| r.eq_ignore_ascii_case(stem))
}

/// A character that turns the reading direction round (core's `is_bidi_control`).
fn turns_direction(c: char) -> bool {
    matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// The name as it would be stored (without the spaces around it; on Windows without dots at the
/// end either), or why it cannot be. `others`: the names the folder already holds, without the
/// item being renamed; each with whether it is a folder.
pub fn check_on<'a>(
    host: Host,
    typed: &str,
    others: impl IntoIterator<Item = (&'a str, bool)>,
) -> Result<String, NameError> {
    let name = match host {
        // File Explorer drops spaces and dots at the end without a word; so does this.
        Host::Windows => typed.trim().trim_end_matches(['.', ' ']),
        Host::Other => typed.trim(),
    };
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name == "." || name == ".." {
        return Err(NameError::Dots);
    }
    let refused = |c: char| match host {
        Host::Windows => matches!(c, '/' | ':' | '\\' | '<' | '>' | '"' | '|' | '?' | '*'),
        Host::Other => matches!(c, '/' | ':'),
    };
    if let Some(c) = name.chars().find(|c| refused(*c)) {
        return Err(NameError::Character(c));
    }
    let hidden = |c: char| match host {
        Host::Windows => c.is_control() || turns_direction(c),
        Host::Other => c == '\0',
    };
    if name.chars().any(hidden) {
        return Err(NameError::Hidden);
    }
    if host == Host::Windows
        && let Some(device) = reserved(name)
    {
        return Err(NameError::Reserved(device.to_string()));
    }
    if name.len() > MAX_BYTES {
        return Err(NameError::TooLong);
    }
    match taken(name, others) {
        Some(taken) => Err(taken),
        None => Ok(name.to_string()),
    }
}

/// Whether the folder already has this name (compared without case): the one rule a name that
/// EXISTS is held to when it moves into another folder. What else this PC cannot hold is the
/// engine's to answer for an item that was named on another computer.
pub fn taken<'a>(name: &str, others: impl IntoIterator<Item = (&'a str, bool)>) -> Option<NameError> {
    let lower = name.to_lowercase();
    others
        .into_iter()
        .find(|(other, _)| other.to_lowercase() == lower)
        .map(|(taken, folder)| NameError::Taken {
            name: taken.to_string(),
            folder,
        })
}

/// Where the extension starts: `Contract draft.pdf` → 14, so typing keeps `.pdf` (DW-0410).
/// A folder, a name without a dot and a name that is only an extension (`.env`) have none.
pub fn stem_len(name: &str, is_folder: bool) -> usize {
    if is_folder {
        return name.len();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => dot,
        _ => name.len(),
    }
}

/// "Untitled folder", or "Untitled folder 2" when the folder has one (DW-0417).
pub fn untitled_folder<'a>(others: impl IntoIterator<Item = &'a str> + Clone) -> String {
    let taken = |candidate: &str| {
        let lower = candidate.to_lowercase();
        others.clone().into_iter().any(|other| other.to_lowercase() == lower)
    };
    let first = "Untitled folder".to_string();
    if !taken(&first) {
        return first;
    }
    (2..)
        .map(|n| format!("Untitled folder {n}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_trimmed_and_checked_against_the_four_rules() {
        let others = [("Contract draft.pdf", false), ("Documents", true)];
        // These hold on every computer.
        let check = |typed: &str, others: [(&'static str, bool); 2]| check_on(Host::this(), typed, others);
        assert_eq!(check("  Plan.pdf ", others), Ok("Plan.pdf".to_string()));
        assert_eq!(check("   ", others), Err(NameError::Empty));
        assert_eq!(check("a/b", others), Err(NameError::Character('/')));
        assert_eq!(check("a:b", others), Err(NameError::Character(':')));
        assert_eq!(check(&"x".repeat(256), others), Err(NameError::TooLong));
        assert_eq!(check(&"x".repeat(255), others), Ok("x".repeat(255)));
        // 128 two-byte letters are 256 bytes.
        assert_eq!(check(&"é".repeat(128), others), Err(NameError::TooLong));
    }

    #[test]
    fn on_windows_a_name_is_one_file_explorer_would_take() {
        let none: [(&str, bool); 0] = [];
        let on = |typed: &str| check_on(Host::Windows, typed, none);
        // The review's name: it would be renamed in Beebeeb and never reach the folder on this PC.
        assert_eq!(on("budget|Q4.pdf"), Err(NameError::Character('|')));
        for c in ['/', ':', '\\', '<', '>', '"', '|', '?', '*'] {
            assert_eq!(on(&format!("a{c}b")), Err(NameError::Character(c)), "{c}");
        }
        assert_eq!(on("a\\b").unwrap_err().message(), "A name can't contain \\.");
        // A device name, in any case, with or without an extension; a longer word is a name.
        assert_eq!(on("nul.txt"), Err(NameError::Reserved("NUL".into())));
        assert_eq!(on("COM1"), Err(NameError::Reserved("COM1".into())));
        assert_eq!(on("Aux .pdf"), Err(NameError::Reserved("AUX".into())));
        assert_eq!(
            on("con").unwrap_err().message(),
            "Windows keeps the name CON for itself. Choose another name."
        );
        assert_eq!(on("Console.log"), Ok("Console.log".to_string()));
        assert_eq!(on("COM10"), Ok("COM10".to_string()));
        // Dots and spaces at the end are dropped, as File Explorer drops them.
        assert_eq!(on(" Plan.pdf. . "), Ok("Plan.pdf".to_string()));
        assert_eq!(on(" ... "), Err(NameError::Empty));
        assert_eq!(on("..."), Err(NameError::Empty));
        assert_eq!(on(".env"), Ok(".env".to_string()));
        // What nobody sees: a tab, a line break, a character that turns the reading round.
        assert_eq!(on("a\tb"), Err(NameError::Hidden));
        assert_eq!(on("a\nb"), Err(NameError::Hidden));
        assert_eq!(on("evil\u{202e}fdp.exe"), Err(NameError::Hidden));
        // The name left after the dots are dropped is the one compared and stored.
        assert_eq!(
            check_on(Host::Windows, "documents.", [("Documents", true)]),
            Err(NameError::Taken {
                name: "Documents".into(),
                folder: true
            })
        );
        // Every sentence ends as a sentence and names no character that is not shown.
        for why in [NameError::Hidden, NameError::Dots, NameError::Reserved("NUL".into())] {
            let said = why.message();
            assert!(said.ends_with('.') && said.chars().all(|c| !c.is_control()), "{said}");
        }
    }

    #[test]
    fn on_a_mac_the_boards_rule_stands_alone() {
        let none: [(&str, bool); 0] = [];
        let on = |typed: &str| check_on(Host::Other, typed, none);
        assert_eq!(on("budget|Q4.pdf"), Ok("budget|Q4.pdf".to_string()));
        assert_eq!(on("nul.txt"), Ok("nul.txt".to_string()));
        assert_eq!(on("Plan."), Ok("Plan.".to_string()));
        assert_eq!(on("a/b"), Err(NameError::Character('/')));
        assert_eq!(on("a:b"), Err(NameError::Character(':')));
        assert_eq!(on("."), Err(NameError::Dots));
        assert_eq!(on(".."), Err(NameError::Dots));
        assert_eq!(on("a\0b"), Err(NameError::Hidden));
    }

    #[test]
    fn a_name_that_exists_is_only_asked_whether_the_folder_has_it() {
        // An item named on a Mac moves on this PC: the dialog says a taken name, nothing else.
        assert_eq!(taken("budget|Q4.pdf", [("Documents", true)]), None);
        assert_eq!(
            taken("DOCUMENTS", [("Documents", true)]),
            Some(NameError::Taken {
                name: "Documents".into(),
                folder: true
            })
        );
    }

    #[test]
    fn a_taken_name_compares_without_case_and_says_the_boards_sentence() {
        let others = [("Contract draft.pdf", false), ("Documents", true)];
        let check = |typed: &str, others: [(&'static str, bool); 2]| check_on(Host::this(), typed, others);
        let taken = check("contract DRAFT.pdf", others).unwrap_err();
        assert_eq!(
            taken.message(),
            "There's already a file called Contract draft.pdf here. Choose another name."
        );
        assert_eq!(
            check("documents", others).unwrap_err().message(),
            "There's already a folder called Documents here. Choose another name."
        );
    }

    #[test]
    fn the_stem_leaves_the_extension_out() {
        assert_eq!(stem_len("Contract draft.pdf", false), "Contract draft".len());
        assert_eq!(stem_len("archive.tar.gz", false), "archive.tar".len());
        assert_eq!(stem_len("README", false), 6);
        assert_eq!(stem_len(".env", false), 4);
        assert_eq!(stem_len("Lisbon v1.2", true), 11);
    }

    #[test]
    fn an_untitled_folder_gets_the_next_free_number() {
        assert_eq!(untitled_folder(["Documents"]), "Untitled folder");
        assert_eq!(untitled_folder(["untitled FOLDER"]), "Untitled folder 2");
        assert_eq!(
            untitled_folder(["Untitled folder", "Untitled folder 2"]),
            "Untitled folder 3"
        );
    }
}

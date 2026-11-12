//! The rules for a file's or a folder's name (DW-0410, DW-0410b), the same for Rename, New folder
//! and a move into a folder that already has the name.

/// Why a name cannot be given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameError {
    /// Nothing but spaces.
    Empty,
    /// A `/` or a `:` in it.
    Character(char),
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

/// The name as it would be stored (without the spaces around it), or why it cannot be.
/// `others`: the names the folder already holds, without the item being renamed; each with
/// whether it is a folder.
pub fn check<'a>(typed: &str, others: impl IntoIterator<Item = (&'a str, bool)>) -> Result<String, NameError> {
    let name = typed.trim();
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if let Some(c) = name.chars().find(|c| matches!(c, '/' | ':')) {
        return Err(NameError::Character(c));
    }
    if name.len() > MAX_BYTES {
        return Err(NameError::TooLong);
    }
    let lower = name.to_lowercase();
    if let Some((taken, folder)) = others.into_iter().find(|(other, _)| other.to_lowercase() == lower) {
        return Err(NameError::Taken {
            name: taken.to_string(),
            folder,
        });
    }
    Ok(name.to_string())
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
    fn a_taken_name_compares_without_case_and_says_the_boards_sentence() {
        let others = [("Contract draft.pdf", false), ("Documents", true)];
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

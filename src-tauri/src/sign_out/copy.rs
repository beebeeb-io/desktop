//! Spec 2026-10-10 §11 (task 1887): the strings Rust shows or returns. Typographic apostrophes, the one-character
//! ellipsis, no emoji. `src/signOutCopy.ts` mirrors them; `tests/signOutCopy.test.ts` pins the two equal.
// Most of these strings are the frontend's: `src/signOutCopy.ts` shows them, and its test reads them from this file.
// Rust itself shows only the alert (K0, K1), K3 and the errors, so outside the tests the rest have no Rust caller. The
// tests use every one (U24 checks the typography of all of them).
#![cfg_attr(not(test), allow(dead_code))]

pub const UNSENT_CHANGES_CODE: &str = "unsent_changes";
pub const UNSENT_COPY_FAILED_CODE: &str = "unsent_copy_failed";
pub const UNSENT_COUNT_FAILED_CODE: &str = "unsent_count_failed";

pub const E1_UNSENT_CHANGES: &str =
    "Some changes haven’t reached the server yet. Sign out from Settings › Account to send or keep them first.";
pub const E2_UNSENT_COPY_FAILED: &str = "Beebeeb couldn’t copy the changes that haven’t reached the server, so it didn’t sign out. They’re still waiting to upload. Try again; if it keeps happening, restart Beebeeb.";
pub const E3_UNSENT_COUNT_FAILED: &str = "Beebeeb couldn’t check for changes that haven’t reached the server, so it changed nothing. Try again. If it keeps happening, restart Beebeeb.";

/// §11: `<code>: <sentence>`. A caller matches the code, never the sentence.
pub fn coded(code: &str, sentence: &str) -> String {
    format!("{code}: {sentence}")
}

pub const P0_TITLE: &str = "Sending changes first";
pub const P1_ONE: &str = "Sending 1 change before signing out…";
pub const P1_MANY: &str = "Sending {n} changes before signing out…";
pub const P2_HINT: &str = "This takes at most a minute.";
pub const P3_STOP: &str = "Stop waiting";

pub const A0_TITLE: &str = "Sign out with changes not sent?";
pub const A0S_TITLE: &str = "Switch accounts with changes not sent?";
pub const A1S_LINE: &str =
    "This Mac is signed in to another Beebeeb account. Switching signs that account out of this Mac.";
pub const A2_ONE: &str = "1 file you changed in Finder hasn’t reached the server.";
pub const A2_MANY: &str = "{n} files you changed in Finder haven’t reached the server.";
pub const A2_COPY_IT: &str = "Before signing out, Beebeeb copies it to a folder on this Mac and shows you where.";
pub const A2_COPY_THEM: &str = "Before signing out, Beebeeb copies them to a folder on this Mac and shows you where.";
pub const A2_COPY_SOME: &str =
    "Before signing out, Beebeeb copies {c} of them to a folder on this Mac and shows you where.";
pub const A2S_ONE: &str = "1 file changed in Finder while that account was signed in hasn’t reached the server.";
pub const A2S_MANY: &str = "{n} files changed in Finder while that account was signed in haven’t reached the server.";
pub const A2S_COPY_IT: &str = "Before switching, Beebeeb copies it to a folder on this Mac and shows you where.";
pub const A2S_COPY_THEM: &str = "Before switching, Beebeeb copies them to a folder on this Mac and shows you where.";
pub const A2S_COPY_SOME: &str =
    "Before switching, Beebeeb copies {c} of them to a folder on this Mac and shows you where.";
pub const A3_ONE_OF_ONE: &str = "Beebeeb’s copy of a change to that file is missing, so that change can’t be kept.";
pub const A3_ONE_OF_MANY: &str =
    "Beebeeb’s copy of a change to 1 of these files is missing, so that change can’t be kept.";
pub const A3_MANY: &str =
    "Beebeeb’s copies of changes to {k} of these files are missing, so those changes can’t be kept.";
pub const A3E_ONE: &str =
    "A sign-out that didn’t finish copied changes to this folder, so a change listed as missing may be there:";
pub const A3E_MANY: &str = "Sign-outs that didn’t finish copied changes to these folders, so a change listed as missing may be in one of them:";
pub const A4_ONE: &str = "1 other change (a rename, move, deletion, new folder or restore) hasn’t reached the server{either}. Signing out drops it, and that item stays as it is on the server.";
pub const A4_MANY: &str = "{m} other changes (renames, moves, deletions, new folders or restores) haven’t reached the server{either}. Signing out drops them, and those items stay as they are on the server.";
pub const A4S_ONE: &str = "1 other change (a rename, move, deletion, new folder or restore) hasn’t reached the server{either}. Switching drops it, and that item stays as it is on the server.";
pub const A4S_MANY: &str = "{m} other changes (renames, moves, deletions, new folders or restores) haven’t reached the server{either}. Switching drops them, and those items stay as they are on the server.";
pub const A5_OFFLINE: &str = "This Mac can’t reach the server right now.";
pub const A5_SESSION: &str = "The server no longer accepts this Mac’s sign-in.";
pub const A5_PAUSED: &str = "Sync is paused.";
pub const A5_HELD: &str = "These are on hold after earlier failures.";
pub const A5_ENGINE_STOPPED: &str = "Sync stopped before everything was sent.";
pub const A5_TIME_LIMIT: &str =
    "Sending took longer than a minute. Choose Cancel to let it finish, then sign out again.";
/// Spec §15 question 4 (answered): option A, this copy.
pub const A5_ACCOUNT: &str = "Beebeeb can’t accept uploads for this account until it has free space or an active plan.";
pub const ASK_CANCEL: &str = "Cancel";
pub const ASK_SIGN_OUT: &str = "Sign out";
pub const ASK_SWITCH: &str = "Sign out and switch";

pub const K0_TITLE: &str = "Files kept on this Mac";
pub const K1_ONE: &str =
    "Beebeeb copied 1 file with changes that hadn’t reached the server to this folder on this Mac:";
pub const K1_MANY: &str =
    "Beebeeb copied {n} files with changes that hadn’t reached the server to this folder on this Mac:";
pub const K2_ROW: &str = "Beebeeb copied changes that had not reached the server to this folder:";
pub const K2_SHOW: &str = "Show in Finder";
pub const K2_DISMISS: &str = "Dismiss";
pub const K3_TOAST: &str = "That folder isn’t on this Mac any more. It may have been moved or deleted.";

fn fill(template: &str, key: &str, value: usize) -> String {
    template.replace(key, &value.to_string())
}

/// P1. Never for 0 (W2): the phase ends first.
pub fn p1(n: usize) -> Option<String> {
    match n {
        0 => None,
        1 => Some(P1_ONE.to_string()),
        n => Some(fill(P1_MANY, "{n}", n)),
    }
}

/// A2 or A2s: the file count, then the copy sentence by c, left out for c = 0.
pub fn a2(n: usize, c: usize, switch: bool) -> Option<String> {
    let (one, many, it, them, some) = if switch {
        (A2S_ONE, A2S_MANY, A2S_COPY_IT, A2S_COPY_THEM, A2S_COPY_SOME)
    } else {
        (A2_ONE, A2_MANY, A2_COPY_IT, A2_COPY_THEM, A2_COPY_SOME)
    };
    let first = match n {
        0 => return None,
        1 => one.to_string(),
        n => fill(many, "{n}", n),
    };
    let second = match c {
        0 => None,
        c if c == n && n == 1 => Some(it.to_string()),
        c if c == n => Some(them.to_string()),
        c => Some(fill(some, "{c}", c)),
    };
    Some(match second {
        Some(second) => format!("{first} {second}"),
        None => first,
    })
}

/// A3, the same in the switch.
pub fn a3(n: usize, k: usize) -> Option<String> {
    match (n, k) {
        (_, 0) => None,
        (1, 1) => Some(A3_ONE_OF_ONE.to_string()),
        (_, 1) => Some(A3_ONE_OF_MANY.to_string()),
        (_, k) => Some(fill(A3_MANY, "{k}", k)),
    }
}

/// A3e's sentence; the frontend puts the paths under it, one per line.
pub fn a3e(folders: usize) -> Option<&'static str> {
    match folders {
        0 => None,
        1 => Some(A3E_ONE),
        _ => Some(A3E_MANY),
    }
}

/// A4 or A4s; " either" only when A2 is shown.
pub fn a4(m: usize, either: bool, switch: bool) -> Option<String> {
    let (one, many) = if switch { (A4S_ONE, A4S_MANY) } else { (A4_ONE, A4_MANY) };
    let text = match m {
        0 => return None,
        1 => one.to_string(),
        m => fill(many, "{m}", m),
    };
    Some(text.replace("{either}", if either { " either" } else { "" }))
}

/// K1, the alert paragraph for our folder.
pub fn k1(n: usize) -> Option<String> {
    match n {
        0 => None,
        1 => Some(K1_ONE.to_string()),
        n => Some(fill(K1_MANY, "{n}", n)),
    }
}

/// §9: one alert per sign-out. Ours first (K1, a blank line, our path), then a blank line and 1882's body verbatim.
/// Only macOS's: 1882's body byte for byte. Neither: no alert.
/// Task 2191 (D11): the Windows sign-out kept files of unknown content beside the sync folder.
pub const W0_TITLE: &str = "Files kept on this PC";
pub const W1_ONE: &str = "Beebeeb copied 1 file from before the last update to this folder on this PC, because it may hold changes the server doesn’t have. The copy isn’t encrypted:";
pub const W1_MANY: &str = "Beebeeb copied {n} files from before the last update to this folder on this PC, because they may hold changes the server doesn’t have. The copies aren’t encrypted:";

pub fn w1(n: usize) -> Option<String> {
    match n {
        0 => None,
        1 => Some(W1_ONE.to_string()),
        n => Some(fill(W1_MANY, "{n}", n)),
    }
}

/// Task 2191 (D11): the Windows sign-out's alert for the files of unknown content it kept.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn windows_kept_files_alert(path: &str, files: usize) -> Option<(&'static str, String)> {
    w1(files).map(|line| (W0_TITLE, format!("{line}\n\n{path}")))
}

pub fn sign_out_alert(ours: Option<(&str, usize)>, macos: Option<&str>) -> Option<(&'static str, String)> {
    let ours = ours.and_then(|(path, files)| k1(files).map(|line| format!("{line}\n\n{path}")));
    let macos = macos.map(crate::finder_removal::preserved_files_message);
    let body = match (ours, macos) {
        (None, None) => return None,
        (Some(ours), None) => ours,
        (None, Some(macos)) => macos,
        (Some(ours), Some(macos)) => format!("{ours}\n\n{macos}"),
    };
    Some((K0_TITLE, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Task 2191: the Windows kept-files lines.
    #[test]
    fn task_2191_the_windows_kept_files_lines() {
        assert_eq!(w1(0), None);
        assert!(w1(1).unwrap().starts_with("Beebeeb copied 1 file from before the last update"));
        assert!(w1(3).unwrap().contains("3 files") && w1(3).unwrap().contains("aren’t encrypted"));
        let (title, body) = windows_kept_files_alert("C:\\kept", 2).unwrap();
        assert_eq!(title, W0_TITLE);
        assert!(body.ends_with("\n\nC:\\kept"));
        assert_eq!(windows_kept_files_alert("C:\\kept", 0), None);
    }

    /// U24.
    #[test]
    fn no_count_string_renders_zero() {
        assert_eq!(p1(0), None);
        assert_eq!(p1(1).as_deref(), Some("Sending 1 change before signing out…"));
        assert_eq!(p1(3).as_deref(), Some("Sending 3 changes before signing out…"));
        assert_eq!(a2(0, 0, false), None);
        assert_eq!(
            a2(1, 1, false).as_deref(),
            Some(
                "1 file you changed in Finder hasn’t reached the server. Before signing out, Beebeeb copies it to a folder on this Mac and shows you where."
            )
        );
        assert_eq!(
            a2(3, 3, false).as_deref(),
            Some(
                "3 files you changed in Finder haven’t reached the server. Before signing out, Beebeeb copies them to a folder on this Mac and shows you where."
            )
        );
        assert_eq!(
            a2(3, 1, false).as_deref(),
            Some(
                "3 files you changed in Finder haven’t reached the server. Before signing out, Beebeeb copies 1 of them to a folder on this Mac and shows you where."
            )
        );
        assert_eq!(
            a2(2, 0, false).as_deref(),
            Some("2 files you changed in Finder haven’t reached the server."),
            "never \"copies 0 of them\""
        );
        assert_eq!(
            a2(1, 1, true).as_deref(),
            Some(
                "1 file changed in Finder while that account was signed in hasn’t reached the server. Before switching, Beebeeb copies it to a folder on this Mac and shows you where."
            )
        );
        assert_eq!(a3(1, 0), None);
        assert_eq!(a3(1, 1).as_deref(), Some(A3_ONE_OF_ONE));
        assert_eq!(a3(4, 1).as_deref(), Some(A3_ONE_OF_MANY));
        assert_eq!(
            a3(4, 2).as_deref(),
            Some("Beebeeb’s copies of changes to 2 of these files are missing, so those changes can’t be kept.")
        );
        assert_eq!(a3e(0), None);
        assert_eq!(a3e(1), Some(A3E_ONE));
        assert_eq!(a3e(2), Some(A3E_MANY));
        assert_eq!(a4(0, true, false), None);
        assert_eq!(
            a4(1, true, false).as_deref(),
            Some(
                "1 other change (a rename, move, deletion, new folder or restore) hasn’t reached the server either. Signing out drops it, and that item stays as it is on the server."
            )
        );
        assert_eq!(
            a4(2, false, true).as_deref(),
            Some(
                "2 other changes (renames, moves, deletions, new folders or restores) haven’t reached the server. Switching drops them, and those items stay as they are on the server."
            )
        );
        assert_eq!(k1(0), None);
        assert_eq!(
            k1(1).as_deref(),
            Some("Beebeeb copied 1 file with changes that hadn’t reached the server to this folder on this Mac:")
        );
        assert_eq!(
            k1(2).as_deref(),
            Some("Beebeeb copied 2 files with changes that hadn’t reached the server to this folder on this Mac:")
        );
        // Spec §11: all strings, every constant here.
        for text in [
            E1_UNSENT_CHANGES,
            E2_UNSENT_COPY_FAILED,
            E3_UNSENT_COUNT_FAILED,
            P0_TITLE,
            P1_ONE,
            P1_MANY,
            P2_HINT,
            P3_STOP,
            A0_TITLE,
            A0S_TITLE,
            A1S_LINE,
            A2_ONE,
            A2_MANY,
            A2_COPY_IT,
            A2_COPY_THEM,
            A2_COPY_SOME,
            A2S_ONE,
            A2S_MANY,
            A2S_COPY_IT,
            A2S_COPY_THEM,
            A2S_COPY_SOME,
            A3_ONE_OF_ONE,
            A3_ONE_OF_MANY,
            A3_MANY,
            A3E_ONE,
            A3E_MANY,
            A4_ONE,
            A4_MANY,
            A4S_ONE,
            A4S_MANY,
            A5_OFFLINE,
            A5_SESSION,
            A5_PAUSED,
            A5_HELD,
            A5_ENGINE_STOPPED,
            A5_TIME_LIMIT,
            A5_ACCOUNT,
            ASK_CANCEL,
            ASK_SIGN_OUT,
            ASK_SWITCH,
            K0_TITLE,
            K1_ONE,
            K1_MANY,
            K2_ROW,
            K2_SHOW,
            K2_DISMISS,
            K3_TOAST,
        ] {
            assert!(!text.contains('\''), "typographic apostrophes only: {text}");
            assert!(!text.contains("..."), "the one-character ellipsis: {text}");
        }
    }

    /// U23 (spec §9, the two-folder alert).
    #[test]
    fn one_alert_names_both_folders_ours_first() {
        use crate::finder_removal::{PRESERVED_FILES_SENTENCE, PRESERVED_FILES_TITLE, preserved_files_message};
        assert_eq!(K0_TITLE, PRESERVED_FILES_TITLE);
        assert_eq!(
            PRESERVED_FILES_SENTENCE, "Files that hadn’t reached your vault yet were kept on this Mac, in this folder:",
            "1882's sentence stays verbatim"
        );
        assert_eq!(sign_out_alert(None, None), None);
        assert_eq!(
            sign_out_alert(Some(("/ours", 2)), None),
            Some((K0_TITLE, format!("{}\n\n/ours", k1(2).unwrap())))
        );
        assert_eq!(
            sign_out_alert(None, Some("/macos")),
            Some((K0_TITLE, preserved_files_message("/macos")))
        );
        assert_eq!(
            sign_out_alert(Some(("/ours", 1)), Some("/macos")),
            Some((
                K0_TITLE,
                format!("{}\n\n/ours\n\n{PRESERVED_FILES_SENTENCE}\n\n/macos", k1(1).unwrap())
            ))
        );
    }
}

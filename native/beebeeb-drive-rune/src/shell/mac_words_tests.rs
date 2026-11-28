//! A Mac says no Windows word (drive-macos item 99): every scene, drawn with this Mac's words, holds no "Windows"
//! and no "PC"; the lines that are not in a scene are checked one by one; Settings' "Open System Settings" opens
//! the Mac's Notifications pane.

#[cfg(target_os = "macos")]
use rune::{Env, Runtime};

use crate::shell::web_pages;

#[cfg(target_os = "macos")]
/// The frames that only exist on Windows and keep the board's words there; a Mac never draws them.
const WINDOWS_ONLY: &[&str] = &[
    // The tray's "Adding / Couldn't add Beebeeb to File Explorer" (Cloud Files; `tray/tests.rs` `board_os`).
    "DW-1215",
    "DW-1216",
    "DW-1221",
    // "Commands and keyboard shortcuts": a Windows app's stand-in for the Mac's menu bar.
    "DW-1232",
    "DW-1233",
    "DW-1234",
    "shortcuts",
];

/// Words about ANOTHER device, which may well run Windows (the board's signed-in sessions, DW-A55 and DW-A56).
#[cfg(target_os = "macos")]
const OTHER_DEVICES: &[&str] = &["Chrome on Windows"];
/// The not-built card quotes the board's section titles ("First run on Windows"): a pointer into FRAMES.md.
#[cfg(target_os = "macos")]
const BOARD_POINTER_KEYS: &[&str] = &["not-built-frames"];

pub(crate) fn windows_word(said: &str) -> Option<&'static str> {
    let padded = format!(" {said} ");
    [
        "Windows",
        " PC ",
        " PC.",
        " PC,",
        " PC's",
        "PC\u{2019}s",
        "Credential Manager",
        "ms-settings",
    ]
    .into_iter()
    .find(|w| padded.contains(w))
}

#[cfg(target_os = "macos")]
#[test]
fn no_scene_says_windows_or_pc_on_a_mac() {
    let mut found = Vec::new();
    for (id, apply) in crate::frames::scenes(super::super::SCENES) {
        if WINDOWS_ONLY.contains(id) {
            continue;
        }
        let mut state = crate::init();
        apply(&mut state);
        let mut rt = Runtime::new(
            state,
            crate::view,
            (1278.0, 798.0),
            Env {
                system_dark: false,
                scale: 2.0,
            },
        );
        rt.frame(0.0);
        rt.frame(0.016);
        for e in rt.elements() {
            for said in [e.text.as_deref(), e.label.as_deref()].into_iter().flatten() {
                let key = e.key.as_deref().unwrap_or("");
                if BOARD_POINTER_KEYS.contains(&key) || OTHER_DEVICES.iter().any(|d| said.contains(d)) {
                    continue;
                }
                if let Some(w) = windows_word(said) {
                    found.push(format!("{id} {}: {w}: {said}", e.key.as_deref().unwrap_or("")));
                }
            }
        }
    }
    found.sort();
    found.dedup();
    assert!(
        found.is_empty(),
        "Windows words on a Mac ({}):\n{}",
        found.len(),
        found.join("\n")
    );
}

/// The lines this item changed that a scene may not reach: each in this Mac's words.
#[cfg(target_os = "macos")]
#[test]
fn the_lines_off_the_scenes_say_mac_words() {
    use crate::frames::viewers::refusal_text::text_with;
    use crate::media::player::Refusal;
    use beebeeb_media_protocol::Codec;
    let mut lines: Vec<String> = vec![
        crate::frames::signin::before_files::keystore::NO_ANSWER.into(),
        crate::frames::show_in_folder::NOT_HERE.into(),
        crate::windows_floor::NEEDS_WINDOWS_UPDATE.into(),
    ];
    for refusal in [
        Refusal::NoSystemDecoder(Codec::Hevc),
        Refusal::NoSystemDecoder(Codec::Aac),
        Refusal::NoSystemMedia(Codec::Aac),
        Refusal::TenBit,
    ] {
        let words = text_with(true, &refusal);
        assert_eq!(words.link, None, "no Microsoft Store link on a Mac");
        lines.push(words.sentence.to_string());
    }
    let bad: Vec<&String> = lines.iter().filter(|l| windows_word(l).is_some()).collect();
    assert!(bad.is_empty(), "Windows words on a Mac: {bad:#?}");
    assert_eq!(
        crate::frames::signin::before_files::keystore::NO_ANSWER,
        "macOS didn't answer. Nothing syncs until it does."
    );
}

#[cfg(target_os = "macos")]
#[test]
fn open_system_settings_opens_the_macs_notifications_pane() {
    assert_eq!(
        web_pages::OS_NOTIFICATIONS,
        "x-apple.systempreferences:com.apple.Notifications-Settings.extension"
    );
    assert!(crate::engine_host::signin::is_allowed_address(
        web_pages::OS_NOTIFICATIONS
    ));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn open_windows_settings_keeps_ms_settings_elsewhere() {
    assert_eq!(web_pages::OS_NOTIFICATIONS, "ms-settings:notifications");
}

#[test]
fn the_board_words_still_trip_the_check() {
    assert!(windows_word("The password stays in Windows Credential Manager on this PC.").is_some());
    assert!(windows_word("Your PC's password works").is_some());
    assert!(windows_word("Your Mac's password works if Touch ID doesn't").is_none());
}

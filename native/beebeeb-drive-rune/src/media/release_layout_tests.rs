//! w31-video-playback: the release installer carries the media helper.
//!
//! the owner's REAL 0.9.0 had no `beebeeb-media-helper.exe` beside the app exe, so `transport::helper_exe()` was `None`,
//! `playback_ready()` was false and a video opened as "No preview". The release workflow built the app only; the
//! helper is its own crate with its own target folder, so `build-installer.ps1`'s default (beside the app exe) never
//! found it. The workflow must build both helpers and hand their paths to the packager.

const WORKFLOW: &str = include_str!("../../../../.github/workflows/release-native-windows.yml");
const PACKAGER: &str = include_str!("../../../../packaging/windows/build-installer.ps1");

#[test]
fn the_release_workflow_builds_the_media_helper_and_gives_it_to_the_installer() {
    assert!(
        WORKFLOW.contains("native/beebeeb-media-helper"),
        "the release workflow never builds native/beebeeb-media-helper"
    );
    assert!(
        WORKFLOW.contains("-MediaHelperExe"),
        "the release workflow does not pass -MediaHelperExe to build-installer.ps1"
    );
    assert!(
        WORKFLOW.contains("beebeeb-media-helper\\target\\x86_64-pc-windows-msvc\\release\\beebeeb-media-helper.exe")
            || WORKFLOW.contains("beebeeb-media-helper/target/x86_64-pc-windows-msvc/release/beebeeb-media-helper.exe"),
        "the helper path handed to the installer is not the release build of the helper"
    );
}

#[test]
fn the_release_workflow_builds_the_office_helper_too() {
    assert!(WORKFLOW.contains("native/beebeeb-office-render"));
    assert!(WORKFLOW.contains("-OfficeRenderExe"));
}

#[test]
fn the_installer_refuses_a_missing_media_helper_and_signs_it() {
    assert!(PACKAGER.contains("no helper binary at"));
    assert!(PACKAGER.contains("$stagedMedia"));
}

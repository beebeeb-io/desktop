//! Tests: file icons.

use super::*;
use crate::theme::file_icon_art::art;
use crate::theme::file_icon_colors::tint_color;
use std::collections::HashSet;

fn id(name: &str, dir: bool, mime: Option<&str>) -> IconId {
    icon_for(name, dir, mime).id
}

fn item(name: &str, folder: bool, protected: bool) -> crate::source::DriveItem {
    crate::source::DriveItem {
        id: "1".into(),
        parent_id: None,
        name: name.into(),
        is_folder: folder,
        size: None,
        child_count: None,
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: vec![],
        pc_status: None,
        is_protected: protected,
    }
}

#[test]
fn file_icon_folders() {
    assert_eq!(icon_for_item(&item("Backups", true, true)).id, IconId::FolderBackups);
    assert_eq!(icon_for("Stuff", true, None), FileIcon { id: IconId::Folder, tint: Tint::Folder });
    assert_eq!(id("PHOTOS", true, None), IconId::FolderPhotos);
    assert_eq!(id("Afbeeldingen", true, None), IconId::FolderPhotos);
    assert_eq!(id("node_modules", true, None), IconId::FolderNodeModules);
    assert_eq!(id(".git", true, None), IconId::FolderGit);
}

#[test]
fn file_icon_files() {
    assert_eq!(id("a.tar.gz", false, None), IconId::Archive);
    assert_eq!(id("types.d.ts", false, None), IconId::TypeScript);
    assert_eq!(id("clip.mts", false, None), IconId::Video);
    assert_eq!(id("Dockerfile", false, None), IconId::Docker);
    assert_eq!(id("README.md", false, None), IconId::Readme);
    assert_eq!(id("Cargo.lock", false, None), IconId::Lock);
    assert_eq!(id("photo.JPG", false, None), IconId::Image);
}

#[test]
fn file_icon_mime_fallback() {
    assert_eq!(id("noext", false, Some("video/mp4")), IconId::Video);
    assert_eq!(icon_for("noext", false, None), FileIcon { id: IconId::File, tint: Tint::Default });
}

#[test]
fn file_icon_tables_are_lowercase_and_unique() {
    for t in [EXTENSIONS, FILE_NAMES, FOLDER_NAMES] {
        let mut seen = HashSet::new();
        for (k, _, _) in t {
            assert_eq!(*k, k.to_lowercase(), "{k} is not lower-case");
            assert!(seen.insert(*k), "{k} appears twice");
        }
    }
}

#[test]
fn file_icon_every_id_has_art_and_every_tint_a_colour() {
    for &i in IconId::ALL {
        let _ = art(i);
    }
    for &t in Tint::ALL {
        let _ = tint_color(t, false);
        let _ = tint_color(t, true);
    }
}

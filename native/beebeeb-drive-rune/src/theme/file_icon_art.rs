//! The art for each icon id. Stub: today's glyphs; ux-04 fills in the real art.
#![allow(dead_code)]

use super::file_icons::IconId;
use super::{icons, icons_files};

pub fn art(id: IconId) -> &'static rune::Icon {
    use IconId::*;
    match id {
        Folder | FolderPhotos | FolderScreenshots | FolderVideos | FolderMusic | FolderDocuments
        | FolderDownloads | FolderDesktop | FolderGit | FolderNodeModules | FolderSrc | FolderTests
        | FolderDocs | FolderBuild | FolderPublic | FolderAssets | FolderScripts | FolderConfig
        | FolderGithub | FolderVscode | FolderLib | FolderArchive | FolderShared | FolderTemplates
        | FolderProjects => &icons::FILES,
        FolderBackups => &icons::BACKUPS,
        Sheet | Csv => &icons::DOC_SPREADSHEET,
        Slides | Image | Svg | Video | Audio => &icons::DOC_MINIMAL,
        File => &icons_files::DOC_BLANK,
        _ => &icons::DOC_GENERIC,
    }
}

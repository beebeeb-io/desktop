//! The `DriveSource` trait: the UI's contract with the data layer.
//!
//! Shaped like the TS `DriveSource` in `packages/shared/src/drive-ui/source.ts` and the Rust
//! DTOs in `src-tauri/src/drive_v2/dto.rs`. Implemented twice:
//! - `BoardFixtureSource`: the board's DW-0401 rows, EXACTLY as drawn.
//! - `LargeFixtureSource`: 5,000 items for scrolling/perf tests.
//! - (future) `EngineSource`: the real `drive_v2::model` over `StateDb`, no Tauri.

#![allow(dead_code)] // Fields are part of the data contract; not all are read by every component yet.

use rune::Color;

/// A file or folder in a listing.
#[derive(Debug, Clone)]
pub struct DriveItem {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub is_folder: bool,
    pub size: Option<i64>,
    pub child_count: Option<i64>,
    pub modified_at: String,
    pub mime: Option<String>,
    pub has_thumbnail: bool,
    /// "Link" or None.
    pub shared_type: Option<String>,
    /// Letter-circle badges in the Shared column.
    pub shared_badges: Vec<SharedBadge>,
    /// "on_this_pc", "online_only", "available_offline".
    pub pc_status: Option<String>,
    /// Protected folder (e.g. Backups): shows lock icon, can't be trashed.
    pub is_protected: bool,
}

#[derive(Debug, Clone)]
pub struct SharedBadge {
    pub initials: String,
    pub color: Color,
}

/// A breadcrumb segment.
#[derive(Debug, Clone)]
pub struct Crumb {
    pub id: Option<String>,
    pub name: String,
}

/// A folder listing.
#[derive(Debug, Clone)]
pub struct Listing {
    pub folder: Crumb,
    pub path: Vec<Crumb>,
    pub items: Vec<DriveItem>,
}

/// Storage meter.
#[derive(Debug, Clone, Copy)]
pub struct StorageSummary {
    pub used_bytes: i64,
    pub quota_bytes: i64,
}

/// The data contract the UI consumes.
pub trait DriveSource: Send + 'static {
    fn list(&self, folder_id: Option<&str>) -> Listing;
    fn storage(&self) -> StorageSummary;
}

/// Format bytes as "84 KB", "1.2 MB", "3.1 GB" etc. Shared by file_list and detail_pane.
pub fn format_size(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{} KB", (b / KB).round() as i64)
    } else {
        format!("{bytes} B")
    }
}

// ---------------------------------------------------------------------------
// Board fixture (DW-0401: EXACTLY the board's 11 items)
// ---------------------------------------------------------------------------

pub struct BoardFixtureSource;

impl BoardFixtureSource {
    pub fn new() -> Self {
        Self
    }
}

impl DriveSource for BoardFixtureSource {
    fn list(&self, _folder_id: Option<&str>) -> Listing {
        use rune::oklch;

        // The board's avatar hues (`.kit-avatar.hue-a…d`). Only the hue is data: the design
        // system picks the tint's lightness and chroma per theme (`ui::file_list::avatar_tint`).
        let hue = |h: f32| oklch(0.88, 0.05, h);
        let (hue_a, hue_b, hue_c, hue_d) = (hue(250.0), hue(160.0), hue(20.0), hue(300.0));
        let badge = |initials: &str, color| SharedBadge {
            initials: initials.into(),
            color,
        };
        let folder = |id: &str, name: &str, count: i64, modified: &str, badges: Vec<SharedBadge>, pc: &str| DriveItem {
            id: id.into(),
            parent_id: None,
            name: name.into(),
            is_folder: true,
            size: None,
            child_count: Some(count),
            modified_at: modified.into(),
            mime: None,
            has_thumbnail: false,
            shared_type: None,
            shared_badges: badges,
            pc_status: Some(pc.into()),
            is_protected: false,
        };
        #[allow(clippy::too_many_arguments)]
        let file = |id: &str,
                    name: &str,
                    size: i64,
                    modified: &str,
                    mime: &str,
                    link: bool,
                    badges: Vec<SharedBadge>,
                    pc: &str| DriveItem {
            id: id.into(),
            parent_id: None,
            name: name.into(),
            is_folder: false,
            size: Some(size),
            child_count: None,
            modified_at: modified.into(),
            mime: Some(mime.into()),
            has_thumbnail: mime.starts_with("image/"),
            shared_type: link.then(|| "Link".to_string()),
            shared_badges: badges,
            pc_status: Some(pc.into()),
            is_protected: false,
        };
        // PC column, as the board draws each row (frames.json DW-0401: "online only,
        // downloaded, available offline"): `online_only`, `on_this_pc`, `available_offline`.
        let (online, downloaded, kept) = ("online_only", "on_this_pc", "available_offline");
        const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
        const KEY: &str = "application/x-iwork-keynote-sffkey";

        let items = vec![
            DriveItem {
                is_protected: true,
                ..folder("f-backups", "Backups", 4, "2026-11-05T07:02:00Z", vec![], online)
            },
            folder("f-docs", "Documents", 48, "2026-11-04T16:40:00Z", vec![], kept),
            folder(
                "f-invoices",
                "Invoices 2026",
                11,
                "2026-10-28T10:00:00Z",
                vec![badge("PS", hue_c)],
                kept,
            ),
            folder(
                "f-lisbon",
                "Lisbon trip",
                212,
                "2026-10-12T14:00:00Z",
                vec![badge("JM", hue_c), badge("MD", hue_a)],
                online,
            ),
            folder(
                "f-shoots",
                "Shoot selects",
                64,
                "2026-10-03T09:00:00Z",
                vec![badge("LD", hue_d)],
                online,
            ),
            file(
                "budget",
                "Budget 2026.xlsx",
                86_016,
                "2026-11-05T09:12:00Z",
                XLSX,
                true,
                vec![],
                kept,
            ),
            file(
                "contract",
                "Contract draft.pdf",
                1_258_291,
                "2026-11-04T16:42:00Z",
                "application/pdf",
                false,
                vec![badge("SO", hue_b)],
                online,
            ),
            // 1 Nov 2026 is the Sunday before the fixture's "today" (Thursday 5 Nov).
            file(
                "harbour",
                "Harbour.heic",
                3_250_585,
                "2026-11-01T14:00:00Z",
                "image/heic",
                true,
                vec![],
                downloaded,
            ),
            file(
                "brand",
                "Northwind brand guide.pdf",
                8_808_038,
                "2026-10-21T11:00:00Z",
                "application/pdf",
                false,
                vec![],
                downloaded,
            ),
            file(
                "q4plan",
                "Q4 plan.key",
                23_697_818,
                "2026-10-15T10:00:00Z",
                KEY,
                false,
                vec![],
                downloaded,
            ),
            file(
                "rent",
                "Studio rent 2026.pdf",
                319_488,
                "2026-10-02T08:00:00Z",
                "application/pdf",
                false,
                vec![],
                downloaded,
            ),
        ];

        Listing {
            folder: Crumb {
                id: None,
                name: "Files".into(),
            },
            path: vec![Crumb {
                id: None,
                name: "Files".into(),
            }],
            items,
        }
    }

    fn storage(&self) -> StorageSummary {
        StorageSummary {
            used_bytes: 240_000_000_000,    // 240 GB
            quota_bytes: 1_000_000_000_000, // 1 TB
        }
    }
}

// ---------------------------------------------------------------------------
// Large fixture (5,000 items for scrolling tests)
// ---------------------------------------------------------------------------

pub struct LargeFixtureSource {
    items: Vec<DriveItem>,
}

impl LargeFixtureSource {
    pub fn new(count: usize) -> Self {
        let mut items = Vec::with_capacity(count + 5);
        let folders = [
            ("f-docs", "Documents"),
            ("f-photos", "Photos"),
            ("f-projects", "Projects"),
            ("f-music", "Music"),
            ("f-downloads", "Downloads"),
        ];
        for (id, name) in &folders {
            items.push(DriveItem {
                id: id.to_string(),
                parent_id: None,
                name: name.to_string(),
                is_folder: true,
                size: None,
                child_count: Some(42),
                modified_at: "2026-11-10T14:30:00Z".into(),
                mime: None,
                has_thumbnail: false,
                shared_type: None,
                shared_badges: vec![],
                pc_status: Some("on_this_pc".into()),
                is_protected: false,
            });
        }
        let exts = [
            ("pdf", "application/pdf"),
            ("jpg", "image/jpeg"),
            ("png", "image/png"),
            (
                "docx",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            ),
            (
                "xlsx",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            ),
            ("mp4", "video/mp4"),
            ("txt", "text/plain"),
            ("zip", "application/zip"),
            ("rs", "text/x-rust"),
            ("md", "text/markdown"),
        ];
        let names = [
            "report",
            "invoice",
            "screenshot",
            "backup",
            "presentation",
            "notes",
            "receipt",
            "contract",
            "design",
            "export",
            "analysis",
            "summary",
            "proposal",
            "meeting",
            "roadmap",
        ];
        for i in 0..(count.saturating_sub(folders.len())) {
            let (ext, mime) = exts[i % exts.len()];
            let name_base = names[i % names.len()];
            let suffix = if i / names.len() > 0 {
                format!("-{}", i / names.len() + 1)
            } else {
                String::new()
            };
            items.push(DriveItem {
                id: format!("file-{i:05}"),
                parent_id: None,
                name: format!("{name_base}{suffix}.{ext}"),
                is_folder: false,
                size: Some((i as i64 + 1) * 1024 * 37),
                child_count: None,
                modified_at: format!("2026-11-{:02}T{:02}:{:02}:00Z", (i % 28) + 1, i % 24, i % 60),
                mime: Some(mime.into()),
                has_thumbnail: ext == "jpg" || ext == "png",
                shared_type: None,
                shared_badges: vec![],
                pc_status: Some(["on_this_pc", "online_only", "available_offline"][i % 3].into()),
                is_protected: false,
            });
        }
        Self { items }
    }
}

impl DriveSource for LargeFixtureSource {
    fn list(&self, _folder_id: Option<&str>) -> Listing {
        Listing {
            folder: Crumb {
                id: None,
                name: "Files".into(),
            },
            path: vec![Crumb {
                id: None,
                name: "Files".into(),
            }],
            items: self.items.clone(),
        }
    }
    fn storage(&self) -> StorageSummary {
        StorageSummary {
            used_bytes: 240_000_000_000,
            quota_bytes: 1_000_000_000_000,
        }
    }
}

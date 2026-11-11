//! The `DriveSource` trait: the UI's contract with the data layer.
//!
//! Shaped like the TS `DriveSource` in `packages/shared/src/drive-ui/source.ts` and the Rust
//! DTOs in `src-tauri/src/drive_v2/dto.rs`. Implemented twice:
//! - `BoardFixtureSource`: the board's DW-0401 rows, EXACTLY as drawn.
//! - `LargeFixtureSource`: 5,000 items for scrolling/perf tests.
//! - (future) `EngineSource`: the real `drive_v2::model` over `StateDb`, no Tauri.

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

        let items = vec![
            DriveItem {
                id: "f-backups".into(),
                parent_id: None,
                name: "Backups".into(),
                is_folder: true,
                size: None,
                child_count: Some(4),
                modified_at: "2026-11-05T07:02:00Z".into(),
                mime: None,
                has_thumbnail: false,
                shared_type: None,
                shared_badges: vec![],
                pc_status: Some("on_this_pc".into()),
                is_protected: true,
            },
            DriveItem {
                id: "f-docs".into(),
                parent_id: None,
                name: "Documents".into(),
                is_folder: true,
                size: None,
                child_count: Some(48),
                modified_at: "2026-11-04T16:40:00Z".into(),
                mime: None,
                has_thumbnail: false,
                shared_type: None,
                shared_badges: vec![],
                pc_status: Some("on_this_pc".into()),
                is_protected: false,
            },
            DriveItem {
                id: "f-invoices".into(),
                parent_id: None,
                name: "Invoices 2026".into(),
                is_folder: true,
                size: None,
                child_count: Some(11),
                modified_at: "2026-10-28T10:00:00Z".into(),
                mime: None,
                has_thumbnail: false,
                shared_type: None,
                shared_badges: vec![SharedBadge {
                    initials: "PS".into(),
                    color: oklch(0.65, 0.14, 150.0),
                }],
                pc_status: Some("on_this_pc".into()),
                is_protected: false,
            },
            DriveItem {
                id: "f-lisbon".into(),
                parent_id: None,
                name: "Lisbon trip".into(),
                is_folder: true,
                size: None,
                child_count: Some(212),
                modified_at: "2026-10-12T14:00:00Z".into(),
                mime: None,
                has_thumbnail: false,
                shared_type: None,
                shared_badges: vec![
                    SharedBadge {
                        initials: "JI".into(),
                        color: oklch(0.62, 0.21, 25.0),
                    },
                    SharedBadge {
                        initials: "MD".into(),
                        color: oklch(0.55, 0.18, 250.0),
                    },
                ],
                pc_status: Some("online_only".into()),
                is_protected: false,
            },
            DriveItem {
                id: "f-shoots".into(),
                parent_id: None,
                name: "Shoot selects".into(),
                is_folder: true,
                size: None,
                child_count: Some(64),
                modified_at: "2026-10-03T09:00:00Z".into(),
                mime: None,
                has_thumbnail: false,
                shared_type: None,
                shared_badges: vec![SharedBadge {
                    initials: "LD".into(),
                    color: oklch(0.65, 0.18, 30.0),
                }],
                pc_status: Some("online_only".into()),
                is_protected: false,
            },
            DriveItem {
                id: "budget".into(),
                parent_id: None,
                name: "Budget 2026.xlsx".into(),
                is_folder: false,
                size: Some(86016),
                child_count: None,
                modified_at: "2026-11-05T09:12:00Z".into(),
                mime: Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into()),
                has_thumbnail: false,
                shared_type: Some("Link".into()),
                shared_badges: vec![],
                pc_status: Some("on_this_pc".into()),
                is_protected: false,
            },
            DriveItem {
                id: "contract".into(),
                parent_id: None,
                name: "Contract draft.pdf".into(),
                is_folder: false,
                size: Some(1258291),
                child_count: None,
                modified_at: "2026-11-04T16:42:00Z".into(),
                mime: Some("application/pdf".into()),
                has_thumbnail: false,
                shared_type: None,
                shared_badges: vec![SharedBadge {
                    initials: "SO".into(),
                    color: oklch(0.65, 0.14, 150.0),
                }],
                pc_status: Some("online_only".into()),
                is_protected: false,
            },
            DriveItem {
                id: "harbour".into(),
                parent_id: None,
                name: "Harbour.heic".into(),
                is_folder: false,
                size: Some(3250585),
                child_count: None,
                modified_at: "2026-11-02T14:00:00Z".into(),
                mime: Some("image/heic".into()),
                has_thumbnail: true,
                shared_type: Some("Link".into()),
                shared_badges: vec![],
                pc_status: Some("available_offline".into()),
                is_protected: false,
            },
            DriveItem {
                id: "brand".into(),
                parent_id: None,
                name: "Northwind brand guide.pdf".into(),
                is_folder: false,
                size: Some(8808038),
                child_count: None,
                modified_at: "2026-10-21T11:00:00Z".into(),
                mime: Some("application/pdf".into()),
                has_thumbnail: false,
                shared_type: None,
                shared_badges: vec![],
                pc_status: Some("available_offline".into()),
                is_protected: false,
            },
            DriveItem {
                id: "q4plan".into(),
                parent_id: None,
                name: "Q4 plan.key".into(),
                is_folder: false,
                size: Some(23068672),
                child_count: None,
                modified_at: "2026-10-15T10:00:00Z".into(),
                mime: Some("application/x-iwork-keynote-sffkey".into()),
                has_thumbnail: false,
                shared_type: None,
                shared_badges: vec![],
                pc_status: Some("available_offline".into()),
                is_protected: false,
            },
            DriveItem {
                id: "rent".into(),
                parent_id: None,
                name: "Studio rent 2026.pdf".into(),
                is_folder: false,
                size: Some(319488),
                child_count: None,
                modified_at: "2026-10-02T08:00:00Z".into(),
                mime: Some("application/pdf".into()),
                has_thumbnail: false,
                shared_type: None,
                shared_badges: vec![],
                pc_status: Some("available_offline".into()),
                is_protected: false,
            },
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

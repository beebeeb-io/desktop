//! The `DriveSource` trait: the UI's contract with the data layer.
//!
//! Shaped like the TS `DriveSource` in `packages/shared/src/drive-ui/source.ts` and the Rust
//! DTOs in `src-tauri/src/drive_v2/dto.rs`. Implemented twice:
//! - `FixtureSource`: invented names, no I/O, for the spike and headless tests.
//! - (future) `EngineSource`: the real `drive_v2::model` over `StateDb`, no Tauri.

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

/// The data contract the UI consumes. All methods are synchronous for the spike;
/// the real engine source will use channels or `rune::Latest` for async.
pub trait DriveSource: Send + 'static {
    fn list(&self, folder_id: Option<&str>) -> Listing;
    fn storage(&self) -> StorageSummary;
}

// ---------------------------------------------------------------------------
// Fixture source
// ---------------------------------------------------------------------------

pub struct FixtureSource {
    items: Vec<DriveItem>,
}

impl FixtureSource {
    pub fn new(count: usize) -> Self {
        let mut items = Vec::with_capacity(count + 5);

        // A few folders first.
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
            });
        }

        // Fill the rest with files.
        let extensions = [
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
            let (ext, mime) = extensions[i % extensions.len()];
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
            });
        }

        Self { items }
    }
}

impl DriveSource for FixtureSource {
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
            used_bytes: 235_000_000_000,    // 235 GB
            quota_bytes: 1_000_000_000_000, // 1 TB
        }
    }
}

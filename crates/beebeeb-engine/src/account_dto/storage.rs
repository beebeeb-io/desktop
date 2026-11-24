//! Local storage breakdown by extension.
//!
//! There is no server endpoint for this: the buckets and largest files are
//! computed from the desktop SQLite mirror by `compute_storage_breakdown`.

use super::*;

// ── 8. Storage breakdown — computed LOCALLY (no server endpoint) ───────────────
//
// The server has no user-facing storage-by-type endpoint. We compute the
// breakdown from the desktop SQLite mirror (`state_db::list_files`): bucket
// each file by extension into Media / Documents / Other, sum sizes, and
// surface the N largest files.

/// A coarse content category derived from a file's extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageCategory {
    Media,
    Documents,
    Other,
}

impl StorageCategory {
    /// Classify by lowercase extension (no dot). Unknown / extensionless →
    /// `Other`. The lists are intentionally conservative — the goal is a
    /// useful breakdown, not MIME-perfect classification.
    pub fn from_extension(ext: &str) -> Self {
        const MEDIA: &[&str] = &[
            // images
            "jpg", "jpeg", "png", "gif", "bmp", "tiff", "tif", "webp", "heic", "heif", "svg", "ico", "raw", "cr2",
            "nef", "arw", "dng", "avif", // video
            "mp4", "mov", "avi", "mkv", "webm", "flv", "wmv", "m4v", "mpg", "mpeg", "3gp", // audio
            "mp3", "wav", "flac", "aac", "ogg", "m4a", "wma", "opus", "aiff", "alac",
        ];
        const DOCUMENTS: &[&str] = &[
            "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt", "md", "csv", "tsv",
            "epub", "pages", "numbers", "key", "tex", "json", "xml", "yaml", "yml", "html", "htm",
        ];
        let ext = ext.to_ascii_lowercase();
        if MEDIA.contains(&ext.as_str()) {
            StorageCategory::Media
        } else if DOCUMENTS.contains(&ext.as_str()) {
            StorageCategory::Documents
        } else {
            StorageCategory::Other
        }
    }
}

/// Per-category roll-up.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageCategoryTotal {
    pub category: StorageCategory,
    pub bytes: i64,
    pub file_count: i64,
}

/// One of the N largest files in the local mirror.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LargestFile {
    pub file_id: String,
    pub path: String,
    pub size_bytes: i64,
    pub category: StorageCategory,
}

/// Result of [`compute_storage_breakdown`].
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageBreakdown {
    /// One entry per category (Media, Documents, Other), always all three
    /// present (a category with no files reports zero) so the UI can render
    /// a stable legend.
    pub by_category: Vec<StorageCategoryTotal>,
    /// Largest files, descending by size, capped at the requested limit.
    pub largest_files: Vec<LargestFile>,
    /// Total file count (folders excluded).
    pub total_files: i64,
    /// Sum of all file sizes (folders excluded).
    pub total_bytes: i64,
}

/// A minimal view of a mirrored file for the breakdown computation. Decoupled
/// from `state_db::FileEntry` so the pure computation is unit-testable
/// without a SQLite handle (the caller maps `FileEntry` → this, skipping
/// folders).
#[derive(Debug, Clone)]
pub struct BreakdownInput {
    pub file_id: String,
    pub path: String,
    pub size_bytes: i64,
}

/// Pure storage-breakdown computation. `files` should already exclude
/// folders. `largest_limit` caps the `largest_files` list. Negative sizes
/// are clamped to 0.
pub fn compute_storage_breakdown(files: &[BreakdownInput], largest_limit: usize) -> StorageBreakdown {
    let mut media = StorageCategoryTotal {
        category: StorageCategory::Media,
        bytes: 0,
        file_count: 0,
    };
    let mut documents = StorageCategoryTotal {
        category: StorageCategory::Documents,
        bytes: 0,
        file_count: 0,
    };
    let mut other = StorageCategoryTotal {
        category: StorageCategory::Other,
        bytes: 0,
        file_count: 0,
    };

    let mut total_bytes: i64 = 0;
    let mut total_files: i64 = 0;
    let mut ranked: Vec<LargestFile> = Vec::with_capacity(files.len());

    for f in files {
        let size = f.size_bytes.max(0);
        let category = StorageCategory::from_extension(&extension_of(&f.path));
        let bucket = match category {
            StorageCategory::Media => &mut media,
            StorageCategory::Documents => &mut documents,
            StorageCategory::Other => &mut other,
        };
        bucket.bytes = bucket.bytes.saturating_add(size);
        bucket.file_count += 1;

        total_bytes = total_bytes.saturating_add(size);
        total_files += 1;

        ranked.push(LargestFile {
            file_id: f.file_id.clone(),
            path: f.path.clone(),
            size_bytes: size,
            category,
        });
    }

    ranked.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then_with(|| a.path.cmp(&b.path)));
    ranked.truncate(largest_limit);

    StorageBreakdown {
        by_category: vec![media, documents, other],
        largest_files: ranked,
        total_files,
        total_bytes,
    }
}

/// Lowercase extension (no dot) of a path, or "" if none. Handles paths with
/// no extension and dotfiles (".gitignore" → "" — a leading-dot-only name has
/// no extension).
pub(super) fn extension_of(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rsplit_once('.') {
        // Leading-dot file like ".gitignore": the part before the dot is empty
        // → treat as no extension.
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

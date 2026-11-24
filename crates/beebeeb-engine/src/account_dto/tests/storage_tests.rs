//! Storage category and breakdown computation tests.
//!
//! Exercises extension classification, the empty-account case and the
//! largest-file selection.

use super::*;
use crate::account_dto::storage::extension_of;

#[test]
fn storage_category_classifies_by_extension() {
    assert_eq!(StorageCategory::from_extension("JPG"), StorageCategory::Media);
    assert_eq!(StorageCategory::from_extension("mp4"), StorageCategory::Media);
    assert_eq!(StorageCategory::from_extension("pdf"), StorageCategory::Documents);
    assert_eq!(StorageCategory::from_extension("docx"), StorageCategory::Documents);
    assert_eq!(StorageCategory::from_extension("bin"), StorageCategory::Other);
    assert_eq!(StorageCategory::from_extension(""), StorageCategory::Other);
}

#[test]
fn extension_of_handles_edge_cases() {
    assert_eq!(extension_of("/a/b/c.PDF"), "pdf");
    assert_eq!(extension_of("photo.jpeg"), "jpeg");
    assert_eq!(extension_of("noext"), "");
    assert_eq!(extension_of("/path/.gitignore"), "");
    assert_eq!(extension_of("archive.tar.gz"), "gz");
    assert_eq!(extension_of("C:\\home\\g\\file.txt"), "txt");
}

#[test]
fn storage_breakdown_computes_buckets_and_largest() {
    let files = vec![
        BreakdownInput {
            file_id: "1".into(),
            path: "/photos/a.jpg".into(),
            size_bytes: 3000,
        },
        BreakdownInput {
            file_id: "2".into(),
            path: "/docs/report.pdf".into(),
            size_bytes: 5000,
        },
        BreakdownInput {
            file_id: "3".into(),
            path: "/misc/blob.bin".into(),
            size_bytes: 1000,
        },
        BreakdownInput {
            file_id: "4".into(),
            path: "/photos/b.png".into(),
            size_bytes: 2000,
        },
        // Negative size is clamped to 0.
        BreakdownInput {
            file_id: "5".into(),
            path: "/weird/neg.dat".into(),
            size_bytes: -10,
        },
    ];
    let b = compute_storage_breakdown(&files, 3);

    assert_eq!(b.total_files, 5);
    assert_eq!(b.total_bytes, 3000 + 5000 + 1000 + 2000);

    let media = b
        .by_category
        .iter()
        .find(|c| c.category == StorageCategory::Media)
        .unwrap();
    assert_eq!(media.bytes, 5000); // 3000 + 2000
    assert_eq!(media.file_count, 2);

    let docs = b
        .by_category
        .iter()
        .find(|c| c.category == StorageCategory::Documents)
        .unwrap();
    assert_eq!(docs.bytes, 5000);
    assert_eq!(docs.file_count, 1);

    let other = b
        .by_category
        .iter()
        .find(|c| c.category == StorageCategory::Other)
        .unwrap();
    assert_eq!(other.bytes, 1000); // blob.bin + neg.dat(clamped 0)
    assert_eq!(other.file_count, 2);

    // Largest three, descending: report.pdf(5000), a.jpg(3000), b.png(2000).
    assert_eq!(b.largest_files.len(), 3);
    assert_eq!(b.largest_files[0].path, "/docs/report.pdf");
    assert_eq!(b.largest_files[0].size_bytes, 5000);
    assert_eq!(b.largest_files[1].size_bytes, 3000);
    assert_eq!(b.largest_files[2].size_bytes, 2000);
}

#[test]
fn storage_breakdown_all_categories_present_when_empty() {
    let b = compute_storage_breakdown(&[], 5);
    assert_eq!(b.total_files, 0);
    assert_eq!(b.total_bytes, 0);
    assert_eq!(b.by_category.len(), 3);
    assert!(b.largest_files.is_empty());
}

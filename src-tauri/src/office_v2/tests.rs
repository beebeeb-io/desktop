//! The engine cache: a file is kept only at its pinned size and hash; the scheme serves pinned paths only.

use super::*;

const DATA: &[u8] = b"engine bytes";

fn asset(sha256: &'static str) -> &'static [PinnedAsset] {
    Box::leak(Box::new([PinnedAsset {
        path: "soffice.wasm",
        bytes: DATA.len() as u64,
        sha256,
        content_type: "application/wasm",
    }]))
}

fn good_hash() -> &'static str {
    Box::leak(hex(&Sha256::digest(DATA)).into_boxed_str())
}

#[test]
fn a_matching_file_is_kept_and_reported_ready() {
    let tmp = tempfile::tempdir().unwrap();
    let bundle: &'static PinnedBundle = Box::leak(Box::new(PinnedBundle {
        version: "v1",
        assets: asset(good_hash()),
    }));
    assert_eq!(status_of(Some(bundle), tmp.path()).state, "missing");
    let dir = tmp.path().join("v1");
    let mut sink = Sink::create(&dir, &bundle.assets[0]).unwrap();
    sink.write(DATA).unwrap();
    sink.finish(&dir, &bundle.assets[0]).unwrap();
    assert_eq!(status_of(Some(bundle), tmp.path()).state, "ready");
}

#[test]
fn a_wrong_hash_keeps_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let bundle: &'static PinnedBundle = Box::leak(Box::new(PinnedBundle {
        version: "v1",
        assets: asset("00"),
    }));
    let dir = tmp.path().join("v1");
    let mut sink = Sink::create(&dir, &bundle.assets[0]).unwrap();
    sink.write(DATA).unwrap();
    assert_eq!(sink.finish(&dir, &bundle.assets[0]).unwrap_err(), "hash: mismatch");
    assert!(!dir.join("soffice.wasm").exists() && !dir.join("soffice.wasm.part").exists());
    assert_eq!(status_of(Some(bundle), tmp.path()).state, "missing");
}

#[test]
fn nothing_pinned_is_unpublished() {
    assert_eq!(status_of(None, Path::new("/nonexistent")).state, "unpublished");
}

#[test]
fn the_scheme_serves_pinned_paths_only() {
    let bundle = PinnedBundle {
        version: "v1",
        assets: asset(good_hash()),
    };
    assert_eq!(resolve(&bundle, "/v1/soffice.wasm"), Some("soffice.wasm"));
    assert_eq!(resolve(&bundle, "/v1/../secrets"), None);
    assert_eq!(resolve(&bundle, "/v2/soffice.wasm"), None);
    assert_eq!(resolve(&bundle, "/v1/other.js"), None);
}

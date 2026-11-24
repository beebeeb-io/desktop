//! Tests of the domain removal (ported from `desktop-v0.8.25:src-tauri/src/finder_removal/tests/`), on a fake bridge:
//! nothing here calls the real File Provider, so nothing removes a domain.
use super::*;

fn kept(path: &str, contents_checked: bool) -> KeptFolder {
    KeptFolder::Kept {
        path: path.to_string(),
        contents_checked,
        empty: false,
    }
}

fn kept_empty(path: &str) -> KeptFolder {
    KeptFolder::Kept {
        path: path.to_string(),
        contents_checked: true,
        empty: true,
    }
}

fn removal(kept: KeptFolder) -> DomainRemoval {
    DomainRemoval { kept }
}

mod bridge;
mod pins;
mod saved;
mod writes;

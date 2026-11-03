//! `thumbnails` items split out of `ipc_socket.rs`.

use super::*;

/// Task 1699: mirror of the Windows Cloud Files thumbnail variant picker
/// (`windows_cf/thumbnail_provider.rs`): the requested maximum pixel
/// dimension maps to the smallest server variant that satisfies it. The
/// server serves three encrypted variants (small/medium/large) per file.
pub(crate) fn thumbnail_variant(max_dimension: u32) -> &'static str {
    if max_dimension <= 96 {
        "small"
    } else if max_dimension <= 256 {
        "medium"
    } else {
        "large"
    }
}

/// The destinations a `FetchThumbnail` staging write may land in — the same
/// allowed roots as `HydrateFile` (task 1247): the configured sync root, this
/// process's temp dir, and on macOS the shared App Group hydrate-cache
/// directory (ensured to exist, owner-only and backup-excluded, and swept for
/// crash-orphaned staging files first — the extension deletes its staging
/// file right after reading it, so the sweep stays a crash backstop).
pub(super) fn thumbnail_allowed_roots() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = crate::config::DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root) {
        roots.push(root);
    }
    roots.push(std::env::temp_dir());
    #[cfg(target_os = "macos")]
    {
        let dir = macos_hydrate_cache_dir();
        if let Err(e) = macos_ensure_hydrate_cache_dir(&dir) {
            tracing::warn!(error = %e, dir = %dir.display(), "could not create macOS hydrate-cache dir");
        }
        if let Err(e) =
            macos_sweep_stale_hydrate_cache_entries(&dir, MACOS_HYDRATE_CACHE_TTL, std::time::SystemTime::now())
        {
            tracing::warn!(error = %e, dir = %dir.display(), "hydrate-cache TTL sweep failed (best-effort)");
        }
        roots.push(dir);
    }
    roots
}

/// `Some(error reply message)` when `dest` is not safely stageable: its
/// parent must exist (nothing is created outside an allowed root) and
/// canonicalize to a path under one of the allowed roots. Both sides are
/// canonicalized so macOS `/var` ↔ `/private/var` symlinks cannot split the
/// check (mirrors the hydrate containment semantics).
pub(super) fn thumbnail_destination_error(dest: &std::path::Path, allowed_roots: &[std::path::PathBuf]) -> Option<String> {
    let parent = dest.parent()?;
    let canonical_parent = match parent.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return Some(format!(
                "thumbnail destination {} is not under an allowed root (parent does not exist)",
                dest.display()
            ));
        }
    };
    let contained = allowed_roots.iter().any(|root| match root.canonicalize() {
        Ok(canonical_root) => canonical_parent == canonical_root || canonical_parent.starts_with(&canonical_root),
        Err(_) => false,
    });
    if contained {
        None
    } else {
        Some(format!(
            "thumbnail destination {} is not under an allowed root",
            dest.display()
        ))
    }
}

/// Stage decrypted thumbnail plaintext at `dest` atomically: write a
/// fresh temp leaf, force owner-only permissions, then rename onto `dest` —
/// the extension can never observe a partial thumbnail, and a crash leaves
/// only the temp file for the TTL sweep to collect. Returns the plaintext byte
/// count. Caller has already validated containment
/// ([`thumbnail_destination_error`]); this is defense in depth for the same
/// rule and refuses to write outside the allowed roots regardless.
///
/// Task 1699 review (PR #103 thread PRRT_kwDOSLX6Xs6oeNfW, P1): this used to
/// be a plain `File::create` on `<dest>.part` after the parent-only
/// containment check — but `File::create` FOLLOWS a pre-created symlink, so a
/// socket client could plant `<dest>.part` → arbitrary file and have the
/// daemon truncate it outside every allowed root. Delegates to the
/// fd-anchored staging writer (`write_hydrated_plaintext`, tasks 1247/1670),
/// which descends O_NOFOLLOW from a trusted root fd, stages under a fresh
/// random temp leaf, and publishes with an anchored `renameat` — it can
/// REPLACE a planted path but can never WRITE THROUGH one. The temp shape
/// (`.{leaf}.{uuid}.part`) is the same one the macOS TTL sweep is proven to
/// leave alone while in flight.
pub(crate) fn persist_thumbnail_plaintext(
    dest: &std::path::Path,
    allowed_roots: &[std::path::PathBuf],
    plaintext: &[u8],
) -> Result<u64, IpcError> {
    if thumbnail_destination_error(dest, allowed_roots).is_some() {
        return Err(IpcError::DestinationNotAllowed {
            dest: dest.to_path_buf(),
        });
    }
    let root_paths: Vec<&std::path::Path> = allowed_roots.iter().map(|p| p.as_path()).collect();
    crate::engine_bridge::write_hydrated_plaintext(dest, &root_paths, plaintext)
        .map(|_| plaintext.len() as u64)
        .map_err(|source| IpcError::Stage {
            dest: dest.to_path_buf(),
            source: anyhow::Error::new(source),
        })
}

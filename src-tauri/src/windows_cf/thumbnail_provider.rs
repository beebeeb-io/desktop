//! Explorer media **thumbnails** for cloud-only Beebeeb placeholders (task 0823).
//!
//! Today Explorer shows a generic file-type icon for a cloud-only image or video
//! placeholder, because nothing tells the shell how to render a tile for a file
//! whose bytes are not on disk. This module installs a Windows Shell
//! **`IThumbnailProvider`** COM handler that hands Explorer a real thumbnail by
//! **reusing the EXISTING encrypted server thumbnail** — the downscaled still
//! (images) or poster frame (video) the original uploading client already
//! generated at upload and stored at `GET /api/v1/files/:id/thumbnail/{variant}`.
//!
//! ## What it does NOT do
//!
//! - It never hydrates the whole file. A 96px tile must not cost a full
//!   photo/video download + decrypt — that would defeat the cloud-only model.
//!   We fetch only the small encrypted thumbnail blob.
//! - It never decodes video on Windows. The served thumbnail IS the poster frame
//!   the uploader made; we just request it.
//! - It never writes plaintext to disk. The decrypted thumbnail bytes live only
//!   in a [`Zeroizing`] buffer (wiped on drop), are decoded straight to an
//!   in-memory `HBITMAP`, and the shell's own thumbnail cache holds the
//!   *derivative* bitmap — the same privacy posture as the web client's
//!   decrypted-thumbnail cache.
//!
//! ## Honest fallback
//!
//! If the file has no server thumbnail (`has_thumbnail=false` — e.g. a legacy
//! upload, or a CLI/desktop video with no poster frame), or the fetch/decrypt
//! fails, or the lookup times out, `GetThumbnail` returns an error and Explorer
//! falls back to the file-type icon. No hang, no spinner, no lie.
//!
//! ## How Windows finds this (registration)
//!
//! Beebeeb's desktop app is an **unpackaged** Win32 Tauri exe, so — exactly like
//! [`super::status_ui`] — the COM class object is registered at runtime with
//! `CoRegisterClassObject` and the CLSID's `LocalServer32` + the
//! `IThumbnailProvider` association are written to the registry. The association
//! is scoped to the Beebeeb **sync-root namespace CLSID** (the ShellFolder CLSID
//! the WinRT [`super::register_shell_sync_root`] mints), NOT the whole
//! filesystem: we write `HKCU\Software\Classes\CLSID\{sync-root-clsid}\ShellEx\
//! {IThumbnailProvider-IID} = {our-clsid}`. As belt-and-braces, `Initialize`
//! **also** rejects any path that is not under the live sync root, so even a
//! stray broad association can never make us serve a non-Beebeeb file.
//!
//! ## Threading + no-block contract
//!
//! Explorer calls `GetThumbnail` on a worker thread and expects it to return
//! promptly. We reach the daemon's tokio runtime via [`super::runtime`] (the same
//! handle the hydration callback uses) and `block_on` the fetch **inside a tight
//! [`tokio::time::timeout`]** so a slow network never wedges the shell — on
//! timeout we return an error and the type icon shows.
//!
//! Purely additive: this module does NOT touch the Win32 sync-root registration,
//! the WinRT shell registration, the callbacks, or hydration. A failure to
//! register the thumbnail provider only costs the tiles — everything else keeps
//! working.

#![cfg(target_os = "windows")]

use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, HBITMAP,
    HDC,
};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, CoRegisterClassObject, CoRevokeClassObject, IClassFactory,
    IClassFactory_Impl, REGCLS_MULTIPLEUSE,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};
use windows::Win32::UI::Shell::{
    IInitializeWithItem, IInitializeWithItem_Impl, IShellItem, IThumbnailProvider,
    IThumbnailProvider_Impl, SIGDN_FILESYSPATH, WTS_ALPHATYPE, WTSAT_ARGB,
};
use windows::core::{GUID, HSTRING, Interface, PCWSTR, Result as WinResult, implement};

use zeroize::Zeroizing;

/// Maximum wall time we let the in-memory fetch+decrypt run before giving up and
/// letting Explorer show the type icon. Kept short so a slow/offline network
/// never wedges a shell worker thread.
const THUMBNAIL_FETCH_TIMEOUT: Duration = Duration::from_secs(8);

// ── Stable CLSID for the Beebeeb thumbnail provider ─────────────────────────
//
// Minted once (UUIDv4) and hardcoded so the registry association written at
// registration and the `CoRegisterClassObject` at startup always agree, and so
// `unregister` can find and remove the same key. Distinct from
// `BEEBEEB_PROVIDER_GUID` (the sync-root identity GUID) and
// `STATUS_UI_FACTORY_CLSID`.
//
// `{b2e0a7d4-9c61-4f3a-8d2e-7a1c5e9b0f44}`
pub const THUMBNAIL_PROVIDER_CLSID: GUID =
    GUID::from_u128(0xb2e0_a7d4_9c61_4f3a_8d2e_7a1c5e9b0f44);

/// The shell's `IThumbnailProvider` interface IID, as a brace-wrapped string —
/// the subkey name under a CLSID's `ShellEx` that binds a thumbnail handler.
/// `{e357fccd-a995-4576-b01f-234630154e96}` (verified against windows-0.58).
const ITHUMBNAILPROVIDER_IID_STR: &str = "{E357FCCD-A995-4576-B01F-234630154E96}";

// ── The provider instance: holds the resolved file_id between Initialize and
//    GetThumbnail ─────────────────────────────────────────────────────────────
//
// The shell creates one instance per file via the class factory, calls
// `IInitializeWithItem::Initialize` with the `IShellItem`, then calls
// `IThumbnailProvider::GetThumbnail`. We resolve the shell item → on-disk path →
// `file_id` (via the CF state DB) in `Initialize` and stash it for `GetThumbnail`.
// A `Mutex<Option<String>>` keeps the `#[implement]` type `Sync` without interior
// `unsafe`; contention is nil (one shell thread drives one instance).
#[implement(IThumbnailProvider, IInitializeWithItem)]
struct ThumbnailProvider {
    file_id: Mutex<Option<String>>,
}

impl ThumbnailProvider {
    fn new() -> Self {
        Self {
            file_id: Mutex::new(None),
        }
    }
}

impl IInitializeWithItem_Impl for ThumbnailProvider_Impl {
    fn Initialize(&self, psi: Option<&IShellItem>, _grfmode: u32) -> WinResult<()> {
        use windows::Win32::Foundation::E_INVALIDARG;
        use windows::Win32::System::Com::CoTaskMemFree;

        let item = psi.ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?;

        // Resolve the shell item to its absolute filesystem path. `GetDisplayName`
        // returns a CoTaskMem-allocated NUL-terminated wide string we own and must
        // free with CoTaskMemFree (same pattern as known_folder.rs).
        // SAFETY: shell-provided item; `GetDisplayName` + the free are the
        // documented ownership contract.
        let path = unsafe {
            let pwstr = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            if pwstr.is_null() {
                return Err(E_INVALIDARG.into());
            }
            let s = pwstr.to_string().ok();
            CoTaskMemFree(Some(pwstr.0 as *const _));
            s.unwrap_or_default()
        };
        if path.is_empty() {
            return Err(E_INVALIDARG.into());
        }

        // Map the on-disk path → file_id. This both (a) finds the server file and
        // (b) ENFORCES that the path is under the live Beebeeb sync root — a path
        // outside the root yields no state-DB row, so we never serve a
        // non-Beebeeb file even if the association were broader than intended.
        let file_id = resolve_file_id_for_path(&path)
            // No row (or no sync root / no bridge yet) → let Explorer fall back to
            // the type icon rather than guess.
            .ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?;

        if let Ok(mut slot) = self.this.file_id.lock() {
            *slot = Some(file_id);
        }
        Ok(())
    }
}

impl IThumbnailProvider_Impl for ThumbnailProvider_Impl {
    fn GetThumbnail(
        &self,
        cx: u32,
        phbmp: *mut HBITMAP,
        pdwalpha: *mut WTS_ALPHATYPE,
    ) -> WinResult<()> {
        use windows::Win32::Foundation::{E_FAIL, E_POINTER};

        if phbmp.is_null() || pdwalpha.is_null() {
            return Err(E_POINTER.into());
        }
        // Null the out-params first per the COM contract so a failure leaves no
        // dangling bitmap handle.
        // SAFETY: caller-provided out-pointers, checked non-null above.
        unsafe {
            *phbmp = HBITMAP::default();
            *pdwalpha = WTSAT_ARGB;
        }

        // The file_id was resolved in Initialize. Absent → fail to the type icon.
        let file_id = match self.this.file_id.lock().ok().and_then(|g| g.clone()) {
            Some(id) => id,
            None => return Err(E_FAIL.into()),
        };

        // Pick the variant by the requested size: small thumbs for small tiles,
        // large for big previews. The server falls back gracefully (a missing
        // `large` 404s and we treat that as "no thumbnail").
        let variant = if cx <= 96 {
            "small"
        } else if cx <= 256 {
            "medium"
        } else {
            "large"
        };

        // Fetch + decrypt the thumbnail entirely in memory, on the daemon's tokio
        // runtime, under a tight timeout so Explorer never blocks on the network.
        let decoded = match fetch_thumbnail_blocking(&file_id, variant) {
            Some(bytes) => bytes,
            None => return Err(E_FAIL.into()),
        };

        // Decode the image bytes (WebP from web, JPEG/PNG from other clients) to
        // RGBA8 and build a 32-bit top-down ARGB HBITMAP. `decoded` is zeroized on
        // drop regardless of which branch we leave through.
        let hbitmap = match decode_to_hbitmap(&decoded) {
            Some(h) => h,
            None => return Err(E_FAIL.into()),
        };

        // SAFETY: out-pointers checked non-null above; we hand the bitmap to the
        // shell, which takes ownership and deletes it.
        unsafe {
            *phbmp = hbitmap;
            *pdwalpha = WTSAT_ARGB;
        }
        Ok(())
    }
}

/// Resolve an absolute on-disk path under the sync root to the server `file_id`,
/// or `None` if the path is not under the live sync root / has no state-DB row /
/// the bridge isn't up. This is the path→file_id map AND the sync-root scoping
/// guard in one.
fn resolve_file_id_for_path(path: &str) -> Option<String> {
    let sync_root = super::sync_root()?;
    let p = std::path::Path::new(path);

    // Must be under the sync root (lexical containment — the CF placeholders all
    // live as real on-disk entries beneath the root).
    let rel = p.strip_prefix(&sync_root).ok()?;

    // The state DB stores server-relative, '/'-separated paths (matching the
    // layout `populate_placeholders` mints). Convert the native relative path to
    // that form and look up the row.
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    if rel_str.is_empty() {
        return None;
    }

    let bridge = super::bridge()?;
    let entry = bridge.db().get_file_by_path(&rel_str).ok().flatten()?;
    Some(entry.file_id)
}

/// Run `EngineBridge::fetch_thumbnail_to_memory` on the daemon's tokio runtime
/// under [`THUMBNAIL_FETCH_TIMEOUT`], returning the decrypted (still-encoded)
/// image bytes or `None` on any failure/timeout. The returned buffer is
/// [`Zeroizing`], so the plaintext image bytes are wiped on drop.
fn fetch_thumbnail_blocking(file_id: &str, variant: &str) -> Option<Zeroizing<Vec<u8>>> {
    let bridge = super::bridge()?;
    let handle = super::runtime()?;
    let file_id = file_id.to_string();
    let variant = variant.to_string();

    handle.block_on(async move {
        match tokio::time::timeout(
            THUMBNAIL_FETCH_TIMEOUT,
            bridge.fetch_thumbnail_to_memory(&file_id, &variant),
        )
        .await
        {
            Ok(Ok(bytes)) => Some(bytes),
            Ok(Err(e)) => {
                // Error message carries no plaintext (status strings only).
                tracing::debug!(file_id = %file_id, variant = %variant, error = %e, "thumbnail fetch failed; type icon will show");
                None
            }
            Err(_elapsed) => {
                tracing::debug!(file_id = %file_id, variant = %variant, "thumbnail fetch timed out; type icon will show");
                None
            }
        }
    })
}

/// Decode the decrypted thumbnail image bytes to a 32-bit top-down ARGB
/// `HBITMAP` (premultiplied not required for `WTSAT_ARGB`). Returns `None` on a
/// decode/allocation failure so the caller falls back to the type icon.
///
/// `image_bytes` is borrowed; the decoded RGBA scratch buffer lives only for the
/// duration of this function and the pixels are copied into the DIB section the
/// shell owns. We do not retain the plaintext beyond the copy.
fn decode_to_hbitmap(image_bytes: &[u8]) -> Option<HBITMAP> {
    // Decode to RGBA8. `image` sniffs the format (WebP/JPEG/PNG/GIF/BMP) from the
    // bytes, so we don't need the server's content type.
    let img = image::load_from_memory(image_bytes).ok()?;
    let rgba = img.to_rgba8();
    let (width, height) = (rgba.width() as i32, rgba.height() as i32);
    if width <= 0 || height <= 0 {
        return None;
    }

    // Build a top-down 32-bit BGRA DIB. Windows DIBs are BGRA in memory; a
    // NEGATIVE biHeight makes the rows top-down (matching `image`'s row order).
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        ..Default::default()
    };

    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `bmi` is a valid BITMAPINFO for a 32bpp top-down DIB; `bits`
    // receives the pixel buffer pointer the GDI object owns. `HDC::default()` =
    // the screen DC. `HANDLE::default()` (null hsection) asks GDI to allocate.
    let hbitmap = unsafe {
        CreateDIBSection(
            HDC::default(),
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            windows::Win32::Foundation::HANDLE::default(),
            0,
        )
    }
    .ok()?;

    if bits.is_null() {
        // SAFETY: clean up the GDI object we just created before bailing.
        unsafe {
            let _ = DeleteObject(hbitmap);
        }
        return None;
    }

    // Copy RGBA → BGRA into the DIB pixel buffer. `image` gives tightly-packed
    // RGBA8 rows; a 32bpp DIB row stride is width*4 with no padding, so a per-
    // pixel swizzle into the contiguous buffer is correct.
    let pixel_count = (width as usize) * (height as usize);
    let src = rgba.as_raw(); // length == pixel_count * 4, RGBA order
    if src.len() < pixel_count * 4 {
        unsafe {
            let _ = DeleteObject(hbitmap);
        }
        return None;
    }
    // SAFETY: `bits` points at a buffer of exactly `pixel_count * 4` bytes owned
    // by the DIB (32bpp, top-down, no padding); we write that many bytes.
    let dst = unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, pixel_count * 4) };
    rgba_to_bgra_into(src, dst, pixel_count);

    Some(hbitmap)
}

/// Swizzle `pixel_count` tightly-packed RGBA8 pixels (`src`) into the BGRA byte
/// order a 32bpp Windows DIB expects (`dst`). Both slices must hold at least
/// `pixel_count * 4` bytes. Pulled out as a pure fn so the load-bearing channel
/// order is unit-testable without a live GDI/HBITMAP (the rest of
/// `decode_to_hbitmap` is unavoidably FFI).
fn rgba_to_bgra_into(src: &[u8], dst: &mut [u8], pixel_count: usize) {
    for i in 0..pixel_count {
        let s = i * 4;
        // RGBA (source) → BGRA (DIB)
        dst[s] = src[s + 2]; // B
        dst[s + 1] = src[s + 1]; // G
        dst[s + 2] = src[s]; // R
        dst[s + 3] = src[s + 3]; // A
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_to_bgra_swizzle_is_correct() {
        // Two pixels: (R=10,G=20,B=30,A=40) and (R=50,G=60,B=70,A=80).
        let src = [10u8, 20, 30, 40, 50, 60, 70, 80];
        let mut dst = [0u8; 8];
        rgba_to_bgra_into(&src, &mut dst, 2);
        // BGRA: (B=30,G=20,R=10,A=40), (B=70,G=60,R=50,A=80).
        assert_eq!(dst, [30, 20, 10, 40, 70, 60, 50, 80]);
    }

    #[test]
    fn media_extensions_cover_image_and_video() {
        // Sanity: the association list includes the reported formats (jpg image
        // + mp4 video) so Explorer asks our handler for both.
        assert!(MEDIA_EXTENSIONS.contains(&".jpg"));
        assert!(MEDIA_EXTENSIONS.contains(&".png"));
        assert!(MEDIA_EXTENSIONS.contains(&".webp"));
        assert!(MEDIA_EXTENSIONS.contains(&".mp4"));
    }
}

// ── IClassFactory so `CoRegisterClassObject` can hand out provider instances ──
#[implement(IClassFactory)]
struct ThumbnailClassFactory;

impl IClassFactory_Impl for ThumbnailClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Option<&windows::core::IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut core::ffi::c_void,
    ) -> WinResult<()> {
        use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_POINTER};
        if ppvobject.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: shell passes a valid out-pointer; null it first per COM contract.
        unsafe { *ppvobject = std::ptr::null_mut() };
        if punkouter.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        // A fresh provider instance per file; QueryInterface to the requested
        // interface (IThumbnailProvider or IInitializeWithItem).
        let provider: IThumbnailProvider = ThumbnailProvider::new().into();
        // SAFETY: `riid`/`ppvobject` come from the shell's QueryInterface-style
        // call; `query` writes the requested interface (or an error).
        unsafe { provider.query(riid, ppvobject).ok() }
    }

    fn LockServer(&self, _flock: windows::Win32::Foundation::BOOL) -> WinResult<()> {
        Ok(())
    }
}

/// COM registration cookie from `CoRegisterClassObject`, kept so
/// [`unregister_thumbnail_provider`] can `CoRevokeClassObject` it. Set once per
/// process.
static CLASS_OBJECT_COOKIE: OnceLock<u32> = OnceLock::new();

/// Media file extensions we register the thumbnail handler for. We cannot bind
/// the handler to the WinRT-minted sync-root namespace CLSID (its value is
/// internal to `StorageProviderSyncRootManager::Register` and not exposed to us),
/// so we associate per-extension under **HKCU** `SystemFileAssociations` and rely
/// on the `Initialize` guard — which rejects any path not under the live sync
/// root — to keep us from EVER serving a non-Beebeeb file. The association is
/// HKCU-only (current user) and only covers media types, so a user's existing
/// per-app thumbnailers for these types elsewhere are unaffected when the file is
/// outside the sync root (our handler refuses it → shell falls through).
const MEDIA_EXTENSIONS: &[&str] = &[
    // images
    ".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".heic", ".heif", ".tif", ".tiff", ".avif",
    // video (poster frames the uploader generated)
    ".mp4", ".mov", ".m4v", ".webm", ".mkv", ".avi",
];

/// Register the Beebeeb thumbnail provider so Explorer renders media tiles for
/// cloud-only placeholders under the sync root.
///
/// Three parts, mirroring [`super::status_ui::register_status_ui_source`]:
/// 1. `CoRegisterClassObject` so this running exe is the local server for the
///    CLSID (once per process).
/// 2. Write `HKCU\Software\Classes\CLSID\{clsid}\LocalServer32 = "<exe>"` so the
///    shell can launch us on demand.
/// 3. Associate the CLSID as the `IThumbnailProvider` handler for each media
///    extension under HKCU:
///    `HKCU\Software\Classes\SystemFileAssociations\{.ext}\ShellEx\
///    {IThumbnailProvider-IID} = {clsid}`. Scoping to the Beebeeb sync root is
///    enforced at runtime by `Initialize` (a path outside the root → no state-DB
///    row → we refuse and the shell falls back), NOT by the registry breadth.
///
/// PURELY ADDITIVE + best-effort. Idempotent: the class-object registration is
/// guarded by `CLASS_OBJECT_COOKIE`; the registry writes are create-or-overwrite.
///
/// `CoRegisterClassObject` requires the calling thread to have initialized COM
/// (the caller handles that on its dedicated STA thread).
pub fn register_thumbnail_provider() -> anyhow::Result<()> {
    // 1. Register the class object so this exe is the CLSID's local server.
    if CLASS_OBJECT_COOKIE.get().is_none() {
        let factory: IClassFactory = ThumbnailClassFactory.into();
        // SAFETY: `THUMBNAIL_PROVIDER_CLSID` is a valid CLSID; the factory is
        // moved into COM's keeping via the returned cookie.
        let cookie = unsafe {
            CoRegisterClassObject(
                &THUMBNAIL_PROVIDER_CLSID,
                &factory,
                CLSCTX_LOCAL_SERVER,
                REGCLS_MULTIPLEUSE,
            )
        }
        .map_err(|e| anyhow::anyhow!("CoRegisterClassObject(thumbnail provider) failed: {e}"))?;
        let _ = CLASS_OBJECT_COOKIE.set(cookie);
        tracing::info!(cookie, "thumbnail provider class object registered");
    }

    // 2. Write the CLSID's LocalServer32 (the exe path).
    let clsid_str = format!("{{{:?}}}", THUMBNAIL_PROVIDER_CLSID);
    let exe = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("resolve current exe for LocalServer32: {e}"))?;
    let exe_str = exe.to_string_lossy().into_owned();
    let local_server_key = format!("Software\\Classes\\CLSID\\{clsid_str}\\LocalServer32");
    write_string_value(HKEY_CURRENT_USER, &local_server_key, None, &exe_str)
        .map_err(|e| anyhow::anyhow!("write thumbnail CLSID LocalServer32: {e}"))?;

    // 3. Associate the handler per media extension. A single failed extension is
    //    logged and skipped, not fatal — the rest still register.
    let mut bound = 0u32;
    for ext in MEDIA_EXTENSIONS {
        let shellex_key = format!(
            "Software\\Classes\\SystemFileAssociations\\{ext}\\ShellEx\\{ITHUMBNAILPROVIDER_IID_STR}"
        );
        match write_string_value(HKEY_CURRENT_USER, &shellex_key, None, &clsid_str) {
            Ok(()) => bound += 1,
            Err(e) => tracing::warn!(ext, error = %e, "bind ThumbnailProvider for extension failed"),
        }
    }

    tracing::info!(
        clsid = %clsid_str,
        extensions = bound,
        "Explorer thumbnail provider registered (scoped to sync root via Initialize guard)"
    );
    Ok(())
}

/// Remove the thumbnail-provider registration (inverse of
/// [`register_thumbnail_provider`]). Revokes the COM class object and deletes the
/// per-extension ShellEx bindings + the CLSID subtree. Best-effort: each step is
/// independent and a failure is logged, never propagated.
pub fn unregister_thumbnail_provider() {
    if let Some(&cookie) = CLASS_OBJECT_COOKIE.get() {
        // SAFETY: `cookie` is the value our own CoRegisterClassObject returned.
        if let Err(e) = unsafe { CoRevokeClassObject(cookie) } {
            tracing::warn!(error = %e, "CoRevokeClassObject(thumbnail provider) failed");
        }
    }

    // Remove each per-extension ShellEx interface subkey (leave the rest of the
    // SystemFileAssociations\.ext tree, which is shared/OS-owned).
    for ext in MEDIA_EXTENSIONS {
        let shellex_key = HSTRING::from(format!(
            "Software\\Classes\\SystemFileAssociations\\{ext}\\ShellEx\\{ITHUMBNAILPROVIDER_IID_STR}"
        ));
        // SAFETY: deletes only our interface subkey under HKCU.
        let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &shellex_key) };
    }

    // Remove our CLSID subtree.
    let clsid_str = format!("{{{:?}}}", THUMBNAIL_PROVIDER_CLSID);
    let clsid_key = HSTRING::from(format!("Software\\Classes\\CLSID\\{clsid_str}"));
    // SAFETY: deletes our own CLSID subtree under HKCU.
    let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &clsid_key) };

    tracing::info!("Explorer thumbnail provider unregistered");
}

/// Create-or-open `subkey` under `root` and set a REG_SZ value (`value_name`
/// `None` ⇒ the key's default value) to `data`. Closes the key before returning.
/// (Local copy mirroring [`super::status_ui`]'s helper so this module is
/// self-contained.)
fn write_string_value(
    root: HKEY,
    subkey: &str,
    value_name: Option<&str>,
    data: &str,
) -> anyhow::Result<()> {
    let subkey_h = HSTRING::from(subkey);
    let mut hkey = HKEY::default();
    // SAFETY: out-params are stack locals; `subkey_h` outlives the call.
    let rc = unsafe {
        RegCreateKeyExW(
            root,
            &subkey_h,
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(anyhow::anyhow!("RegCreateKeyExW({subkey}) failed: {:?}", rc));
    }

    // REG_SZ data is a NUL-terminated wide string; pass the bytes including the
    // trailing NUL.
    let wide: Vec<u16> = data.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes: &[u8] =
        unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };
    let name_h = value_name.map(HSTRING::from);
    let name_pcwstr = name_h.as_ref().map_or(PCWSTR::null(), |h| PCWSTR(h.as_ptr()));
    // SAFETY: `hkey` is open; `bytes` outlives the call; `name_pcwstr` valid for
    // the call's duration (HSTRING kept alive in `name_h`).
    let set_rc = unsafe { RegSetValueExW(hkey, name_pcwstr, 0, REG_SZ, Some(bytes)) };
    // SAFETY: close the key we opened regardless of the set result.
    let _ = unsafe { RegCloseKey(hkey) };
    if set_rc != ERROR_SUCCESS {
        return Err(anyhow::anyhow!("RegSetValueExW failed: {:?}", set_rc));
    }
    Ok(())
}

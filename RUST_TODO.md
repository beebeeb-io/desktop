# Pending Rust work tracked from the TS side

Frontend tasks ship in advance of the matching Rust IPC handlers.
The pages handle the missing commands gracefully (warning banners,
disabled actions) until the Rust side lands. This file lists what's
outstanding so rust-engineer can pick them up.

Source of truth for each item: `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`.

## Task 8 — native settings window (ts-engineer commit `1e7cf8f`)

In `src-tauri/src/lib.rs`, replace whatever currently loads
`app.beebeeb.io` (likely `WebviewWindowBuilder::from_config(...)` in
the Tauri config) with a local-asset settings window:

```rust
tauri::WebviewWindowBuilder::new(
    app,
    "settings",
    tauri::WebviewUrl::App("index.html".into()),
)
.title("Beebeeb")
.inner_size(680.0, 540.0)
.resizable(false)
.decorations(true)
.visible(false)  // shown only when user opens from tray
.build()?;
```

Tray "Open Settings" handler:

```rust
"open_settings" => {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}
```

## Task 9 — settings page IPC (ts-engineer commit `659e5a0`)

Pages call these commands and degrade if they're missing:

- `get_desktop_config` → `Result<DesktopConfig, String>`
- `set_desktop_config` → `Result<(), String>`, `{ config: DesktopConfig }`
- `account_email` → `Result<Option<String>, String>` (best-effort)

`DesktopConfig` shape used by Bandwidth.tsx + Notifications.tsx:

```ts
interface DesktopConfig {
  upload_kbps_limit: number   // KB/s, 0 = unlimited
  download_kbps_limit: number
  pause_sync: boolean
  notify_conflicts: boolean
  notify_sync_complete: boolean
  notify_quota_warnings: boolean
}
```

Persistence: extend `desktop.toml` next to the existing `sync_root`
field. The plan describes this in §1463.

Also: Status.tsx expects `sync_status` to return `{ logged_in, engine,
sync_root, syncing, cloud_only, conflicts }`. The current handler
only returns `{ logged_in, sync_root, engine_running }`. Expanding
this is mentioned in the plan but not yet done.

## Task 12 — conflict window IPC (ts-engineer commit pending)

`ConflictWindow.tsx` is opened with the URL
`index.html?window=conflict&fileId=...&fileName=...&isText=true|false`.

Two new IPC commands needed:

```rust
#[tauri::command]
async fn open_conflict_window(
    app: tauri::AppHandle,
    file_id: String,
    file_name: String,
    is_text: bool,
) -> Result<(), String> {
    let url = format!(
        "index.html?window=conflict&fileId={file_id}&fileName={enc_name}&isText={is_text}",
        enc_name = urlencoding::encode(&file_name),
    );
    tauri::WebviewWindowBuilder::new(
        &app,
        format!("conflict-{file_id}"),
        tauri::WebviewUrl::App(url.into()),
    )
    .title(format!("Conflict — {file_name}"))
    .inner_size(720.0, 480.0)
    .resizable(true)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn resolve_conflict(
    state: tauri::State<'_, AppState>,
    file_id: String,
    choice: String,  // "local" | "remote" | "both"
) -> Result<(), String> {
    let session = state.session.lock().await;
    if session.is_none() {
        return Err("not signed in".into());
    }
    // TODO: wire to engine_bridge::resolve_conflict(file_id, choice)
    tracing::info!("Conflict resolved: {file_id} → {choice}");
    Ok(())
}
```

Register both in the `invoke_handler!` macro alongside the existing
commands.

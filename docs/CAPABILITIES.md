# Desktop capability contract — 2026-09-29 (task 1611)

The shipping UI is desktop-owned React (`src/main.tsx`, compact `App.tsx`, main
`WindowsApp.tsx`). `?platform=windows` selects a layout even on a Mac; it cannot
authorize host actions. Rust `desktop_capabilities` supplies the actual host.
The active scheduler is desktop `runner.rs` / `engine_bridge.rs` / `state_db.rs`;
core supplies shared crypto/protocol primitives.

| Surface | Compact shell | Main shell | Commands |
| --- | --- | --- | --- |
| Trash | Main window entry | Trash | desktop_trash_list / desktop_trash_restore / desktop_trash_delete_permanently |
| History | Global dialog, Versions & conflicts | Global dialog, Files action | list_file_versions / restore_file_version / list_version_conflict_center |
| Activity | Main window entry | Home summary, Activity; Windows tray recent list | account_activity / account_activity_feed / tray_recent_activity |
| Notifications | Local preferences | Activity inbox, Settings preferences | account_notifications / account_mark_notification_read / account_mark_all_notifications_read |
| Recipient shares | Mac File Provider namespace; Shared.tsx unmounted | Filesystem read access, no administration page | list_shared_roots; EngineBridge shared metadata/decryption |

Preserve supported routes and File Provider behavior. Unsupported actions are
absent, including deep-link entry points, with a web alternative. Do not expose
the unfinished Shared.tsx. Share creation/invites stay in the web app (1243/1245).

Backend capabilities describe implemented support, not authenticated state or
successful installation. File Provider and CFAPI availability do not mean the
root is registered; existing installation-state commands still own that fact.
Native credentials mean an implemented OS backend, not an unlocked vault.
Linux currently has no native credential backend and runner does not mount the
FUSE prototype: report no on-demand integration, web-only sharing, and unknown
tray visibility. This makes no decision about the future Linux filesystem model.
Unknown package formats remain unknown; only known updater-compatible formats
offer in-app install. Package-managed Linux installs direct users to their package
manager. No credentials or mount probes are performed by this read-only command.

Verification: backend JSON fixtures, frontend route/render checks and browser
screenshots; Linux suites; Desktop CI Rust check (macOS) --all-targets, followed
by operator Mac unlock/Finder enumerate-hydrate-edit/sign-out/re-login before
promotion of shared IPC changes. Native evidence is never inferred from fixtures.

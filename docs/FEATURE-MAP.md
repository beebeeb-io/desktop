# Feature map

Status of each desktop v2 surface, from the code on this branch. Live means the screen reads or writes a command named in the last column. Placeholder means the screen draws fixed figures, or a control that closes without calling a command. Missing means the screen says the command is not in this build and does not invent the data.

The id is the pane, dialog, or section id. Settings ids come from `src/v2/sections/settings/shell/panes.ts`, `panes-today.ts`, and `panes-account.ts`. Where one id is registered twice, the row says which one the window mounts (`DIALOGS_TODAY` overrides `DIALOGS`).

| Surface | Id | Where | Status | What it reads, or why it is not live |
|---|---|---|---|---|
| General | general | src/v2/sections/settings/shell/general.tsx | Live | get_desktop_config (theme) and app_version. The Storage location row shows the fixed word Europe and opens region. |
| Notifications | notifications | src/v2/sections/settings/shell/notifications.tsx | Live | get_desktop_config and set_desktop_config. Each switch is stored. The unread count is account_notifications. |
| Storage | storage | src/v2/sections/settings/drive/storage.tsx | Live | drive_storage for used and quota, and desktop_storage_summary for this PC. Preview bytes are not reported. Clear previews is unsupported. |
| Thumbnails and previews | thumbnails | src/v2/sections/settings/drive/thumbnails.tsx | Live | get_desktop_config and set_desktop_config for photo_previews, preview_size and preview_format. Usage comes from the storage summaries, or says it is not reported yet. Rebuild previews is unsupported and does not start a rebuild. |
| Compare previews | compare | src/v2/sections/settings/drive/thumbnails.tsx | Live | Compares the saved size and format with the original only when both are known. The frame is blank. Choose photo is unsupported. |
| Backups | backups | src/v2/sections/settings/backups/index.tsx | Live | get_known_folder_backup, drive_status (the backup view, no meter), drive_sync_problems, and drive_list of the Backups folder. Back up now is unsupported. |
| Backup failures | backup-failures | src/v2/sections/settings/backups/index.tsx | Live | drive_sync_problems (the backup rows) and drive_sync_retry. |
| A device's backup | device | src/v2/sections/settings/backups/index.tsx | Live | No device command. The row is the Backups folder drive_list already read (name, child count, modified time). With no device, it says to choose one. |
| Bandwidth | bandwidth | src/v2/sections/settings/today/bandwidth.tsx | Live | app_activity_snapshot, get_bandwidth_history, drive_transfers, and drive_sync_summary. Limits go through set_desktop_config. |
| Storage insights | insights | src/v2/sections/settings/today/insights.tsx | Live | account_storage_breakdown and drive_storage. The insights dialog only redirects to this page. |
| Recent notifications | recent | src/v2/sections/settings/today/recent.tsx | Live | account_notifications. No mark-read command is registered, so opening the page leaves the unread count as it was. |
| Storage location | location | src/v2/sections/settings/today/location.tsx | Missing | No get_storage_location or set_storage_location in this build. The page says so and does not draw a country list. |
| Diagnostics | diagnostics | src/v2/sections/settings/today/diagnostics.tsx | Live | app_activity_snapshot, once a second while the page is open, and report_problem. |
| About | about | src/v2/sections/settings/today/about.tsx | Live | app_version, get_desktop_config for the release channel (saved with set_desktop_config), and check_for_updates_now. |
| Sync and File Explorer | sync | src/v2/sections/settings/today/sync.tsx | Live | drive_sync_summary, windows_shell_integration_state, desktop_storage_summary, get_desktop_config, and autostart_enabled. Taking Beebeeb out of File Explorer is unsupported. |
| Account | account | src/v2/sections/settings/account/AccountPane.tsx | Live | account_profile and account_display_name. |
| Security | security | src/v2/sections/settings/account/SecurityPane.tsx | Live | account_security_score, account_passkeys_list, account_activity, and account_activity_logging. App lock says it is not available and cannot be changed. |
| Devices and sessions | devices | src/v2/sections/settings/account/DevicesPane.tsx | Live | account_session_list, with sync_status, app_version, and desktop_platform for this PC. Sign out calls account_revoke_session. |
| Help and report a problem | report | src/v2/sections/settings/shell/general.tsx | Live | report_problem. |
| Your plan | plan | src/v2/sections/settings/shell/general.tsx | Live | account_subscription. Paying opens the website. |
| Storage location dialog | region | src/v2/sections/settings/panes-today.ts | Missing | The mounted dialog is LocationRedirect, which opens the location page. RegionDialog in general.tsx is not what opens. |
| Preview limit | limit-previews | src/v2/sections/settings/drive/storage.tsx | Missing | No config field and no command report a preview limit. The dialog says there is nothing to change. |
| Download limit | limit-downloads | src/v2/sections/settings/drive/storage.tsx | Live | local_cache_limit_bytes through get_desktop_config and set_desktop_config, and desktop_storage_summary for bytes in use. |
| Free up space | free-up | src/v2/sections/settings/drive/storage.tsx | Live | free_up_space, with the size from desktop_storage_summary. |
| Turn off File Explorer | explorer-off | src/v2/sections/settings/drive/sync.tsx | Live | unsent_changes for the warning count. Turn off is unsupported and changes nothing. |
| Apply preview settings | apply | src/v2/sections/settings/drive/thumbnails.tsx | Live | Confirm writes preview_size and preview_format. Cancel writes nothing. New preview bytes say they are not reported yet. |
| Stop backing up a folder | stop | src/v2/sections/settings/backups/index.tsx | Placeholder | Both actions only close the dialog. No command stops a folder backup. |
| Delete this backup | delete-backup | src/v2/sections/settings/backups/index.tsx | Missing | No command deletes another device's backup. The dialog refuses and deletes nothing. |
| Choose a folder to back up | picker | src/v2/sections/settings/backups/index.tsx | Missing | No command adds a folder beyond the known folders. Back up refuses and adds nothing. |
| Repair File Explorer | repair | src/v2/sections/settings/today/sync.tsx | Missing | No command repairs the File Explorer registration. Repair says so and changes nothing. |
| Keep on this PC | keep | src/v2/sections/settings/today/keep.tsx | Live | list_vault_folders and get_selective_sync. Apply writes set_selective_sync. |
| Stop keeping folders | keep-confirm | src/v2/sections/settings/today/keep.tsx | Live | set_selective_sync, using the tree Keep already read. |
| Release notes | release-notes | src/v2/sections/settings/today/about.tsx | Live | The version, body, and notes address from the last check_for_updates_now answer. With none yet, it says to check. |
| Files | files | src/v2/sections/files/index.tsx | Live | drive_list through FolderView. |
| Search | search | src/v2/sections/search/index.tsx | Live | drive_search, and drive_paths for each hit's folder. Recent queries stay in localStorage. |
| Photos | photos | src/v2/sections/photos/index.tsx | Live | drive_photos, drive_photo_meta, and drive_photo_sources. A device's backup progress is in no command, so that line stays absent. |
| Shared | shared | src/v2/sections/shared/index.tsx | Live | drive_shared and share_list. Actions use the share_* and file_request_* commands. |
| Available offline | available-offline | src/v2/sections/available-offline/index.tsx | Live | drive_offline_list. Remove calls set_recursive_pin. |
| Trash | trash | src/v2/sections/trash/index.tsx | Live | drive_trash and drive_restore_from_trash. Permanent delete calls drive_delete_permanently. |
| Backups section | backups | src/v2/sections/backups/index.tsx | Live | drive_list, rooted at the Backups folder (FolderView mode backups). |
| Sync history | sync-history | src/v2/sections/sync-history/index.tsx | Live | popover_snapshot, sync_activity_summary, and drive_sync_problems. Try again is drive_sync_retry. |
| Open | open | src/v2/sections/open/index.tsx | Live | drive_read_text, drive_read_bytes, drive_save_text, and drive_save_bytes through DriveSource. |
| Settings | settings | src/v2/sections/settings/index.tsx | Live | No data command of its own. It mounts the pane and dialog tables for the page route. |
| Guide | guide | src/v2/guide/Guide.tsx | Live | No engine command. startGuide stores bb.v2.guide.due in localStorage, and GuideHost draws the tips over the open window. |
| Onboarding | onboarding | src/v2/onboarding/index.tsx | Live | sync_status, get_known_folder_backup, set_known_folder_backup, windows_shell_integration_state, and autostart_enabled. The photos step still shows a fixed 4K HEIC sample (30.9 MB) and does not save it. Lock and notification choices are only a local summary. |
| Auth gate | auth | src/v2/auth/Gate.tsx | Live | sync_status, then account_subscription. Signed out continues in AccountEntry (desktop_login). Locked uses auth_unlock_with_windows_hello. |
| Tray | tray | src/v2/tray/index.tsx | Live | popover_snapshot and drive_status. Each action is one existing command. |

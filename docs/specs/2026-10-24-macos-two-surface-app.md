# 2026-10-24 — macOS: two surfaces, the menu-bar popover and one Beebeeb window (task 1683)

**Status:** ~~design for Guus's review.~~ Approved by Guus on 24 Oct 2026, with the four rulings in §0; slice 1 starts
after this amendment merges. This PR changes no code; it adds this spec and the hi-fi file, and amends three
earlier specs in place (§2). ~~Code starts only after Guus approves (task 1683, "Next, design before code").~~ — rulings
amendment, 2026-10-24
**Date:** 24 Oct 2026
**Repo:** desktop. macOS only. Windows and Linux keep their windows (§12, every slice says what it leaves alone).
**Task:** 1683 (P1, epic). This spec takes the place of the "spec B" that specs A and C deferred to
(`2026-10-06-macos-finder-setup-reconciler.md` line 9, `2026-10-07-desktop-first-run-sign-in.md` line 9: "the popover
becomes the only macOS surface"), revised by the ruling below to two surfaces.
**Hi-fi:** [`design/hifi/macos-two-surface.html`](../../design/hifi/macos-two-surface.html): popover states, the
signed-out window (sign-in, browser code, email and password, sign in again, recovery phrase, Finder, account switch),
Home, Review with the conflict sheet, Trash, ~~every Settings tab~~ every Settings section in System Settings style
(each drawn light and dark, plus Settings while signed out), the 1887 sheets and the kept alert, light and dark.
**Builds on, unchanged in behaviour:** spec A (Finder reconciler), spec C (first run is sign-in only; approved, not yet
built, tasks 1747/1748), spec 2026-10-10 (task 1887, sign-out keeps waiting Finder saves), spec 2026-10-09 (task 1882).
**Read at:** desktop `origin/main` `5ef050dd` (24 Oct 2026). `file:line` cites that tree; `lib.rs` means
`src-tauri/src/lib.rs`, `.tsx`/`.ts` paths are under `src/`.

## 0. Rulings (Guus, 2026-10-24)

Guus answered the three questions of §13 and the go/no-go on 24 Oct 2026. The lead relayed them to this lane; the
option he chose is quoted verbatim in each row, and the description after it is the lead's, as relayed. At the time of
writing they were not yet in task 1683's file.

| # | Question | Ruling (the option chosen) | What it changes here |
| --- | --- | --- | --- |
| 1 | Settings layout (§13 question 1) | **"System Settings style"**: a sidebar list of sections with the pane on the right, like macOS System Settings. This replaces the toolbar tabs the spec recommended. | §7.8 is rewritten: the gear switches the window's one sidebar to the Settings section list, with a back control. Rows C1, F11, F15; §7.1, §7.2, §7.6, §8.2, §9.3, §10.2, §11; slices 1, 5 and 7. |
| 2 | Windows-shell pages that do not come to the Mac (§13 question 2) | **"Drop them (Recommended)"**: the Bandwidth chart and sessions, App Activity, the account notifications list, and the 4 switches macOS never honours stay off the Mac app (still available in the web app). | Rows E10, E15, E17, E18, E20, E29 and E30 stay Removed, now by ruling. |
| 3 | The app menu's "Sign Out…" (§13 question 3) | **"Yes, same sheet (Recommended)"**: once the 1887 device rung (workspace task 2175) passes, the app-menu Sign Out… goes through the same confirm sheet, so there is no silent sign-out from the menu. | Rows F4 and A14, §8.1 entry point 5, and a new slice 1b gated on task 2175. |
| 4 | Go / no-go | **"Yes, start slice 1"**: implementation starts after this amendment merges. | Status line; slice 0. |
| 5 | Updates on a Mac (task 2176; the lead's decision of 24 Oct 2026 22:47 under Guus's standing "take decisions, follow your recommendation") | **"Download update", never an in-place install.** The App Sandbox denies the install, so on a Mac an update is a link to `https://beebeeb.io/download/macos-dmg` with the line "Drag Beebeeb into Applications to replace this version." | Rows C15, E27, #27, #28 and §7.8's About row below, and the popover's update row (A19, §6.3, the popover lane's). The confirm-downgrade sheet (E27) is out on a Mac until an install mechanism exists (task 2176's next step). — slice 5, 2026-10-25, lead-approved (macos-beebeeb-teamlead) |

**One correction to ruling 2's description, checked against web `fc4b9aa` (24 Oct 2026).** "Still available in the web
app" holds for the account notifications list only: the web app has its notification inbox
(`src/components/notification-inbox.tsx`). The Bandwidth chart and live sessions (E10), App Activity (E30) and the four
switches (E17, E18, E20, E29) are views of this device's engine. The web app has none of them; they stay in the Windows
app, which this spec does not change. The ruling itself, "drop them" on a Mac, is unaffected.

## 1. The ruling

Guus, 24 Oct 2026, verbatim (during the 1887 device rung; recorded in task 1683 on workspace `origin/main`):

> "we already created an artifact, but how the signin is right now and that we have multiple screens doesnt make
> sense. So i want max 2 screens, the little one below the B in the topbar and just the app itself where you get
> signin, etc etc. Including settings. The gear menu described in the side menu should just navigate to the bigger
> screen settings and should look the same as macos apps regularly look like."

The design he refers to (30 Sep, "The macOS app becomes a menu-bar popover") answered this one, which still stands:

> "I want it more like how proton looks. Now also our version is not nicely aligned, not small, has too much info etc.
> Also when i just start beebeeb that window is randomly in the screen and not at the top bar. Often see too many
> windows going around."

**What this spec reads the ruling to mean:**

1. **Exactly two surfaces Beebeeb draws on a Mac:** the popover under the b status item, and one main window (called
   "the Beebeeb window" in copy and in this spec; window label `main`).
2. **The Beebeeb window holds everything else:** sign-in, the account switch, setting up this Mac (recovery phrase),
   the Finder status, Home, Review, Trash, and Settings. Every question, sheet and confirmation opens inside it.
3. **"The gear menu … should just navigate to the bigger screen settings":** the popover's gear menu item
   "Settings…" opens the Beebeeb window on Settings, and the gear at the bottom of the window's sidebar selects
   Settings inside the same window. Neither opens a separate Settings window.
4. **"Should look the same as macOS apps regularly look like":** ~~Settings is drawn as a standard macOS Settings pane:
   icon tabs in the toolbar, grouped inset rows, label left, control right, footnotes under groups (§7.8; open
   question 1 offers the System Settings variant).~~ Settings is drawn in System Settings style (ruling 1): a sidebar
   list of sections, the selected section's pane on the right, grouped inset rows, label left, control right,
   footnotes under groups (§7.8). — rulings amendment, 2026-10-24
5. **Not surfaces in this sense:** what macOS itself draws for us and that holds no Beebeeb content of its own, namely
   the status item and its tooltip, native alerts (`NSAlert`, shown by UserNotificationCenter when there is no parent
   window), notifications in Notification Center, the native menu bar, the native menus, and the system file picker.
   They stay where 1882/1887 and spec C put them (§5 rows A12–A18). The native About panel is the one exception: it
   shows Beebeeb's own version and channel, so it moves into Settings › About (row A9).

## 2. What this supersedes, and where it is amended

Each earlier text is amended in place: the old words are struck (`~~…~~`) and kept, with a pointer to this spec
beneath, signed "— two-surface design, 2026-10-24".

| Earlier text | What changes | Amended in |
| --- | --- | --- |
| Popover spec: "Settings shrinks to 4 tabs … (shipped, slice 4)" | Settings is a ~~pane in~~ mode of the Beebeeb window, not a window; it gains a fifth ~~tab~~ section, Security, in a System Settings style sidebar list (§7.8, ruling 1). | `2026-09-30-macos-menubar-popover.md` |
| Popover spec: state "signed out" | No signed-out popover: with no session a click opens the Beebeeb window on sign-in (spec C C-W3, kept). | same |
| Popover ruling 2, "Dock icon … while Settings, onboarding or review is open" | A Dock icon while the Beebeeb window is open (§9.1). | same |
| Popover ruling 4, "Version history stays, in a single review window" | Version history and conflict review are sheets in the Beebeeb window's Review section. No review window. | same |
| Popover ruling 6, "Settings opens top-right under the menu bar … Onboarding stays centred (860×640)" | The Beebeeb window opens where it was left, else centred on the icon's display (§7.2). The anchored Settings placement (slice 6) retires. | same |
| Popover spec: "Fewer windows: 6→4 window kinds, 4→1 idle webviews" | 7 → 2 window kinds; 1 idle webview (§4). | same |
| Settings dialogs spec: the three dialogs and the 1887 sheets live in the Settings window; "closing Settings ends the flow" | Same dialogs, same copy, same keys, hosted by the Beebeeb window's Settings pane; closing the Beebeeb window ends the flow (§8). | `2026-10-02-macos-settings-dialogs.md` |
| Spec C C-W3: "In Ready, a click opens the compact window on spec A's Finder location view … until spec B" | In Ready a click toggles the popover. | `2026-10-07-desktop-first-run-sign-in.md` |
| Spec C C-W7: the Keychain unlock opens the Settings window | The popover's "Unlock vault" unlocks in place; Settings › Account keeps its Unlock row. | same |
| Spec C C-W8: "The account window is the existing `onboarding` window … centred at 860×640" | The account window is the Beebeeb window in its signed-out state (§7.4). Its title still follows the screen. | same |
| Spec C §7.5 "Where": the Ready notice lives in Settings until spec B moves it to the popover | The notice shows in the popover header and in Settings › Account (§6.3). | same |
| Spec C §7.6: the window "closes by itself on Ready" | It moves on to Home and stays open (§7.5). | same |
| Spec C C-W4 and C-W9: in Ready a manual launch keeps "today’s launch behaviour" (the compact window, keyed on `sync_root`) | A manual launch opens the Beebeeb window on Home; an updater relaunch opens nothing (§7.3). | same |
| Task 1683 slice 6 (`macos-settings` window, anchored, `show_macos_settings_window`) | Retired by slice 1 here. | task file (lead) |

Specs 2026-10-10 (1887) and 2026-10-09 (1882) keep every rule, sentence and test. Where they say "Settings" or
"the Settings window", read "the Settings pane of the Beebeeb window"; §8 maps each of their surfaces. They are not
edited here, because 1887's device rung is still open and its text is that rung's reference.

## 3. What a Mac shows today (findings at `5ef050dd`)

| Finding | Evidence |
| --- | --- |
| The tray click on a Mac shows the compact window (`App.tsx`, label `settings`) in "popover mode": chrome-less, always on top, anchored under the icon, hidden on blur. The same window in "regular mode" is the "Beebeeb macOS Drive" control centre. The 30 Sep popover was never built: only its data contract exists. | `lib.rs:15555` `show_compact_app_window_with_nav`; `lib.rs:15371` `setup_tray`; `src/popoverContract.ts:1-8` ("Nothing here is rendered yet") |
| A configured Mac opens the compact window at every launch; a Mac with no `sync_root` opens the onboarding window. | `lib.rs` setup, `startup_surface` (`surfaces/policy.rs:51`) |
| The onboarding window is a separate window, "Welcome to Beebeeb", 860×640, centred. | `lib.rs:1333` `open_onboarding_window_impl`, `open_reauth_window` |
| The Settings window (`macos-settings`, 560×640) is built on demand and anchored under the icon. Preferences, Check for updates and the menu's Sign out open it. | `tauri.conf.json` `macos-settings`; `lib.rs` `show_macos_settings_window`; `lib.rs:4092` `open_sign_out_flow` |
| The Windows shell window `main-app` (`?window=main-app&platform=windows`, 1100×720) is declared in the shared `tauri.conf.json` with no `create: false`, so Tauri builds it on every platform, hidden. `tray` and `windows-onboarding` are built hidden on a Mac the same way. | `tauri.conf.json` `app.windows`; `tauri.macos.conf.json` only overrides the tray icon |
| The window-state plugin saves and restores every window except `tray` and `windows-onboarding`, with default flags (which include visibility). | `lib.rs:14165` `window_state_denylist_labels`; the `tauri_plugin_window_state::Builder::default()` call |
| Each conflict opens its own window automatically. | `surfaces/policy.rs:73` `conflict_auto_opens_window` returns `true`; `lib.rs:12787` `open_conflict_window_impl` |
| The app always has a Dock icon (no activation-policy code exists), and the native menu has no Window menu, so ⌘W and ⌘M do nothing. | no `set_activation_policy` in `src-tauri/src`; `lib.rs:14930` `setup_native_menu` |
| The app menu binds ⌘, twice ("Preferences..." in the Beebeeb menu, "Settings..." in the File menu). | `lib.rs` `DESKTOP_MENU_SPECS` |

**Not confirmed:** why the lead saw the Windows shell window on screen. The likeliest cause read from the code is the
config-built `main-app` window plus the window-state plugin restoring a saved visible state. It was not reproduced
here (this lane does not run the app). Slice 1 removes both causes regardless (§12).

## 4. The two surfaces

| | Popover | Beebeeb window |
| --- | --- | --- |
| Label | `popover` | `main` |
| Size | 372 × 488 pt, fixed | 920 × 620 pt default, 760 × 520 pt minimum, resizable, full screen allowed |
| Position | under the b, never remembered (§6.1) | remembered, else centred on the icon's display (§7.2) |
| Created | on the first click, then hidden and reused | on demand; destroyed when closed |
| Webview alive when idle | yes (hidden), the one idle webview | no |
| Close | click away, Esc, a second click on the b | red button, ⌘W, Esc does nothing at window level |
| Dock icon | never | while open |

**Registry.** `surfaces::registry` holds two kinds, `Popover` and `Main` (`MACOS_LABELS` becomes `["popover", "main"]`,
`registry.rs:39`). Its "at most one visible" rule changes: opening the Beebeeb window hides the popover; opening the
popover leaves the Beebeeb window where it is (the popover floats above it and hides when it loses focus).
`main_window_label` (`policy.rs:62`) answers `main` on macOS. Windows keeps `main-app`; Linux keeps `settings`.

**Counts on a Mac.** Window kinds Beebeeb can create: 7 → 2. Webviews alive while you work elsewhere: up to 5 (+1 per
conflict) → 1. Beebeeb windows visible at once: up to 4 → 1 (the popover closes when you click away).

## 5. Surface inventory

Every window, page, section, setting, dialog, toast and banner a Mac can reach today, with its source and its new
home. **Status** is one of: **Moved** (it has a new home, named), **Kept** (a native surface or behaviour that stays as
it is), **Removed** (gone on a Mac; the reason is given), **Not on a Mac** (Windows-only and never shown on a Mac by
design; Windows keeps it), **Added** (nothing today; listed so the menu change is complete).

**Counts:** 117 current surfaces found: 78 moved, 11 kept, 24 removed, 4 not on a Mac; plus 1 added (F15), 118 rows.
Recount per status over this section: `grep -cE '^\| [A-F][0-9]+ \|.*\| Moved \|$'` (and Kept, Removed, Not on a Mac,
Added).

### 5.A Windows and app-level surfaces

| # | Surface today | Source | New home | Status |
| --- | --- | --- | --- | --- |
| A1 | Compact window in "popover mode" (the tray flyout) | `lib.rs:15555`, `App.tsx` | The popover (§6), a window of its own | Moved |
| A2 | Compact window in "regular mode": the "Beebeeb macOS Drive" control centre (7 pages, B1–B15) | `lib.rs:15555`, `App.tsx`, `compactNavigation.ts` | Not built on a Mac. Its pages move as rows B1–B15 say | Removed |
| A3 | Onboarding window "Welcome to Beebeeb", 860×640, and its reauth mode | `lib.rs:1333`, `Onboarding.tsx` | The Beebeeb window, signed out (§7.4) | Moved |
| A4 | Settings window `macos-settings`, 560×640, anchored | `tauri.conf.json`, `lib.rs` `show_macos_settings_window`, `MacSettings.tsx` | The Beebeeb window, Settings pane (§7.8) | Moved |
| A5 | Windows shell window `main-app`, built on a Mac from the shared config | `tauri.conf.json`, `WindowsApp.tsx` | Not built on a Mac. Its pages move as rows E1–E32 say | Removed |
| A6 | Windows tray flyout `tray`, built hidden on a Mac | `tauri.conf.json`, `WindowsTray.tsx` | Not built on a Mac | Not on a Mac |
| A7 | Windows first-run window `windows-onboarding`, built hidden on a Mac | `tauri.conf.json`, `WindowsFirstRun.tsx` | Not built on a Mac (its browser sign-in screen is shared, §7.4) | Not on a Mac |
| A8 | One conflict window per conflict, opened automatically | `lib.rs:12787`, `policy.rs:73`, `ConflictWindow.tsx` | Review › conflict sheet; nothing opens by itself on a Mac (§7.6) | Moved |
| A9 | Native About panel ("About Beebeeb", version, "Current channel: …") | `lib.rs:14918` `about_metadata` | Settings › About | Moved |
| A10 | Tray context menu: Open Beebeeb, Hide, Start at login, Quit | `lib.rs:15291` `build_tray_menu` | Removed on a Mac. Open Beebeeb → gear menu; Hide → click away / ⌘H; Start at login → Settings › General; Quit → gear menu, ⌘Q | Removed |
| A11 | Tray left click toggles the compact window | `lib.rs:15371` `setup_tray` | Valid session: toggles the popover. No session: opens the Beebeeb window on sign-in. Right click does the same (§6.2) | Moved |
| A12 | Tray tooltip (`engine_status::tray_tooltip`) and icon | `lib.rs:15323` | Unchanged; spec C's crossed b and its tooltips when built | Kept |
| A13 | Native menu bar (Beebeeb, File, Edit, View, Help) | `lib.rs:14930` | Kept, items retargeted: rows F1–F15 | Moved |
| A14 | Native alerts "Sign-out paused" and "Signed out" (menu sign-out, late results) | `lib.rs` `handle_desktop_menu_action` SignOut arm; `Shown::Failure` / `Shown::Warning` | ~~Unchanged~~ Late results (`show_late_result`): unchanged. The menu sign-out's own alerts: unchanged until slice 1b, which routes the menu through the confirm sheet (ruling 3); its results are then the Settings flow's toast and Account note (§8.1) | Kept |
| A15 | Native alert "Files kept on this Mac" (K0/K1, 1882's sentence) | `lib.rs:4142`, `lib.rs:4186`, `sign_out/copy.rs` `sign_out_alert` | Unchanged | Kept |
| A16 | Native alerts for a failed menu "Open in Finder" or pause | `lib.rs:15205` `show_menu_open_failure`, `show_menu_pause_failure` | Unchanged | Kept |
| A17 | Notifications "Conflict detected", "All caught up", "Local cache nearly full" | `lib.rs:12866`, `:12884`, `notify_quota_warning_impl` | Unchanged copy; ~~a click opens the Beebeeb window (conflict → Review, others → Home)~~ a click is meant to open the Beebeeb window (conflict → Review, others → Home): **not wired in slice 4**. `tauri-plugin-notification` 2.3.3 (through `notify-rust`) gives a macOS app no click callback, so nothing can tell the app a notification was clicked. The route is built and pinned (`main_nav_for_notification`, `activate_notification`); the trigger, a native `UNUserNotificationCenter` delegate with a device pass, is task 2181. Until then a conflict reaches the person by the notification, Review's sidebar badge, ⌘2 and View › Review. — slice 4, 2026-10-25, approved by the lead | Kept |
| A18 | System file picker "Upload files to Beebeeb" (File menu) | `lib.rs:10599` | Unchanged (§14, item 2) | Kept |
| A19 | Update banner (a toast at the root of `App.tsx` and `WindowsApp.tsx`) | `UpdateBanner.tsx` | Popover update row (§6.3) and Settings › About row | Moved |
| A20 | Auth-expired banner "You're signed out on this device" / "Sign in again" | `AuthExpiredBanner.tsx` | The Beebeeb window's "Sign in again" screen (§7.4) and the crossed b | Moved |
| A21 | Toast host | `windows/ui.tsx` `ToastProvider` | The Beebeeb window only; the popover never shows a toast (§6.4) | Moved |

### 5.B The control centre's pages (`App.tsx`, `src/pages/`)

| # | Surface today | Source | New home | Status |
| --- | --- | --- | --- | --- |
| B1 | Sidebar brand "Beebeeb" / "macOS Drive" | `App.tsx`, `compactNavigation.ts` `brandSubtitleForPlatform` | None: a label, not information | Removed |
| B2 | Quick search (sidebar button and modal) | `DesktopQuickSearch.tsx` | None: popover ruling 4 (approved 1 Oct) cut it; Finder and Spotlight search the files | Removed |
| B3 | Version history (sidebar button and modal) | `DesktopVersionHistory.tsx` | Review › "Version history…" sheet | Moved |
| B4 | Sidebar footer: status pill, "Finder-backed vault" / "Finder location not installed", "Version …" | `App.tsx` | Status → popover status row and Home; version → Settings › About; the two lines about Finder are dropped (Home's Finder row says it) | Moved |
| B5 | "Drive status" page (metrics, sync health, Finder row, vault access, storage, links) | `pages/Status.tsx` | Home (§7.6). Spec C §11 already removes the page | Moved |
| B6 | "Finder location" page: status, notices, location, Open in Finder | `pages/SyncFolder.tsx` | Settings › Sync "Beebeeb in Finder" and Home's Finder row | Moved |
| B7 | "Finder location" › "Drive model" panel | `pages/SyncFolder.tsx:336-361` | None: it described internals ("Remote-first vault namespace.") | Removed |
| B8 | "Finder location" › "Finder repair" (Reset Finder integration) | `pages/SyncFolder.tsx:364-401` | Settings › Sync "Repair…" (the same `reset_macos_integration`) | Moved |
| B9 | "Selective sync" page (pin a folder) | `pages/SelectiveSync.tsx` | Settings › Sync "Keep on this Mac" and its Choose folders sheet | Moved |
| B10 | "Versions & conflicts" page | `pages/VersionCenter.tsx` | Review (§7.6) | Moved |
| B11 | "Account & security" page: identity, session token, vault lock, plan, diagnostics, start at login | `pages/Account.tsx` | Settings › Account (identity, plan, lock), General (login), About (support bundle). "Session token" and "Master key lifetime" are dropped | Moved |
| B12 | "Network" page: Pause sync, upload and download sliders | `pages/Bandwidth.tsx` | Pause → popover gear menu and app menu; limits → Settings › Sync › Speed | Moved |
| B13 | "Notifications" page | `pages/Notifications.tsx` | Settings › General › "Tell me about" | Moved |
| B14 | "Shared roots" page | `pages/Shared.tsx` | None: no platform routes it today (`compactPageFromString('shared')` → `finder`), and on a Mac shared-with-me is web-only (desktop CLAUDE.md, task 1885) | Removed |
| B15 | The Account page's own "Sign out" (1887 entry point 2) | `pages/Account.tsx:149-170` | None: the page goes; the one in-window sign-out is Settings › Account (§8) | Removed |

### 5.C The Settings window (`MacSettings.tsx`, `macSettingsModel.ts`)

| # | Surface today | Source | New home | Status |
| --- | --- | --- | --- | --- |
| C1 | Tab strip: General, Account, Sync, About | `MacSettings.tsx:1214-1231` | ~~Settings pane toolbar tabs, plus Security~~ The Settings sidebar list in System Settings style (§7.8, ruling 1): back control, account row (Account), General, Sync, Security, About | Moved |
| C2 | General › "Open Beebeeb at login" | `MacSettings.tsx:233-243` | Settings › General, unchanged | Moved |
| C3 | General › "Tell me about": Conflicts, Sync finished, Local cache almost full | `MacSettings.tsx:247-259` | Settings › General, unchanged | Moved |
| C4 | Account › header: initial, email, plan line, storage bar, Upgrade / Manage plan | `MacSettings.tsx:501-530` | Settings › Account, plus "Storage details…" (E8) | Moved |
| C5 | Account › Vault: Lock now / Unlock / "Your session ended" | `MacSettings.tsx:532-574` | Settings › Account, unchanged | Moved |
| C6 | Account › "Where your files are stored" (rows, strict switch, note, load failure) | `MacSettings.tsx:280-377` | Settings › Account, unchanged | Moved |
| C7 | Account › "Sign out of this Mac" and its confirm sheet | `MacSettings.tsx:578-601` | Settings › Account, unchanged (§8) | Moved |
| C8 | Account, signed out: "You’re signed out" / "Sign in from the Beebeeb menu to start syncing." | `MacSettings.tsx:483-492` | None: a signed-out window shows sign-in, and Settings › Account needs a session (§7.8) | Removed |
| C9 | Account › status notes (a Lock's or a sign-out's warning sentence) | `MacSettings.tsx:451-456` | Settings › Account, unchanged | Moved |
| C10 | Sync › "Beebeeb in Finder": state, notices, Repair…, repair notes | `MacSettings.tsx:897-921` | Settings › Sync, unchanged; Home shows the same state in one row | Moved |
| C11 | Sync › 1887 kept row (K2, Show in Finder, Dismiss) | `MacSettings.tsx:922-936` | Settings › Sync, unchanged, and mirrored on Home (§8.3) | Moved |
| C12 | Sync › 1882 kept row (Dismiss) | `MacSettings.tsx:937-946` | Settings › Sync, unchanged, and mirrored on Home | Moved |
| C13 | Sync › "Keep on this Mac" and the Choose folders sheet | `MacSettings.tsx:947-1023` | Settings › Sync, unchanged | Moved |
| C14 | Sync › Speed: Upload, Download | `MacSettings.tsx:959-986` | Settings › Sync, unchanged | Moved |
| C15 | About › "Beebeeb for Mac": version, Check for updates, Restart to update | `MacSettings.tsx:1093-1111` | ~~Settings › About, unchanged~~ Settings › About; "Restart to update" becomes "Download update" (§0 ruling 5, task 2176) — slice 5, 2026-10-25, lead-approved (macos-beebeeb-teamlead) | Moved |
| C16 | About › Help: Get help, Export a support bundle | `MacSettings.tsx:1112-1134` | Settings › About, unchanged | Moved |
| C17 | Repair confirm sheet | `MacSettings.tsx:989-997` | Settings › Sync, unchanged | Moved |
| C18 | Sign-out send sheet (P0–P3) | `MacSettings.tsx:665-672` | Settings › Account, unchanged (§8) | Moved |
| C19 | Sign-out ask sheet (A0–A5, A3e) | `MacSettings.tsx:675-702` | Settings › Account, unchanged (§8) | Moved |
| C20 | Toasts and inline notes ("Couldn’t save that setting", "Couldn’t change the login setting", "Couldn’t change where files are stored", "Couldn’t lock the vault", "Couldn’t unlock the vault", "Couldn’t sign out", "Couldn’t dismiss that note", K3, "Couldn’t keep that folder on this Mac", "Couldn’t stop keeping that folder", "Update install failed", "Couldn’t save the support bundle", "Support bundle saved", "Couldn’t load these settings", "Couldn’t load storage locations", "Couldn’t load your account", "Couldn’t repair Beebeeb in Finder") | `MacSettings.tsx` | The Beebeeb window's toast host and the same inline notes, unchanged | Moved |
| C21 | The native "Check for updates…" request, drained by the About tab | `MacSettings.tsx:1170-1178`, `lib.rs:15043` | The Beebeeb window, Settings › About (the target label becomes `main`) | Moved |
| C22 | The sign-out flow request (`menu:sign-out-flow`, `consume_sign_out_flow_request`) | `MacSettings.tsx:1197-1200`, `lib.rs:4092` | The Beebeeb window, Settings › Account (§8) | Moved |

### 5.D The onboarding window (`Onboarding.tsx`)

| # | Surface today | Source | New home | Status |
| --- | --- | --- | --- | --- |
| D1 | Five-step rail, subtitle "Private macOS file access", footer "{region} \| Zero-knowledge" | `Onboarding.tsx:30-36`, rail render | None: spec C §11 removes the rail; the region is in Settings › Account | Removed |
| D2 | "Welcome back" email and password form | `Onboarding.tsx` `SignInStep` | Signed-out window, behind "Use email and password" (spec C §7.1) | Moved |
| D3 | Two-factor (authenticator code, backup code) | `Onboarding.tsx` `SignInStep` | Signed-out window, same card | Moved |
| D4 | Account switch step (A0s–A4s, E3 line) | `Onboarding.tsx` `AccountSwitchStep` | Signed-out window (§7.4, §8) | Moved |
| D5 | "Set up this Mac" (recovery phrase) | `Onboarding.tsx` `UnlockStep` | Signed-out window, or opened from the popover's Unlock vault when no key is on this Mac | Moved |
| D6 | macOS Finder step (Adding / notice / action) | `Onboarding.tsx` `MacFinderStep` | Signed-out window's Finder status screen (§7.5) | Moved |
| D7 | Install-era Finder step | `Onboarding.tsx` `FinderInstallStep` | Windows/Linux and an unknown platform only (spec A §10) | Not on a Mac |
| D8 | "Start online-only" (choose offline folders) | `Onboarding.tsx` `PinningStep` | None: spec C §11 removes it; Settings › Sync › Keep on this Mac does it | Removed |
| D9 | "Control center is ready" / "Open control center" | `Onboarding.tsx` `ReadyStep` | None: spec C §11 removes it; the window moves on to Home | Removed |
| D10 | Reauth mode (`open_reauth_window`, `?mode=reauth`) | `lib.rs` `open_reauth_window` | The "Sign in again" screen in the Beebeeb window | Moved |
| D11 | Lines `SIGN_IN_OUTCOME_UNREADABLE`, `KEY_REPLACED_RECOVERY_COPY` | `accountSwitchCopy.ts` | Same screens, unchanged | Moved |
| D12 | Its toasts ("Couldn’t sign out", "Couldn’t make folder offline", …) | `Onboarding.tsx` | The Beebeeb window's toast host; the pinning toasts go with D8 | Moved |
| D13 | The window closing itself (capability `onboarding-close`) | `capabilities/onboarding-close.json`, `Onboarding.tsx:146`, `:985` | None: the Beebeeb window never closes itself; the capability retires with the label | Removed |

### 5.E The Windows shell, as a Mac reaches it today (`WindowsApp.tsx`, `src/windows/`)

Everything here is visible on a Mac only because of A5. Line numbers from the inventory read at `5ef050dd`.

| # | Surface today | Source | New home | Status |
| --- | --- | --- | --- | --- |
| E1 | Home: status strip, engine refusal sentence, storage card, recent activity, "Account at a glance" | `WindowsApp.tsx:265-541` | Home: status, refusal sentence, storage, recent activity. "Account at a glance" goes: sessions and security are in Settings › Security, shares are web-only on a Mac | Moved |
| E2 | Signed-out gate "Sign in to Beebeeb" / "Your files are encrypted on this PC…" / Sign in (opens the separate onboarding window) | `WindowsApp.tsx:200-214` | The signed-out Beebeeb window itself (§7.4) | Moved |
| E3 | Files: a sync summary (folder card, metrics, by state, recently changed, empty and error states) | `WindowsApp.tsx:1249-1311` | Home: Finder row, counts line, recent activity. No separate Files page: files are in Finder | Moved |
| E4 | "Manage backup" and the known-folder dialog | `WindowsApp.tsx:1134-1235`, `KnownFolderOnboarding.tsx` | Windows only; both render nothing on a Mac today | Not on a Mac |
| E5 | Trash: list, Refresh, Restore, Delete permanently (password sheet) | `WindowsApp.tsx:1358-1532` | Trash (§7.7) | Moved |
| E6 | Account: profile, frozen state, plan and subscription, "Password & billing live on the web" | `views/AccountView.tsx` | Settings › Account; frozen → spec C's notice line; the web row becomes "Password, email and billing". "Member since" and the role chip are dropped (kept small) | Moved |
| E7 | Account › "Disconnect this device" | `views/AccountView.tsx:349-476` | None: it is a second sign-out that skips 1887's sheet (the Rust gate refuses it, but the person sees a raw error). One sign-out: Settings › Account | Removed |
| E8 | Insights: storage hero, by type, largest files | `views/InsightsView.tsx` | Settings › Account › "Storage details…" sheet | Moved |
| E9 | Bandwidth › Speed test | `views/BandwidthView.tsx:674-778` | Settings › About › "Test connection speed" | Moved |
| E10 | Bandwidth › 20-hour traffic chart and live session telemetry | `views/BandwidthView.tsx`, `BandwidthChart.tsx` | None on a Mac: engine telemetry; the popover shows live transfers (~~open question 2~~ ruling 2) | Removed |
| E11 | Selective sync tree with Apply (`set_selective_sync`) | `views/SelectiveSyncView.tsx` | None on a Mac: a Mac pins folders (Keep on this Mac, C13); this is Windows' online-only model | Removed |
| E12 | Devices | `views/DevicesView.tsx` | Settings › Security › Devices (no revoke claim, §10) | Moved |
| E13 | Security: score, active sessions, Revoke, Sign out everywhere else, two-factor | `views/SecurityView.tsx` | Settings › Security | Moved |
| E14 | Activity › Timeline | `views/ActivityView.tsx` | Settings › Security › "Recent account activity" | Moved |
| E15 | Activity › Notifications (account alerts list, no mark-as-read) | `views/ActivityView.tsx:387-405` | None on a Mac: native notifications and the popover cover alerts; the web app's notification inbox keeps the list (~~open question 2~~ ruling 2) | Removed |
| E16 | Sidebar extras: Search, Version history, storage widget with Upgrade, region label | `WindowsApp.tsx:1599-1638`, `:1855-1917` | Search → B2; Version history → B3; storage → sidebar footer and popover header; region → Settings › Account | Moved |
| E17 | Settings › Sync › "Keep files encrypted end-to-end" (a switch that does nothing on a Mac) | `views/SettingsView.tsx:340-367` | None: a control that does nothing; encryption state is the amber lock in the popover and on Home (ruling 2) | Removed |
| E18 | Settings › Sync › "Sync on metered connections" | `views/SettingsView.tsx:82-83` | None on a Mac: nothing in `src-tauri` reads `metered` (only `config.rs` stores it) (ruling 2) | Removed |
| E19 | Settings › Sync › read-only upload and download rate limits | `views/SettingsView.tsx:86-96` | Settings › Sync › Speed (selects) | Moved |
| E20 | Settings › Sync › "Files on Demand" and "Show sync overlays in File Explorer" | `views/SettingsView.tsx:98-103` | None on a Mac: File Explorer concepts; a Mac is always on demand and Finder draws its own badges (ruling 2) | Removed |
| E21 | Settings › Sync › "Free up space" | `views/SettingsView.tsx:383-391` | None on a Mac: Keep on this Mac and macOS's own eviction cover it; it needs `sync_root` (`lib.rs:9542-9543`), which a File Provider Mac may not have (§14) | Removed |
| E22 | Settings › Data residency | `views/SettingsView.tsx:658-812` | Settings › Account (already there, C6) | Moved |
| E23 | Settings › Notifications › Desktop alerts | `views/SettingsView.tsx:570-589` | Settings › General › Tell me about (already there, C3) | Moved |
| E24 | Settings › Notifications › account preferences: New device sign-in, Share received, File updated, Storage warning, Backup complete | `views/SettingsView.tsx`, `notificationPreferenceMeta.ts` | Settings › General › "From your account" | Moved |
| E25 | Settings › Launch | `views/SettingsView.tsx:814-861` | Settings › General (already there, C2) | Moved |
| E26 | Settings › Finder integration panel | `views/SettingsView.tsx:1009-1067` | Settings › Sync › Beebeeb in Finder (already there, C10) | Moved |
| E27 | Settings › Updates: status, check, release channel, downgrade, release-notes and confirm-downgrade dialogs | `views/SettingsView.tsx:1069-1483`, `updateCheckViewModel.ts` | Settings › About: version row (C15), "Release channel" row, downgrade row and ~~its two sheets~~ its release-notes sheet when a downgrade is offered. ~~The confirm-downgrade sheet~~ **Out on a Mac until an install mechanism exists (§0 ruling 5, task 2176):** a downgrade is an install, so the row says what the channel carries and offers "Release notes" and "View release" (a link), never a Downgrade button — slice 5, 2026-10-25, lead-approved (macos-beebeeb-teamlead) | Moved |
| E28 | Settings › Advanced › Theme | `views/SettingsView.tsx:1633-1700` | Settings › General › Appearance | Moved |
| E29 | Settings › Advanced › Local cache limit | `views/SettingsView.tsx`, `advancedSettingsModel.ts` | None on a Mac: in `src-tauri` only the warning reads it (`runner.rs:291-300`, `lib.rs:12902-12914`), so "Cap file content stored on this device" would overclaim. "Local cache almost full" keeps the default limit (ruling 2) | Removed |
| E30 | Settings › Advanced › App Activity (CPU, Memory RSS, PID, samples) | `views/SettingsView.tsx:1581-1613` | None on a Mac: developer diagnostics; the support bundle is the support path (~~open question 2~~ ruling 2) | Removed |
| E31 | "This feature is not available on this device." gate | `capabilities.tsx` `CapabilityAlternative` | Kept as the fallback for an unsupported route; no Mac route reaches it | Kept |
| E32 | Its dialogs and toasts ("Delete permanently", "Restore failed", residency toasts, launch toasts, "Couldn’t save setting", release notes, confirm downgrade) | `WindowsApp.tsx`, `views/*` | With the page each belongs to, in the Beebeeb window | Moved |

### 5.F The native menu (`lib.rs` `DESKTOP_MENU_SPECS`)

| # | Item today | New item and target | Status |
| --- | --- | --- | --- |
| F1 | Beebeeb › "Preferences..." ⌘, → Settings window | "Settings…" ⌘, → the Beebeeb window, Settings | Moved |
| F2 | Beebeeb › "Check for updates..." ⇧⌘U → Settings window, About | "Check for Updates…" ⇧⌘U → the Beebeeb window, Settings › About, and checks | Moved |
| F3 | Beebeeb › "Pause/Resume syncing" ⇧⌘P | "Pause Sync" or "Resume Sync", the label follows the state | Moved |
| F4 | Beebeeb › "Sign out" ⇧⌘L (1887 entry point 5) | "Sign Out…" ⇧⌘L; ~~behaviour unchanged, its flow opens in the Beebeeb window (§8; open question 3)~~ slice 1: behaviour unchanged, its flow opens in the Beebeeb window. Slice 1b, once task 2175 passes (ruling 3): it opens the Beebeeb window on Settings › Account with the "Sign out of this Mac?" sheet, the same flow as Settings (§8.1) | Moved |
| F5 | Beebeeb › "Quit Beebeeb" ⌘Q | Unchanged | Kept |
| F6 | File › "Open Beebeeb folder" ⌘O | "Open in Finder" ⌘O | Moved |
| F7 | File › "Upload files..." ⌘U | "Upload Files…" (behaviour unchanged, §14) | Moved |
| F8 | File › "New folder..." ⇧⌘N | "New Folder…" (behaviour unchanged, §14) | Moved |
| F9 | File › "Open web app" ⇧⌘O | "Open Web App" | Moved |
| F10 | File › "Settings..." ⌘, (a second ⌘, binding) | None: a duplicate of F1 | Removed |
| F11 | View › Files ⌘1, Activity ⌘2, Trash ⌘4 (into the compact window's pages) | View › Home ⌘1, Review ⌘2, Trash ⌘3 (each also leaves Settings) | Moved |
| F12 | View › Zoom in, Zoom out, Actual size | Unchanged; applies to the Beebeeb window | Kept |
| F13 | Help › Documentation, Keyboard shortcuts, Report a problem, Service status | Unchanged | Kept |
| F14 | Edit › Cut, Copy, Paste, Select All | Unchanged | Kept |
| F15 | (none today) | Window › Minimize ⌘M, Zoom, Close ⌘W, Bring All to Front; Beebeeb › Hide Beebeeb ⌘H, Hide Others ⌥⌘H, Show All; View › Back ⌘[ (enabled while Settings shows; leaves it, as System Settings' View › Back does; ruling 1) | Added |

## 6. The popover

### 6.1 Size and placement

- **372 × 488 pt, fixed** (unchanged from 30 Sep): the body never resizes between states, so nothing jumps.
- **Placement** (30 Sep rule, unchanged): centred on the status item, 6 pt below the menu bar, on the display that holds
  the icon (read from AppKit); clamped 8 pt from that screen's visible frame. If the icon rect is unavailable, top
  right of the primary display's visible frame. Never centred on the screen, never remembered.
- **Window:** label `popover`, `create: false` (built on the first click), no decorations, opaque (ruling 3: public
  APIs only), always on top, not in the Dock or the window list, shown on the active Space, hidden on focus loss (the
  debounced blur rule of `lib.rs:15555` moves here). Close = hide; the webview is reused.

### 6.2 Opening it

| State of the Mac | A click on the b (either mouse button) |
| --- | --- |
| A valid session (every state in §6.3) | Toggles the popover. |
| No session: signed out, session ended, or a sign-in in progress (spec C C-R1, C-R2, and C-R3 while signed out) | Opens or focuses the Beebeeb window on its sign-in screen. No popover (spec C C-W3, C-W10). |

The tray context menu is gone on a Mac (row A10), so a right click no longer shows a menu; it acts as a left click.

### 6.3 States

Precedence is `popoverContract.ts` `POPOVER_PHASES` (`surfaces/phase.rs`), with spec C's account states above it
(spec C C-R8: update required, then locked, checking, no plan). A notice line (spec C §7.5: trial, read-only, frozen,
payment failed) sits under the storage bar in any state that shows the header's storage line; its link replaces nothing
in the popover.

| State | Header | Body | Status row | Primary action | Copy source |
| --- | --- | --- | --- | --- | --- |
| Up to date (`synced`) | email, storage line and bar | "Recent activity", 5 rows | ✓ "Up to date" · amber lock "Encrypted" | none | 30 Sep (approved) |
| Syncing | same | "{n} files left", size and %, rows with per-file progress (upload bar amber: encryption; download neutral), "Done" | spinner "Syncing" · "Encrypted" | none | 30 Sep |
| Needs a decision (conflicts > 0 over any non-message state) | same | attention row "{n} files need a decision" / "Both changed on two devices." + "Review…", then activity | as the state below it | "Review…" opens the window on Review | new (§10) |
| Update available (over any non-message state) | same | row "Version {v} is available." + "Restart to update" (amber), then the state's body | as the state below it | "Restart to update" | `macSettingsModel.ts` `updateRow`, `UpdateBanner.tsx` |
| Paused | same | "Sync is paused" / "{n} changes on this Mac are waiting. Nothing syncs until you resume." | none | "Resume sync" | 30 Sep |
| Offline (spec C C-R7; crossed b) | same | "Can’t reach Beebeeb" / "Check your connection. Files on this Mac are unchanged." | none | "Try again" | spec C + 30 Sep (§10) |
| Error | same | "Sync stopped" / "Beebeeb couldn’t finish syncing. Files on this Mac are unchanged." + mono reason (≤ 30 characters) | none | "Try again" | new (§10) |
| Vault locked (C-R4) | email only | "Your vault is locked" / "Sync is paused until you unlock it. Files already downloaded to this Mac stay where they are." | none | "Unlock vault": the Keychain unlock (`unlock_vault`) when the key is in the Keychain; otherwise it opens the window on "Set up this Mac" | 30 Sep |
| Storage full | storage bar red | "Your storage is full" / "{n} changes can’t upload until you free up space or add storage. They stay on this Mac." | none | "Add storage" | 30 Sep |
| Finder: adding | as up to date | activity | spinner "Adding Beebeeb to Finder…" | none | spec A |
| Finder: failed / turned off / not in Applications | as up to date | "Couldn’t add Beebeeb to Finder" + spec A's one sentence per reason + mono reason | none | spec A's one action per reason (Try again, Open System Settings, Show in Finder, Copy details); "Open in Finder" disabled | 30 Sep title, spec A §6.2 |
| Sign-out in progress (1887 send phase) | as up to date | spinner, "Sending changes first" / "Sending {n} changes before signing out…" / "This takes at most a minute." | none | "Open Beebeeb" (neutral): brings the window and its sheet forward | 1887 P0–P2 |
| Checking (C-R11) | email, skeleton line | busy indicator, no words | none | none | spec C |
| No plan yet (C-R5) | email only | "Choose a plan" / "Your account is ready. Choose a plan on beebeeb.io, including the free trial. Beebeeb continues here by itself."; after the click: "Waiting for your plan." | none | "Choose a plan on beebeeb.io" | spec C (§10 for the shortened line) |
| Update required (C-R3, with a session) | email only | "Update Beebeeb" / "This version of Beebeeb is too old to connect. Update to continue." | none | "Update now" | spec C |

The 1887 ask (A0–A5) is never shown in the popover: a question with consequences belongs to the window's sheet.

### 6.4 Actions and rules

- **Footer, every state:** "Open in Finder" · "View online" · "Add storage" (ruling 5). Disabled where they cannot work:
  Open in Finder while Finder is not Ready; View online and Add storage while offline.
- **Gear menu** (a native `NSMenu` anchored to the gear): "Open Beebeeb" · "Settings…" ⌘, · — · "Pause Sync" /
  "Resume Sync" ⇧⌘P · "Check for Updates…" · — · "Help" › (Documentation, Keyboard shortcuts, Report a problem,
  Service status) · — · "Quit Beebeeb" ⌘Q. "Open Beebeeb" opens the window on Home; "Settings…" opens it on Settings;
  "Check for Updates…" opens it on Settings › About and checks.
- **The popover opens no window but the Beebeeb window.** Its other actions go to Finder, the system browser or System
  Settings.
- **One error surface:** the popover never shows a toast (popover ruling 7). A transient failure of an action started
  in the popover replaces the body with the error state; everything else waits for the next snapshot.
- **Data:** `popover_snapshot` and `popover-shown` (`popoverContract.ts`), unchanged; slice 3 renders them.

## 7. The Beebeeb window

### 7.1 One window

Label `main`, URL `index.html?window=main&platform=macos`, `create: false`, title bar style Overlay with the title
hidden (as `macos-settings` today), traffic lights over the sidebar. The toolbar area (52 pt~~; 64 pt on Settings~~,
Settings included: ruling 1 removes the tab strip) and the top of the sidebar are the drag region. One instance: every
opener shows and focuses the existing window and navigates it (`main:navigate` event with a section and, for
Settings, a ~~tab~~ Settings section).

### 7.2 Size and position (the answer to "randomly in the screen")

- **Default** 920 × 620 pt, **minimum** 760 × 520 pt (Settings' ~~tabs and~~ rows need 546 pt of content width; the
  sidebar is 214 pt, in Settings too, §7.8).
- **First open:** centred on the display that holds the menu-bar icon, so it appears on the screen you were looking at.
- **After that:** where you left it, at the size you left it: the window-state plugin restores size and position for
  `main` only, never its visibility (`StateFlags::SIZE | POSITION`; every other label is denied).
- **Off-screen guard:** if less than half of the saved frame is on a connected display, centre it again as on first
  open.
- **The popover never moves it,** and no Beebeeb code positions it except this rule.

### 7.3 When it opens by itself

| Event | Beebeeb window |
| --- | --- |
| Login launch (spec C C-W1, C-W5) | Nothing. |
| Manual launch, or opening the app while it runs (Finder, Dock, Spotlight; `RunEvent::Reopen`) | Opens: sign-in with no session, else Home. |
| First launch after install | Opens on sign-in. |
| Updater relaunch | Nothing with a session (popover ruling 1: "not after updates"); sign-in without one. Detection: a flag the updater writes before `restart`, consumed at launch. |
| Session ends while it is closed | Nothing; the b is crossed. A click opens sign-in. |
| A conflict | Nothing (a notification and a popover row). |

### 7.4 Signed out: sign-in and setting up this Mac

No sidebar while there is no session: the window shows one centred card. The window title follows the screen ("Sign in
to Beebeeb", spec C C-W8). Screens, in the order a person meets them; copy is spec C's and today's, changes in §10:

1. **Sign in** (C-R1): "Sign in to Beebeeb"; amber "Sign in with browser"; link "Use email and password"; footer "New
   to Beebeeb? Create an account on beebeeb.io" and "Quit Beebeeb". Offline: the buttons become "Can’t reach Beebeeb.
   Check your connection." + "Try again". Spec A's `not_in_applications` sentence sits above the buttons when it holds.
2. **Browser code** (task 1734): one shared component, moved out of `WindowsFirstRun.tsx`; copy from
   `browserLoginCopy.ts`; the code in mono on amber; "Copy link" when the browser did not open.
3. **Email and password**, then **two-factor**: today's `SignInStep`, unchanged, plus a "Sign in with browser instead"
   link back.
4. **Sign in again** (C-R2): title "Sign in again"; "Sync is paused until you sign in again. Your files stay in
   Finder."; the same two methods; footer "Signed in before as {email}".
   When an opener asks for it (`open_reauth_window`) and the session is in fact fine, the same card shows, with a quiet
   "Back to Home" under it that leaves it; a session that really ended has no such link. A request that arrives under a
   sheet waits for the sheet to close (§8.2). — lead ruling, 2026-10-25 (delta review P3-2 on PR #143); the hi-fi draws
   no such link, a recorded deviation.
5. **Account switch** (1887 W11): today's `AccountSwitchStep` and its A0s–A4s copy, unchanged (§8.2).
6. **Set up this Mac** (recovery phrase), only when this Mac has no key: today's `UnlockStep`, unchanged.
7. **Spec C's account states** that need the whole window when they hold with no Ready account behind them: Checking
   (busy), Update required, No plan yet. Copy as spec C §7.

No rail, no "Choose offline folders", no "Control center is ready" (spec C §11).

### 7.5 Finder status, then Home

After the keys arrive, the card shows "Beebeeb in Finder" / "Beebeeb adds itself to Finder after sign-in." and spec
A's "Adding Beebeeb to Finder…". On Ready the window **moves on to Home** with the first-run hint (§7.6); it does not
close (this replaces spec C §7.6's "closes by itself"). On a failure or a turned-off extension: spec A's one sentence
and one action, inline (ink on `--err-bg`). ~~"Continue" is not offered: Home already shows the Finder row's state.~~
While Finder is not Ready the card also offers one secondary button, "Continue to Home", which moves the window on at
once; Home shows the Finder row's state and its action. A status screen is not a trap: a Finder add that fails, or an
extension that is turned off, must not hold the person on it. — lead ruling, 2026-10-25 (review P3-6 on PR #143). **A
recorded deviation from the hi-fi**, which draws no such button: the ruling supersedes the artefact (precedent: task 1357),
and the hi-fi's figure 5 gains the button when the design is next redrawn.

### 7.6 Signed in: sidebar, Home, Review

**Sidebar** (214 pt): Home ⌘1 · Review ⌘2 (count badge when > 0) · Trash ⌘3. Footer: the account initial, email and
plan line (or "Vault locked"), and the **gear**, which selects Settings (§7.8): the sidebar then shows the Settings
section list in its place. ~~Arrow keys move through the list; Return opens.~~ ↑ and ↓ move and select, Home and End
jump, Return and Space also select: the automatic activation the shipped Settings tabs use (`tabAfterKey`), so the
one sidebar has one keyboard model in both of its lists (§11). There is no Files item: files are in Finder (row E3),
and the Windows shell's "Files" was a summary. — rulings amendment, 2026-10-24

**Home** (replaces "Drive status", the Windows shell's Home and Files):

- One-time hint, after the first sign-in on this Mac: "Beebeeb lives in the menu bar" / "Click the b at the top of
  your screen to see what’s syncing. You can close this window; Beebeeb keeps running." / "Got it". Dismissed once,
  stored in `desktop.toml`.
- Status card: the popover's state in one row ("Up to date" · "Last checked {time}", or the syncing summary, or the
  message state's title and action); Finder row "Beebeeb in Finder" / "Your vault appears under Locations in Finder."
  with the one amber button "Open in Finder"; a counts line "On this Mac {n} files · Online-only {n} · Kept on this Mac
  {n} folders", shown only when `desktop_file_overview` answers (no placeholder otherwise; §14).
- "Needs your attention": the top three Review items, plus the 1887 and 1882 kept rows (§8.3).
- "Storage": the bar, "{used} of {quota}", "Add storage".
- "Recent activity": the popover's rows, more of them.
- The engine refusal sentence (`sync_status.engine_refusal`) as a status note when present.
- Locked: one card, the popover's sentence and "Unlock vault". Review and Trash show the same card. If the unlock finds
  no key on this Mac (a password sign-in that was never finished with the recovery phrase), the card says the Set up this
  Mac sentence ("This Mac does not have your encryption keys yet. Restore them to continue.") and offers one secondary
  button, "Use your recovery phrase", which opens Set up this Mac in this window. — lead wording, 2026-10-25 (review P2-3
  b on PR #143); a recorded deviation, the hi-fi draws no such state. A window reopened while that step was still owed
  resumes at Set up this Mac by itself (Rust keeps the step for the life of the process; a relaunch is task 1747's Part 2).

**Review** (replaces "Versions & conflicts", the conflict windows and the version-history modal):

- Intro: "Files that need a decision from you, and changes Beebeeb couldn’t send yet."
- One row per `list_version_conflict_center` entry: a mono kind tag, the file name, folder and time, Rust's detail
  line, and one action from `reviewEntryAction` (labels in §10).
- **Conflict sheet** (520 pt, from `ConflictWindow.tsx`): "Conflict: {file}" / "This file was modified on two
  devices. Choose which version to keep."; side-by-side "This Mac" / "Other device" (the text diff as today);
  "Keep this Mac’s", "Keep the other", amber "Keep both". Esc and the close button cancel. On success the sheet closes,
  the row leaves, and a success toast says "Conflict resolved". "Keep the other" is the one choice that throws something
  away (this Mac's file is overwritten and nothing keeps it), so it asks first: the same button becomes the red
  "Replace this Mac’s version" with "Replace this Mac’s version? Your changes here are discarded." above it, and any
  other choice ends the question. If a choice fails, the conflict stays in Review, still a conflict, to try again.
  — slice 4, 2026-10-25, review of #142, the lead's option
- **Version history…** (toolbar button): a sheet that searches file names and lists versions with "Restore" (today's
  `DesktopVersionHistory` and VersionCenter's list, one implementation).
- Empty: "Nothing needs your attention."

### 7.7 Trash

From the Windows shell, copy unchanged: "Deleted files in your vault. Restore puts them back through sync; permanent
delete removes them from Trash." Rows with Restore and "Delete…"; "Delete…" asks for the password in a sheet
("Delete permanently", as today). Until task 1717 wires Finder's Put Back, this is the Mac's restore path.

### 7.8 Settings

**Reached by:** the sidebar gear, ⌘,, the app menu's "Settings…", the popover gear menu's "Settings…", and the
openers that need a ~~tab~~ section (Check for Updates → About; the menu's Sign Out… → Account). It is a ~~section~~
mode of the Beebeeb window, never a window of its own.

~~**Look:** the content area becomes a standard macOS Settings pane: the toolbar shows icon tabs (General, Account,
Sync, Security, About), the selected tab is filled; below, grouped inset rows on `--paper-2`, label left, control
right, a hint under the label, a small grey title above each group. This is the shipped `MacSettings.tsx` look,
moved: its tab components mount in the Beebeeb window's content area unchanged, and `tabAfterKey` keeps the WAI-ARIA
tab keys. The sidebar stays visible with the gear marked current.~~

~~**Signed out:** ⌘, shows General and About only, in place of the sign-in card, with a "‹ Sign in" button in the
toolbar; the other tabs need a session.~~

**Layout (ruling 1, "System Settings style").** Settings looks like macOS System Settings: a sidebar list of sections
on the left, the selected section's pane on the right. It uses the window's one sidebar. Choosing Settings swaps the
sidebar's contents from Home / Review / Trash to the Settings list, and the back control at the top of that list swaps
them back. Nothing else about the window changes: the same frame, the same 214 pt sidebar column, the same 52 pt
toolbar, the same `main` webview.

**Why one sidebar that switches, and why it is still one window.** Two sidebars side by side would leave 366 pt for the
pane at the 760 pt minimum width (760 − 214 − 180), where the rows need 546 pt, and System Settings itself has one
sidebar. Settings stays inside the one window because it is a state of `main` that the same `main:navigate` event
selects, so no opener, the gear, ⌘,, the menus or the popover, has a window to build.

**The Settings sidebar, top to bottom:**

| Row | What it shows | Selecting it |
| --- | --- | --- |
| Back control | A chevron and the name of where it returns to: "Home", "Review" or "Trash", whichever showed before Settings ("Home" when the window opened straight on Settings). Signed out: "Sign in". | Leaves Settings. The sidebar shows Home / Review / Trash again with that one selected, and focus goes to the gear. ⌘[ and View › Back do the same (F15). |
| Heading | "Settings" (17 pt bold), so the swapped list says what it is; it labels the `navigation` landmark | — |
| Account row | The account initial (32 pt), the email, and the plan line or "Vault locked": the identity the normal sidebar keeps in its footer | Account |
| General · Sync · Security | A 22 pt tile icon and the label | that section |
| About, after a 10 pt gap | the same | About |

- **The sections are the five asked for: General, Account, Sync, Security, About.** System Settings suggested one
  change, made here: Account is drawn as the account row at the top, as System Settings draws the Apple Account row
  above its panes. It is still one of the five sections, and the identity stays in the same column in both modes. One
  System Settings habit is not followed: About is not nested under General. With five sections there is no second
  level to put it in, and the app menu's "About Beebeeb" lands on it (row A9). The gap before About sets the reference
  section apart, the way System Settings separates its groups with space rather than lines.
- **No search field.** System Settings needs one for more than 30 panes. Beebeeb has five, with about 30 rows.
- **No sidebar footer in Settings.** The account moved to the top row, and the gear has nothing to do while Settings
  shows.
- **Brand over System Settings where they differ.** System Settings uses coloured icon squares and selects in the
  system accent colour. Here the icons are ink on a `--paper-3` tile; the selected row is a `--paper-3` fill with
  semibold text, the same as Home / Review / Trash; amber stays for encryption state and the one primary action
  (§10.1). The sidebar is drawn by Beebeeb's webview, so the macOS accent does not apply to it. The gear menu is a
  native `NSMenu` and still uses the accent.
- **The pane.** The 52 pt toolbar shows the section's name (15 pt bold), as on Home, Review and Trash. It has no back
  and forward arrows: System Settings uses those for pane history, and one back control is enough here. Below the
  toolbar: grouped inset rows on `--paper-2`, label left, control right, a hint under the label, a small grey title
  above each group, footnotes under groups. This is the shipped `MacSettings.tsx` row look, unchanged. The pane
  scrolls; the sidebar never does (seven items).
- **Which section opens.** ⌘, and "Settings…" open the last section used in this window, else General, as the tabs did.
  Openers that name a section open that one.
- **Signed out.** ⌘, or "Settings…" shows the Settings sidebar with "Sign in" as its back control, no account row, and
  General and About only; the other sections need a session. General then leaves out "From your account", because
  those preferences are stored with the account. The signed-out window shows a sidebar only while Settings shows. Back
  returns to the centred sign-in card (§7.4).
- **Leaving Settings.** The back control, ⌘[, View › Back, and ⌘1–⌘3 all leave it. Esc does not, as in System
  Settings (§9.3).
- **Code.** The shipped section bodies (`GeneralTab`, `AccountTab`, `SyncTab`, `AboutTab` in `MacSettings.tsx`) mount
  in the pane unchanged; only the toolbar tab strip (`MacSettings.tsx:1214-1231`, `role="tablist"`) is replaced by the
  sidebar list. `tabAfterKey` keeps its automatic activation, wrap, Home and End, with ↑/↓ in place of ←/→. 1887's
  flow object and its listeners (`MacSettings.tsx:1190-1200`) move up to the window's shell (`MacApp.tsx`), so a flow
  request reaches them whichever section shows. Their `setTab('account')` becomes selecting Settings › Account.

— rulings amendment, 2026-10-24

| ~~Tab~~ Section | Rows (top to bottom) | From |
| --- | --- | --- |
| General | "Open Beebeeb at login" · "Appearance" (System / Light / Dark; "Follow macOS.") · group "Tell me about": Conflicts, Sync finished, Local cache almost full · group "From your account": New device sign-in, Share received, File updated, Storage warning, Backup complete | C2, E28, C3, E24 |
| Account | header (initial, email, plan line, storage bar, "Storage details…", Upgrade / Manage plan; spec C's notice line under the plan line, its link replacing the plan button) · group "Vault" · group "Where your files are stored" · "Password, email and billing" / "Changed on beebeeb.io, in your browser." / "Open beebeeb.io" · "Sign out of this Mac" | C4–C9, E6, E8, E22 |
| Sync | "Beebeeb in Finder" (+ Repair…, notices, the 1887 and 1882 kept rows) · "Keep on this Mac" · group "Speed": Upload, Download | C10–C14, C17, E19, E26 |
| Security | "Authenticator app (TOTP)" with its state, read from the server each time the section opens and each time the window is looked at again (never from what the session cached at sign-in): "Checking…" until it answers, an alert with Try again when it cannot, so the row never says On for an authenticator that was turned off on the web (PR #140 delta review) — and "Manage on beebeeb.io" / "Set up on beebeeb.io" · group "Active sessions" with Revoke, and "Sign out everywhere else" · group "Devices" · group "Recent account activity" (the last 10 events) | E12–E14 |
| About | "Beebeeb for Mac" (version, Check for updates / ~~Restart to update~~ Download update, §0 ruling 5) · "Release channel" (Stable / Beta / Alpha, with their hints) · the downgrade row when one is offered (release notes and a link, never an install) · group "Help": Get help, "Test connection speed", Export a support bundle. A note under the first group: "Beebeeb checks for updates at launch and every 4 hours." — slice 5, 2026-10-25, lead-approved (macos-beebeeb-teamlead) | C15, C16, E9, E27, A9 |

"Storage details…" opens a sheet with the Insights content (by type, largest files), copy unchanged except the
subtitle (§10).

## 8. Sign out (task 1887) in the new layout

### 8.1 Where each part lives

| 1887 part | Today | After | Copy |
| --- | --- | --- | --- |
| Confirm sheet "Sign out of this Mac?" (Dialog 3) | Settings window, Account tab | Beebeeb window, Settings › Account, window-modal sheet | unchanged |
| Send phase (P0–P3) | same | same place, window-modal sheet; mirrored read-only in the popover (§6.3) | unchanged |
| The ask (A0–A5, A3e) | same | same place, window-modal sheet (440 pt) | unchanged |
| A refusal (`unsent_changes`) at the gate | back to the ask from a fresh count | same | unchanged |
| Failure toast "Couldn’t sign out" (E1, E2, E3, purge failure) | Settings window toast | Beebeeb window toast | unchanged |
| A completed sign-out's warning sentence | Account tab note | Settings › Account note; the window then shows sign-in (the note is held, `accountSession.ts`, and shown above the sign-in card) | unchanged |
| Kept alert "Files kept on this Mac" | native alert | native alert | unchanged |
| Kept row (K2: Show in Finder, Dismiss) | Settings › Sync | Settings › Sync, and mirrored on Home › Needs your attention (§8.3) | unchanged |
| Late results ("Sign-out paused", "Signed out") | native dialogs | native dialogs | unchanged |
| Switch step ask (A0s–A4s) | onboarding window | the Beebeeb window's sign-in flow (§7.4) | unchanged |
| Entry point 1 (Settings › Account › Sign out…) | Settings window | Settings pane | — |
| Entry point 2 (compact Account page) | compact window | removed with the page (B15) | — |
| Entry points 3, 4 (Sign in again in the banner, in Versions) | `forceReauth` → `open_reauth_window` | the Beebeeb window's "Sign in again" (no purge, unchanged) | — |
| Entry point 5 (app menu Sign out) | runs `clear_session_impl`; on `unsent_changes` opens the Settings window with a flow request | ~~the same, but the flow request opens the Beebeeb window on Settings › Account~~ Slice 1: the same, but the flow request opens the Beebeeb window on Settings › Account. Slice 1b, after task 2175 passes (ruling 3): the menu no longer calls `clear_session_impl`. It stores a flow request that starts at the confirm sheet (`flow.open()`, not `flow.request()`), and opens the Beebeeb window on Settings › Account, so the menu and entry point 1 are one flow. Its results are entry point 1's: the toast, the Account note, and the kept alert (1882, 1887) raised by `clear_session`. | — |
| Entry point 6 (account switch) | onboarding window | Beebeeb window | — |

### 8.2 Rules that carry over unchanged

- The flow owner label becomes `main` everywhere it is `macos-settings` today (`sign_out_count`'s snapshot owner, the
  send phase's window, `sign_out_flow_window_closed`, the late-result record, `open_sign_out_flow`'s `emit_to`), and
  the switch step's `onboarding` owner becomes `main` too. With one window, both flows share an owner; only one can
  run at a time, because the switch step exists only signed out and Settings › Account only signed in.
- **Closing the window** (red button, ⌘W) runs the existing hook on `CloseRequested` and again on `Destroyed`: at the
  confirm sheet or the ask it is Cancel; during the count or the send phase the phase ends `closed`, nothing changed;
  after the gate the sign-out goes on and Rust shows a pending result in the native dialog. Closing now destroys the
  window (§4), which the hook already handles (spec 1887 §5.1, "A close ends the flow"). Reopening shows no sheet.
- **Navigating away** during a sheet is impossible: the sheets are window-modal and cover the sidebar too~~.~~, the
  Settings back control included. While a sheet is open the View menu's Home, Review, Trash and Back items and the app
  menu's "Settings…" are disabled, so ⌘1–⌘3, ⌘[ and ⌘, cannot unmount Settings › Account under its sheet; the
  menu's "Sign Out…" brings the running flow forward (1887, "One flow at a time"). — rulings amendment, 2026-10-24
- **Minimizing or hiding** (⌘M, ⌘H) is not a close: the phase runs on and ends within 60 s (1887, "What is not a
  close").
- **Quitting** (⌘Q, gear menu) ends the process and the phase; nothing on this Mac changed (1887).
- **Keys:** Esc is "Stop waiting" in the send phase and Cancel at the ask; Enter on open resolves to Cancel; the red
  confirm has its own focus (10-02 destructive-Enter rule).

### 8.3 The kept rows on Home

The two kept rows (1887's K2 with "Show in Finder" and "Dismiss"; 1882's with "Dismiss") stay in Settings › Sync, where
both specs put them. Home's "Needs your attention" shows the same component with the same words and buttons, reading
the same `kept_unsent_folder` / `kept_unsynced_folder` state, so a dismiss in either place clears both. Reason: Guus
would otherwise have to open Settings › Sync to learn that files were kept.

### 8.4 The device rung

The 1887 device rung waived for 0.8.12 (D2–D11, D10b, D2b; `RELEASE_BLOCKERS.md`) runs on the redesigned surfaces after
slice 1, with "the Settings window" read as "the Beebeeb window, Settings › Account" and the extra close paths (⌘W).

That rung is workspace task 2175. Ruling 3 waits for it: slice 1b changes the menu's path only after 2175 passes, so
2175's D4 (⇧⌘L) tests today's menu behaviour in the new window. Slice 1b then runs D4 again, plus D3 (Cancel keeps
everything), on the menu's new path through the sheet. — rulings amendment, 2026-10-24

## 9. Window lifecycle, menus, keys

### 9.1 Dock icon and activation

- **Dock icon while the Beebeeb window is open** (activation policy Regular), none otherwise (Accessory). The popover
  never adds one (popover ruling 2, applied to the one window).
- With no Dock icon macOS shows no app menu bar, so the popover handles ⌘, and ⌘Q itself (key handling in its webview,
  calling the same commands as the menu).

### 9.2 Menus

Rows F1–F15. The app menu keeps macOS order: About Beebeeb · Settings… · Check for Updates… · Pause Sync · — · Sign
Out… · — · Hide Beebeeb · Hide Others · Show All · — · Quit Beebeeb.

### 9.3 Keys

| Key | Action |
| --- | --- |
| ⌘, | the Beebeeb window on Settings (last ~~tab~~ section used in this window, else General) |
| ⌘1 / ⌘2 / ⌘3 | Home / Review / Trash (from Settings too: the sidebar switches back) |
| ⌘[ | in Settings: back to where Settings was entered from (View › Back; the sidebar's back control) |
| ↑ / ↓, Home / End | in either sidebar list: move and select (automatic activation), first / last |
| ⌘W | close (destroy) the Beebeeb window |
| ⌘M | minimize |
| ⌘H | hide Beebeeb |
| ⌘Q | quit, from the app menu or the popover |
| ⇧⌘P | pause or resume sync |
| Esc | closes the popover; cancels a sheet (Stop waiting in the send phase); does nothing on the window itself |

### 9.4 Launch, login item, quit

- **Login item:** Settings › General "Open Beebeeb at login". A login launch shows the b only (spec C C-W5). Whether
  the login item works in the sandboxed build is task 1845's question, unchanged here.
- **Manual launch and reopen:** §7.3.
- **Quit:** ⌘Q or the gear menu. No confirmation; waiting uploads stay queued for the next launch (as today).

## 10. Copy

### 10.1 Rules

- **No developer words** in anything a person reads on a Mac: "daemon", "hydrate", "engine" (except Rust's refusal
  sentences, which are already person-facing), "control center", "file surface", "namespace", "state DB", "socket",
  "telemetry", "heartbeat", "wired", "this build", raw command names, "Future native wiring…".
- **Platform-correct:** Mac, Finder, macOS; never PC, Explorer, File Explorer, Windows, OneDrive on a Mac.
- **Honest:** never a control that does nothing (E17, E18, E29), never a claim without a control (E12), never "should".
- **House rules:** typographic ’ and …; Inter for people, JetBrains Mono for machine strings (codes, paths, reasons);
  no emoji (the conflict window's "✓" text goes); "Europe" / "the EU" per the EU wording rule; no provider names.
- **Amber** only for encryption state (the lock, upload bars, the sign-in code) and the one primary action per view.
- **Unchanged word for word:** 1887's P/A/K/E strings, 1882's sentences, spec A's Finder copy, spec C's copy, every
  shipped Settings row, `browserLoginCopy.ts`, `accountSwitchCopy.ts`, the support-bundle text.

### 10.2 Every copy change (old → new)

| # | Where | Old | New |
| --- | --- | --- | --- |
| 1 | Drive status intro (`pages/Status.tsx`) | "Beebeeb Desktop is the control center. Finder is the file surface; this page shows whether the daemon can hydrate, upload, and keep versions safe." | removed (Home has no intro) |
| 2 | Drive status loading | "Loading daemon status…" | "Loading…" |
| 3 | Drive status metrics and rows | "Hydrates and uploads waiting now", "Visible without local bytes", "Version reviews needing action", "Session present · engine {state}" | the page keeps its three counters and the Vault access row (Windows and Linux still use them), in plain words: "Downloads and uploads waiting now", "Download when you open them", "Files that need a decision from you", "Signed in" / "Not signed in" (PR #140 review, P3; was "removed"); a Mac shows Home's counts line "On this Mac {n} files · Online-only {n} · Kept on this Mac {n} folders" |
| 4 | Account page sign-out (`pages/Account.tsx`) | "Signing out stops sync and removes the session. Future native wiring should also clear cached plaintext and wrapped-key material." | removed; the row hint stays "You’ll need your password to sign back in." |
| 5 | Account page intro | "Session token, unlock state, diagnostics, sign-out, and launch behavior. Locking clears file-content access while leaving the Finder location visible." | removed |
| 6 | Account page rows | "Session token" / "Stored by native session layer"; "Master key lifetime" / "Unwrapped in memory" / "Unavailable to file operations" | removed; the Vault row's "Vault is unlocked" / "Vault is locked" |
| 7 | Selective sync intro (`pages/SelectiveSync.tsx`) | "The drive is online-only by default. Pin a folder to recursively keep it hydrated on this Mac; unpinning returns it to remote-first behavior after pending uploads settle." | "Folders you choose stay on this Mac. Everything else downloads when you open it." (the shipped sheet) |
| 8 | Selective sync panels and empty state | "Persisted by" / "Local daemon state"; "Scope" / "Recursive folder toggles"; "No remote tree is available. The UI is ready for `list_remote_tree` and `set_recursive_pin`, but this build has no folder data to pin yet." | the three-cell panel stays in plain words ("No folders kept on this computer", "A folder and everything inside it", "Saved" / "On this computer"; PR #140 review, P3); the shipped hints "Couldn’t load your folders." / "Not available in this version yet." |
| 9 | Finder location "Drive model" | "Remote-first vault namespace." and three more rows | the panel stays, rewritten: "Your vault, stored in the cloud.", "Shared folders appear next to your own files.", "Folders you choose to keep on this computer.", "Files that need a decision from you." (PR #140 review, P3; was "removed") |
| 10 | Finder repair (`pages/SyncFolder.tsx`) | "Unregisters the Finder location, turns off Start at login, clears the local socket, and keeps queued uploads." / "Reset Finder integration? … remove the stale local socket …" | `REPAIR_TITLE` "Repair Beebeeb in Finder?" / `REPAIR_BODY` (shipped) |
| 11 | Versions intro (`pages/VersionCenter.tsx`) | "Desktop writes create server-side versions. Stale-base edits, upload failures, quota errors, and restore actions stay visible here until the daemon can finish them." | "Files that need a decision from you, and changes Beebeeb couldn’t send yet." |
| 12 | Versions metrics | "Rows marked conflict by the daemon", "Failed or queued Finder writes", "Local bytes preserved for review" | the three counters stay, rewritten: "Files changed in two places", "Changes Beebeeb couldn’t send yet", "Your changes, kept safe for you to review" under "Changed elsewhere" (PR #140 review, P3; was "removed") |
| 13 | Versions empty | "No conflicts, failed writes, or restore reviews are recorded in the local state DB." | "Nothing needs your attention." |
| 14 | Versions history unavailable | "Version history is unavailable for this file until the server list-versions route responds." | "Couldn’t load this file’s versions. Try again." |
| 15 | Restore toast | "Restore queued" / "{file} version {n} will be restored by the sync engine." | "Restore queued" / "{file} goes back to version {n} on the next sync." |
| 16 | Review kind tags | "Stale base", "Metadata", "Quota", "Permission" | "Changed elsewhere", "Rename or move", "Storage full", "No permission" (Conflict, Upload, Delete, Restore unchanged). Added in slice 4, review of #142: "Signed out" (a sign-in failure; the shipped list called it Conflict), "Not sent" (a change that was parked, paused or given up and will not go on its own), "Plan" (the plan refuses uploads right now) |
| 17 | Review actions (`reviewEntryAction`) | "Review" (conflict), "Upgrade" (quota) | "Decide…", "Add storage" ("Sign in again" unchanged); "Manage plan" for a plan refusal, which opens the billing page: adding storage does not cure a lapsed trial |
| 18 | Notifications page toast | "Notification preferences are not wired up yet; toggles shown are defaults and will not persist." | removed (the Settings rows read the real config; failure: "Couldn’t load these settings") |
| 19 | Bandwidth page toast | "Limits are not wired up yet; values shown are defaults and will not persist." | removed |
| 20 | Shared roots intro | "Shared content is part of the Finder namespace, with permissions enforced by the daemon before any hydrate or write operation." | removed (page unreachable) |
| 21 | Onboarding ready step | "Control center is ready" / "Use the app for sync health, … Finder remains the file surface." / "Open control center" | removed (spec C §11) |
| 22 | Onboarding pinning step | "… now or later from the control center." | removed (spec C §11) |
| 23 | Compact sidebar | "macOS Drive", "Finder-backed vault", "Finder location not installed", "Needs unlock" | removed; the popover's "Vault locked" |
| 24 | `commandUnavailableLabel` (`desktopApi.ts:510-512`), every platform | "{command} is not wired in this build yet." | "This isn’t available in this version of Beebeeb." (the command name goes to the log) |
| 25 | Windows-shell "not wired" states reachable on a Mac (`AccountView.tsx:161`, `ActivityView.tsx:149`, `BandwidthView.tsx:534`, `DevicesView.tsx:331`, `InsightsView.tsx:332`, `SecurityView.tsx:212`, `SelectiveSyncView.tsx:505`, `SettingsView.tsx:205`) | "… isn’t wired into this desktop build yet …" | on a Mac, in the rows that move: "This isn’t available in this version of Beebeeb." (Windows copy is not changed here) |
| 26 | Speed test hint (`BandwidthView.tsx:682-684`) | "Tests your connection to the Beebeeb API. Parity with the CLI `bb speedtest` command: 5 pings + 3 × 10 MB upload + 3 × 10 MB download." | "Measures how fast this Mac reaches Beebeeb: 5 pings, then 3 uploads and 3 downloads of 10 MB. Takes 30 to 90 seconds." |
| 27 | Updates footer (`SettingsView.tsx:1146`, `:1479`) | "When one is available, a toast appears in the app." / "Updates are verified with a minisign signature before installation. No update is applied without your confirmation via the restart toast." | "Beebeeb checks for updates at launch and every 4 hours. ~~Updates are signed, and Beebeeb checks the signature before it installs one.~~" The second sentence is out on a Mac: nothing installs in place (§0 ruling 5), so it would be false — slice 5, 2026-10-25, lead-approved (macos-beebeeb-teamlead) |
| 28 | Update available detail (`updateCheckViewModel.ts`) | "Use the update toast to restart and apply the {Channel} update." | on a Mac the row's own ~~"Restart to update"~~ "Download update" (§0 ruling 5, task 2176); detail "Version {v} is available. Drag Beebeeb into Applications to replace this version." (`macUpdateCopy.ts`, shared with the popover) — slice 5, 2026-10-25, lead-approved (macos-beebeeb-teamlead) |
| 29 | Devices subtitle (`DevicesView.tsx`) | "… Revoke a device to re-encrypt your data against a new key." | "Every device holding a copy of your vault key, and the sync sessions running on each." (no revoke control exists) |
| 30 | Insights subtitle (`InsightsView.tsx:431`) | "Where your storage goes — by content type and your largest files. Everything here is computed from what’s already encrypted on this PC." | "Where your storage goes, by type and by your largest files." |
| 31 | Signed-out gate (`WindowsApp.tsx:207`) | "Your files are encrypted on this PC before they leave. …" | replaced by spec C's sign-in screen |
| 32 | Windows-shell Account (`AccountView.tsx:351`, `:382`, `:618`) | "Files stay encrypted on this PC …", "Signs out this PC.", "Disconnect this account from this PC" | removed on a Mac (the Account tab's copy; E7) |
| 33 | Windows-shell Files subtitle (`WindowsApp.tsx:1282`) | "What’s synced to this PC … Explorer stays your file surface; this is the device view." | removed (Home) |
| 34 | Counts labels (`WindowsApp.tsx:567`, `:699`) | "On this PC" | "On this Mac" |
| 35 | Conflict window (`ConflictWindow.tsx`) | "This device" / "Other device"; "Keep Mine" / "Keep Theirs" / "Keep Both"; "✓ Conflict resolved" | "This Mac" / "Other device"; "Keep this Mac’s" / "Keep the other" / "Keep both"; success toast "Conflict resolved" |
| 36 | Window titles | "Welcome to Beebeeb", "Beebeeb Settings" | "Sign in to Beebeeb" signed out (spec C C-W8), "Beebeeb" signed in |
| 37 | App menu | "Preferences...", "Check for updates...", "Pause/Resume syncing", "Sign out", "Open Beebeeb folder", "Upload files...", "New folder...", "Open web app" | "Settings…", "Check for Updates…", "Pause Sync" / "Resume Sync", "Sign Out…", "Open in Finder", "Upload Files…", "New Folder…", "Open Web App" (macOS title case and the ellipsis character) |
| 38 | Auth-expired banner (`AuthExpiredBanner.tsx`) | "You're signed out on this device" / "Sync is paused until you sign in again." / "Sign in again" | screen "Sign in again" / "Sync is paused until you sign in again. Your files stay in Finder." / the sign-in methods |
| 39 | Popover offline body | 30 Sep: "Beebeeb didn’t answer. Files on this Mac are unchanged." + mono timeout; spec C: "Can’t reach Beebeeb. Check your connection." | title "Can’t reach Beebeeb" / "Check your connection. Files on this Mac are unchanged." |
| 40 | Popover error (new state copy) | 30 Sep's error drawing reused the offline words | "Sync stopped" / "Beebeeb couldn’t finish syncing. Files on this Mac are unchanged." + mono reason |
| 41 | Popover, no plan, after the click | spec C (window): "Waiting for your plan. This window updates by itself." | popover: "Waiting for your plan." (the window keeps spec C's line) |
| 42 | New: popover attention row | — | "{n} files need a decision" / "Both changed on two devices." (n = 1: "1 file needs a decision" / "It changed on two devices."); "Review…"; activity meta "Changed on two devices" |
| 43 | New: gear menu and sign-out-in-progress button | tray menu "Open Beebeeb" | "Open Beebeeb" (same words) |
| 44 | New: sign-in screens | — | "Sign in with browser instead" (link back from the email form); "Signed in before as {email}" (Sign in again footer) |
| 45 | New: Home | — | "Beebeeb lives in the menu bar" / "Click the b at the top of your screen to see what’s syncing. You can close this window; Beebeeb keeps running." / "Got it"; "Last checked {time}"; group titles "Needs your attention", "Storage", "Recent activity" |
| 46 | New: Settings | — | "Appearance"; "From your account"; "Storage details…"; "Password, email and billing" / "Changed on beebeeb.io, in your browser." / "Open beebeeb.io"; "Release channel"; "Test connection speed" / "Run test"; ~~"‹ Sign in" (signed-out Settings toolbar)~~ "Sign in" (the signed-out Settings sidebar's back control, §7.8) |
| 47 | Popover Finder failure title | spec A gives the sentence only | "Couldn’t add Beebeeb to Finder" (the 30 Sep title) above spec A's sentence |
| 48 | New: Settings sidebar (ruling 1) | — | back control: a chevron and "Home", "Review", "Trash" or "Sign in" (existing words), accessible name "Back to {Home}"; the sidebar's label "Settings"; View menu "Back" ⌘[ |

## 11. Accessibility

- **Keyboard.** Popover: focus starts on the first actionable element (the state's primary action, else the gear);
  Tab cycles header → body → footer; Esc closes; the gear menu is native, so arrows and type-select work. Window:
  ~~sidebar items are a list with arrow keys and Return; ⌘1–⌘3 and ⌘, jump; Settings tabs keep the WAI-ARIA tab model
  (`tabAfterKey`: arrows, Home, End).~~ each sidebar list (Home / Review / Trash, and the Settings sections) is one Tab
  stop with a roving tabindex: ↑/↓ move and select (automatic activation, `tabAfterKey`'s model with vertical
  arrows), Home and End jump to the first and last, Return and Space also select. In Settings the Tab order is back
  control → section list → pane. Entering Settings puts focus on the selected section; leaving it (back control, ⌘[)
  puts focus on the gear; ⌘1–⌘3 put it on the section they chose. ⌘1–⌘3, ⌘, and ⌘[ jump. Sheets trap focus and open
  on their close button (Esc cancels, Enter on open is Cancel), except Version history, whose first act is typing a
  file name: it opens on its search field, where Enter runs nothing (a Restore needs its own press) — slice 4,
  2026-10-25, review of #142, the lead's option; the send sheet's Esc is Stop waiting.
- **VoiceOver.** The status item's accessibility description follows the tooltip ("Beebeeb, Up to date", "Beebeeb:
  Offline", "Beebeeb: Signed out"). The popover is a dialog labelled "Beebeeb". Activity rows read as one sentence
  ("Q3 ledger reconciliation.xlsx, uploaded, Investigations, 2 minutes ago"); direction arrows have labels ("Uploaded",
  "Downloaded"). Progress bars are `progressbar` with a value text ("62 percent, encrypting and uploading"). Count
  changes in the send phase are `aria-live="polite"` (as shipped). The sidebar is a `navigation` landmark with
  `aria-current="page"`; the gear is a button labelled "Settings". In Settings the same landmark is labelled
  "Settings"; the back control is a button named "Back to Home" (or Review, Trash, Sign in); the account row reads
  "Account, {email}, {plan line}"; the buttons a list repeats are named for what they act on, the visible word first
  ("Decide…, {file}", "Restore, {file}", "Delete…, {file}", "Restore, Version 4, {file}": slice 4, review of #142);
  the selected section has `aria-current="page"`; the pane is a `main` region
  labelled by its toolbar title (a heading). Every icon-only button has a label (gear: "Beebeeb
  menu" in the popover, "Settings" in the sidebar; Get help: "Get help, opens in your browser").
- **Contrast, both themes, from the OKLCH tokens** (hi-fi §7 has the table): all body and hint text is ≥ 4.5 : 1
  (`--ink-3` on `--paper-2` is 5.02 light, 5.92 dark). The Settings sidebar adds three pairs (ruling 1): `--ink-2` on
  `--paper-2` 10.7 / 10.2 (section rows, back control), `--ink-2` on `--paper-3` 10.0 / 8.90 (the selected account
  row's plan line), `--ink` on `--paper-3` 16.0 / 13.0 (selected labels). Error states are ink on `--err-bg` (title 16.7 light, 13.3
  dark; body 10.4 / 9.05) and never red text (`--red` on paper is 3.86 in light): red is only for discs, icons and the
  full storage bar, which as non-text need ≥ 3 : 1. `--ink-4` is never text (2.76 light). The amber primary is 10.6;
  the red destructive confirm keeps `--red-ink` (design.css: 5.10 / 6.13). Toasts keep slice 5's rule (text ≥ 4.5,
  icon ≥ 3, every variant, all three theme scopes; `tests/toastContrast.test.tsx`).
- **Motion.** Spinners and the popover's appearance respect `prefers-reduced-motion` (static indicator, no slide).
- **Size.** Text follows the shipped scale (13 pt rows, 11.5 pt hints); the window's minimum size keeps every Settings
  row on one line pair without truncating a path (paths wrap, never "…").

## 12. Implementation slices

Each slice ships on its own, keeps Windows and Linux unchanged (their window configs, `startup_surface` rows and pin
tests), writes its tests before its code and sees them fail first, and names its verification rung. Slices 1, 2 and 3
can run as two lanes (Rust + TypeScript) under the two-lane cap.

| # | Slice | What changes | Verification rung |
| --- | --- | --- | --- |
| 0 | This design | spec + hi-fi + the three amendments; then the rulings amendment (§0) | ~~Guus approves (open questions answered)~~ Answered 24 Oct (§0, ruling 4: "Yes, start slice 1"); slice 1 starts when this amendment merges |
| 1 | **The Beebeeb window and Settings in it** | Rust: `main` window config (`create: false`, Overlay title bar), `main_window_label` macOS → `main`, registry `[popover, main]`, close = destroy, window-state for `main` only with SIZE \| POSITION and the off-screen guard, activation policy (§9.1), menus F1–F15, `main-app`/`tray`/`windows-onboarding` not built on macOS (a `tauri.macos.conf.json` override or `create: false` with Windows building them), `macos-settings` retired, every 1887 owner label → `main`. TS: `MacApp.tsx` shell (sidebar, sections, `main:navigate`, 1887's flow object and listeners), ~~the Settings pane hosting the shipped tab components~~ Settings in System Settings style (ruling 1, §7.8): the sidebar switching to the Settings list (back control, account row, General, Sync, About; Security joins in slice 5) and the pane hosting the shipped section bodies with the tab strip removed, and Home's first cut (status, Finder row, storage, the kept rows). Launch and every opener that showed the compact window now show `main`; until slice 3 a tray click still shows today's compact flyout, and nothing else opens the compact window. | `cargo test` per-binary counts (registry, labels, menu specs incl. View › Back, window-list pins, 1887 `sign_out_flow_window_closed` with `main`); `bun test` count (~~tabs reused~~ section bodies reused; sidebar switch and back, focus to the gear on back; ↑/↓/Home/End, ⌘[; last section remembered; the View items and ⌘, disabled while a sheet is open, §8.2; sheets; F11 flow-reset tests with `main`); tsc, eslint; Chromium renders light and dark vs the hi-fi. **Device:** ⌘, / menu / gear / popover-less tray click open one window, and Settings swaps the sidebar in that window; back and ⌘[ return; close and reopen keeps the frame; unplug-display guard; then **1887's D2–D11, D10b, D2b on this build** (§8.4, task 2175) |
| 1b | **Sign Out… through the confirm sheet** (ruling 3) | Starts only after task 2175 passes. Rust: the menu's SignOut arm stops calling `clear_session_impl` and stores a flow request that starts at the confirm sheet; it opens `main` on Settings › Account. The menu's immediate alerts retire; late results keep `show_late_result` (A14). TS: the window shell answers that request with `flow.open()`. 1887's pins on the menu arm are updated in the same change, never deleted. | cargo (the arm calls no sign-out; the request kind; pins red first on today's arm); `bun test` (menu request opens the confirm sheet, Cancel changes nothing, focus on Cancel). **Device:** 1887 D4 (⇧⌘L) and D3 (Cancel keeps everything) on this path |
| 2 | **Sign-in and setting up this Mac in the window** | Signed-out content (§7.4): spec C's sign-in screen and the shared browser-code component, email/2FA, Sign in again (R8), switch step, recovery phrase, Finder status → Home; `onboarding` window and `open_onboarding_window`/`open_reauth_window` route to `main` on macOS; spec C §11 removals. Spec C's Rust account view (1747) is its own lane; this slice renders today's `sync_status` and gains C-R3/C-R5/C-R11 screens when 1747 lands. | `bun test` (screen routing per state, copy pins, switch ask, focus on Cancel); cargo (open-sign-in routes to `main`; `onboarding` never built on macOS). **Device:** fresh install → sign-in → browser and password paths → recovery → Finder Ready → Home in one window; Sign in again after a revoked session keeps Finder |
| 3 | **The popover** | `popover` window (§6.1), `PopoverApp.tsx` on `popover_snapshot` (every §6.3 state), gear `NSMenu`, click rules (§6.2), tray context menu removed on macOS, compact window not built on macOS (`App.tsx` stays for Linux). | `bun test` (state per phase fixture, precedence, footer disabling, no toast); cargo (anchor math exists from slice 1 groundwork; click routing by session). **Device:** 10 clicks on two displays, notch, blur hides, Esc, ⌘Q and ⌘, from the popover, crossed b when signed out |
| 4 | **Review and Trash** | Review (conflict sheet, version history sheet, kind labels), `conflict_auto_opens_window(Macos) = false`, ~~notification click → Review~~ the notification click's route (not its trigger: A17, task 2181); Trash from the Windows shell; `ConflictWindow` not opened on macOS. | `bun test` (rows, actions, sheet keys); cargo (no conflict window on macOS; notification route). **Device:** a real conflict (Mac + web edit) resolved three ways; restore from Trash |
| 5 | **Settings additions and the copy sweep** | Security ~~tab~~ section (its sidebar row); General Appearance and "From your account"; About release channel, downgrade, speed test; Account storage details and web row; every §10.2 change; a copy guard test that fails on the §10.1 word list in any string reachable on macOS (proven red on today's tree). | `bun test` incl. the guard's red-then-green; screenshots of each ~~tab~~ section both themes. **Device:** each ~~tab's~~ section's controls write and read back |
| 6 | **Launch and lifecycle** | Login launch shows nothing (spec C C-W1, depends on 1845 for a working login item), manual launch / `RunEvent::Reopen` → window, updater relaunch flag, first-run hint once. | cargo (pure launch-kind and relaunch-flag functions, red first). **Device:** login item launch, Dock and Finder reopen, an alpha update relaunch |
| 7 | **Device rung on a signed alpha** (Guus or Bram) | The whole two-surface pass, light and dark, with VoiceOver on for the popover and one Settings ~~tab~~ section, including the sidebar switch and back. | Evidence under the workspace `_qa-evidence/1683/`; nothing ships to stable before it |

## 13. ~~Open questions for Guus~~ Questions for Guus, answered 24 Oct 2026

1. ~~**Settings look.** Recommended: icon tabs in the window's toolbar (drawn), the look approved on 30 Sep, now inside
   the main window; it reuses the shipped tab components and keeps 1887's code where it is. Alternative: System
   Settings style, where the categories replace the sidebar while you are in Settings, with a back arrow.~~
   **Ruling 1: "System Settings style."** The alternative won. Settings is a sidebar list of sections with the pane on
   the right, and it replaces the window's sidebar while Settings shows, with a back control (§7.8). The shipped
   section bodies and 1887's code are kept; only the tab strip goes.
2. ~~**Windows-shell pages that do not come to the Mac.** Recommended: leave out Bandwidth's traffic chart and live
   sessions (E10), App Activity (E30), the account notifications list (E15), and the switches a Mac never honours
   (E17, E18, E20, E29). Alternative: keep any of them, in Settings.~~
   **Answered, ruling 2: "Drop them (Recommended)."** E10, E15, E17, E18, E20, E29 and E30 stay Removed (§0 also
   records where each still lives).
3. ~~**The app menu's "Sign Out…".** Recommended: after 1887's device rung passes, make it open the same "Sign out of
   this Mac?" sheet as Settings, so there is one way to sign out. Until then (and in this design) it keeps today's
   behaviour: it signs out at once when nothing waits.~~
   **Answered, ruling 3: "Yes, same sheet (Recommended)."** Slice 1b, after task 2175 passes (F4, §8.1, §8.4).

— rulings amendment, 2026-10-24

Decided here, with the reason, so they need no answer: no Files section (files are in Finder; the Windows "Files" was a
summary); no signed-out popover (spec C already rules a click opens sign-in); the window closes by destroy (one idle
webview, as 30 Sep promised); the window remembers its frame (standard macOS, and it answers "randomly in the
screen"); nothing opens after an update (popover ruling 1); the kept rows mirror on Home (§8.3).

## 14. What this spec could not determine from the code

1. Why the Windows shell window was visible on the lead's Mac (§3): likely the config-built `main-app` plus the
   window-state plugin restoring visibility; not reproduced.
2. Whether these work against a File Provider Mac: `desktop_file_overview` (Home's counts line), `free_up_space` and
   the File menu's "Upload Files…" and "New Folder…" (all read `sync_root`; `lib.rs:9542-9543`, `:10599`). Home shows
   the counts line only when the command answers; slice 5 checks the File menu items on a device and removes any that
   cannot work.
3. Whether macOS enforces any local cache cap: in `src-tauri`, `local_cache_limit_bytes` is read only by the warning
   (`runner.rs:291-300`, `lib.rs:12902-12914`). Hence E29.
4. Whether the login item works in the sandboxed build (task 1845, open).
5. Whether Search and Version history in the Windows shell worked while signed out (not traced; both move behind a
   session here).
6. Nothing in this spec has run on a Mac; the hi-fi is a browser drawing of macOS chrome.

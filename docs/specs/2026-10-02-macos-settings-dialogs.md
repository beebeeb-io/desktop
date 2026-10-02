# macOS Settings dialogs — design ruling (task 1683 slice-4 follow-up)

**Status:** ruling — pending Guus’s review
**Date:** 2 Oct 2026
**Design source:** `design/hifi/macos-settings-dialogs.html`
**Context:** decision file `.claude/tasks/decisions/1683-s4-undrawn-and-unbacked.md`, question 1: the three Settings dialogs built in slice 4 follow the existing `Modal` styling — acceptable as-is, or do they need their own dialog treatment before slice 6 flips the window on?

## Verdict (question 1)

**All three dialogs are acceptable as built.** The shared `Modal` (title, close button, body, footer, Esc/backdrop dismiss, focus trap) is the right anatomy for a Settings window; no bespoke dialog treatment is needed. No app code changes are required for slice 6 to turn the window on. One treatment follow-up is recommended and queued for a later slice:

1. **Sign out confirm button should be destructive red**, not amber-primary (details in “Destructive-action treatment” below). One-line change: a `danger` prop on `ConfirmSheet`.
2. **Enter should trigger the confirm action** in the two confirmation dialogs (not built today). Queued with it.

If no further work is wanted, record the outcome as **“approved as built”** with the Sign-out follow-up noted.

## Visual language

The dialogs reuse the Settings-window system exactly: `design.css` tokens, the 13 px/18 px Inter type ramp, `ms-card`/`ms-row` styling for body content, `ms-btn` (26 pt, radius 6) controls, the neutral `ms-switch`, and the Modal chrome built in slice 4 (15 px/700 title, 18/20 padding, radius 14, `--shadow-3`, `--win-scrim`). The hifi file draws the Modal at its built widths (400 for the ConfirmSheet dialogs, 440 for Choose folders) over the scrim, in both light and dark themes.

## Dialog 1 — Choose folders (Sync tab)

- **Purpose:** pick which synced folders stay fully on this Mac instead of downloading on open.
- **Trigger:** Sync tab row **Keep on this Mac** → button **Choose folders…** (disabled when nothing is uploaded yet — `keep.kind === 'unreported'` — or there are no folders).
- **Copy (exact, built):**
  - Row: “Keep on this Mac”, count label “3 of 12” (example), button “Choose folders…”
  - Dialog title: “Keep on this Mac”
  - Body: “Folders you choose stay on this Mac. Everything else downloads when you open it.”
  - Rows: folder name, optional `where` path hint, a switch per folder.
  - Failure toasts: “Couldn’t keep that folder on this Mac” / “Couldn’t stop keeping that folder”
- **Actions:** this is a settings sheet, not a confirmation — every switch saves immediately (`set_recursive_pin`). No footer actions beyond the `Modal` default **Close** button.
- **Destructive treatment:** none. Turning a switch off changes availability, not data; no red.
- **Keyboard:** Esc closes (built). Tab traps through close button → switches → Close. Space toggles the focused switch. Enter must do nothing on the sheet (a stray Return must not flip a switch or close the dialog with a save still in flight). Switches disable while `savingFolder`; a failed save shows the toast and leaves the switch unchanged.
- **Modal mapping:** `Modal` at `maxWidth 440`, no `footer` prop → built-in default footer (Close). Nothing changes.

## Dialog 2 — Repair Beebeeb in Finder (Sync tab)

- **Purpose:** confirm before `reset_macos_integration` removes Beebeeb’s Finder location and turns off Open-at-login.
- **Trigger:** Sync tab, Finder row with `kind === 'added'` → button **Repair…** (busy label **Repairing…** while running).
- **Copy (exact, built):**
  - Title: “Repair Beebeeb in Finder?”
  - Body: “Beebeeb removes its Finder location and turns off Open Beebeeb at login. Files waiting to upload are kept. **You can add it back afterwards.**”
  - Buttons: “Cancel” / “Repair”
  - Failure note (inline in the Sync pane): “Couldn’t repair Beebeeb in Finder” / “Nothing was changed that you need to undo. Try again.”
- **Actions:** primary confirm **Repair**; **Cancel** dismisses. Dismiss-on-confirm already: the dialog closes when Repair runs; the busy state lives on the trigger button.
- **Destructive-action treatment:** **amber-primary, deliberately.** Repair is reversible — the body says you can add it back — so it is a caution, not a destruction. Red is reserved for irreversible actions. Keep as built.
- **Keyboard:** Esc cancels (built). Enter should trigger Repair once Enter handling is added; focus starts on the close button, which keeps a stray Return harmless until then.
- **Modal mapping:** `ConfirmSheet` (Modal 400, footer Cancel + primary confirm). Nothing changes.

## Dialog 3 — Sign out of this Mac (Account tab)

- **Purpose:** confirm before `clear_session` signs this Mac out and stops sync.
- **Trigger:** Account tab row **Sign out of this Mac** (hint: “You’ll need your password to sign back in.”) → button **Sign out…**
- **Copy (exact, built):**
  - Dialog title: “Sign out of this Mac?”
  - Body: “Sync stops until you sign in again.”
  - Buttons: “Cancel” / “Sign out”
  - Failure toast: “Couldn’t sign out”
- **Actions:** confirm **Sign out**; **Cancel** dismisses. Failure is a toast (transient), consistent with the popover spec ruling.
- **Destructive-action treatment:** **the one recommended change.** Signing out stops sync on this Mac — a destructive consequence. The confirm button should render as `ms-btn--danger` (red fill `--red`, paper text) instead of amber-primary, so its weight matches Repair’s caution level being exceeded: Repair is reversible and amber; Sign out is not offered as reversible and should be red. Because confirmations keep the safe action first, order stays Cancel → Sign out; the red fill is the guard, and Enter (once wired) must land on **Cancel**, not the red button, on open.
- **Keyboard:** Esc cancels (built). Enter should trigger the confirm only when the person tabs to it — never on open. Initial focus on the close button (built) satisfies this until Enter handling lands.
- **Modal mapping:** `ConfirmSheet` (Modal 400). **Queued change:** add `danger?: boolean` to `ConfirmSheet`; when set, the confirm button gets `ms-btn--danger`. In `MacSettings.tsx` pass `danger` on the Sign-out sheet. Nothing else changes.

## Keyboard rules (summary)

| Key | Confirmations (Repair, Sign out) | Choose folders |
| --- | --- | --- |
| Esc | Cancel — dismiss, no action (built) | Close (built) |
| Enter | Trigger confirm (queued; not built) | No-op |
| Tab | Focus trap as built; initial focus = close button | Same |

Destructive-Enter rule: on open, Enter must resolve to **Cancel** (or nothing), never the red confirm. Initial focus on the close button satisfies this today; preserve it when Enter handling is added.

## What this pass does not change

No `src/`, `src-tauri/` or test files are touched. The queued follow-ups (Sign-out `danger` prop, Enter handling) are recorded here and in the hifi file for a future app slice.

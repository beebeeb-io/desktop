# src/v2: desktop v2 (Windows first)

Spec: workspace `docs/superpowers/specs/2026-11-08-desktop-v2-design.md`. Work split and file ownership:
workspace `design/desktop-v2/BATCHES.md`.

| Path | What |
|---|---|
| `entry.tsx`, `v2.css` | Loaded on demand by `src/main.tsx`; Tailwind 4 + `@beebeeb/shared` tokens + board-only tokens |
| `V2App.tsx` | The window: caption bar, sidebar, one section, status line; `?frame=` renders a registered frame |
| `routes.ts` | Sections and the URL route (`section`, `id`, `sub`), history-backed Back and Forward |
| `shortcuts.ts` | The board's keyboard shortcuts; sections listen with `useCommand(id, handler)` |
| `chrome/` | Windows chrome: `CaptionBar` (app-drawn, 46 x 32 buttons), `Sidebar`, `StatusLine` |
| `source/` | `DriveSource` implementations: `desktop.ts` (Tauri commands), `fixtures.ts` (board data), `index.ts` picks |
| `sections/<id>/` | One directory per section, each owned by one batch |
| `frames/bNN.tsx` | Each batch's frame table; `registry.ts` merges them and never changes |
| `auth/`, `tray/`, `onboarding/` | Sign-in (B10), the flyout (B09), first run (B11); `enabled.ts` flips a window to v2 |

Run a frame headless: `bun run dev`, then open
`http://localhost:5173/?window=main-app&platform=windows&frame=DW-0401` (`&theme=dark`, `&scenario=offline|empty|full|nearly-full|syncing`).
Without `frame`, the plain window renders with the fixtures in a browser and with the desktop source inside Tauri.
The legacy Windows shell stays at `?window=legacy-main-app&platform=windows` until B13 retires it.

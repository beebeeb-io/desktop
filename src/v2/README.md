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
| `source/` | `DriveSource` implementations: `desktop.ts` (Tauri commands), `fixtures/board.ts` (the one fixture set: every batch's fixtures, which the frames render too), `index.ts` picks |
| `sections/<id>/` | One directory per section, each owned by one batch |
| `frames/bNN.tsx` | Each batch's frame table; `registry.ts` merges them and never changes |
| `live.tsx`, `seed.ts`, `dev/` | The live state route (dev server only, never bundled): `dev/states-*.ts` hold one recipe per frame |
| `auth/`, `tray/`, `onboarding/` | Sign-in (B10), the flyout (B09), first run (B11); `enabled.ts` flips a window to v2 |

Run a frame headless: `bun run dev`, then open
`http://localhost:5173/?window=main-app&platform=windows&frame=DW-0401` (`&theme=dark`, `&scenario=offline|empty|full|nearly-full|syncing`).
Without `frame`, the plain window renders with the fixtures in a browser and with the desktop source inside Tauri.

Open a frame's state in the REAL window (batch M2): `http://localhost:5173/?window=main-app&platform=windows&state=DW-0402`.
It mounts the live window (real sections, handlers, routing) and drives it into that frame's state from the one fixture
set: the recipe's route and scenario go into the URL (another window's frame moves to `?window=tray` or `onboarding`),
`dev/reads.ts` answers the read commands a browser lacks, a seed (`seed.ts`) gives a component its first state where no
event reaches it, and the recipe's steps replay through real DOM events. `<html data-v2-state-result>` says `ready`,
`native: …` (Windows draws that frame) or `failed: …`; `data-v2-state-gap` names what the live app cannot draw yet.
The map audit (`scripts/v2-map-audit/audit.mjs`) opens every source frame this way.
The legacy Windows shell stays at `?window=legacy-main-app&platform=windows` until B13 retires it.

# beebeeb-io/desktop

Beebeeb desktop app. **Tauri v2** shell around the web client (`repos/web`) plus the Rust sync engine from `core`. One codebase, three OS targets (macOS, Windows, Linux).

## Release status — Windows + Linux released, macOS not yet

**`desktop-v0.1.0` is published** (Windows + Linux):
https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.1.0 — signed MSI + NSIS installer
for Windows, AppImage/.deb/.rpm for Linux, each with a minisign `.sig` for the auto-updater.

`release.yml` builds Windows and Linux only. **macOS is deliberately disabled** in the build
matrix (commented out, not deleted) — two separate gaps block it:
- `scripts/build-fileprovider-extension.sh`'s arch-detection doesn't support
  `TAURI_ENV_ARCH=universal` (a real universal build needs building the File Provider extension
  for both arches and `lipo`-combining them).
- No Developer ID / notary credentials are wired into CI yet (task 0342).

Re-enable by uncommenting the macOS entry in `release.yml`'s build matrix once both are resolved.

**Not yet verified:** a real install + close-to-tray smoke test on physical Windows hardware
(only build/sign/publish has been confirmed, from CI + local Linux verification — see below).

**Local verification:** on Linux, `bun run tauri build --target x86_64-unknown-linux-gnu --
--locked` reproduces the real build + `.deb`/`.rpm` bundling (needs `webkit2gtk-4.1`,
`libayatana-appindicator`, `patchelf`, `fuse2`, `fuse3`, and a `TAURI_SIGNING_PRIVATE_KEY` — a
throwaway local key via `bunx tauri signer generate` is fine for this, no need for the real prod
key). `.AppImage` bundling specifically cannot be verified on Arch/WSL2: the vendored
`linuxdeploy` tool's bundled `strip` binary is too old to parse the `.relr.dyn` ELF section
modern Arch shared libraries use, and ignores the `STRIP` env override — this does not reproduce
on GitHub's `ubuntu-latest` runners, so CI remains the source of truth for that one step. Windows
and macOS builds cannot be reproduced locally at all (no cross-compile path from Linux).

## Releases

Before cutting a desktop release, follow `docs/RELEASING.md`. The release workflow fails closed
unless the released commit contains a root `RELEASE_NOTES.md` whose contents mention the exact
semver value passed to the workflow; author and commit per-release notes before triggering it.

## Architecture

```mermaid
graph TD
    WEB["repos/web (React 19 + Vite 6)<br/>UI for files, settings, sharing"]
    TAURI["src-tauri (Rust)<br/>WebView host, IPC, OS integration"]
    SYNC["beebeeb-sync (Rust, from core)<br/>File watcher, conflicts, selective sync"]
    CORE["beebeeb-core (Rust, from core)<br/>AES-256-GCM, Argon2id, HKDF, BIP39"]
    OS["macOS / Windows / Linux<br/>Native menu bar, tray, notifications, autostart"]

    WEB --> TAURI
    TAURI -- "Cargo dep" --> SYNC
    SYNC --> CORE
    TAURI --> OS
```

Why Tauri (not Electron): native OS WebView (~10MB binaries vs ~150MB), Rust backend that imports `beebeeb-core` directly, no Node runtime in production builds.

## Layout

```
desktop/
├── src-tauri/            Rust app (Tauri v2)
│   ├── Cargo.toml
│   ├── build.rs
│   ├── tauri.conf.json   App config (window, bundle, identifier)
│   ├── capabilities/     Permission grants per window
│   ├── icons/            Bundle icons (PNG, ICNS, ICO)
│   └── src/
│       ├── main.rs       Entry — calls into lib::run()
│       └── lib.rs        Tauri Builder, command handlers
├── src/                  Placeholder frontend (replaced by repos/web)
├── index.html            Vite entry
├── package.json          Dev/build scripts (bun)
├── vite.config.ts        Frontend bundler config
└── tsconfig.json
```

The `linux/`, `macos/`, `windows/` folders are pre-Tauri scaffolds (separate native shells per OS). They are kept for reference but the Tauri shell supersedes them — new work goes in `src-tauri/`.

## Build & run

Requires Rust stable, bun, and Tauri's per-OS prerequisites (https://tauri.app/start/prerequisites/).

```sh
bun install
bun run tauri:dev      # spawns vite on :5173, opens native window
bun run tauri:build    # produces .dmg / .msi / .deb / .AppImage
```

To develop against the real web client instead of the placeholder:
- Run `repos/web` separately (`cd ../web && bun dev`) on :5173.
- The Tauri window will load it automatically (`devUrl` in `tauri.conf.json`).
- For production bundles, the web client is built and copied to `../dist/` (or `tauri.conf.json` is repointed at `../web/dist/`).

`cargo check` from `src-tauri/` validates the Rust side without the frontend.

## Key files

- `src-tauri/tauri.conf.json` — identifier `io.beebeeb.app`, 1200×800 default window.
- `src-tauri/src/lib.rs` — `run()` builds the Tauri app and registers `#[tauri::command]` handlers. Add new commands here.
- `src-tauri/capabilities/default.json` — permissions the frontend can invoke. Add explicit permissions before exposing new tauri APIs.

## Current macOS integration state

macOS is the first desktop product target. Keep cross-platform logic in Rust
where possible: auth/session handling, crypto, sync, queueing, conflict/version
logic, cache policy, and API protocol code belong in `src-tauri` or shared Rust
crates rather than being duplicated in the frontend.

Current release identifiers:

- App bundle id: `io.beebeeb.app`
- File Provider extension id: `io.beebeeb.app.FileProvider`
- File Provider domain id: `io.beebeeb.app.domain`
- App group: `R8352WDJJR.io.beebeeb.app.fileprovider`

The active Finder/File Provider blocker is Apple provisioning/profile
entitlement mismatch. Both the containing app and extension profiles must
include the exact app group above. Do not mask this as a UI timeout or switch to
a fake local folder root; fix signing/provisioning first.

The Tauri warning that `io.beebeeb.app` ends in `.app` is known. Keep this
identifier unless Guus creates a new macOS App Store Connect record and chooses
to migrate. Expo is unrelated to desktop builds; it only applies to mobile.

## Adding commands (Rust → JS bridge)

1. Add a `#[tauri::command]` function in `src-tauri/src/lib.rs`.
2. Register it in the `invoke_handler!` macro.
3. Call from JS: `import { invoke } from "@tauri-apps/api/core"; await invoke("my_cmd", { arg })`.
4. If it touches an OS API (fs, dialog, notification…), add the matching permission to `capabilities/default.json`.

## Sync engine

`src-tauri` is expected to own the background Rust sync runtime and call shared
core/sync logic instead of duplicating protocol behavior in TypeScript.
Conflict resolution policy: never silently drop a version — default `KeepBoth`
(rename loser as `file (Device, HH:MM).ext`). File watcher debounce: 100ms.
The backend must create a new server version for Finder writes when the server
versioning contract supports it.

## Auto-update signing

Tauri's updater verifies update bundles with minisign. The keypair was
generated with `bunx tauri signer generate -w ~/.tauri/beebeeb-desktop.key`
on 2026-05-07.

- **Public key** lives in `src-tauri/tauri.conf.json` under
  `plugins.updater.pubkey`. Safe to commit; it's the verification key.
- **Private key** lives at `~/.tauri/beebeeb-desktop.key` on the build
  machine, mode `0600`. **Never commit it.** It's not in any `.gitignore`
  because it's not in the repo — it sits in the user's home dir.
- The key was generated **without a password** for initial pre-launch
  setup (the `-p ""` flag). Before public release, regenerate with a
  strong password, update the pubkey in tauri.conf.json, and rotate the
  CI secret. Track this work in `.claude/tasks/` if not already.

**For CI** (GitHub Actions, EAS, etc.) building signed updates:

- `TAURI_SIGNING_PRIVATE_KEY` — the *contents* of `~/.tauri/beebeeb-desktop.key`,
  set as a repository secret (not the file path)
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — the password if the key has one;
  unset for the current passwordless setup

To inspect the public key locally: `cat ~/.tauri/beebeeb-desktop.key.pub`.
The fingerprint at time of generation is `545D7BA77EDEA7E1`.

If the key is lost, no clients with already-installed builds can verify
new updates — they'll need a manual reinstall to onboard the new pubkey.

## Brand

- Icons: amber rounded square with lowercase `b` (`icons/icon.png`). Replace with the production icon set via `bunx tauri icon path/to/source.png` once ready.
- App name: "Beebeeb". Identifier: `io.beebeeb.app`.

## Graphify

This repo has a knowledge graph at graphify-out/.
- Before exploring code, read graphify-out/GRAPH_REPORT.md for module structure and relationships.
- After modifying code, run `graphify update .` and commit the updated graphify-out/.
- The graph tracks modules, functions, types, and their relationships.
- `graphify query "<question>"` to ask questions about the codebase.
- `graphify path "<A>" "<B>"` to find connections between two concepts.

## Keep shared docs in sync

When you add/change/remove endpoints, types, build commands, or dependencies: update the relevant skill file in `/Users/guuslangelaar/Development/Beebeeb/beebeeb.io/.claude/skills/` (beebeeb-api.md, beebeeb-designs.md, beebeeb-stack.md, beebeeb-dev.md). Other agents depend on these being accurate.

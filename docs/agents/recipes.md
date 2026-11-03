# Task recipes — desktop

The four changes this repo is asked for most, in the order the change travels (command → IPC →
menu → screen). `MAP.md` is how to find anything else; `recipes.ts` proves every file and item
below still exists.

## Add a Tauri command

A command is `#[tauri::command]`, registered once in `run()`, and reached only through `desktopApi.ts`.

- Files: `src-tauri/src/macos_popover.rs` (1), `src-tauri/src/lib.rs` (2), `src/desktopApi.ts` (3)
- Pattern: `popover_retry_sync` in `src-tauri/src/macos_popover.rs` (the shape: `AppHandle`/`WebviewWindow`, `Result<_, String>`, `require_*` guard first); `run` in `src-tauri/src/lib.rs` (the `invoke_handler(tauri::generate_handler![…])` list); `command` in `src/desktopApi.ts` (the one door the screen calls)
- Steps:
  1. Write the command in its own module next to the code it drives — `pub(super)` when only
     `lib.rs` registers it — and guard its surface (`require_popover`) before doing any work.
  2. Make it async and `spawn` heavy work; a command otherwise runs on the main thread, and the
     menu tests assert a failure path never blocks the caller.
  3. Register it in `generate_handler![…]` inside `run`. That macro takes no `cfg`, so a mac-only
     command must exist on every platform and say no there.
  4. Name it snake_case and never rename one that ships: the Swift extension and an older screen
     still call the old name.
  5. Call it from `desktopApi.ts` (last recipe), never with a bare `invoke` in a component.
- Check: `cargo check --manifest-path src-tauri/Cargo.toml --all-targets`

## Add an IPC message

The socket is a tagged JSON request and response shared with the File Provider extension; the tag is
a wire name.

- Files: `src-tauri/src/ipc_socket/protocol.rs` (1), `src-tauri/src/ipc_socket/server.rs` (2), `BeebeebFileProvider/IPCFraming.swift` (3), `docs/IPC_PROTOCOL.md` (4)
- Pattern: `IpcRequest` in `src-tauri/src/ipc_socket/protocol.rs` (the request variants and their serde tags); `IpcResponse` in `src-tauri/src/ipc_socket/protocol.rs` (the answers); `Connection` in `src-tauri/src/ipc_socket/server.rs` (the dispatch match in `impl Connection` and the handler beside it)
- Steps:
  1. Add the variant and its fields to `IpcRequest`; the serde tag is the wire name, so add a new
     one and never rename one that ships.
  2. Add the payload structs to `protocol.rs` (or its area file) and the answer to `IpcResponse`;
     a newly added field is `Option<…>` so an older extension still parses the message.
  3. Add the arm to the `match` in `impl Connection` (`server.rs`) and the handler method beside
     it; the handler takes no socket, so `tests/` can drive it directly.
  4. Call it from `BeebeebFileProvider/IPCFraming.swift` (or the extension's caller). Swift mirrors
     the literal names; only the macOS job compiles it, so keep them identical by copy.
  5. Document it in `docs/IPC_PROTOCOL.md` and add a case to
     `src-tauri/src/ipc_socket/tests/payload_contracts.rs`.
- Check: `cargo check --manifest-path src-tauri/Cargo.toml --all-targets`

## Add a Finder menu item

A menu item is a row in `DESKTOP_MENU_SPECS`, an entry in `MACOS_MENU_LAYOUT` when macOS draws it,
and one arm in `handle_desktop_menu_action`.

- Files: `src-tauri/src/lib.rs` (1), `src-tauri/src/finder_open.rs` (2), `docs/CAPABILITIES.md` (3)
- Pattern: `DESKTOP_MENU_SPECS` in `src-tauri/src/lib.rs` (the spec rows and their `MENU_*_ID`s); `MACOS_MENU_LAYOUT` in `src-tauri/src/lib.rs` (which submenu draws the id); `handle_desktop_menu_action` in `src-tauri/src/lib.rs` (the action arm); `finder_open_outcome` in `src-tauri/src/finder_open.rs` (the Finder-side effect and its sentence)
- Steps:
  1. Add the `MENU_*_ID` const and a `DesktopMenuSpec` row: id, label, accelerator, action. The
     label is user-facing copy; the accelerator exists only where that platform has the key.
  2. Reuse a `DesktopMenuAction` (`OpenFolder`, `OpenExternal`, `OpenView`) when one fits; add a
     variant only for a new kind of effect.
  3. Put `Item(MENU_*_ID)` in `MACOS_MENU_LAYOUT` under its submenu (File for Finder verbs); an
     item that is also on Windows/Linux lives in `DESKTOP_MENU_SPECS`, which they keep unchanged.
  4. Add the arm in `handle_desktop_menu_action`. A macOS-only body carries
     `#[cfg(target_os = "macos")]` and a non-Mac arm: the function compiles on Linux CI too.
  5. Do the Finder work through `finder_open` (`beebeeb_open_scoped_url`, never `/usr/bin/open`) and
     pass a failure to `menu_open_failure_alert`, so the person sees it.
  6. Add the row to "Native menu and keyboard checklist" in `docs/CAPABILITIES.md`; a shortcut must
     not be a WebView2 key (`webview_browser_keys` test).
- Check: `cargo check --manifest-path src-tauri/Cargo.toml --all-targets`

## Add a desktop API call

A screen never calls `invoke`: one typed wrapper in `desktopApi.ts` per command, with its interface
beside it.

- Files: `src/desktopApi.ts` (1), `src/pages/Status.tsx` (2), `tests/statusFinderSetup.test.tsx` (3)
- Pattern: `command` in `src/desktopApi.ts` (the only invoke: revision guard, `CommandResult`); `loadSyncStatus` in `src/desktopApi.ts` (a wrapper: typed result, `ok` branch, no throw); `Status` in `src/pages/Status.tsx` (a consumer that reads the wrapper, not the socket)
- Steps:
  1. Write `export async function load<Thing>(): Promise<Thing | null>` beside its interface and call
     `command<Thing>('<snake_name>')`; the name is the Rust command, spelled once.
  2. Return `null` (or a typed refusal) on `!result.ok`; never throw into a component and never show
     `result.reason` raw — the copy lives in the page model.
  3. Add the name to `ANSWER_STANDS_WHEN_THE_ACCOUNT_CHANGES` only when an answer must survive a
     revision that moved while the call ran (a sign-in moves it).
  4. Read it in the page through its model or hook, so the 1 Hz `sync_status` and a one-off call do
     not each format their own copy.
  5. Cover it in the page's test (`tests/statusFinderSetup.test.tsx`): ok, refusal, revision moved.
- Check: `bunx tsc --noEmit`

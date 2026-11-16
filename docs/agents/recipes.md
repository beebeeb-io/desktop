# Task recipes — desktop

One recipe, in the order the change travels (engine → extension → docs). `MAP.md` is how to find anything else;
`recipes.ts` proves every file and item below still exists. The 0.8.x recipes for a Tauri command, a Tauri menu item and a desktop API call went with the
Tauri app and its React frontend in 0.9.0 (tag `desktop-v0.8.25` has them).

## Add an IPC message

The socket is a tagged JSON request and response shared with the File Provider extension; the tag is
a wire name.

- Files: `crates/beebeeb-engine/src/ipc_socket/protocol.rs` (1), `crates/beebeeb-engine/src/ipc_socket/server.rs` (2), `BeebeebFileProvider/IPCFraming.swift` (3), `docs/IPC_PROTOCOL.md` (4)
- Pattern: `IpcRequest` in `crates/beebeeb-engine/src/ipc_socket/protocol.rs` (the request variants and their serde tags); `IpcResponse` in `crates/beebeeb-engine/src/ipc_socket/protocol.rs` (the answers); `Connection` in `crates/beebeeb-engine/src/ipc_socket/server.rs` (the dispatch match in `impl Connection` and the handler beside it)
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
     `crates/beebeeb-engine/src/ipc_socket/tests/payload_contracts.rs`.
- Check: `cargo check --manifest-path crates/beebeeb-engine/Cargo.toml --all-targets --locked`

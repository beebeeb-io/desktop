# packages/shared: a generated mirror. Do not edit here.

The source of truth is `repos/web/packages/shared/src/` (`@beebeeb/shared`). This copy carries only the
subpaths desktop may import: `design/`, `drive-ui/`, `types/` (type-only, `drive-ui` imports it) and `tokens.css`. Desktop never imports `api/` or
`telemetry/` (the session token and keys stay in Rust; desktop v2 spec, "Data and crypto boundary").

Mirrored from web `v2/shared-drive-ui@92ebd2f9` (desktop v2 B01). From batch B1 on, the workspace
`make sync-shared` writes this directory (it gains a `DESKTOP_DST` with the same subpath filter).

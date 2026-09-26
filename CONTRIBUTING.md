# Contributing to Beebeeb Desktop

Thanks for your interest in the Beebeeb desktop app — the Tauri (Rust) shell, the tray/settings UI, and the
bridge to the shared sync engine and cryptography from [core](https://github.com/beebeeb-io/core).

## Prerequisites

- Rust stable (the toolchain is pinned in `rust-toolchain.toml`)
- [bun](https://bun.sh/)
- Tauri's per-OS prerequisites: <https://tauri.app/start/prerequisites/>. On Debian/Ubuntu that is
  `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf libfuse2 libfuse-dev` — the same
  list the release workflow installs.

## Development setup

```sh
git clone https://github.com/beebeeb-io/desktop.git
cd desktop
bun install
bun run tauri:dev      # Vite on :5173 plus the native window
```

The app talks to the production API unless you point it elsewhere. When you work against your own local
server, set `BB_API_BASE` (for example `BB_API_BASE=http://localhost:3001`) so you never sync test files into a
real account. `cargo check` in `src-tauri/` validates the Rust side without the frontend.

## Tests

Run both suites before you open a pull request. Report the **counts**, not just "tests pass" — a run that
reports no count is not a pass.

```sh
bun test                         # frontend: 53 pass, 0 fail across 11 files (2026-09-25)

cd src-tauri
cargo test --locked              # Rust: lib 351 passed, keychain 20 passed, 0 failed (2026-09-25)
```

The counts above were measured on `main` on 2026-09-25; your change may add tests, but should never lower them.

On macOS, run `cargo test` with a short `TMPDIR` (for example `TMPDIR=/tmp/bb cargo test --locked`): four IPC
tests bind a Unix socket inside `TMPDIR`, and a long temp path goes past the socket path limit
(`path must be shorter than SUN_LEN`) and fails those tests for a reason that has nothing to do with your change.
The tests create a `beebeeb` folder under your user config folder and write scratch files under your user
cache folder (`…/beebeeb/finder-writes/`). If you also use the app on the same machine, run them with `HOME`
(and on Linux `XDG_CONFIG_HOME` / `XDG_CACHE_HOME`) pointed at a scratch folder so they stay away from your real
install.

Also run:

```sh
bunx tsc --noEmit                # must exit 0
bun run lint                     # must exit 0
cd src-tauri && cargo clippy --locked --all-targets
```

`cargo clippy` currently reports existing warnings on `main`; please don't add new ones.

If you change the README's install or verification instructions, run `bun run check:docs`. It checks that
`README.md` still has the Install / Verify your download / Updating sections, and that the minisign key in it
actually verifies the signature of the current GitHub release (it needs network access to GitHub).

## Pull request process

1. Fork the repository and create a feature branch from `main`.
2. Make your change, with tests for any behaviour change.
3. Run the checks above and paste the counts into the pull request description.
4. Open a pull request that says what changed and why.

There is **no CI on pull requests yet**: the full test gate (`bun test` + `cargo test --locked`) runs in the
release workflow before anything is built. That is why the counts in your pull request description matter.
Every review conversation must be resolved before a pull request can merge, and every merge is done by a
person.

### Commit messages

Say what changed and why in the first line. If an AI coding tool wrote part of the change, keep its
`Co-Authored-By:` trailer — we disclose authorship, we don't strip it.

## Changes that need extra care

- **Sync engine and cryptography** live in [core](https://github.com/beebeeb-io/core), not here. Changes to
  primitives, key derivation, or encryption formats go through core's extended review — open an issue first.
- **Auto-update signing** (`plugins.updater.pubkey` in `src-tauri/tauri.conf.json`) and the release workflow:
  read [`docs/RELEASING.md`](docs/RELEASING.md) first. Changing the updater key wrongly strands every installed
  copy.
- **Conflict handling** must never silently drop a version.

## Contributor license

Beebeeb does not require a separate Contributor License Agreement at this time. By opening a pull request, you
confirm you have the right to submit the work and agree that it is licensed under AGPL-3.0-or-later.

## Security

If you find a security vulnerability, **do not open a public issue**. Email
[security@beebeeb.io](mailto:security@beebeeb.io) instead. See [SECURITY.md](SECURITY.md) for details.

## License

By contributing, you agree that your contributions will be licensed under the [AGPL-3.0-or-later](LICENSE).

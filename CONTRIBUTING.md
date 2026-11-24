# Contributing to Beebeeb Desktop

Thanks for your interest in the Beebeeb desktop app — the native Rust app (`native/beebeeb-drive-rune`, a Rune
UI with no web view), its engine (`crates/beebeeb-engine`), and the bridge to the shared sync engine and
cryptography from [core](https://github.com/beebeeb-io/core). The 0.8.x Tauri app was removed for 0.9.0.

## Prerequisites

- Rust stable for the engine (`rust-toolchain.toml`); the native app needs Rust 1.99.1 or later (Rune; see
  `native/beebeeb-drive-rune/Cargo.toml`)
- [bun](https://bun.sh/) for the release tooling tests in `scripts/tests/`

## Development setup

```sh
git clone https://github.com/beebeeb-io/desktop.git
cd desktop/native/beebeeb-drive-rune
cargo run              # the native app; a window on Windows
```

The app talks to the production API unless you point it elsewhere. When you work against your own local
server, pass `--api-base` to a development build (for example `cargo run -- --api-base http://localhost:3001`) so
you never sync test files into a real account; a shipped build of the real app ignores the flag.

## Tests

Run the suites your change touches before you open a pull request. Report the **counts**, not just "tests
pass" — a run that reports no count is not a pass.

```sh
cd crates/beebeeb-engine && cargo test --locked         # the engine: one `test result:` line per binary
cd native/beebeeb-drive-rune && cargo test --locked     # the native app
bun test scripts/tests                                  # the release tooling
```

On macOS, run `cargo test` with a short `TMPDIR` (for example `TMPDIR=/tmp/bb cargo test --locked`): the IPC
tests bind a Unix socket inside `TMPDIR`, and a long temp path goes past the socket path limit
(`path must be shorter than SUN_LEN`) and fails those tests for a reason that has nothing to do with your change.
The tests create a `beebeeb` folder under your user config folder and write scratch files under your user
cache folder (`…/beebeeb/finder-writes/`). If you also use the app on the same machine, run them with `HOME`
(and on Linux `XDG_CONFIG_HOME` / `XDG_CACHE_HOME`) pointed at a scratch folder so they stay away from your real
install.

Also run, in each crate you changed:

```sh
cargo clippy --locked --all-targets
```

Please don't add new clippy warnings.

If you change the README's install or verification instructions, run `node scripts/check-install-docs.mjs`. It
checks that `README.md` still has the Install / Verify your download / Updating sections, and that the minisign
key in it actually verifies the signature of the current GitHub release (it needs network access to GitHub).

## Pull request process

1. Fork the repository and create a feature branch from `main`.
2. Make your change, with tests for any behaviour change.
3. Run the checks above and paste the counts into the pull request description.
4. Open a pull request that says what changed and why.

Every pull request runs **Desktop CI** (`.github/workflows/ci.yml`): the engine's tests on Linux, Windows and
macOS, the native app's tests on Linux and its build on Windows, the updater and Office sandbox crates, and the
File Provider Swift job on macOS (a change to docs, design files or the native app only, its `Cargo.toml` aside,
skips the two macOS jobs). The release workflow runs its test gate again before anything is built. Paste your own counts into the pull request description as well: they show what you ran.
Every review conversation must be resolved before a pull request can merge, and every merge is done by a
person.

### Commit messages

Say what changed and why in the first line. If an AI coding tool wrote part of the change, keep its
`Co-Authored-By:` trailer — we disclose authorship, we don't strip it.

## Changes that need extra care

- **Sync engine and cryptography** live in [core](https://github.com/beebeeb-io/core), not here. Changes to
  primitives, key derivation, or encryption formats go through core's extended review — open an issue first.
- **Auto-update signing** (the public keys in `crates/beebeeb-updater/keys/`: `updater-v2.pub` for 0.9.0 and
  later, `updater-0.8.pub` for the 0.8.x line) and the release workflow:
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

# Beebeeb Desktop 0.5.0 — Shared with me

Desktop can now show you what other people have shared with you. If someone shares a file or folder with your account, it now appears under Shared in the app — not just on web.

### What's New

- **Shared with me is real now.** Previously this page always said "no shared roots available," even if you had active shares — desktop was never wired into the feature. It now shows the files and folders shared with you, who shared them, and whether you can edit them or only view them. Opening a shared file decrypts and reads its real content, the same as any file in your own vault.
- This is the read side only: browsing what's shared with you. Creating or revoking shares from desktop itself is a separate piece of work, not in this release.

### Verification

- `cargo test --locked` — 307 passed, 0 failed.
- The encryption/decryption logic went through two rounds of dedicated security review before merging, not just functional testing: the first round caught two real issues (a way a shared item's name could have been used to write a file to the wrong place, and a case where an unverified name could have been trusted instead of a properly decrypted one) — both were fixed and independently re-reviewed before this shipped. We'd rather say this plainly than pretend the first draft was already perfect.
- The decryption implementation is checked byte-for-byte compatible with how the web app already does this in production, including a test vector generated from the real web code, not just our own mirrored version of it.
- Not verified: real two-account sharing on physical hardware (this needs an actual second account sharing something with a real device, which we haven't run yet).

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.

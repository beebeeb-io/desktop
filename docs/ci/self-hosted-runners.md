# Self-hosted runners for the Rust jobs (prepared, not registered)

Status 2026-11-11: **nothing is registered.** Registering a runner needs organisation or repository admin
(the lead's token gets 403) and is Guus's action. This page is the procedure and the safe configuration.
Measurements behind the expected timings: `docs/ci/2026-11-11-rust-ci-speed.md`.

## Read this first: `beebeeb-io/desktop` is a PUBLIC repository

A self-hosted runner executes whatever a workflow run tells it to, as a real user on a real machine of ours.
On a public repository anyone can open a pull request, and a pull request's workflow runs **the pull request's
own copy of the workflow file**. So a stranger can open a pull request that changes `runs-on:` to our runner's
label and runs `curl … | sh` on our machine. This is the documented reason GitHub recommends against
self-hosted runners on public repositories. It is a real risk, not a theoretical one; the machines in question
(the testrig, the Legion, the Mac) hold source checkouts, signing material and a WireGuard path to production.

The `runs-on` expression in `ci.yml` keeps a fork's pull request on a GitHub-hosted runner. **That expression is
a convenience, not a control**: the fork can edit it. The controls are the five below. Do not register a runner
until all five are in place.

| # | Control | Where | Why |
|---|---|---|---|
| 1 | **Require approval for ALL outside collaborators** | Repository → Settings → Actions → General → "Fork pull request workflows from outside collaborators" → *Require approval for all outside collaborators* | No workflow of a fork's pull request starts, on any runner, until a maintainer has read the diff and pressed Approve. Read `.github/` in that diff before approving, every time. |
| 2 | **A dedicated, unprivileged OS user per runner** (`ghrunner`) | each machine | No sudo, not in `docker`/`wheel`/`admin`, no login shell for people. Its home holds the runner and its build directories and nothing else. It cannot read `~guus`, the dev checkouts, `~/.ssh`, `~/.tauri`, `~/.config/gh`, the Keychain of the login user, `/etc/wireguard`. Check: `sudo -u ghrunner cat /home/guus/.ssh/id_ed25519` must print "Permission denied". |
| 3 | **No route to production from the runner user** | each machine | WireGuard is up on these machines. On Linux, drop the runner user's traffic to the WireGuard interface (`iptables -A OUTPUT -o wg0 -m owner --uid-owner ghrunner -j REJECT`, persisted); on Windows a firewall rule for the runner's account; on macOS a `pf` rule by user. Check from the runner user: `ssh app01` and `curl` to a WireGuard address must fail. |
| 4 | **No secrets in the jobs that run there** | `ci.yml` | The three Rust jobs use no secret, `permissions: contents: read`, `persist-credentials: false`. Keep it so: a job that needs a secret (release, signing) never gets a self-hosted label. Runners are registered at REPOSITORY level for `desktop` only, never at organisation level (private repositories' jobs would land on a machine a public pull request can reach). |
| 5 | **Ephemeral runners** | each runner's service | Register with `--ephemeral`: the runner takes ONE job and exits; a small supervisor re-registers it with a fresh just-in-time token and clears `_work`. A job cannot leave a process, a modified toolchain or a poisoned work tree for the next job. The dependency build cache (`CARGO_TARGET_DIR`, `sccache`) is the one thing kept between jobs; see "Cache poisoning" below. |

**Cache poisoning.** A kept `CARGO_TARGET_DIR` is writable by every job that runs on the runner. A malicious job
could plant a compiled artefact that a later job links and runs. With control 1 only approved code runs, so the
residual risk is an approved-but-hostile pull request. Two rules contain it: these runners build **test
binaries only** (nothing built here is ever shipped; releases are built by `release.yml` on GitHub-hosted
runners and on the Mac by hand), and the supervisor deletes the cache after any job from a fork (`github.event.pull_request.head.repo.fork`).
If that residual risk is not acceptable, run fork pull requests on GitHub-hosted runners only (control 1 plus
never approving a fork run whose `runs-on` was edited) — which is what the `runs-on` expression already does by default.

What is NOT protected by any of this: the kernel and the machine itself. A job is arbitrary code as a local
user; a local privilege escalation breaks every control above. The strongest configuration is a runner inside a
throwaway VM or container (Linux: a rootless Podman container or a small KVM guest on the testrig; Windows: a
Hyper-V guest or Windows Sandbox on the Legion; macOS: a Tart/`Virtualization.framework` guest). Start with the
testrig (it holds the least), and treat a runner on the Legion's own Windows install or on the Mac's login
session as the weaker option that it is.

## How a job moves to a runner

`ci.yml` reads three repository variables (Settings → Secrets and variables → Actions → Variables):

| Variable | Value when set | Unset means |
|---|---|---|
| `RUST_LINUX_RUNNER` | `bb-rust-linux` | `ubuntu-latest` |
| `RUST_WINDOWS_RUNNER` | `bb-rust-windows` | `windows-latest` |
| `RUST_MACOS_RUNNER` | `bb-rust-macos` | `macos-latest` |

Set a variable: the next run of that job goes to the runner with that label. Delete it: the job is back on a
GitHub-hosted runner. No code change either way. A job queued for a label no online runner has waits (up to
24 h) and blocks the pull request, so delete the variable before taking a runner down.

`.github/actions/rust-suite` adapts by itself (`runner.environment`): on a self-hosted runner it does not
restore or save the GitHub dependency cache, keeps incremental compilation on, and (Linux) skips `apt-get`.

## Per-runner layout (all three)

```
~ghrunner/actions-runner/        the runner (ephemeral; `_work/` is cleared by the supervisor after every job)
~ghrunner/cache/target-desktop/  CARGO_TARGET_DIR: kept between jobs (this is the speed)
~ghrunner/cache/sccache/         only if sccache is used (see below)
~ghrunner/.cargo, ~ghrunner/.rustup   the toolchain (rustup; `rust-toolchain.toml` picks the channel)
```

The runner's `.env` file (next to `run.sh`; read at start) sets, for every job:

```
CARGO_TARGET_DIR=/home/ghrunner/cache/target-desktop
CARGO_INCREMENTAL=1
```

One `CARGO_TARGET_DIR` per runner and ONE job at a time per runner (an ephemeral runner does that by
construction). Do not point two runners at one target directory.

`sccache`: not needed while one crate builds on one runner with a kept target directory (cargo's own
fingerprints already skip everything unchanged; an incremental rebuild after a one-line change is 11.7 s on the
Legion). It earns its place when several crates or several runners share dependencies (the native UI crates):
then `RUSTC_WRAPPER=sccache`, `SCCACHE_DIR=~ghrunner/cache/sccache`, `SCCACHE_CACHE_SIZE=20G`, and
`CARGO_INCREMENTAL=0` (sccache cannot cache incremental units). Measure before keeping it.

**Cleanup policy, with a cap.** The supervisor runs this after every job; a kept target directory otherwise
grows without bound (the Legion caps its shared target directory at 70 GB for the same reason).

- `_work/` is deleted after every job (ephemeral).
- `CARGO_TARGET_DIR`: cap **30 GB**. Over the cap: delete the whole directory (the next job is one clean build,
  about a minute on the testrig or the Legion). No partial pruning: a half-pruned target directory fails in ways
  that look like compiler bugs.
- Weekly: delete it regardless (a toolchain update leaves a full dead copy behind).
- `sccache` (if used) caps itself (`SCCACHE_CACHE_SIZE`).
- Refuse to start a job with less than 20 GB free on the volume; the supervisor leaves the runner offline and
  says why.

## Linux runner on the testrig

1. `sudo useradd --create-home --shell /usr/sbin/nologin ghrunner` (no sudo, no extra groups).
2. Once, as root: the build dependencies the hosted job installs with `apt-get`
   (`libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf libfuse2 libfuse-dev`, plus
   `build-essential pkg-config python3 git curl`). The job itself has no sudo and does not need it.
3. As `ghrunner`: install rustup (`--default-toolchain stable --profile minimal -c rustfmt -c clippy`).
4. As `ghrunner`: download the runner (`actions-runner-linux-x64`), verify the published SHA-256.
5. Controls 2 and 3: the permission check and the WireGuard reject rule above. `/dev/shm` must be writable
   (the tests' temporary files go there).
6. Guus: Repository → Settings → Actions → Runners → New self-hosted runner gives a registration token (valid
   1 h). `./config.sh --url https://github.com/beebeeb-io/desktop --token <token> --name testrig-1 --labels bb-rust-linux --ephemeral --unattended --disableupdate=false`.
   Never paste the token into a chat, a task file or a commit; it goes from the browser to the terminal.
7. A systemd unit (`User=ghrunner`, `NoNewPrivileges=yes`, `ProtectHome=tmpfs` with `BindPaths=/home/ghrunner`,
   `PrivateTmp=yes`, `ProtectSystem=strict`, `ReadWritePaths=/home/ghrunner /dev/shm`) runs the supervisor loop:
   cleanup → fetch a just-in-time runner config (`POST /repos/beebeeb-io/desktop/actions/runners/generate-jitconfig`,
   with a fine-grained token that has ONLY "Administration: write" on this one repository, stored root-owned
   mode 0600 and read by the unit, not by the job) → `./run.sh --jitconfig …` → repeat.
8. Guus: set `RUST_LINUX_RUNNER=bb-rust-linux`. Verify: the next pull request's "Rust test (Linux)" job shows
   `Runner name: testrig-1`; its truth line and count equal the hosted run's.

## Windows runner on the Legion (native Windows, not WSL)

The Windows job tests Cloud Files and the Credential Manager, so it must run on Windows itself.

1. A local standard user `ghrunner` (not an administrator; "Deny log on through Remote Desktop").
2. Once, as an administrator: Visual Studio Build Tools (MSVC v143, Windows 11 SDK), Git for Windows (the job's
   steps use `bash`), Python 3 on PATH as `python`, WebView2 runtime (present on Windows 11).
3. As `ghrunner`: rustup with the `stable-x86_64-pc-windows-msvc` toolchain.
4. Runner in `C:\ghrunner\actions-runner`, cache in `C:\ghrunner\cache\target-desktop` (a short path: MSVC and
   cargo hit the 260-character limit otherwise). `.env`: `CARGO_TARGET_DIR=C:\ghrunner\cache\target-desktop`,
   `CARGO_INCREMENTAL=1`.
5. Controls 2 and 3: NTFS ACLs deny `ghrunner` the dev folders and the WSL virtual disk (`ext4.vhdx`); a
   Windows Firewall outbound block for that account on the WireGuard interface; the account is not in
   `docker-users` and cannot start `wsl.exe` (WSL holds every checkout and credential on this machine).
6. Add an exclusion for `C:\ghrunner` in Microsoft Defender's real-time scanning (it roughly doubles a Rust
   build), and for nothing else.
7. Registration as on Linux, label `bb-rust-windows`, ephemeral, started by a Scheduled Task that runs as
   `ghrunner` "whether the user is logged on or not" (not as a service under SYSTEM).
8. Guus: set `RUST_WINDOWS_RUNNER=bb-rust-windows`.

The Legion is Guus's daily machine and a development machine (LD-0081). A CI job there competes with dev
builds for the same 16 logical CPUs; the Windows job is the one that cannot go anywhere else. Prefer a Hyper-V
guest (4–8 vCPUs) so the runner cannot see the host at all.

## macOS runner on the Mac

1. A standard (non-admin) user `ghrunner`; never the login user (its Keychain holds the Developer ID identity
   and the updater key lives in `~/.tauri`).
2. Xcode command-line tools, rustup (`stable-aarch64-apple-darwin`), Python 3.
3. Runner in `/Users/ghrunner/actions-runner`, cache in `/Users/ghrunner/cache/target-desktop`.
4. Controls 2 and 3: `ghrunner` has no access to `/Users/<login>`; a `pf` rule blocks its traffic to the
   WireGuard interface; "Full Disk Access" is NOT granted to the runner.
5. A LaunchDaemon with `UserName=ghrunner` runs the supervisor loop (a LaunchAgent would need a logged-in
   session). Label `bb-rust-macos`, ephemeral.
6. Guus: set `RUST_MACOS_RUNNER=bb-rust-macos`.

The Rust job only. "File Provider Swift (macOS)" stays on GitHub-hosted runners: it needs nothing kept.

## Expected timings on warm runners

Measured on the Legion (WSL2, 16 logical CPUs) with this pull request's configuration; a self-hosted Linux
runner on comparable hardware does the same work. "Warm" = `CARGO_TARGET_DIR` kept from the previous job.

| Step | Warm self-hosted (measured on the Legion) | GitHub-hosted, this PR |
|---|---|---|
| Queue + set up job + checkout | ~5 s (no VM to boot) | see the PR's table |
| apt-get, cache restore | 0 s (skipped) | see the PR's table |
| Compile, PR touching `src-tauri/src` | 11.7 s incremental (one-line change); 49 s if a dependency changed | see the PR's table |
| Compile, clean (after a cleanup) | 58.5 s | – |
| Tests | 66–68 s: one test needs 65 s of real time (§7 of the speed doc); 22 s without that test | see the PR's table |
| **Total, typical PR** | **about 85 s; about 40 s if the 65 s test moves out of the per-PR suite** | |

Windows and macOS runners: not measured (no runner exists). The same reasoning gives "incremental compile +
the 65 s floor"; the first real job on each replaces this sentence with a number.

## What Guus must do

1. Decide that a self-hosted runner on a public repository is acceptable with the five controls (or: VM-only).
2. Set control 1 in the repository settings (it is useful today, without any runner).
3. Provide the machine user and the registration (steps 6–8 of each section); the lead can prepare everything
   up to the token.
4. Set the repository variable for each runner that is up; delete it before taking the runner down.

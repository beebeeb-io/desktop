# w27 account-switch probes (VM 401, Windows desktop)

Reusable UI probes for "account X signs out, account Y signs in on the same PC": what the switch removes, and what it
refuses (with words) because a file may hold bytes that were never sent. They drive the installed test build in the
VM's desktop session, not the engine API. Test stack on localhost only, never production.

Probe scripts (PowerShell, run in the VM desktop; each prints `Res ...` pass/fail lines into `C:\bb\rec\chapters.log`):
`pw27c.ps1` (X/Y pairs), `pw27d.ps1`, `pw27e.ps1` (unsent work keeps Y out), `pw27f.ps1` (a document made in the app, a
file put in the folder while signed out, a change queued with the server away).

Host scripts (bash): `run.sh <probe.ps1> <out dir>` runs one probe and copies the VM's logs out; `vmrun.sh` opens the tunnel
(VM 3051 -> test API 47851, VM 47736 -> code sink 47826) and the PIN pipe; `lockrun.sh <log> <cmd...>` wraps a command in
`with-lock.sh vm401` and waits while another builder holds it; `start-api.sh` / `stop-api.sh` / `watch.sh` start and stop the
test API (the probes ask for `READY-FOR-API-DOWN` / `READY-FOR-API-UP`).

Environment (nothing is hard-coded): `BB_WORKSPACE` (the beebeeb.io workspace), `W27_STACK` (dir holding `run/`, `blobs/` and
the `opaque` file), `W27_API_BIN` (a debug `beebeeb-api`), `W27_DATABASE_URL` (a throw-away Postgres), `W27_OUT` (where the PIN
pipe log goes). Every VM command goes through `scripts/coord/with-lock.sh vm401 -- ...`.

The VM must already hold `C:\bb\rec\` (`rec-lib*.ps1`, `clean-state.ps1`, `pin-pipe-rec.ps1`) and `C:\bb\w25\` (`head.ps1`,
`lib19.ps1`); `lib25.ps1` and `lib26.ps1` here are copied there by `vmrun.sh`. The installer under test is built by
`scripts/coord/remote-build.sh win desktop <branch>` with `packaging/windows/build-installer.ps1` (`-Mode test`) into
`C:\bb\w27as\installer`.

A UI probe cannot hit the millisecond window between the switch's check and its purge (nothing is logged before the
removal). That window is proven by the engine test `real_purge::a_file_dropped_in_the_folder_during_the_switch_survives_and_the_second_account_is_refused`
(`crates/beebeeb-engine/src/app/own_data_start_switch_tests.rs`, real Windows port, run on VM 401 with
`scripts/coord/remote-build.sh win desktop <branch> --dir crates/beebeeb-engine -- cargo test --locked --lib real_purge`).

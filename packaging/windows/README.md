# Windows packaging of the native app (0.9.0 and later)

Plain `makensis`, no Tauri CLI. Spec: `docs/superpowers/specs/2026-11-11-native-installer-updater.md`.

| File | What it is |
|---|---|
| `installer.nsi`, `utils.nsh`, `English.nsh` | The installer script Tauri generated for 0.8.x, kept line for line; the changes are listed at the top of `installer.nsi`. |
| `hooks.nsh` | Our hooks: remove the WebView2 profile after install, the staged updates on uninstall. |
| `build-installer.ps1` | Builds the installer; refuses a script that still handles WebView2. `-Mode test` ("Beebeeb Test", `io.beebeeb.app.test`, unsigned) or `-Mode release` (signed, fails closed). |
| `verify-install.ps1` | Read-only checks of an installed or uninstalled state; ends with a count line. |
| `rehearsal/serve.ts`, `rehearsal/db-fingerprint.ts` | Helpers for the upgrade rehearsal below. |

**Status.** Section A and native check 1 ran on Windows for the first time on 2026-11-12, on the build VM
`bb-build-win11` (Windows 11 23H2, beside an installed Beebeeb 0.8.25), and the scripts needed no change: steps 1 to
7 printed what they say below. Two parts are still open: that VM has no interactive desktop, so the app started
there but drew no window and step 5's "look at it" was not done; and 0.8.25 was not running, so step 8 had nothing to
compare. Native check 4 runs in CI; native checks 2 and 3 and section B have never run.

Needs: NSIS 3.11 and `nsis_tauri_utils.dll` 0.5.3. On a PC that has built 0.8.x both are in
`%LOCALAPPDATA%\tauri\NSIS`, where the script looks by default. Elsewhere: NSIS 3.11 from its official release, the
plugin from `github.com/tauri-apps/nsis-tauri-utils` (release `nsis_tauri_utils-v0.5.3`) in
`<NSIS folder>\Plugins\x86-unicode\additional\`, and `-NsisDir <NSIS folder>` on the command line (the build VM:
`-NsisDir 'C:\Program Files (x86)\NSIS'`). Every file is checked by SHA-256. A plugin that is not 0.5.3 is refused
in both modes. `makensis.exe`, `Bin\makensis.exe` and the stub `Stubs\lzma_solid-x86-unicode` must be NSIS 3.11 as
released: a release build refuses any other file, a test build prints a `NOTE` line and goes on. The output folder
`out\` is ignored by git.

**A test build shares nothing with the real Beebeeb (review P2-4).** It is `Beebeeb Test`, `io.beebeeb.app.test`,
`beebeeb-desktop-test.exe`: its own install folder, data folder (`%LOCALAPPDATA%\io.beebeeb.app.test`), config
(`%APPDATA%\beebeeb-test\desktop.toml`) and credentials (`io.beebeeb.app.test/…`). It has no server of its own:
without a flag its API base is the engine's built-in default, production (`https://api.beebeeb.io`; the closed
loopback port is a unit-test build's only). It takes `--api-base <url>` in any profile (decision 2462; so does a dev
build of either app, and the real app as it ships never does), and its data on a computer stays with the server it
was first opened for (`native/beebeeb-drive-rune/src/shell/README.md`, "Starting").
The installer looks only at processes of its own binary name and **never ends a process**: it asks a
running native app to quit (`--quit-for-update`), waits 20 seconds, then says "still running" and offers Retry or
Cancel. Which app a binary is, is decided when it is built (`native\beebeeb-drive-rune\build.rs`):
`BEEBEEB_APP_IDENTIFIER=io.beebeeb.app.test`, or the variable unset, gives the test app; `io.beebeeb.app` gives the
real one; any other value, and a release (`BEEBEEB_RELEASE_VERSION` set) that is not `io.beebeeb.app`, stops the
build. The profile (`--release`) decides nothing. `build-installer.ps1 -Mode test` refuses a binary that does not
carry the test app's mark (`beebeeb-app-identity=io.beebeeb.app.test;`), because any other binary would open the
real account's data; `-Mode release` refuses one without the real app's mark.

**The installer refuses to run over an install older than 0.8.25** (review P2-2): a message says to update through
the app first, nothing is changed, exit code 3; when an old updater started it (`/UPDATE`), the old app is started
again.

## A. Test installer of the native app: build, install, verify, uninstall, verify

PowerShell, from a clone of this branch (for example `C:\bb-win\native\desktop`). The real Beebeeb can stay
running: nothing below looks at it. Check that afterwards (step 8).

```powershell
# 1. Build the native binary (its own toolchain: native\beebeeb-drive-rune\rust-toolchain.toml).
$env:CARGO_TARGET_DIR = 'C:\bb-win\native\target'
Set-Location C:\bb-win\native\desktop\native\beebeeb-drive-rune
$env:BEEBEEB_APP_IDENTIFIER = 'io.beebeeb.app.test'     # the test identity is compiled in (also the default); the packager checks the binary's mark
cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
Remove-Item Env:BEEBEEB_APP_IDENTIFIER
$real = @(Get-Process beebeeb-desktop -ErrorAction SilentlyContinue | ForEach-Object Id)   # for step 8

# 2. Build the test installer. Prints INSTALLER, SIZE, SHA256.
Set-Location C:\bb-win\native\desktop\packaging\windows
.\build-installer.ps1 -Mode test -Version 0.9.0 -Exe C:\bb-win\native\target\release\beebeeb-drive.exe
$installer = Join-Path $PWD 'out\Beebeeb Test_0.9.0_x64-setup.exe'

# 3. Install silently (per user, no prompt) and wait for it.
#    Exit 3 = refused: an existing install is older than 0.8.25 (update through the app first).
$proc = Start-Process -FilePath $installer -ArgumentList '/S' -Wait -PassThru
if ($proc.ExitCode -ne 0) { throw "Installer exited $($proc.ExitCode)" }
# 4. Verify: must end with "VERIFY installed: 14 passed, 0 failed".
.\verify-install.ps1 -Mode test -Version 0.9.0 -Phase installed

# 5. Start it once, look at it, quit it. Then: no WebView2 process may belong to it, and no profile may appear.
#    Do not sign in: its server is production (see above). A signed-out start makes no request; the firewall lines
#    (elevated shell only) make sure of it for this step, as on the build VM. Without an interactive desktop (an SSH
#    session, the build VM) it starts and stays up, but Rune cannot create its windows ("the window's GPU surface
#    refused its configuration").
$exe = "$env:LOCALAPPDATA\Beebeeb Test\beebeeb-desktop-test.exe"
New-NetFirewallRule -DisplayName 'Beebeeb Test: no network' -Direction Outbound -Action Block -Program $exe | Out-Null
$app = Start-Process $exe -PassThru
Start-Sleep 10
@(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Where-Object { $_.CommandLine -like '*io.beebeeb.app.test*' }).Count   # must print 0
Test-Path "$env:LOCALAPPDATA\io.beebeeb.app.test\EBWebView"                                                                            # must print False
if (!$app.HasExited) { Stop-Process -Id $app.Id }        # quit it by its own process id
Remove-NetFirewallRule -DisplayName 'Beebeeb Test: no network'

# 6. Uninstall silently (the uninstaller copies itself to %TEMP% and returns at once: wait for the folder to go).
Start-Process -FilePath "$env:LOCALAPPDATA\Beebeeb Test\uninstall.exe" -ArgumentList '/S' -Wait
1..30 | ForEach-Object { if (Test-Path "$env:LOCALAPPDATA\Beebeeb Test") { Start-Sleep 1 } }
# 7. Verify: must end with "VERIFY uninstalled: 6 passed, 0 failed".
.\verify-install.ps1 -Mode test -Version 0.9.0 -Phase uninstalled

# 8. The real Beebeeb was never closed: the same process ids as before step 1. Must print True. Without a real
#    Beebeeb running at step 1 there is nothing to compare, and it says so instead of a True that proves nothing.
$now = "$(@(Get-Process beebeeb-desktop -ErrorAction SilentlyContinue | ForEach-Object Id))"
if ($real.Count -eq 0) { 'NOT CHECKED: no real Beebeeb was running at step 1' } else { $now -eq "$real" }
```

**Native checks added by the security review's delta (the lead's; 1 ran on the build VM, 4 runs in CI, 2 and 3
have not run):**

1. Step 2 with a binary built with `BEEBEEB_APP_IDENTIFIER=io.beebeeb.app` must print `FAILED the binary was not
   built as the test app` and exit 1 (the guard can fail). A build with `BEEBEEB_APP_IDENTIFIER=banana` must not
   compile at all (`is not an app`). Both seen on 2026-11-12 (cargo exit 101 for `banana`).
2. With the test app running, run the installer without `/S`: after about 20 seconds it must show "Beebeeb Test is
   still running" with Retry and Cancel, and `Get-Process beebeeb-desktop-test` must still list the same id.
3. (Also with `/S`, and also over an MSI install: the scan runs in `.onInit` since Delta 3.) The version refusal: `reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Beebeeb Test" /v
   DisplayVersion /d 0.8.24 /f`, run the installer: the message, exit code 3, no file changed. Then with `/d
   0.8.25`: it installs. Then with `/d banana`: refused (a version that does not parse is not "at least 0.8.25").
4. `cargo test --locked --features net` in `crates\beebeeb-updater` on Windows: the two `#[cfg(windows)]` tests
   (`a_held_installer_cannot_be_written_replaced_renamed_or_deleted`,
   `the_staged_file_cannot_be_replaced_between_the_check_and_the_start`) run only there. CI runs them (`ci.yml`,
   job `updater-windows`, hosted `windows-latest`, each test named in the gate): on `v2-native` on 2026-11-12 both
   printed `ok` (`test result: ok. 78 passed; 0 failed`). Not yet run on the lead's PC or the build VM.

Prove the checker can fail before trusting it: run step 4 with `-Version 9.9.9` (must print `FAIL  uninstall entry:
DisplayVersion` and exit 1), and step 7 while the app is still installed (must fail 5 of 6).

What a silent uninstall leaves, on purpose. Measured after a signed-out start: the settings file
`%APPDATA%\beebeeb-test\desktop.toml` and the install-location key `HKCU\Software\beebeeb\Beebeeb Test` (the 0.8.25
uninstaller leaves `HKCU\Software\beebeeb\Beebeeb` the same way); that start made no data folder. By the code, after
a signed-in session, also: the state folder `%LOCALAPPDATA%\io.beebeeb.app.test`, the scratch and cache folders
`%TEMP%\beebeeb-test` and `%LOCALAPPDATA%\beebeeb-test`, the rest of `%APPDATA%\beebeeb-test` (for example
`Changes not sent`, where a sign-out keeps unsent changes), the `io.beebeeb.app.test/…` credentials and the
Explorer sync root (spec §5). "Delete the application data" removes only `%APPDATA%\io.beebeeb.app.test` and
`%LOCALAPPDATA%\io.beebeeb.app.test`. After it, a test build that was signed in does not start again (one line on
stderr and the start-refusal box, exit 2): its stored credentials are left with no local data to say which server
they were made for, so no server gets them. It starts once those `io.beebeeb.app.test/…` credentials are removed too.

No step changed anything of the real Beebeeb (`%LOCALAPPDATA%\Beebeeb`, `%LOCALAPPDATA%\io.beebeeb.app`,
`%APPDATA%\beebeeb`, `%APPDATA%\io.beebeeb.app`, its registry keys, shortcuts and `io.beebeeb.app/…` credentials),
and none looks at its processes: the installer waits only for `beebeeb-desktop-test.exe`. Every name the test build
writes is its own. What the two share is the parent key `HKCU\Software\beebeeb`, which holds one subkey each (the
uninstaller removes the parent only when it is empty, and only with "Delete the application data" ticked), and the
uninstall entries' publisher, `beebeeb`.

## B. Upgrade rehearsal: a Tauri build updates itself to the native build

> **2026-11-11, after the security review: do not run this section on a PC with a real Beebeeb account, and not
> as written.** A test build no longer shares the binary name, the data folder or the credentials with anything
> (see above), so an upgrade from a Tauri "Beebeeb Test" to a native "Beebeeb Test" lands on another binary name
> and other credential names: it can no longer show that the autostart entry, the Explorer integration and the
> sign-in survive. That proof needs the RELEASE identity on both sides, in a disposable Windows VM
> (`docs/runbook/build-vms.md`) with a test account and no real install, with the rehearsal's own throwaway key
> pair baked into both builds. The steps below are kept as the outline; the release-identity variant (an unsigned
> `-Mode rehearsal` that refuses to run outside a VM) is **not built** and is an open rung of this PR.

What it proves: the updater inside a real Tauri build accepts a v2-shaped manifest (spec R1), runs our installer
with its own flags, and ends on the native binary in the same folder with the same data (R2-R5). It uses the test
identity, a **throwaway key generated on the spot** (the real updater key is never involved) and a manifest served
from `127.0.0.1`. Two things differ from production and are the only patches: the manifest base URL and the key.

```powershell
$work = 'C:\bb-win\rehearsal'; New-Item -ItemType Directory -Force $work, "$work\site" | Out-Null

# 1. A throwaway updater key. The password is typed at the prompt; nothing goes on a command line.
Set-Location C:\bb-win\testbuild\desktop          # the Tauri clone tools/agent-team/testbuild.ps1 uses
git fetch -q origin; git checkout -q -f --detach origin/main; bun install --frozen-lockfile
bun run tauri signer generate -w "$work\rehearsal.key"
$pub = (Get-Content "$work\rehearsal.key.pub" -Raw).Trim()

# 2. Point this build's updater at the local server. Exactly one line may change; stop if it is not one.
$file = 'src-tauri\src\activity_conflict_commands.rs'
$old = 'pub(super) const UPDATE_MANIFEST_BASE_URL: &str = "https://releases.beebeeb.io";'
$text = Get-Content $file -Raw
if (([regex]::Matches($text, [regex]::Escape($old))).Count -ne 1) { throw 'the base URL line was not found exactly once' }
Set-Content $file ($text.Replace($old, 'pub(super) const UPDATE_MANIFEST_BASE_URL: &str = "http://127.0.0.1:8787";')) -NoNewline -Encoding utf8

# 3. Build and install the Tauri side as "Beebeeb Test" 0.8.25, with the updater ON and the throwaway key.
$conf = @{ productName = 'Beebeeb Test'; identifier = 'io.beebeeb.app.test'; version = '0.8.25'
           bundle = @{ createUpdaterArtifacts = $false }
           plugins = @{ updater = @{ pubkey = $pub; dangerousInsecureTransportProtocol = $true } } } | ConvertTo-Json -Depth 5
Set-Content "$work\rehearsal.conf.json" $conf -Encoding ascii
$env:BEEBEEB_RELEASE_VERSION = '0.8.25'; $env:CARGO_TARGET_DIR = 'C:\bb-win\testbuild\target'
bun run tauri build --config "$work\rehearsal.conf.json" --bundles nsis
$tauriSetup = Get-ChildItem 'C:\bb-win\testbuild\target\release\bundle\nsis\Beebeeb Test_0.8.25_x64-setup.exe'
Start-Process -FilePath $tauriSetup.FullName -ArgumentList '/S' -Wait
git checkout -q -f -- $file                       # the patch never leaves this clone

# 4. Run it, sign in with a TEST account, choose a sync folder, let a few files sync, turn "Start at login" on.
#    Leave it RUNNING. In Explorer, open one online-only file's Properties dialog and leave it open (spec R4).
Start-Process "$env:LOCALAPPDATA\Beebeeb Test\beebeeb-desktop.exe"

# 5. Record the "before" facts.
$data = "$env:LOCALAPPDATA\io.beebeeb.app.test"
bun C:\bb-win\native\desktop\packaging\windows\rehearsal\db-fingerprint.ts "$data\state.db" | Select-Object -Last 1 | Set-Content "$work\db-before.txt"
(Get-FileHash "$data\.device.json" -Algorithm SHA256).Hash | Set-Content "$work\device-before.txt"
(cmdkey /list | Select-String 'io\.beebeeb\.app/').Count | Set-Content "$work\creds-before.txt"     # names are counted, never values
(Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run').'Beebeeb Test' | Set-Content "$work\run-before.txt"
@(Get-ChildItem 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager' | Where-Object PSChildName -like 'Beebeeb!*').Count | Set-Content "$work\roots-before.txt"

# 6. Publish the native test installer (built in A.2) as 0.9.0 in the v2 shape, signed with the throwaway key.
Copy-Item 'C:\bb-win\native\desktop\packaging\windows\out\Beebeeb Test_0.9.0_x64-setup.exe' "$work\site\Beebeeb_0.9.0_x64-setup.exe"
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content "$work\rehearsal.key" -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = Read-Host 'throwaway key password'
bun run tauri signer sign "$work\site\Beebeeb_0.9.0_x64-setup.exe" | Out-Null
$sig = (Get-Content "$work\site\Beebeeb_0.9.0_x64-setup.exe.sig" -Raw).Trim()
$platform = @{ url = 'http://127.0.0.1:8787/Beebeeb_0.9.0_x64-setup.exe'; signature = $sig; size = (Get-Item "$work\site\Beebeeb_0.9.0_x64-setup.exe").Length }
$manifest = @{ version = '0.9.0'; notes = 'Rehearsal.'; pub_date = '2026-11-11T12:00:00Z'; product = 'desktop'; channel = 'stable'
               platforms = @{ 'windows-x86_64-nsis' = $platform; 'windows-x86_64' = $platform } } | ConvertTo-Json -Depth 5
New-Item -ItemType Directory -Force "$work\site\desktop" | Out-Null
[IO.File]::WriteAllText("$work\site\desktop\latest.json", $manifest)      # the address a build from main asks today
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY, Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
Start-Process bun -ArgumentList 'C:\bb-win\native\desktop\packaging\windows\rehearsal\serve.ts', "$work\site", '8787'

# 7. In the running app: Settings > About > "Check for updates", then "Restart to update".
#    Expected: the app closes, a small "Installing" progress window shows, the NATIVE app opens. No UAC prompt.
#    The server window must show 200 for desktop/latest.json and for the installer.

# 8-10. Assert. Every line prints True or a count line.
Set-Location C:\bb-win\native\desktop\packaging\windows
.\verify-install.ps1 -Mode test -Version 0.9.0 -Phase installed                       # VERIFY installed: 14 passed, 0 failed
@(Get-Process beebeeb-desktop -ErrorAction SilentlyContinue).Count -eq 1              # one process, the native one
(Get-Item "$env:LOCALAPPDATA\Beebeeb Test\beebeeb-desktop.exe").Length -eq (Get-Item C:\bb-win\native\target\release\beebeeb-drive.exe).Length
(bun rehearsal\db-fingerprint.ts "$data\state.db" | Select-Object -Last 1) -eq (Get-Content "$work\db-before.txt")
(Get-FileHash "$data\.device.json" -Algorithm SHA256).Hash -eq (Get-Content "$work\device-before.txt")
"$((cmdkey /list | Select-String 'io\.beebeeb\.app/').Count)" -eq (Get-Content "$work\creds-before.txt")
(Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run').'Beebeeb Test' -eq (Get-Content "$work\run-before.txt")
Test-Path ((Get-Content "$work\run-before.txt").Trim().Trim('"'))                     # the autostart entry still points at a file
"$(@(Get-ChildItem 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager' | Where-Object PSChildName -like 'Beebeeb!*').Count)" -eq (Get-Content "$work\roots-before.txt")
```

Reading the result:

- The `state.db` fingerprint compares what the database holds. It is equal only while the native build does not
  write to the database (true for today's UI build, which does not open it). Once the native engine is wired in,
  an unequal fingerprint is expected and the assertion becomes: `integrity=ok`, the same tables, and the account
  owner row unchanged (engine lane).
- The Properties dialog left open in step 4 must still be open and the file still listed in Explorer.
- "Credentials still readable" is checked here by name and count only. That the native app can *use* them (it
  opens signed in) is the session lane's rung once sign-in is wired.

Variants, each a full run from step 3 (uninstall "Beebeeb Test" first, keep `$data`):

- **B2, the app is not running** when the installer runs (close it after step 6, run
  `& "$work\site\Beebeeb_0.9.0_x64-setup.exe" /P /R /UPDATE` by hand): same assertions.
- **B3, from an MSI install:** in step 3 build with `--bundles msi`, install the `.msi`; expected: one UAC prompt
  (the MSI is removed), then a per-user install; `verify-install.ps1` must still report exactly one uninstall
  entry.
- **B4, the bridge** (after the `main` lane has built 0.8.25 from
  `docs/superpowers/plans/2026-11-11-bridge-0.8.25.md`): write the manifest to `desktop\v2\stable.json`, sign it
  (`bun run tauri signer sign`), keep a `desktop\latest.json` that still says 0.8.25; expected: the server shows
  the v2 manifest asked first, and `update-handoff.json` appears in `$data`.

Afterwards: uninstall "Beebeeb Test" (A.6-7), delete `C:\bb-win\rehearsal` (it holds the throwaway key), and remove
the test account's sync folder registration by signing out before uninstalling.

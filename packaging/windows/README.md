# Windows packaging of the native app (0.9.0 and later)

Plain `makensis`, no Tauri CLI. Spec: `docs/superpowers/specs/2026-11-11-native-installer-updater.md`.

| File | What it is |
|---|---|
| `installer.nsi`, `utils.nsh`, `English.nsh` | The installer script Tauri generated for 0.8.x, kept line for line; the changes are listed at the top of `installer.nsi`. |
| `hooks.nsh` | Our hooks: remove the WebView2 profile after install, the staged updates on uninstall. |
| `build-installer.ps1` | Builds the installer; refuses a script that still handles WebView2. `-Mode test` ("Beebeeb Test", `io.beebeeb.app.test`, unsigned) or `-Mode release` (signed, fails closed). |
| `verify-install.ps1` | Read-only checks of an installed or uninstalled state; ends with a count line. |
| `rehearsal/serve.ts`, `rehearsal/db-fingerprint.ts` | Helpers for the upgrade rehearsal below. |

**Status: written on Linux, never run on Windows.** A lane cannot run Windows binaries; every command below is for
the lead, on the Windows host, under the native-windows lock. Expect to fix small things on the first run and
record them in the PR.

Needs: NSIS 3.11 and `nsis_tauri_utils.dll` 0.5.3. Both are in `%LOCALAPPDATA%\tauri\NSIS` on a PC that has built
0.8.x (the lead's has: `C:\Users\GuusLangelaar\AppData\Local\tauri\NSIS`). The plugin, `Bin\makensis.exe` and the
stub `Stubs\lzma_solid-x86-unicode` are checked by SHA-256 (NSIS 3.11 as released): a release build refuses any
other file, a test build prints a `NOTE` line and goes on.

**A test build shares nothing with the real Beebeeb (review P2-4).** It is `Beebeeb Test`, `io.beebeeb.app.test`,
`beebeeb-desktop-test.exe`: its own install folder, data folder (`%LOCALAPPDATA%\io.beebeeb.app.test`), config and
credentials. The installer looks only at processes of its own binary name and **never ends a process**: it asks a
running native app to quit (`--quit-for-update`), waits 20 seconds, then says "still running" and offers Retry or
Cancel. `build-installer.ps1 -Mode test` refuses a binary that was not built with
`BEEBEEB_APP_IDENTIFIER=io.beebeeb.app.test`, because such a binary would open the real account's data.

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
$env:BEEBEEB_APP_IDENTIFIER = 'io.beebeeb.app.test'     # the test identity is compiled in; the packager checks it
cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
Remove-Item Env:BEEBEEB_APP_IDENTIFIER
$real = @(Get-Process beebeeb-desktop -ErrorAction SilentlyContinue | ForEach-Object Id)   # for step 8

# 2. Build the test installer. Prints INSTALLER, SIZE, SHA256.
Set-Location C:\bb-win\native\desktop\packaging\windows
.\build-installer.ps1 -Mode test -Version 0.9.0 -Exe C:\bb-win\native\target\release\beebeeb-drive.exe
$installer = Join-Path $PWD 'out\Beebeeb Test_0.9.0_x64-setup.exe'

# 3. Install silently (per user, no prompt) and wait for it.
Start-Process -FilePath $installer -ArgumentList '/S' -Wait
# 4. Verify: must end with "VERIFY installed: 14 passed, 0 failed".
.\verify-install.ps1 -Mode test -Version 0.9.0 -Phase installed

# 5. Start it once, look at it, quit it. Then: no WebView2 process may belong to it, and no profile may appear.
Start-Process "$env:LOCALAPPDATA\Beebeeb Test\beebeeb-desktop-test.exe"
Start-Sleep 10
@(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Where-Object { $_.CommandLine -like '*io.beebeeb.app.test*' }).Count   # must print 0
Test-Path "$env:LOCALAPPDATA\io.beebeeb.app.test\EBWebView"                                                                            # must print False

# 6. Uninstall silently (the uninstaller copies itself to %TEMP% and returns at once: wait for the folder to go).
Start-Process -FilePath "$env:LOCALAPPDATA\Beebeeb Test\uninstall.exe" -ArgumentList '/S' -Wait
1..30 | ForEach-Object { if (Test-Path "$env:LOCALAPPDATA\Beebeeb Test") { Start-Sleep 1 } }
# 7. Verify: must end with "VERIFY uninstalled: 6 passed, 0 failed".
.\verify-install.ps1 -Mode test -Version 0.9.0 -Phase uninstalled

# 8. The real Beebeeb was never touched: the same process ids as before step 1. Must print True.
"$(@(Get-Process beebeeb-desktop -ErrorAction SilentlyContinue | ForEach-Object Id))" -eq "$real"
```

**Native checks added by the security review's delta (never run; the lead's):**

1. Step 2 with a binary built WITHOUT `BEEBEEB_APP_IDENTIFIER` must print `FAILED the binary was not built as the
   test app` and exit 1 (the guard can fail). Until the shell lane makes the native binary honour
   `BEEBEEB_APP_IDENTIFIER`, every test build is refused: that is the intended state, not a bug in the script.
2. With the test app running, run the installer without `/S`: after about 20 seconds it must show "Beebeeb Test is
   still running" with Retry and Cancel, and `Get-Process beebeeb-desktop-test` must still list the same id.
3. (Also with `/S`, and also over an MSI install: the scan runs in `.onInit` since Delta 3.) The version refusal: `reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Beebeeb Test" /v
   DisplayVersion /d 0.8.24 /f`, run the installer: the message, exit code 3, no file changed. Then with `/d
   0.8.25`: it installs. Then with `/d banana`: refused (a version that does not parse is not "at least 0.8.25").
4. `cargo test --locked --features net` in `crates\beebeeb-updater` on Windows: the two `#[cfg(windows)]` tests
   (`a_held_installer_cannot_be_written_replaced_renamed_or_deleted`,
   `the_staged_file_cannot_be_replaced_between_the_check_and_the_start`) run only there. CI runs them too
   (`ci.yml`, job `updater-windows`, hosted `windows-latest`, each test named in the gate); the first run of that
   job on this PR is the first time they execute anywhere.

Prove the checker can fail before trusting it: run step 4 with `-Version 9.9.9` (must print `FAIL  uninstall entry:
DisplayVersion` and exit 1), and step 7 while the app is still installed (must fail 5 of 6).

The silent uninstall keeps the data folder (`%LOCALAPPDATA%\io.beebeeb.app.test`) on purpose. The real Beebeeb's
files and registry entries (`%LOCALAPPDATA%\Beebeeb`, `io.beebeeb.app`) are not touched by any step, but a running
real Beebeeb is closed (see above), and the two builds share the credentials
(`io.beebeeb.app/...` in Credential Manager) and `%APPDATA%\beebeeb\desktop.toml`, as the Tauri test build does
today (spec §3).

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

<#
.SYNOPSIS
  Builds the Windows installer of the native Beebeeb app with plain makensis. No Tauri CLI, no bundler.

.DESCRIPTION
  Input: the built native binary (native/beebeeb-drive-rune -> beebeeb-drive.exe). Output: one NSIS installer.

  -Mode test     "Beebeeb Test", identifier io.beebeeb.app.test, unsigned. Installs beside the real Beebeeb
                 (%LOCALAPPDATA%\Beebeeb Test), as tools/agent-team/testbuild.ps1 does for the Tauri build.
  -Mode release  "Beebeeb", identifier io.beebeeb.app. The app binary, the uninstaller and the installer are
                 Authenticode-signed through scripts/windows/sign-artifact.ps1 (Azure Artifact Signing, ambient az
                 login); refuses to run unless BEEBEEB_REQUIRE_SIGNING=1, so a release installer is never unsigned.

  The binary is installed as beebeeb-desktop.exe, the name 0.8.x installed: the autostart entry, the shortcuts, the
  Explorer integration and 0.8.x's own updater all point at that file name (spec section 3).

  The script needs NSIS 3 (makensis.exe) and nsis_tauri_utils.dll 0.5.3 (the NSIS helper plugin the 0.8.x installer
  already carries: process lookup, semver compare, start-as-user). Both are in the Tauri toolchain cache of a PC that
  has built 0.8.x (%LOCALAPPDATA%\tauri\NSIS); the plugin is checked by SHA-256.

.EXAMPLE
  .\build-installer.ps1 -Exe C:\bb-win\native\target\release\beebeeb-drive.exe -Version 0.9.0 -Mode test
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [Parameter(Mandatory = $true)][string]$Version,
  [ValidateSet('test', 'release')][string]$Mode = 'test',
  [string]$OutDir = (Join-Path $PSScriptRoot 'out'),
  [string]$NsisDir = (Join-Path $env:LOCALAPPDATA 'tauri\NSIS'),
  [string]$PluginDir = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# nsis_tauri_utils.dll 0.5.3, as released at github.com/tauri-apps/nsis-tauri-utils (MIT OR Apache-2.0).
$PluginSha256 = '5ba143b5db4a87d32d6e7802e033330aae56cbceabe0d1e3ba41948385ad4709'
$MainBinaryName = 'beebeeb-desktop'

function Fail([string]$Message) { Write-Host "FAILED $Message"; exit 1 }

if ($Version -notmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$') { Fail "version '$Version' must be plain semver like 0.9.0" }
if (!(Test-Path -LiteralPath $Exe -PathType Leaf)) { Fail "no binary at $Exe" }
$makensis = Join-Path $NsisDir 'makensis.exe'
if (!(Test-Path -LiteralPath $makensis)) { Fail "no makensis.exe in $NsisDir (pass -NsisDir)" }
if (!$PluginDir) { $PluginDir = Join-Path $NsisDir 'Plugins\x86-unicode\additional' }
$plugin = Join-Path $PluginDir 'nsis_tauri_utils.dll'
if (!(Test-Path -LiteralPath $plugin)) { Fail "no nsis_tauri_utils.dll in $PluginDir (pass -PluginDir)" }
$pluginHash = (Get-FileHash -LiteralPath $plugin -Algorithm SHA256).Hash.ToLowerInvariant()
if ($pluginHash -ne $PluginSha256) { Fail "nsis_tauri_utils.dll is not 0.5.3 (sha256 $pluginHash)" }

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$icon = Join-Path $repo 'src-tauri\icons\icon.ico'
if (!(Test-Path -LiteralPath $icon)) { Fail "no icon at $icon" }
$signScript = Join-Path $repo 'scripts\windows\sign-artifact.ps1'

if ($Mode -eq 'release') { $product = 'Beebeeb'; $bundleId = 'io.beebeeb.app' }
else { $product = 'Beebeeb Test'; $bundleId = 'io.beebeeb.app.test' }

$stage = Join-Path $OutDir "stage-$Mode"
if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
New-Item -ItemType Directory -Force $stage | Out-Null
$staged = Join-Path $stage "$MainBinaryName.exe"
Copy-Item -LiteralPath $Exe -Destination $staged
$outFile = Join-Path $OutDir ("{0}_{1}_x64-setup.exe" -f $product, $Version)
if (Test-Path -LiteralPath $outFile) { Remove-Item -LiteralPath $outFile -Force }

$uninstallerSign = ''
if ($Mode -eq 'release') {
  if ($env:BEEBEEB_REQUIRE_SIGNING -ne '1') { Fail 'release mode needs BEEBEEB_REQUIRE_SIGNING=1 (a release installer is never unsigned)' }
  # The binary must say what it is: the version it reports is what Explorer and the uninstall entry show beside it.
  $info = (Get-Item -LiteralPath $staged).VersionInfo
  if ($info.ProductName -ne 'Beebeeb' -or "$($info.ProductVersion)" -notlike "$Version*") {
    Fail "the binary's version resource says '$($info.ProductName)' '$($info.ProductVersion)', expected 'Beebeeb' '$Version' (the native crate must embed icon and version)"
  }
  & powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $signScript $staged
  if ($LASTEXITCODE -ne 0) { Fail "signing the app binary (exit $LASTEXITCODE)" }
  $uninstallerSign = "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$signScript`" %1"
}

$sizeKiB = [int][Math]::Ceiling((Get-Item -LiteralPath $staged).Length / 1024)
$defines = @(
  "/DPRODUCTNAME=$product",
  "/DBUNDLEID=$bundleId",
  "/DVERSION=$Version",
  "/DVERSIONWITHBUILD=$Version.0",
  "/DMAINBINARYSRCPATH=$staged",
  "/DINSTALLERICON=$icon",
  "/DOUTFILE=$outFile",
  "/DADDITIONALPLUGINSPATH=$PluginDir",
  "/DESTIMATEDSIZE=$sizeKiB"
)
if ($uninstallerSign) { $defines += "/DUNINSTALLERSIGNCOMMAND=$uninstallerSign" }

Push-Location $PSScriptRoot
try {
  & $makensis /V3 /INPUTCHARSET UTF8 @defines 'installer.nsi'
  if ($LASTEXITCODE -ne 0) { Fail "makensis (exit $LASTEXITCODE)" }
} finally { Pop-Location }
if (!(Test-Path -LiteralPath $outFile)) { Fail 'makensis produced no installer' }

if ($Mode -eq 'release') {
  & powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $signScript $outFile
  if ($LASTEXITCODE -ne 0) { Fail "signing the installer (exit $LASTEXITCODE)" }
}

$built = Get-Item -LiteralPath $outFile
"INSTALLER $($built.FullName)"
"SIZE $($built.Length)"
"SHA256 $((Get-FileHash -LiteralPath $outFile -Algorithm SHA256).Hash.ToLowerInvariant())"
"MODE $Mode  PRODUCT $product  IDENTIFIER $bundleId  VERSION $Version"

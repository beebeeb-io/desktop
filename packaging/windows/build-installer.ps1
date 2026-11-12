<#
.SYNOPSIS
  Builds the Windows installer of the native Beebeeb app with plain makensis. No Tauri CLI, no bundler.

.DESCRIPTION
  Input: the built native binary (native/beebeeb-drive-rune -> beebeeb-drive.exe). Output: one NSIS installer.

  -Mode test     "Beebeeb Test", identifier io.beebeeb.app.test, binary beebeeb-desktop-test.exe, unsigned. Installs
                 beside the real Beebeeb (%LOCALAPPDATA%\Beebeeb Test) and shares NOTHING with it: another binary
                 name (the installer looks only at processes of its own binary's name, and never ends one), another
                 identifier, and so another data folder, config file and set of credentials. The script refuses a
                 binary that was not built as the test app (BEEBEEB_APP_IDENTIFIER=io.beebeeb.app.test at build
                 time): a binary with the real identity would open the real account's data from a test install.
  -Mode release  "Beebeeb", identifier io.beebeeb.app. The app binary, the uninstaller and the installer are
                 Authenticode-signed through scripts/windows/sign-artifact.ps1 (Azure Artifact Signing, ambient az
                 login); refuses to run unless BEEBEEB_REQUIRE_SIGNING=1, so a release installer is never unsigned.

  A release binary is installed as beebeeb-desktop.exe, the name 0.8.x installed: the autostart entry, the
  shortcuts, the Explorer integration and 0.8.x's own updater all point at that file name (spec section 3).

  The script needs NSIS 3.11 (makensis.exe) and nsis_tauri_utils.dll 0.5.3 (the NSIS helper plugin the 0.8.x
  installer already carries: process lookup, semver compare, start-as-user). Both are in the Tauri toolchain cache of
  a PC that has built 0.8.x (%LOCALAPPDATA%\tauri\NSIS). The plugin, the compiler (Bin\makensis.exe) and the stub it
  packs into the installer (Stubs\lzma_solid-x86-unicode) are checked by SHA-256: release mode refuses anything but
  NSIS 3.11 as released; test mode says so and goes on.

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
# NSIS 3.11 as released (nsis-3.11.zip, sha256 c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1):
# the launcher, the compiler and the stub that becomes the first bytes of every installer and uninstaller.
$NsisSha256 = [ordered]@{
  'makensis.exe'                 = 'f497e92deb9f179f7b8f7553fcb3bd04f511bb2949e5e4a2aee80a10f7b20431'
  'Bin\makensis.exe'             = '42850802704ecb11163f7e0329d35ee54bd288953200d4966e226d572848cfc5'
  'Stubs\lzma_solid-x86-unicode' = 'a0d065b62d34be5f0aaaf7c162e101a5e25d7cd3eb10a13fdb37f91b02ebfce2'
}
$RealIdentifier = 'io.beebeeb.app'
$TestIdentifier = 'io.beebeeb.app.test'

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
foreach ($entry in $NsisSha256.GetEnumerator()) {
  $file = Join-Path $NsisDir $entry.Key
  $hash = if (Test-Path -LiteralPath $file) { (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() } else { 'missing' }
  if ($hash -ne $entry.Value) {
    if ($Mode -eq 'release') { Fail "$($entry.Key) in $NsisDir is not NSIS 3.11 as released (sha256 $hash)" }
    Write-Host "NOTE $($entry.Key) is not NSIS 3.11 as released (sha256 $hash): fine for a test build, refused for a release"
  }
}

# True when the file holds `text` as ASCII or as UTF-16LE (how a Rust or a Windows string constant sits in an exe).
function Test-BinaryHolds([string]$Path, [string]$Text) {
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  $latin1 = [System.Text.Encoding]::GetEncoding(28591)
  $haystack = $latin1.GetString($bytes)
  $wide = $latin1.GetString([System.Text.Encoding]::Unicode.GetBytes($Text))
  return ($haystack.Contains($Text) -or $haystack.Contains($wide))
}

# Absolute paths from here on: makensis runs with packaging\windows as its working directory.
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path
$Exe = (Resolve-Path -LiteralPath $Exe).Path
$PluginDir = (Resolve-Path -LiteralPath $PluginDir).Path

# The installer must not fetch or carry WebView2. The built installer is LZMA-compressed, so this is checked on
# the script that builds it: no download call and no bootstrapper name outside a comment.
foreach ($source in 'installer.nsi', 'hooks.nsh', 'utils.nsh') {
  $code = Get-Content -LiteralPath (Join-Path $PSScriptRoot $source) | Where-Object { $_ -notmatch '^\s*;' }
  $hits = @($code | Select-String -Pattern 'NSISdl::|MicrosoftEdgeWebview2|LinkId=2124703|EdgeUpdate' -CaseSensitive:$false)
  if ($hits.Count -ne 0) { Fail "$source still handles WebView2: $($hits[0].Line.Trim())" }
}

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$icon = Join-Path $repo 'src-tauri\icons\icon.ico'
if (!(Test-Path -LiteralPath $icon)) { Fail "no icon at $icon" }
$signScript = Join-Path $repo 'scripts\windows\sign-artifact.ps1'

if ($Mode -eq 'release') { $product = 'Beebeeb'; $bundleId = $RealIdentifier; $MainBinaryName = 'beebeeb-desktop' }
else { $product = 'Beebeeb Test'; $bundleId = $TestIdentifier; $MainBinaryName = 'beebeeb-desktop-test' }

# The binary's identity must be the installer's. The native app takes its identifier at build time
# (BEEBEEB_APP_IDENTIFIER; unset, a build is the test app; native\beebeeb-drive-rune\build.rs) and every name it and
# the engine use comes from it: data folder, config file, credential names, sync folder, Cloud Files registration.
# A test installer around a real-identity binary would run a second engine on the real account's data. The binary
# carries ONE mark, `beebeeb-app-identity=<identifier>;` (shell\identity.rs, BUILD_MARK): the installer is built
# only around a binary with this mode's mark and without the other mode's. The bare identifiers are not searched
# for: both are in every binary, because the engine names both apps.
$realMark = "beebeeb-app-identity=$RealIdentifier;"
$testMark = "beebeeb-app-identity=$TestIdentifier;"
$isRealBinary = Test-BinaryHolds $Exe $realMark
$isTestBinary = Test-BinaryHolds $Exe $testMark
if ($isRealBinary -eq $isTestBinary) {
  Fail "the binary does not carry exactly one identity mark (real: $isRealBinary, test: $isTestBinary): build it from this branch's native\beebeeb-drive-rune"
}
if ($Mode -eq 'test' -and !$isTestBinary) {
  Fail "the binary was not built as the test app: build it with BEEBEEB_APP_IDENTIFIER=$TestIdentifier, or with the variable unset (it would share credentials, config and state with the real Beebeeb)"
}
if ($Mode -eq 'release' -and !$isRealBinary) { Fail "the binary was built as the test app ($TestIdentifier): not a release binary" }

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
  if ($info.ProductName -ne 'Beebeeb' -or "$($info.ProductVersion)" -notin @($Version, "$Version.0")) {
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
  "/DMAINBINARYNAME=$MainBinaryName",
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
"MODE $Mode  PRODUCT $product  IDENTIFIER $bundleId  BINARY $MainBinaryName.exe  VERSION $Version"

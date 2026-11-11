<#
.SYNOPSIS
  Checks an installed (or uninstalled) native Beebeeb against what the installer promises. Read-only.

.DESCRIPTION
  -Phase installed    files, uninstall entry (exactly one), install-location key, Start-menu and desktop shortcut,
                      and no WebView2: none in the binary, no EBWebView profile left. (That the installer itself
                      carries none is checked by build-installer.ps1 on the script: the built file is compressed.)
  -Phase uninstalled  the install folder, the uninstall entry, the shortcuts and the autostart entry are gone, and
                      the app's data folder is still there if it was (a silent uninstall never deletes data).

  Ends with one count line, "VERIFY <phase>: N passed, M failed", and exits 1 unless M is 0 and N is the expected
  number of checks (a run that checked nothing is a failure).

.EXAMPLE
  .\verify-install.ps1 -Mode test -Version 0.9.0 -Phase installed
#>
[CmdletBinding()]
param(
  [ValidateSet('test', 'release')][string]$Mode = 'test',
  [Parameter(Mandatory = $true)][string]$Version,
  [ValidateSet('installed', 'uninstalled')][string]$Phase = 'installed'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($Mode -eq 'release') { $product = 'Beebeeb'; $bundleId = 'io.beebeeb.app' }
else { $product = 'Beebeeb Test'; $bundleId = 'io.beebeeb.app.test' }

$installDir = Join-Path $env:LOCALAPPDATA $product
$exe = Join-Path $installDir 'beebeeb-desktop.exe'
$uninstKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$product"
$manuKey = "HKCU:\Software\beebeeb\$product"
$startLnk = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\$product.lnk"
$desktopLnk = Join-Path ([Environment]::GetFolderPath('Desktop')) "$product.lnk"
$dataDir = Join-Path $env:LOCALAPPDATA $bundleId

$script:passed = 0; $script:failed = 0
function Check([string]$Name, [bool]$Ok, [string]$Detail = '') {
  if ($Ok) { $script:passed++; "ok    $Name" } else { $script:failed++; "FAIL  $Name  $Detail" }
}
function RegValue([string]$Key, [string]$Name) {
  $item = Get-ItemProperty -LiteralPath $Key -ErrorAction SilentlyContinue
  if ($null -eq $item) { return $null }
  $property = $item.PSObject.Properties[$Name]
  if ($null -eq $property) { return $null }
  return $property.Value
}
function ShortcutTarget([string]$Path) {
  if (!(Test-Path -LiteralPath $Path)) { return $null }
  return (New-Object -ComObject WScript.Shell).CreateShortcut($Path).TargetPath
}
# Every uninstall entry on this PC whose display name is exactly this product: per-user, per-machine, both views.
function UninstallEntries {
  $roots = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
           'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall',
           'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'
  $found = @()
  foreach ($root in $roots) {
    if (!(Test-Path -LiteralPath $root)) { continue }
    foreach ($key in Get-ChildItem -LiteralPath $root) {
      $name = $key.GetValue('DisplayName')
      if ($name -eq $product) { $found += $key.Name }
    }
  }
  return ,$found
}
# True when the file holds the text, as ASCII or UTF-16LE. Reads the whole file (the app is tens of MiB).
function FileContains([string]$Path, [string]$Text) {
  $bytes = [IO.File]::ReadAllBytes($Path)
  $latin = [Text.Encoding]::GetEncoding(28591).GetString($bytes)
  $wide = [Text.Encoding]::GetEncoding(28591).GetString([Text.Encoding]::Unicode.GetBytes($Text))
  return ($latin.IndexOf($Text, [StringComparison]::OrdinalIgnoreCase) -ge 0) -or ($latin.IndexOf($wide, [StringComparison]::OrdinalIgnoreCase) -ge 0)
}

if ($Phase -eq 'installed') {
  $expected = 14
  Check 'the app binary is installed' (Test-Path -LiteralPath $exe)
  Check 'the uninstaller is installed' (Test-Path -LiteralPath (Join-Path $installDir 'uninstall.exe'))
  $files = @(Get-ChildItem -LiteralPath $installDir -Force -ErrorAction SilentlyContinue | ForEach-Object Name | Sort-Object)
  Check 'the install folder holds exactly the binary and the uninstaller' (($files -join ',') -eq 'beebeeb-desktop.exe,uninstall.exe') ($files -join ',')
  Check 'uninstall entry: DisplayName' ((RegValue $uninstKey 'DisplayName') -eq $product)
  Check 'uninstall entry: DisplayVersion' ((RegValue $uninstKey 'DisplayVersion') -eq $Version) "$(RegValue $uninstKey 'DisplayVersion')"
  Check 'uninstall entry: Publisher' ((RegValue $uninstKey 'Publisher') -eq 'beebeeb')
  Check 'uninstall entry: MainBinaryName' ((RegValue $uninstKey 'MainBinaryName') -eq 'beebeeb-desktop.exe')
  Check 'uninstall entry: UninstallString' ((RegValue $uninstKey 'UninstallString') -eq "`"$installDir\uninstall.exe`"")
  $entries = UninstallEntries
  Check 'exactly one uninstall entry for this product' ($entries.Count -eq 1) ($entries -join ' | ')
  Check 'the install location is recorded' ((RegValue $manuKey '(default)') -eq $installDir) "$(RegValue $manuKey '(default)')"
  Check 'the Start-menu shortcut opens the binary' ((ShortcutTarget $startLnk) -eq $exe) "$(ShortcutTarget $startLnk)"
  Check 'the desktop shortcut opens the binary' ((ShortcutTarget $desktopLnk) -eq $exe) "$(ShortcutTarget $desktopLnk)"
  $webviewInBinary = (Test-Path -LiteralPath $exe) -and ((FileContains $exe 'WebView2Loader') -or (FileContains $exe 'EBWebView'))
  Check 'the binary does not load WebView2' (!$webviewInBinary)
  Check 'no WebView2 profile is left in the app data folder' (!(Test-Path -LiteralPath (Join-Path $dataDir 'EBWebView')))
} else {
  $expected = 6
  Check 'the install folder is gone' (!(Test-Path -LiteralPath $installDir))
  Check 'the uninstall entry is gone' (!(Test-Path -LiteralPath $uninstKey))
  $entries = UninstallEntries
  Check 'no uninstall entry for this product anywhere' ($entries.Count -eq 0) ($entries -join ' | ')
  Check 'the Start-menu shortcut is gone' (!(Test-Path -LiteralPath $startLnk))
  Check 'the desktop shortcut is gone' (!(Test-Path -LiteralPath $desktopLnk))
  Check 'the autostart entry is gone' ($null -eq (RegValue 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' $product))
  if (Test-Path -LiteralPath $dataDir) { "note  the data folder is kept: $dataDir" } else { "note  no data folder: $dataDir" }
}

"VERIFY ${Phase}: $script:passed passed, $script:failed failed"
if ($script:failed -ne 0 -or $script:passed -ne $expected) { "expected $expected checks to pass"; exit 1 }

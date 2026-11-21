<# Self test of verify-install.ps1's file check (plain PowerShell, no Pester): a fake install folder in $env:TEMP,
   checked once when right and once with a stray file, once with a file missing. Exits 1 on any miss. #>
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$verify = Join-Path $PSScriptRoot 'verify-install.ps1'
$nsi = Join-Path $PSScriptRoot 'installer.nsi'
$shell = (Get-Process -Id $PID).Path
$label = 'the install folder holds exactly the files the installer writes'
$binary = 'beebeeb-desktop-test.exe'
$files = 'beebeeb-office-render.exe', 'beebeeb-media-helper.exe', 'THIRD-PARTY-LICENSES.md', 'uninstall.exe', $binary

$root = Join-Path $env:TEMP ("bb-verify-selftest-" + [guid]::NewGuid().ToString('N'))
$script:bad = 0
function Run([string]$Dir) {
  $out = & $shell -NoProfile -File $verify -Mode test -Version 0.0.0 -Phase installed -InstallDir $Dir -InstallerScript $nsi 2>&1 | Out-String
  return ($out -split "`r?`n" | Where-Object { $_ -like "*$label*" } | Select-Object -First 1)
}
function Expect([string]$Name, [string]$Dir, [string]$Prefix) {
  $line = Run $Dir
  if ($line -and $line.StartsWith($Prefix)) { "ok    $Name" } else { $script:bad++; "FAIL  $Name  got: $line" }
}
try {
  $right = Join-Path $root 'right'; New-Item -ItemType Directory -Force $right | Out-Null
  foreach ($f in $files) { Set-Content -LiteralPath (Join-Path $right $f) 'x' }
  Expect 'the five shipped files pass' $right 'ok'

  $extra = Join-Path $root 'extra'; Copy-Item $right $extra -Recurse
  Set-Content -LiteralPath (Join-Path $extra 'stray.dll') 'x'
  Expect 'a stray file fails' $extra 'FAIL'

  $short = Join-Path $root 'short'; Copy-Item $right $short -Recurse
  Remove-Item -LiteralPath (Join-Path $short 'beebeeb-media-helper.exe')
  Expect 'a missing helper fails' $short 'FAIL'

  $old = Join-Path $root 'old'; New-Item -ItemType Directory -Force $old | Out-Null
  foreach ($f in $binary, 'uninstall.exe') { Set-Content -LiteralPath (Join-Path $old $f) 'x' }
  Expect 'the old two-file folder fails' $old 'FAIL'
} finally {
  if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
}
"SELFTEST: $script:bad failed"
if ($script:bad -ne 0) { exit 1 }

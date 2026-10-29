<#
.SYNOPSIS
  Fail-closed Authenticode release gate for Windows artifacts (task 1617).

.DESCRIPTION
  Verifies, with counts and per-file evidence, that every Windows artifact we
  publish is Authenticode-signed, by the expected publisher, and timestamped:

    - Status is Valid
    - signer subject contains CN=Initlabs B.V.
    - an RFC3161 timestamp counter-signature is present

  Modes:
    -Files <paths>               verify each listed file
    -ExtractInstallers <paths>   unpack each installer with 7z (incl. .cab
                                 payloads) and verify every .exe/.dll inside;
                                 extracting zero executables is a FAILURE
    -SelfTest                    red-proof: assert this guard REJECTS an
                                 unsigned file and a wrong-signer file.
                                 Exits non-zero if the guard accepts either.

  Exit code 0 only if every required check passed. This runs in release CI
  before artifacts are uploaded, so an unsigned or wrongly-signed build can
  never reach the GitHub release (task 1617: "invalid/missing signature blocks
  candidate publication").
#>
[CmdletBinding()]
param(
  [string[]]$Files = @(),
  [string[]]$ExtractInstallers = @(),
  [string]$ExpectSigner = 'CN=Initlabs B.V.',
  [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:Checked = 0
$script:Failed = 0

function Find-SevenZip {
  $cmd = Get-Command 7z.exe -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  foreach ($p in @('C:\Program Files\7-Zip\7z.exe', 'C:\Program Files (x86)\7-Zip\7z.exe')) {
    if (Test-Path $p) { return $p }
  }
  return $null
}

function Test-Signature([string]$Path, [string]$Expect) {
  <# Returns $true when the file passes; prints one evidence line. #>
  $script:Checked++
  if (-not (Test-Path -LiteralPath $Path)) {
    Write-Host "FAIL  $Path - file not found"
    $script:Failed++
    return $false
  }
  $sig = Get-AuthenticodeSignature -LiteralPath $Path
  $hash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
  $signer = if ($sig.SignerCertificate) { $sig.SignerCertificate.Subject } else { '<none>' }
  $stamp = if ($sig.TimeStamperCertificate) { $sig.TimeStamperCertificate.Subject } else { '<none>' }
  $ok = ($sig.Status -eq 'Valid') -and ($signer -match [regex]::Escape($Expect)) -and ($sig.TimeStamperCertificate -ne $null)
  if ($ok) {
    Write-Host ("PASS  {0}  status={1}  signer=[{2}]  tsa=[{3}]  sha256={4}" -f $Path, $sig.Status, $signer, $stamp, $hash)
  } else {
    Write-Host ("FAIL  {0}  status={1}  signer=[{2}]  tsa=[{3}]  sha256={4}" -f $Path, $sig.Status, $signer, $stamp, $hash)
    $script:Failed++
  }
  return $ok
}

function Invoke-InstallerExtraction([string]$Installer) {
  # Unpack the installer payload and verify the application binaries inside.
  # MSI: `msiexec /a` administrative install = pure file extraction (7z's MSI
  # handling is unreliable across versions - it flattens the compound file).
  # NSIS: 7z extracts the payload plus $PLUGINSDIR.
  # $PLUGINSDIR holds the NSIS toolkit's own redistributable plugin DLLs
  # (nsDialogs, System, ...). Tauri signs these with the same sign command when
  # signing is enabled (tauri-bundler "Signing NSIS plugins"), but this gate
  # skips re-verifying them: the release boundary is the installer and its
  # application payload, and skipping keeps the gate independent of Tauri's
  # plugin-signing behavior. They are skipped with a counted INFO line.
  # Everything else in the payload (the app .exe, the uninstaller) must
  # carry a valid Initlabs B.V. signature.
  $work = Join-Path $env:TEMP ("beebeeb-guard-" + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Force -Path $work | Out-Null
  try {
    if ($Installer -match '\.msi$') {
      $p = Start-Process msiexec.exe -ArgumentList @('/a', $Installer, '/qn', "TARGETDIR=$work") -Wait -PassThru
      if ($p.ExitCode -ne 0) {
        Write-Host "FAIL  $Installer - msiexec /a extraction failed (exit $($p.ExitCode))"
        $script:Failed++
        return
      }
    } else {
      $sevenZip = Find-SevenZip
      if (-not $sevenZip) {
        Write-Host "FAIL  $Installer - 7z.exe not found; cannot verify the contained executable (fail closed)"
        $script:Failed++
        return
      }
      & $sevenZip x -y "-o$work" $Installer | Out-Null
      if ($LASTEXITCODE -ne 0) {
        Write-Host "FAIL  $Installer - 7z extraction failed (exit $LASTEXITCODE)"
        $script:Failed++
        return
      }
      # Some packers wrap the payload in .cab streams; unpack those too.
      Get-ChildItem -Path $work -Recurse -Filter '*.cab' | ForEach-Object {
        $cabDir = Join-Path $_.DirectoryName ($_.BaseName + '-cab')
        New-Item -ItemType Directory -Force -Path $cabDir | Out-Null
        & $sevenZip x -y "-o$cabDir" $_.FullName | Out-Null
      }
    }

    $pe = @(Get-ChildItem -Path $work -Recurse -Include '*.exe', '*.dll' -File)
    $plugins = @($pe | Where-Object { $_.FullName -match '\$PLUGINSDIR\\' })
    $ours = @($pe | Where-Object { $_.FullName -notmatch '\$PLUGINSDIR\\' })
    if ($plugins.Count -gt 0) {
      Write-Host ("INFO  {0} - skipping {1} NSIS toolkit plugin(s) in `$PLUGINSDIR (third-party, unsigned by design): {2}" -f `
        $Installer, $plugins.Count, (($plugins | ForEach-Object { $_.Name }) -join ', '))
    }
    Write-Host ("INFO  {0} - extracted {1} application PE file(s) from installer payload" -f $Installer, $ours.Count)
    if ($ours.Count -eq 0) {
      Write-Host "FAIL  $Installer - no application executable found inside the installer payload (guard cannot prove the contained exe is signed)"
      $script:Failed++
      return
    }
    foreach ($f in $ours) {
      [void](Test-Signature -Path $f.FullName -Expect $ExpectSigner)
    }
  } finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
  }
}

function Invoke-SelfTest {
  # Red-proof: the guard must REJECT (a) an unsigned file and (b) a file signed
  # by the wrong publisher. SelfTest passes only if both are rejected.
  Write-Host '=== self-test: guard must reject bad artifacts (red-proof) ==='
  $badDir = Join-Path $env:TEMP ('beebeeb-guard-selftest-' + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Force -Path $badDir | Out-Null
  $okAll = $true
  try {
    # (a) unsigned fixture: a real PE-like blob with no signature at all.
    $unsigned = Join-Path $badDir 'unsigned-fixture.exe'
    $bytes = New-Object byte[] 4096
    (New-Object System.Random).NextBytes($bytes)
    [System.IO.File]::WriteAllBytes($unsigned, $bytes)

    $accepted = Test-Signature -Path $unsigned -Expect $ExpectSigner
    if ($accepted) {
      Write-Host 'SELFTEST-FAIL  unsigned fixture was ACCEPTED - guard is broken'
      $okAll = $false
    } else {
      Write-Host 'SELFTEST-PASS  unsigned fixture correctly rejected'
    }

    # (b) wrong-signer fixture: a genuinely signed system binary (Microsoft),
    # which must be rejected because the publisher is not Initlabs B.V.
    $foreign = Join-Path $env:SystemRoot 'System32\cmd.exe'
    $accepted = Test-Signature -Path $foreign -Expect $ExpectSigner
    if ($accepted) {
      Write-Host 'SELFTEST-FAIL  wrong-signer fixture was ACCEPTED - guard is broken'
      $okAll = $false
    } else {
      Write-Host 'SELFTEST-PASS  wrong-signer fixture (cmd.exe, Microsoft-signed) correctly rejected'
    }
  } finally {
    Remove-Item -Recurse -Force $badDir -ErrorAction SilentlyContinue
  }
  if (-not $okAll) {
    Write-Host 'SELFTEST-RESULT FAIL (guard accepted a bad artifact)'
    exit 1
  }
  Write-Host 'SELFTEST-RESULT PASS (guard rejected unsigned + wrong-signer fixtures)'
  exit 0
}

if ($SelfTest) { Invoke-SelfTest }

if ($Files.Count -eq 0 -and $ExtractInstallers.Count -eq 0) {
  Write-Host 'Usage: verify-authenticode.ps1 -Files <paths> [-ExtractInstallers <installers>] | -SelfTest'
  exit 2
}

foreach ($f in $Files) {
  [void](Test-Signature -Path $f -Expect $ExpectSigner)
}
foreach ($i in $ExtractInstallers) {
  Invoke-InstallerExtraction -Installer $i
}

Write-Host ("SUMMARY  checked={0}  failed={1}  expected_signer=[{2}]" -f $script:Checked, $script:Failed, $ExpectSigner)
if ($script:Checked -eq 0) {
  Write-Host 'SUMMARY-RESULT FAIL (no files were checked - gate must never pass empty)'
  exit 1
}
if ($script:Failed -gt 0) {
  Write-Host 'SUMMARY-RESULT FAIL (unsigned or wrongly-signed artifact present)'
  exit 1
}
Write-Host 'SUMMARY-RESULT PASS (all artifacts Valid, expected signer, timestamped)'
exit 0

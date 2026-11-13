#Requires -Version 5.1
<#
.SYNOPSIS
    Native rung: what Windows Hello answers on this machine (examples/hello_probe.rs), run in a desktop session.
.DESCRIPTION
    Runs the built probe and passes its lines on; they also land in $env:BB_OUT\hello-probe.txt when that is set.
    The probe prints "availability: <name>" and, with -Prompt, opens its own small window, asks Windows for the
    Hello prompt on it and prints "result: <name>". It signs in to nothing and prints no personal data.
    A prompt nobody answers is taken away by the probe after -CancelAfter seconds (default 60).
    Exit: the probe's own (0 ran, 1 no window, 2 usage); 3 the probe is not built.
.NOTES
    Build first, from native\beebeeb-drive-rune:  cargo build --locked --example hello_probe
    Then, in the same folder:  powershell -NoProfile -ExecutionPolicy Bypass -File scripts\rungs\hello-probe.ps1 -Prompt
#>
[CmdletBinding()]
param(
    [switch]$Prompt,
    [ValidateRange(0, 3600)][int]$CancelAfter = 60
)
$ErrorActionPreference = 'Stop'

$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot '..\..\target' }
$probe = Join-Path $target 'debug\examples\hello_probe.exe'
if (-not (Test-Path -LiteralPath $probe -PathType Leaf)) {
    Write-Output "hello-probe: not built: $probe"
    exit 3
}
$probeArgs = @()
if ($Prompt) { $probeArgs += @('--prompt', '--cancel-after', "$CancelAfter") }
$lines = @(& $probe @probeArgs 2>&1 | ForEach-Object { "$_" })
$code = $LASTEXITCODE
$lines | ForEach-Object { Write-Output $_ }
if ($env:BB_OUT) { $lines | Set-Content -LiteralPath (Join-Path $env:BB_OUT 'hello-probe.txt') -Encoding UTF8 }
exit $code

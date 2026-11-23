#Requires -Version 5.1
<#
.SYNOPSIS
    The two tests that measure Windows' decoders inside the sandbox, run where they can measure: a desktop session.
.DESCRIPTION
    In session 0 (an SSH login, a service) ole32.dll does not initialise in any AppContainer (1114), so there both
    tests print "NOT MEASURED" and pass without measuring. This runs them in the session it is started in and fails
    unless each one ran, measured and passed:
      crates\beebeeb-office-sandbox   windows_decoders_start_in_the_system_decoder_profile_only
      native\beebeeb-media-helper     confined_system_profile_plays_aac_with_sound
    Exit: 0 both measured and passed; 1 one failed, did not run or did not measure; 2 this is session 0.
.NOTES
    At a desktop:  powershell -NoProfile -ExecutionPolicy Bypass -File scripts\windows\decoder-session-tests.ps1
    Over SSH, through whatever starts a script in the logged-in desktop session, from the checkout's root.
#>
$ErrorActionPreference = 'Continue'

$session = (Get-Process -Id $PID).SessionId
if ($session -eq 0) {
    Write-Output 'decoder-session-tests: this is session 0, which has no desktop: nothing was run'
    exit 2
}
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).ProviderPath
$runs = @(
    @{ Dir = 'crates\beebeeb-office-sandbox'; Test = '--test sandbox windows_decoders_start_in_the_system_decoder_profile_only' },
    @{ Dir = 'native\beebeeb-media-helper'; Test = '--test confined confined_system_profile_plays_aac_with_sound' }
)
$bad = 0
foreach ($run in $runs) {
    Push-Location -LiteralPath (Join-Path $root $run.Dir)
    # Through cmd: Windows PowerShell 5.1 would turn cargo's stderr lines into errors of this script.
    $lines = @(cmd.exe /c "cargo test --locked $($run.Test) 2>&1")
    $code = $LASTEXITCODE
    Pop-Location
    $lines | ForEach-Object { Write-Output "$_" }
    $passed = @($lines | Select-String -SimpleMatch 'test result: ok. 1 passed; 0 failed').Count
    $unmeasured = @($lines | Select-String -SimpleMatch 'NOT MEASURED').Count
    if ($code -ne 0 -or $passed -ne 1 -or $unmeasured -ne 0) {
        Write-Output "decoder-session-tests: FAILED in $($run.Dir) (cargo exit $code, passing runs $passed, not measured $unmeasured)"
        $bad++
    }
}
if ($bad -ne 0) { exit 1 }
Write-Output "decoder-session-tests: ok, 2 of 2 measured and passed in session $session"
exit 0

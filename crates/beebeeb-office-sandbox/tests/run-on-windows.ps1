# Runs the crate's tests on a Windows machine and counts what the run left behind.
# Usage (from the crate folder): powershell -NoProfile -ExecutionPolicy Bypass -File tests\run-on-windows.ps1 [cargo test arguments]
# It changes no system setting. It prints counts only: no path, no address, no name.
$ErrorActionPreference = 'Continue'

function Count-State {
    $state = [ordered]@{}
    $state['probe-processes'] = @(Get-Process -Name 'beebeeb-sandbox-probe' -ErrorAction SilentlyContinue).Count
    $places = @($env:TEMP, $env:USERPROFILE, (Join-Path $env:USERPROFILE 'AppData\LocalLow'), $env:PUBLIC) | Where-Object { $_ -and (Test-Path $_) }
    $state['scratch-files'] = @($places | ForEach-Object { Get-ChildItem -Path $_ -Filter 'beebeeb-sandbox-test-*.tmp' -File -ErrorAction SilentlyContinue }).Count
    $mappings = 'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings'
    $state['container-profiles'] = if (Test-Path $mappings) { @(Get-ChildItem $mappings -ErrorAction SilentlyContinue).Count } else { 0 }
    $packages = Join-Path $env:LOCALAPPDATA 'Packages'
    $state['package-folders'] = if (Test-Path $packages) { @(Get-ChildItem $packages -Directory -ErrorAction SilentlyContinue).Count } else { 0 }
    try { $state['firewall-rules'] = @(Get-NetFirewallRule -ErrorAction Stop).Count } catch { $state['firewall-rules'] = 'unreadable' }
    return $state
}

$before = Count-State
$log = Join-Path $env:TEMP ("beebeeb-sandbox-run-{0}.log" -f $PID)
& cargo test --locked @args -- --nocapture --test-threads=1 *> $log
$exit = $LASTEXITCODE
Get-Content $log | Where-Object { $_ -notmatch '^\s*(Compiling|Downloading|Downloaded|Updating|Locking|Blocking|Finished|note: run with)' -and $_.Trim() -ne '' }
Remove-Item $log -Force
$after = Count-State

"cargo test exit code: $exit"
foreach ($key in $before.Keys) {
    "state: $key before=$($before[$key]) after=$($after[$key])"
}
$left = $after['probe-processes'] + $after['scratch-files']
$moved = @($before.Keys | Where-Object { "$($before[$_])" -ne "$($after[$_])" }).Count
"leftover: $left processes and files, $moved of $($before.Count) counts moved"
if ($exit -ne 0) { exit $exit }
if ($left -ne 0 -or $moved -ne 0) { exit 3 }

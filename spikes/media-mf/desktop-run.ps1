# Runs the spike's runner inside the logged-in desktop session (the build VM's run-in-desktop.ps1 starts this file
# there; that runner passes no arguments, so they come from a file the caller wrote first: one invocation per line,
# its arguments separated by '|').
# The runner exe is the one the same remote-build run just built.
$ErrorActionPreference = 'Continue'
$argFile = 'C:\bb\out\media-mf-spike\desktop-args.txt'
$target = $env:CARGO_TARGET_DIR
if (-not $target) { $target = 'C:\bb\target\desktop\spike_media-mf' }
$exe = Join-Path $target 'release\media-mf-run.exe'
"desktop-run: session=$((Get-Process -Id $PID).SessionId) exe-exists=$(Test-Path -LiteralPath $exe)"
foreach ($line in @(Get-Content -LiteralPath $argFile | Where-Object { $_.Trim() -ne '' })) {
  $runArgs = @($line -split '\|')
  "desktop-run: invocation args=$($runArgs -join ' ')"
  & $exe @runArgs 2>&1 | ForEach-Object { "$_" }
  "desktop-run: exit=$LASTEXITCODE"
}

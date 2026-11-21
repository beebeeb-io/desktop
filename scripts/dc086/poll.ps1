# dc-086: one line every 15 s: entries in the sync folder, state.db size, app log lines, app CPU and handles. Read-only.
param([int]$Minutes = 45)
$root = Join-Path $env:USERPROFILE 'Beebeeb Test'; $l = "$env:LOCALAPPDATA\io.beebeeb.app.test\logs\beebeeb-drive.log"; $db = "$env:LOCALAPPDATA\io.beebeeb.app.test"
$end = (Get-Date).AddMinutes($Minutes)
while ((Get-Date) -lt $end) {
  $sw = [Diagnostics.Stopwatch]::StartNew(); $n = 0; $d = 0
  try { foreach ($e in [IO.Directory]::EnumerateFileSystemEntries($root, '*', [IO.SearchOption]::AllDirectories)) { $n++ } } catch { }
  try { foreach ($e in [IO.Directory]::EnumerateDirectories($root, '*', [IO.SearchOption]::AllDirectories)) { $d++ } } catch { }
  $ms = $sw.ElapsedMilliseconds
  $p = @(Get-Process beebeeb-desktop-test -ea 0 | Sort-Object WorkingSet64 -Descending)[0]
  $dbs = @(Get-ChildItem $db -Recurse -File -ea 0 | Where-Object { $_.Name -match '^state\.db' } | ForEach-Object { '{0}={1:0.0}MB' -f $_.Name, ($_.Length / 1MB) }) -join ' '
  $ll = 0; try { $ll = @(Get-Content $l -ea 0).Count } catch {}
  '{0} entries={1} dirs={2} list_ms={3} {4} log_lines={5} app_cpu={6:0}s handles={7} ws={8}MB' -f (Get-Date).ToUniversalTime().ToString('HH:mm:ss'), $n, $d, $ms, $dbs, $ll, $(if ($p) { $p.TotalProcessorTime.TotalSeconds } else { -1 }), $(if ($p) { $p.HandleCount } else { -1 }), $(if ($p) { [int]($p.WorkingSet64 / 1MB) } else { -1 })
  if (-not $p) { 'app gone'; break }
  Start-Sleep -Seconds 15
}

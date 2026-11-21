# dc-086 sampler (not product code): one CSV row every $Every s. Counts and sizes only.
param([string]$Out, [int]$Every = 5)
$ErrorActionPreference = 'SilentlyContinue'
$pc = @{}; foreach ($n in 'Pool Paged Bytes', 'Pool Nonpaged Bytes', 'Committed Bytes', 'Available MBytes') { $pc[$n] = New-Object System.Diagnostics.PerformanceCounter('Memory', $n) }
$stateDirs = @("$env:LOCALAPPDATA\io.beebeeb.app.test", "$env:APPDATA\beebeeb-test")
'utc,app_pid,app_handles,app_ws_mb,app_priv_mb,app_threads,app_cpu_s,sys_handles,sys_cpu_s,all_handles,pool_paged_mb,pool_nonpaged_mb,commit_mb,avail_mb,explorer_cpu_s,explorer_handles,explorer_ws_mb,defender_cpu_s,indexer_cpu_s,fake_cpu_s,fake_ws_mb,db_mb,wal_mb,app_write_ops,app_write_mb,app_read_ops,app_other_ops' | Set-Content -LiteralPath $Out -Encoding ASCII
function Cpu($p) { if ($p) { [Math]::Round((($p | Measure-Object -Property KernelModeTime -Sum).Sum + ($p | Measure-Object -Property UserModeTime -Sum).Sum) / 1e7, 1) } else { '' } }
while ($true) {
  $t = (Get-Date).ToUniversalTime().ToString('HH:mm:ss')
  $all = @(Get-CimInstance Win32_Process -Property Name, ProcessId, KernelModeTime, UserModeTime, HandleCount, WorkingSetSize, PrivatePageCount, ThreadCount, WriteOperationCount, WriteTransferCount, ReadOperationCount, OtherOperationCount)
  $a = @($all | Where-Object { $_.Name -eq 'beebeeb-desktop-test.exe' } | Sort-Object WorkingSetSize -Descending)[0]
  $sys = @($all | Where-Object { $_.ProcessId -eq 4 })[0]
  $ex = @($all | Where-Object { $_.Name -eq 'explorer.exe' })
  $def = @($all | Where-Object { $_.Name -eq 'MsMpEng.exe' }); $idx = @($all | Where-Object { $_.Name -like 'Search*' })
  $fk = @($all | Where-Object { $_.Name -eq 'dc086_large_fake.exe' })
  $db = 0; $wal = 0
  foreach ($d in $stateDirs) { foreach ($f in @(Get-ChildItem -LiteralPath $d -Recurse -File -ea 0 | Where-Object { $_.Name -match '\.db(-wal)?$' })) { if ($f.Name -like '*-wal') { $wal += $f.Length } else { $db += $f.Length } } }
  $row = @($t,
    $(if ($a) { $a.ProcessId } else { '' }), $(if ($a) { $a.HandleCount } else { '' }), $(if ($a) { [int]($a.WorkingSetSize / 1MB) } else { '' }), $(if ($a) { [int]($a.PrivatePageCount / 1MB) } else { '' }), $(if ($a) { $a.ThreadCount } else { '' }), (Cpu $a),
    $sys.HandleCount, (Cpu $sys), ($all | Measure-Object -Property HandleCount -Sum).Sum,
    [int]($pc['Pool Paged Bytes'].NextValue() / 1MB), [int]($pc['Pool Nonpaged Bytes'].NextValue() / 1MB), [int]($pc['Committed Bytes'].NextValue() / 1MB), [int]$pc['Available MBytes'].NextValue(),
    (Cpu $ex), ($ex | Measure-Object -Property HandleCount -Sum).Sum, [int](($ex | Measure-Object -Property WorkingSetSize -Sum).Sum / 1MB),
    (Cpu $def), (Cpu $idx), (Cpu $fk), [int](($fk | Measure-Object -Property WorkingSetSize -Sum).Sum / 1MB),
    [Math]::Round($db / 1MB, 1), [Math]::Round($wal / 1MB, 1), $(if ($a) { $a.WriteOperationCount } else { '' }), $(if ($a) { [int]($a.WriteTransferCount / 1MB) } else { '' }), $(if ($a) { $a.ReadOperationCount } else { '' }), $(if ($a) { $a.OtherOperationCount } else { '' }))
  Add-Content -LiteralPath $Out -Value ($row -join ',') -Encoding ASCII
  Start-Sleep -Seconds $Every
}

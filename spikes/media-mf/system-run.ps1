# Runs the probe unconfined under LocalSystem, an account with no registration of the Store codec packages (they are
# registered per user): what Media Foundation says when the HEVC extension is not there for the account, without
# removing or installing anything. A one-shot scheduled task of the spike's own name, removed at the end.
# Needs an elevated caller (the build VM's SSH session is). Usage: system-run.ps1 <runner exe> <out log>
param([Parameter(Mandatory = $true)][string]$Runner, [Parameter(Mandatory = $true)][string]$Log)
$ErrorActionPreference = 'Stop'
$name = 'bb-mfspike-system'
$runArgs = 'C:\bb\out\media-mf-spike\fixtures plain E,h264.mp4,hevc-main.mp4,hevc-main.mov,hevc-main10.mp4,hevc-main10.mov,vp9.webm 0'
$action = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument ('/c ""{0}" {1} > "{2}" 2>&1"' -f $Runner, $runArgs, $Log)
Register-ScheduledTask -TaskName $name -Action $action -User 'SYSTEM' -RunLevel Highest | Out-Null
try {
  Start-ScheduledTask -TaskName $name
  $deadline = (Get-Date).AddMinutes(5)
  Start-Sleep -Seconds 2
  while ((Get-ScheduledTask -TaskName $name).State -eq 'Running' -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 1 }
  "system-run: state=$((Get-ScheduledTask -TaskName $name).State) result=$((Get-ScheduledTaskInfo -TaskName $name).LastTaskResult)"
} finally {
  Unregister-ScheduledTask -TaskName $name -Confirm:$false
  "system-run: task removed=$(-not (Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue))"
}
if (Test-Path -LiteralPath $Log) { Get-Content -LiteralPath $Log } else { 'system-run: no log' }

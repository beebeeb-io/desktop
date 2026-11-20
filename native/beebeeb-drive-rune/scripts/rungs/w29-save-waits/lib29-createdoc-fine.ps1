# w29: New document timed with a fine poll (the old CreateDoc polled every ~0.5-3 s: Pump 500 + Wait-Text up to 3 s). secs = Create pressed -> the editor's status line is on screen.
function global:CreateDoc([string]$name) {
  Back-To-Files; Beat 0.8; $null = Click-Name 'New' 'Button' 5; Beat 1.2; $null = Click-Rx '^New document' 'MenuItem' 4; Beat 2
  $null = Click-Rx '^Letter' 'RadioButton' 3; Beat 1; $null = TypeNew 'Name' $name; Beat 1; $h = @(App-Windows)[0]; $t = Get-Date; $null = Click-Name 'Create' 'Button' 3
  $st = ''; $err = $false; $polls = 0
  while (((Get-Date) - $t).TotalSeconds -lt 90) { $polls++; $tx = @(try { FastTexts $h } catch { @() }); $m = @($tx | ? { $_ -match '^Page \d+ of' }); if ($m.Count) { $st = $m[0]; break }; if ($tx -contains "Couldn't read the copy on this PC") { $err = $true; break }; Start-Sleep -Milliseconds 30 }
  $secs = [Math]::Round(((Get-Date) - $t).TotalSeconds, 2)
  S "create-$name"
  @{ status = $st; secs = $secs; err = $err }
}

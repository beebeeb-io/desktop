# w25 account and share helpers (not product code)
function global:Use-Secret([string]$dir) { New-Item -ItemType Directory -Force C:\bb\rec\secret | Out-Null; Copy-Item "$dir\*" C:\bb\rec\secret -Force; $global:phrase = $null; $global:Email = (Get-Content C:\bb\rec\secret\email.txt -Raw).Trim(); "secret now $global:Email" }
function global:Who { $t = Texts2; @($t | ? { $_ -match '@example\.org$' })[0] }
function global:SignOut3 { Clear-Editor; $null = Front; $null = Click-Name 'Files' 'Button' 4; Beat 0.8; $null = Click-Name 'Settings' 'Button' 6; Beat 1.5; $null = Click-Name 'Security' 'Hyperlink' 10; Beat 2; $null = Click-Rx '^Sign out of Drive' 'Hyperlink' 10; Beat 2.5; $null = Click-Rx '^Sign out' 'Button' 10
  $w = Wait-Text '^Sign in to Drive|^Welcome to Beebeeb' 20; Beat 2
  Start-Process $global:Installed -ArgumentList '--quit-for-update' -Wait -WindowStyle Hidden; Beat 2; $null = Start-App2; Beat 7
  $still = [bool](Find-El 'Settings' 'Button'); "signed out: screen '$w', still signed in after restart: $still" }
function global:SignInAs([string]$dir) { Use-Secret $dir | Out-Null; $t = Get-Date; SignIn2; Beat 2; FirstRun2 900 | Out-Null; Beat 1; $null = Click-Name 'Got it' 'Button' 3; $null = Click-Rx '^Skip' '' 2; Beat 1.5
  $ws = Window-State; if ($ws -match '(\d+)x(\d+)' -and [int]$matches[2] -lt 740) { $null = Click-Name 'Maximize' 'Button' 5; Beat 1.5 }
  "signed in as $(Who) in $([Math]::Round(((Get-Date) - $t).TotalSeconds)) s" }
# Shared, With me: texts read at once and then every second for $sec; returns the lines seen with their time
function global:WithMe([string]$tag, [int]$sec = 12) {
  $null = Front; $null = Click-Name 'Shared' 'Button' 8; $t = Get-Date; $null = Click-Name 'With me' 'TabItem' 4
  $seen = @(); $last = ''
  while (((Get-Date) - $t).TotalSeconds -lt $sec) {
    $tx = Texts2; $i = [Array]::IndexOf($tx, 'Ctrl+F'); $body = if ($i -ge 0) { ($tx[($i + 1)..($tx.Count - 1)] -join ' ; ') } else { $tx -join ' ; ' }
    if ($body -ne $last) { $s = [Math]::Round(((Get-Date) - $t).TotalSeconds, 1); $seen += "+$s s: $body"; $last = $body; S "withme-$tag-$($seen.Count)" }
    Pump 500
  }
  $seen
}
function global:SignUp([string]$email, [string]$dir) {
  $r = Click-Rx '^Create a' '' 15; "create link: $r"; Beat 2
  $null = TypeNew 'Email' $email; Beat 1; $null = Click-Name 'Send the code' 'Button' 10; Beat 3
  $code = ''
  for ($i = 0; $i -lt 25 -and $code.Length -ne 8; $i++) { Pump 1000; try { $code = (Invoke-WebRequest -UseBasicParsing ($global:CodeUrl + $email)).Content; if ($code -is [byte[]]) { $code = [Text.Encoding]::ASCII.GetString($code) } } catch { $code = '' } }
  "code length: $($code.Length) (not shown)"
  $e = @(Edits2); if ($e.Count) { Click-El $e[0] | Out-Null; Type-Slow $code }; $code = ''
  Beat 1; if (Find-El 'Verify' 'Button') { Click-Name 'Verify' 'Button' 5 | Out-Null }; Beat 3
  $null = Click-Rx '^I accept the Terms' '' 10; $null = Click-Rx '^I understand that Beebeeb cannot recover' '' 10; Beat 1
  Click-Name 'Continue' 'Button' 10 | Out-Null; Beat 3
  $null = TypeNew 'Password' $global:Pw; $null = TypeNew 'Confirm password' $global:Pw; Beat 2
  Click-Name 'Continue' 'Button' 20 | Out-Null; Beat 5
  [System.Windows.Forms.Clipboard]::Clear(); Click-Name 'Copy' 'Button' 10 | Out-Null; Beat 1
  $words = @(); try { $words = @(([System.Windows.Forms.Clipboard]::GetText()) -split '\s+' | ? { $_ }) } catch {}
  [System.Windows.Forms.Clipboard]::Clear(); "phrase words: $($words.Count)"
  New-Item -ItemType Directory -Force $dir | Out-Null
  Set-Content "$dir\phrase.txt" -Value ($words -join ' ') -Encoding ASCII; Set-Content "$dir\email.txt" -Value $email -Encoding ASCII
  $null = Click-Rx '^I have saved my recovery phrase' '' 10; Beat 1; Click-Rx '^I saved it' 'Button' 10 | Out-Null; Beat 3
  $asked = @(); foreach ($pair in Els) { try { $c = $pair[1].Current; if ($c.ControlType.ProgrammaticName -eq 'ControlType.Edit' -and $c.Name -match '^Recovery word (\d+)$') { $asked += ,@([int]$Matches[1], $pair) } } catch {} }
  foreach ($a in $asked) { Click-El $a[1] | Out-Null; Type-Slow $words[$a[0] - 1] }
  $words = @(); Beat 1; Click-Name 'Confirm' 'Button' 10 | Out-Null; Beat 4; Click-Name 'Create my account' 'Button' 10 | Out-Null
  $done = ''; for ($i = 0; $i -lt 90 -and -not $done; $i++) { Pump 1000; $t = Texts2; foreach ($w in 'Add a passkey', 'Lock Beebeeb', 'We could not create your account') { if (-not $done -and ($t -ccontains $w)) { $done = $w } } }
  "after create: '$done' ($i s)"
  FirstRun2 1200 | Out-Null; Beat 2; $null = Click-Name 'Got it' 'Button' 4; Beat 1; $null = Click-Rx '^Skip' '' 2; Beat 2
  $ws = Window-State; if ($ws -match '(\d+)x(\d+)' -and [int]$matches[2] -lt 740) { $null = Click-Name 'Maximize' 'Button' 5; Beat 1.5 }
  "signed up as $(Who)" }
function global:Invite([string]$folder, [string]$email) {
  Back-To-Files; Beat 1.5; $null = Click-Name $folder 'Text' 8; Beat 1
  $null = Click-Rx ('^Share' + [char]0x2026 + '?\.?$') 'Button' 6; Beat 3; $null = Click-Rx '^Invite someone' 'Button' 5; Beat 2.5
  $null = TypeNew 'Email' $email; Beat 1; $clk = Get-Date; $s = Click-Rx '^Send invite' 'Button' 4; Beat 6; D 'invite-sent'; S "invite-$($email.Split('@')[0])"
  Send '{ESC}'; Beat 1; Send '{ESC}'; Beat 1; "invite $email to '$folder': $s at $($clk.ToString('HH:mm:ss'))" }
function global:Clean-Install { . C:\bb\rec\clean-state.ps1; $setup = 'C:\bb\w25\installer\Beebeeb Test_0.9.0_x64-setup.exe'; $p = Start-Process -FilePath $setup -ArgumentList '/S' -PassThru -Wait; Start-Sleep 3; "clean install: installer exit $($p.ExitCode), exe $((Get-FileHash $global:Installed).Hash.Substring(0,16))" }
function global:Body { $tx = Texts2; $i = [Array]::IndexOf($tx, 'Ctrl+F'); if ($i -ge 0) { ($tx[($i + 1)..($tx.Count - 1)] -join ' ; ') } else { $tx -join ' ; ' } }

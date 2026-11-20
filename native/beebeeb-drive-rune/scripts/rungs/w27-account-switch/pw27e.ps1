# w27 account-switch proof, part 2 (not product code): work that is not sent keeps the next account out, with words, nothing deleted.
# A: a file put in the folder while signed out. B: a change queued with the server away. (C, the w26 case, ran in the run before.)
. C:\bb\rec\rec-lib.ps1
. C:\bb\rec\rec-lib3.ps1
. C:\bb\w25\head.ps1
. C:\bb\w25\lib19.ps1
. C:\bb\w25\lib25.ps1
. C:\bb\w25\lib26.ps1
# my own server and code sink (own tunnel ports; 3041 on this VM leads to another builder's server)
$global:Base = 'http://127.0.0.1:3051'
$global:CodeUrl = 'http://127.0.0.1:47736/code?to='
$global:t0 = Get-Date
. C:\bb\rec\clean-state.ps1
$setup = 'C:\bb\w27as\installer\Beebeeb Test_0.9.0_x64-setup.exe'
"installer sha256: $((Get-FileHash $setup).Hash)"
$p = Start-Process -FilePath $setup -ArgumentList '/S' -PassThru -Wait; "installer exit $($p.ExitCode)"; Start-Sleep 3
Remove-Item C:\bb\rec\chapters.log -ea 0
$stamp = Get-Date -Format 'HHmm'
$global:Root = Join-Path $env:USERPROFILE 'Beebeeb Test'
$global:Log = "$env:LOCALAPPDATA\io.beebeeb.app.test\logs\beebeeb-drive.log"
function global:Stop-Run([string]$why) { N "STOPPED: $why"; S 'stopped'; Get-Content C:\bb\rec\chapters.log; exit 3 }
function global:Health { try { $null = Invoke-WebRequest -UseBasicParsing "$($global:Base)/health" -TimeoutSec 3; $true } catch { $false } }
function global:Names { if (Test-Path -LiteralPath $global:Root) { @(Get-ChildItem -LiteralPath $global:Root -Force -Recurse -ea 0 | % { $_.FullName.Substring($global:Root.Length + 1) }) } else { @() } }
function global:Files-Texts { $null = Front; $null = Click-Name 'Files' 'Button' 5; Beat 1.5; Back-To-Files; Beat 1.5; (Texts2) -join ' ; ' }
function global:SwitchLines { @(Select-String -LiteralPath $global:Log -Pattern 'account switch|sync did not start' -ea 0 | % { $_.Line }) }
function global:Wait-Files([string[]]$need, [int]$sec) { $t = Get-Date; $ft = ''
  while (((Get-Date) - $t).TotalSeconds -lt $sec) { $ft = Files-Texts; $all = $true; foreach ($n in $need) { if ($ft -notmatch [regex]::Escape($n)) { $all = $false } }; if ($all -and $ft -match 'Up to date') { return $ft }; Beat 4 }
  '' }
function global:New-Account([string]$email, [string]$dir) { foreach ($l in (SignUp $email $dir)) { N $l }
  $global:Dialog = ''; $t = Texts2; if ($t -match "^Sync can.t start on this PC") { $global:Dialog = ($t | Select-Object -Last 4) -join ' ; '; N "a dialog came up: $global:Dialog"; S 'refusal-dialog'; $null = Click-Name 'Close' 'Button' 5; Beat 2
    FirstRun2 1200 | Out-Null; Beat 2; $null = Click-Name 'Got it' 'Button' 4; Beat 1; $null = Click-Rx '^Skip' '' 2; Beat 2 }
  $who = Who; if ($who -ne $email) { S 'signup-failed'; Stop-Run "sign-up of $email did not end signed in (screen shows '$who'): $((Texts2 | Select-Object -Last 6) -join ' ; ')" } }
function global:Add-Files([string]$tag) {
  for ($i = 0; $i -lt 60 -and -not (Test-Path -LiteralPath $global:Root); $i++) { Pump 1000 }
  foreach ($i in 1, 2) { Set-Content -LiteralPath (Join-Path $global:Root "W27 $tag secret $i.txt") -Value "w27 $tag file $i" }
  New-Item -ItemType Directory -Force (Join-Path $global:Root "W27 $tag folder") | Out-Null
  Set-Content -LiteralPath (Join-Path $global:Root "W27 $tag folder\inside.txt") -Value "inside $tag"
  $ft = Wait-Files @("W27 $tag secret 1", "W27 $tag secret 2", "W27 $tag folder") 180; S "$tag-files"
  N "$tag folder holds: $((Names) -join ' | ')"; N "$tag Files: $ft"
  if (-not $ft) { Stop-Run "the files of $tag were not all uploaded (3 names in Files and 'Up to date') within 180 s: $(Files-Texts)" }
  Beat 20 }
if (-not (Health)) { Stop-Run 'my server does not answer through the tunnel' }
"app: $(Start-App2)"; Beat 6
function global:Install-Clean { . C:\bb\rec\clean-state.ps1; $p = Start-Process -FilePath $setup -ArgumentList '/S' -PassThru -Wait; Start-Sleep 3; Remove-Item $global:Log -ea 0; N "clean install: $(Start-App2)"; Beat 6 }
# the sign-out of lib26, with what the app says on the way
function global:SignOut-Say { Clear-Editor; $null = Front; $null = Click-Name 'Files' 'Button' 4; Beat 0.8; $null = Click-Name 'Settings' 'Button' 6; Beat 1.5; $null = Click-Name 'Security' 'Hyperlink' 10; Beat 2; $null = Click-Rx '^Sign out of Drive' 'Hyperlink' 10; Beat 2.5
  N "sign-out asks: $((Texts2 | Select-Object -Last 8) -join ' ; ')"; $null = Click-Rx '^Sign out' 'Button' 10; Beat 8; S 'signout-answer'
  N "after Sign out: $((Texts2 | Select-Object -Last 10) -join ' ; ')"
  Start-Process $global:Installed -ArgumentList '--quit-for-update' -Wait -WindowStyle Hidden; Beat 2; $null = Start-App2; Beat 7
  $still = [bool](Find-El 'Settings' 'Button'); "signed out: $(-not $still)" }
function global:Refused([string]$case, [string]$who, [string]$file, [int]$before, [string]$kind) {
  Beat 15; $ft = Files-Texts; S "$case-refused"; $names = (Names) -join ' | '; $new = @(SwitchLines | Select-Object -Skip $before)
  N "$case Files of ${who}: $ft"; N "$case folder holds: $names"; N "$case start, log: $($new -join ' // ')"
  Res "W27 $case : $who is told in words that these files are not synced" (("$global:Dialog ; $ft") -match 'confirm that the files on this PC belong|confirmed as belonging|Sync is off') "dialog: $global:Dialog ;; Files: $ft"
  Res "W27 $case : $who sees no name of the first account in Files" ($ft -notmatch 'W27 (X|Y|A|B|C)') $ft
  Res "W27 $case : the unsent file is still in the folder" ($names -match [regex]::Escape($file)) "folder='$names'"
  Res "W27 $case : the log says the data stays ($kind), nothing removed" ((@($new | ? { $_ -match "local data stays why=$kind" }).Count -ge 1) -and (@($new | ? { $_ -match 'was removed' }).Count -eq 0)) ($new -join ' // ') }
New-Account "w27-ay-$stamp@example.org" 'C:\bb\w27\secretAY'
$clean = $true
# A: Y (signed in, clean) adds a file, all uploaded, signs out; then a file is put in the folder with nobody signed in; P signs up.
if ($clean) {
  Add-Files 'A'
  N "A Y $(SignOut-Say)"
  Set-Content -LiteralPath (Join-Path $global:Root 'W27 A put here signed out.txt') -Value 'nobody was signed in'
  $before = @(SwitchLines).Count
  New-Account "w27-ap-$stamp@example.org" 'C:\bb\w27\secretAP'
  Refused 'A (a file put in the folder while signed out)' 'P' 'W27 A put here signed out.txt' $before "the folder.s items stay"
} else { N 'A skipped: Y of case C is not on a clean start' }
# B: X signs up, a file uploaded; the server goes away; X adds a file (queued, not sent); X tries to sign out; the session is lost; Q signs up.
Install-Clean
New-Account "w27-bx-$stamp@example.org" 'C:\bb\w27\secretBX'
Add-Files 'B'
N 'READY-FOR-API-DOWN'; $t = Get-Date; $down = $false
while (((Get-Date) - $t).TotalSeconds -lt 120 -and -not $down) { $down = -not (Health); Pump 1000 }
N "server down seen: $down"; if (-not $down) { Stop-Run 'the server did not go away' }
Set-Content -LiteralPath (Join-Path $global:Root 'W27 B unsent.txt') -Value 'queued, not sent'; Beat 30
N "B Files with the server away: $(Files-Texts)"; S 'b-unsent'; N "B folder: $((Names) -join ' | ')"
$so = SignOut-Say; N "B X with the server away, $so"
if ($so -match 'signed out: False') {
  # the app keeps X signed in. The state the guard is for (signed out, work queued) is made the other way a session ends:
  # the app is closed and its stored session (TEST credentials of this app only) is gone.
  Start-Process $global:Installed -ArgumentList '--quit-for-update' -Wait -WindowStyle Hidden; Beat 3
  $t = @([N]::Targets('io.beebeeb.app.test/*')); $gone = @($t | ? { [N]::DeleteGeneric($_) }).Count; N "B stored session removed with the app closed: $gone of $($t.Count) TEST credentials"
}
N 'READY-FOR-API-UP'; $t = Get-Date; $up = $false
while (((Get-Date) - $t).TotalSeconds -lt 120 -and -not $up) { $up = Health; Pump 1000 }
N "server up seen: $up"; if (-not $up) { Stop-Run 'the server did not come back' }
if ($so -match 'signed out: False') { $null = Start-App2; Beat 8; if (Find-El 'Settings' 'Button') { Stop-Run "X is still signed in: $((Texts2 | Select-Object -Last 8) -join ' ; ')" } }
N "B folder before Q: $((Names) -join ' | ')"
$before = @(SwitchLines).Count
New-Account "w27-bq-$stamp@example.org" 'C:\bb\w27\secretBQ'
Refused 'B (a change queued with the server away)' 'Q' 'W27 B unsent.txt' $before '(queued change|staged copy|upload to finish|file not at rest)'
Get-Content C:\bb\rec\chapters.log

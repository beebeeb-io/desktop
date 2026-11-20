# w27 account-switch proof, part 2 (not product code): work that is not sent keeps the next account out, with words, nothing deleted.
# C: the w26 case (a document made and saved in the editor). A: a file put in the folder while signed out. B: a change queued with the server away.
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
  Res "W27 $case : $who is told in words that these files are not synced" ($ft -match 'confirmed as belonging|Sync is off|can.t start|haven.t been sent') $ft
  Res "W27 $case : $who sees no name of the first account in Files" ($ft -notmatch 'W27 (X|Y|A|B|C)') $ft
  Res "W27 $case : the unsent file is still in the folder" ($names -match [regex]::Escape($file)) "folder='$names'"
  Res "W27 $case : the log says the data stays ($kind), nothing removed" ((@($new | ? { $_ -match "local data stays why=$kind" }).Count -ge 1) -and (@($new | ? { $_ -match 'was removed' }).Count -eq 0)) ($new -join ' // ') }
# C: the w26 case. X makes a folder and a document in the app, types, saves, leaves the editor, signs out; Y signs up.
$cx = "w27-cx-$stamp@example.org"; $cy = "w27-cy-$stamp@example.org"; $clean = $false
New-Account $cx 'C:\bb\w27\secretCX'
P 'c-folder' { Back-To-Files; Beat 1; $null = Click-Name 'New' 'Button' 5; Beat 1.2; N "C new folder item: $(Click-Rx '^New folder' 'MenuItem' 4)"; Beat 1.5
  N "C name: $(TypeNew 'Folder name' 'W27 C Plans')"; Beat 1; $c = Click-Rx '^(Create|Create folder)$' 'Button' 3; if ($c -notmatch '^clicked') { Send '{ENTER}' }; Beat 5 }
$saved = $false
P 'c-doc' { $null = DClick-Name 'W27 C Plans' 'Text' 8; Beat 2; $c = CreateDoc 'W27 C Budget'; N "C doc: opened in $($c.secs) s '$($c.status)' err=$($c.err)"
  if ($c.status) { Type-Slow 'Numbers of C. '; Beat 1; $sv = Save-Verified 'c-doc'; N "C doc save: $($sv.ok) '$($sv.status)'"; $script:saved = [bool]$sv.ok; $global:CSaved = [bool]$sv.ok }; $null = Leave2; Beat 1 }
Beat 20; N "C Files before sign-out: $(Files-Texts)"; N "C folder holds: $((Names) -join ' | ')"
$before = @(SwitchLines).Count
N "C X $(SignOut-Say)"
New-Account $cy 'C:\bb\w27\secretCY'
Beat 12; $ft = Files-Texts; S 'c-y-files'; $names = (Names) -join ' | '; $new = @(SwitchLines | Select-Object -Skip $before)
N "C Files of Y: $ft"; N "C folder holds: $names"; N "C start of Y, log: $($new -join ' // ')"
$removed = (@($new | ? { $_ -match 'local data was removed' }).Count -ge 1)
N "C the save in the editor reported saved: $([bool]$global:CSaved)"
Res 'W27 C (a document made in the app''s editor): Y is signed in, Files is up, none of X''s names' (($ft -match [regex]::Escape($cy)) -and ($ft -match 'Up to date') -and ($ft -notmatch 'W27 C') -and ($names -notmatch 'W27 C') -and $removed) "folder='$names' log=$($new -join ' // ')"
$clean = $removed
Get-Content C:\bb\rec\chapters.log

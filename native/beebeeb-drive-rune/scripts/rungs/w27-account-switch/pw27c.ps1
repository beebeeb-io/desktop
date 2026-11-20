# w27 account-switch proof (not product code). Pair k (2 times): X signs up, adds files, all uploaded, signs out; Y signs up -> Y's Files and sync work, none of X's names.
# Then unsent work: Y2 adds a file with the server away (queued), signs out; Q signs up -> refused with words, the file still there.
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
foreach ($k in 1, 2) {
  $x = "w27-x$k-$stamp@example.org"; $y = "w27-y$k-$stamp@example.org"
  New-Account $x "C:\bb\w27\secretX$k"
  N "start of X$k, log: $((SwitchLines | Select-Object -Last 1))"
  Add-Files "X$k"
  if ($k -eq 2) { P 'x2-app-folder' { Back-To-Files; Beat 1; $null = Click-Name 'New' 'Button' 5; Beat 1.2; N "new folder item: $(Click-Rx '^New folder' 'MenuItem' 4)"; Beat 1.5
      N "name: $(TypeNew 'Folder name' 'W27 X2 made in app')"; Beat 1; $c = Click-Rx '^(Create|Create folder)$' 'Button' 3; if ($c -notmatch '^clicked') { Send '{ENTER}' }; Beat 6
      $ft = Wait-Files @('W27 X2 made in app') 60; N "X2 folder made in the app shows, Up to date: $([bool]$ft)"; N "X2 folder holds: $((Names) -join ' | ')" } }
  $before = @(SwitchLines).Count
  N (SignOut3)
  N "signed out, folder holds: $((Names) -join ' | ')"
  New-Account $y "C:\bb\w27\secretY$k"
  Beat 10; $ft = Files-Texts; S "y$k-files"; $names = (Names) -join ' | '; $lines = @(SwitchLines); $new = @($lines | Select-Object -Skip $before)
  N "Y$k Files: $ft"; N "Y$k folder holds: $names"; N "Y$k start, log: $($new -join ' // ')"
  Res "W27 pair $k : Y is signed in and Files is up (its email, Up to date, no refusal)" (($ft -match [regex]::Escape($y)) -and ($ft -match 'Up to date') -and ($ft -notmatch 'confirmed as belonging|Sync is off|can.t start')) $ft
  Res "W27 pair $k : Files of Y shows no name of X, the folder holds none" (($ft -notmatch 'W27 X') -and ($names -notmatch 'W27 X')) "folder='$names'"
  Res "W27 pair $k : the log says the previous account's data was removed" ((@($new | ? { $_ -match 'local data was removed' }).Count -ge 1) -and (@($new | ? { $_ -match 'stays|did not start' }).Count -eq 0)) ($new -join ' // ')
  Set-Content -LiteralPath (Join-Path $global:Root "W27 Y$k own.txt") -Value "y$k"; $ft2 = Wait-Files @("W27 Y$k own") 120; S "y$k-own"
  Res "W27 pair $k : Y's own new file uploads and shows in Files" ([bool]$ft2) $(if ($ft2) { $ft2 } else { Files-Texts })
  $names = (Names) -join ' | '
  Res "W27 pair $k : no name of X after Y's sync" (((Files-Texts) -notmatch 'W27 X') -and ($names -notmatch 'W27 X')) "folder='$names'"
  if ($k -eq 1) { Beat 15; N (SignOut3) }
}
# unsent work: Y2 is signed in; the server goes away, Y2 adds a file (queued, not sent), signs out; the server is back; Q signs up
Get-Content C:\bb\rec\chapters.log

# dc-086 probe (diagnosis only, not product code): the installed TEST build signed in to the fake server seeded
# with LargeAccount::GUUS. Times, counts and sizes only. Runs in the VM's desktop session.
$RUN = 2; $IDLE_MIN = 30; $BACKUP = $true
. C:\bb\dc086\rec-lib.ps1
. C:\bb\dc086\ks.ps1
$ErrorActionPreference = 'Continue'
$O = $env:BB_OUT
$Installed = "$env:LOCALAPPDATA\Beebeeb Test\beebeeb-desktop-test.exe"
$Root = Join-Path $env:USERPROFILE 'Beebeeb Test'
$LogDir = "$env:LOCALAPPDATA\io.beebeeb.app.test\logs"
$Setup = 'C:\bb\dc086\installer\Beebeeb Test_0.9.0_x64-setup.exe'
$Switch = 'C:\bb\dc086\backups.on'
$Expected = 41510   # 38,764 files + 2,739 folders + the fixture's 7 sample items
$global:t0 = Get-Date
function L([string]$s) { $line = '{0} +{1,7:0.0}s {2}' -f (Get-Date).ToUniversalTime().ToString('HH:mm:ss.fff'), ((Get-Date) - $global:t0).TotalSeconds, $s; Add-Content -LiteralPath "$O\marks.log" -Value $line -Encoding UTF8; Write-Host $line }
$global:nextWU = (Get-Date).AddSeconds(-1); $global:NoJiggle = $false; $global:nextJig = Get-Date
function Pump([int]$ms) { $u = (Get-Date).AddMilliseconds($ms); while ((Get-Date) -lt $u) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 50 }
  if ((Get-Date) -gt $global:nextWU) { $global:nextWU = (Get-Date).AddSeconds(8)
    foreach ($n in 'PickerHost', 'OpenWith') { foreach ($p in @(Get-Process $n -ea 0)) { Stop-Process -Id $p.Id -Force -ea 0; L "closed a stray $n window" } } }
  if (-not $global:NoJiggle -and (Get-Date) -gt $global:nextJig) { $global:nextJig = (Get-Date).AddSeconds(45); [M]::mouse_event(1, 1, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 30; [M]::mouse_event(1, -1, 0, 0, [IntPtr]::Zero) } }
function Texts2 { @((Dump '.') | ? { $_ -match '^Text\|' } | % { ($_ -split '\|')[1] }) }
function Btns2 { @((Dump '.') | ? { $_ -match '^Button\|' } | % { ($_ -split '\|')[1] }) }
function TypeNew([string]$name, [string]$text) { $f = Wait-El $name 'Edit' 15; if (-not $f) { return "no $name" }; Click-El $f | Out-Null; Send '^a'; Pump 150; Send '{DEL}'; Type-Slow $text; 'typed' }
function S([string]$n) { try { [M]::FullShot("$O\$n.png") } catch {} }
function Status-Word { $s = @((Dump '^(Up to date|Uploading|Syncing|Downloading|Paused|Sync is off|Backing up|Checking)') | ? { $_ -match '^Button\|' } | % { ($_ -split '\|')[1] }); if ($s.Count) { $s[0] } else { '' } }
function Count-Entries {
  $sw = [Diagnostics.Stopwatch]::StartNew(); $n = 0; $d = 0
  try { foreach ($e in [IO.Directory]::EnumerateFileSystemEntries($Root, '*', [IO.SearchOption]::AllDirectories)) { $n++ } } catch { }
  try { foreach ($e in [IO.Directory]::EnumerateDirectories($Root, '*', [IO.SearchOption]::AllDirectories)) { $d++ } } catch { }
  @($n, $d, $sw.ElapsedMilliseconds)
}
function App-Proc { @(Get-Process beebeeb-desktop-test -ea 0 | Sort-Object WorkingSet64 -Descending)[0] }
function App-Line { $p = App-Proc; if (-not $p) { return 'app: not running' }; $s = Get-Process -Id 4; "app pid=$($p.Id) handles=$($p.HandleCount) ws=$([int]($p.WorkingSet64/1MB))MB private=$([int]($p.PrivateMemorySize64/1MB))MB threads=$($p.Threads.Count) cpu=$([int]$p.TotalProcessorTime.TotalSeconds)s | System handles=$($s.HandleCount)" }
# handle counts by type for the app and for System, and the pool tags that grew most since the baseline
$global:TypeName = @{}; $global:Pool0 = $null
function Calibrate-Types {
  $me = [long]$PID; $keep = New-Object System.Collections.ArrayList
  $tmp = [IO.Path]::GetTempFileName()
  $steps = [ordered]@{
    File    = { 1..64 | % { [void]$keep.Add((New-Object IO.FileStream($tmp, 'Open', 'Read', 'ReadWrite'))) } }
    Event   = { 1..64 | % { [void]$keep.Add((New-Object Threading.ManualResetEvent($false))) } }
    Key     = { 1..64 | % { [void]$keep.Add([Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software')) } }
    Process = { 1..64 | % { $p = [Diagnostics.Process]::GetProcessById($PID); $null = $p.Handle; [void]$keep.Add($p) } }
    Section = { 1..64 | % { [void]$keep.Add([IO.MemoryMappedFiles.MemoryMappedFile]::CreateNew(('dc086-' + [guid]::NewGuid().ToString('N')), 4096)) } }
  }
  foreach ($k in $steps.Keys) { $a = [KS]::Handles($me); & $steps[$k]; $b = [KS]::Handles($me)
    foreach ($i in @($b.Keys)) { $was = 0L; if ($a.ContainsKey($i)) { $was = $a[$i] }; if (($b[$i] - $was) -ge 60) { $global:TypeName[$i] = $k } } }
  foreach ($x in $keep) { try { $x.Dispose() } catch {} }; Remove-Item $tmp -ea 0
  L "handle type indexes: $((@($global:TypeName.Keys) | % { "$_=$($global:TypeName[$_])" }) -join ' ')"
}
function Handles-Of([long]$id) { try { $h = [KS]::Handles($id); (@(@($h.Keys) | Sort-Object { -$h[$_] } | Select-Object -First 7 | % { $n = if ($global:TypeName[$_]) { $global:TypeName[$_] } else { "type$_" }; "$n=$($h[$_])" }) -join ' ') } catch { "failed: $($_.Exception.Message)" } }
function Snap([string]$tag) {
  $p = App-Proc
  L "SNAP $tag | $(App-Line)"
  if ($p) { L "SNAP $tag | app handles by type: $(Handles-Of $p.Id)" }
  L "SNAP $tag | System handles by type: $(Handles-Of 4)"
  try { $pool = [KS]::Pool()
    if (-not $global:Pool0) { $global:Pool0 = $pool; L "SNAP $tag | pool baseline taken ($($pool.Count) tags)" }
    else { $g = @(); foreach ($k in @($pool.Keys)) { $was = 0; if ($global:Pool0.ContainsKey($k)) { $z = $global:Pool0[$k]; $was = $z[1] + $z[3] }; $d = ($pool[$k][1] + $pool[$k][3]) - $was; $g += ,@($k, $d, ($pool[$k][0] + $pool[$k][2])) }
      L "SNAP $tag | pool tags grown since baseline (tag=+MB/live allocs): $((@($g | Sort-Object { -$_[1] } | Select-Object -First 10 | % { '{0}=+{1:0.0}/{2}' -f $_[0], ($_[1] / 1MB), $_[2] }) -join ' '))" } } catch { L "SNAP $tag | pool read failed: $($_.Exception.Message)" }
}
function Quit-App { if (Test-Path $Installed) { Start-Process $Installed -ArgumentList '--quit-for-update' -Wait -WindowStyle Hidden }; $u = (Get-Date).AddSeconds(30); while ((Get-Date) -lt $u -and (App-Proc)) { Pump 500 }; $p = App-Proc; if ($p) { Stop-Process -Id $p.Id -Force -ea 0; Pump 2000; 'stopped by PID after 30 s' } else { 'quit' } }
function Wipe {
  $q = Quit-App
  Get-Process beebeeb-media-helper, beebeeb-office-render -ea 0 | Stop-Process -Force
  $un = "$env:LOCALAPPDATA\Beebeeb Test\uninstall.exe"; $ux = ''
  if (Test-Path $un) { $p = Start-Process $un -ArgumentList '/S' -PassThru -Wait; $ux = $p.ExitCode; Pump 4000 }
  $had = Test-Path -LiteralPath $Root; $rm = 0
  if ($had) { if ([N]::ProviderOf($Root)) { $null = [N]::Unregister($Root) }
    $sw = [Diagnostics.Stopwatch]::StartNew(); cmd /c "rd /s /q `"$Root`"" 2>$null | Out-Null
    if (Test-Path -LiteralPath $Root) { Remove-Item -LiteralPath $Root -Recurse -Force -ea 0 }
    $rm = $sw.ElapsedMilliseconds }
  foreach ($d in (Join-Path $env:APPDATA 'beebeeb-test'), (Join-Path $env:LOCALAPPDATA 'io.beebeeb.app.test')) { if (Test-Path -LiteralPath $d) { Remove-Item -LiteralPath $d -Recurse -Force -ea 0 } }
  $t = @([N]::Targets('io.beebeeb.app.test/*')); $gone = @($t | ? { [N]::DeleteGeneric($_) }).Count
  "wipe: app $q; uninstall exit '$ux'; sync folder was there: $had (removed in $rm ms, gone: $(-not (Test-Path -LiteralPath $Root))); TEST credentials removed $gone of $($t.Count)"
}
function Explorer-Close2 { foreach ($r in @([N]::AllVisible() | ? { $_ -match '\|CabinetWClass\|' })) { [P]::Close([IntPtr][long](($r -split '\|')[0])) | Out-Null }; Pump 1500 }
function Explorer-Time([string]$path, [int]$want, [string]$tag) {
  $sh = New-Object -ComObject Shell.Application
  $before = @($sh.Windows() | % { try { $_.HWND } catch {} })
  $a0 = App-Proc; $ecpu0 = (@(Get-Process explorer -ea 0) | % { $_.TotalProcessorTime.TotalSeconds } | Measure-Object -Sum).Sum; $acpu0 = $a0.TotalProcessorTime.TotalSeconds; $ah0 = $a0.HandleCount
  $sw = [Diagnostics.Stopwatch]::StartNew()
  Start-Process explorer.exe "`"$path`""
  $w = $null; $tWin = -1; $tCount = -1; $tReady = -1; $tBar = -1; $n = 0; $bar = ''; $hw = [IntPtr]::Zero
  while ($sw.Elapsed.TotalSeconds -lt 240) {
    if (-not $w) { foreach ($x in @($sh.Windows())) { try { if ($before -notcontains $x.HWND -and $x.Document.Folder.Self.Path -eq $path) { $w = $x; $tWin = $sw.ElapsedMilliseconds; $hw = [IntPtr][long]$x.HWND } } catch {} } }
    if ($w) {
      if ($tCount -lt 0) { try { $n = $w.Document.Folder.Items().Count } catch { $n = -1 }; if ($n -ge $want) { $tCount = $sw.ElapsedMilliseconds } }
      if ($tReady -lt 0) { try { if (-not $w.Busy -and $w.ReadyState -eq 4) { $tReady = $sw.ElapsedMilliseconds } } catch {} }
      if ($tBar -lt 0) { try {
          $r = [System.Windows.Automation.AutomationElement]::FromHandle($hw)
          $c = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::StatusBar)
          $sb = $r.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $c)
          if ($sb) { foreach ($k in @($sb.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition))) { $nm = $k.Current.Name; if ($nm -match '^\s*[\d.,\s]+items?\b') { $bar = $nm; if ((($nm -replace '[^\d]', '') -as [int]) -ge $want) { $tBar = $sw.ElapsedMilliseconds } } } }
        } catch {} }
      if ($tCount -ge 0 -and $tReady -ge 0 -and ($tBar -ge 0 -or $sw.Elapsed.TotalSeconds -gt 90)) { break }
    }
    Start-Sleep -Milliseconds 100
  }
  Pump 1500; S "explorer-$tag"
  $a1 = App-Proc; $ecpu1 = (@(Get-Process explorer -ea 0) | % { $_.TotalProcessorTime.TotalSeconds } | Measure-Object -Sum).Sum
  L ("EXPLORER $tag | window after {0} ms | shell folder reports {1} items after {2} ms | view ready after {3} ms | status bar '{4}' after {5} ms | explorer CPU +{6:0.0} s, app CPU +{7:0.0} s, app handles {8} -> {9}" -f $tWin, $n, $tCount, $tReady, $bar, $tBar, ($ecpu1 - $ecpu0), ($a1.TotalProcessorTime.TotalSeconds - $acpu0), $ah0, $a1.HandleCount)
}
function Threads([string]$tag, [int]$sec = 5) { try { foreach ($l in @(& powershell -NoProfile -ExecutionPolicy Bypass -File C:\bb\dc086\threads2.ps1 -Seconds $sec 2>&1 | Select-Object -First 7)) { L "THREADS $tag | $l" } } catch { L "THREADS $tag failed" } }
function Log-Lines([string]$rx, [int]$last = 40) { $f = @(Get-ChildItem -LiteralPath $LogDir -Filter *.log -ea 0 | Sort-Object LastWriteTime | Select-Object -Last 1)[0]; if ($f) { @(Select-String -LiteralPath $f.FullName -Pattern $rx | Select-Object -Last $last | % { $_.Line -replace '^(.{0,300}).*$', '$1' }) } else { @() } }
# Sign-in to the main window: the email, the password, the twelve words, every set-up step answered "not now".
function Sign-In([string]$email, [string]$pw, [string]$phrase) {
  for ($i = 0; $i -lt 4; $i++) { $null = Click-Name 'Sign in' 'Button' 10; Pump 1500; if (@(Texts2 | ? { $_ -match '^Sign in to Drive$' }).Count) { break } }
  L "sign-in form: email $(TypeNew 'Email' $email), password $(TypeNew 'Password' $pw)"
  Pump 400; $global:tSign = Get-Date; Send '{ENTER}'; L 'SIGNIN password sent (t = 0 of the population)'
  $asked = $false; $seen = @{}; $main = $false; $next = Get-Date; $nowin = 0
  for ($i = 0; $i -lt 400 -and -not $main; $i++) {
    Pump 500
    if ((Get-Date) -ge $next) { $next = (Get-Date).AddSeconds(10); if (Test-Path -LiteralPath $Root) { $c = Count-Entries; L "entries=$($c[0]) dirs=$($c[1]) (listing took $($c[2]) ms) [during set-up, t=$([Math]::Round(((Get-Date) - $global:tSign).TotalSeconds, 1)) s]" } }
    if (-not $asked -and (Find-El 'Recovery word 1' 'Edit')) {
      $asked = $true; L 'twelve words asked'
      [System.Windows.Forms.Clipboard]::SetText($phrase); Click-El (Find-El 'Recovery word 1' 'Edit') | Out-Null; Send '^v'; Pump 1200; [System.Windows.Forms.Clipboard]::Clear()
      $null = Click-Name 'Restore keys' 'Button' 10; L 'Restore keys pressed'; continue
    }
    $b = Btns2
    if ($b -contains 'Settings' -and $b -contains 'New') { $main = $true; break }
    if (-not @(App-Windows).Count) { $nowin++; if ($nowin -gt 40) { L 'the app has no window for 40 polls; giving up'; break } } else { $nowin = 0 }
    if (@(Prompt-Rows).Count) { L "Windows Hello prompt: $(Close-Credential)"; continue }
    $act = ''
    foreach ($a in 'Use this folder', 'Not now', 'Continue for now', 'Open my Beebeeb', 'Continue', 'Got it') { if ($b -contains $a) { $act = $a; break } }
    if ($act) { $k = "$act|$(($b | Select-Object -First 6) -join ',')"; $seen[$k]++; if ($seen[$k] -gt 6) { L "stuck: '$act' pressed 6 times; buttons: $($b -join ', ')"; S 'stuck'; break }
      $title = @(Texts2 | Select-Object -First 3) -join ' / '; L "set-up: '$title' -> $act"; Pump 400; $null = Click-Name $act 'Button' 3; Pump 800
      if (@(Prompt-Rows).Count) { Pump 1500; L "Windows Hello prompt: $(Close-Credential)" } }
  }
  $null = Click-Name 'Got it' 'Button' 2
  $ws = Window-State; if ($ws -match '(\d+)x(\d+)' -and [int]$matches[2] -lt 740) { $null = Click-Name 'Maximize' 'Button' 4; Pump 1200 }
  L "SIGNIN main window: $main after $([Math]::Round(((Get-Date) - $global:tSign).TotalSeconds, 1)) s (words asked: $asked); $(Window-State)"
  S 'signed-in'
  $main
}

# ---------------------------------------------------------------------------------------------------------------
L "=== dc-086 run $RUN (idle $IDLE_MIN min, backup $BACKUP) on $env:COMPUTERNAME; Windows $([Environment]::OSVersion.Version); $((Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors) CPUs, $([int]((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB)) GB"
Calibrate-Types
Remove-Item $Switch -ea 0
$was = if (Test-Path $Installed) { (Get-FileHash $Installed).Hash.Substring(0, 16) } else { 'none' }
L "TEST app found before the run: exe $was, sync folder: $(Test-Path -LiteralPath $Root), TEST credentials: $(@([N]::Targets('io.beebeeb.app.test/*')).Count)"
L (Wipe)
$p = Start-Process -FilePath $Setup -ArgumentList '/S' -PassThru -Wait; Start-Sleep 3
L "installed: exit $($p.ExitCode), exe sha256 $((Get-FileHash $Installed).Hash)"
if ($BACKUP) {
  $docs = [Environment]::GetFolderPath('MyDocuments'); $src = Join-Path $docs 'dc086-src'
  if (-not (Test-Path $src)) { foreach ($d in 0..29) { $dir = Join-Path $src ('d{0:00}' -f $d); New-Item -ItemType Directory -Force $dir | Out-Null; foreach ($f in 0..99) { Set-Content -LiteralPath (Join-Path $dir ('f{0:000}.txt' -f $f)) -Value ("dc086 backup source file $d/$f " + ('x' * (200 + 13 * $f))) } } }
  $all = @(Get-ChildItem -LiteralPath $docs -Recurse -File -Force -ea 0)
  L "backup source: Documents holds $($all.Count) files, $([int](($all | Measure-Object Length -Sum).Sum / 1KB)) KB"
}
# the fake
$fakeOut = "$O\fake.out.txt"; $fakeErr = "$O\fake-requests.txt"
$fake = Start-Process -FilePath C:\bb\dc086\dc086_large_fake.exe -ArgumentList '127.0.0.1:47486', $Switch -PassThru -WindowStyle Hidden -RedirectStandardOutput $fakeOut -RedirectStandardError $fakeErr
$lines = @(); for ($i = 0; $i -lt 480 -and $lines.Count -lt 4; $i++) { Pump 250; $lines = @(Get-Content -LiteralPath $fakeOut -ea 0) }
$field = { param($name) ($lines | ? { $_ -like "${name}: *" } | Select-Object -First 1) -replace "^${name}: ", '' }
$base = & $field 'base'; $email = & $field 'email'; $pw = & $field 'password'; $phrase = & $field 'phrase'
if ($base -notmatch '^http://127\.0\.0\.1:\d+$') { L "the fake did not start (base '$base'); stopping"; exit 3 }
L "fake at $base (PID $($fake.Id)); $(@(Get-Content -LiteralPath $fakeErr -ea 0 | ? { $_ -match 'dc086:' }) -join ' ; ')"
$sampler = Start-Process powershell -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'C:\bb\dc086\sampler.ps1', '-Out', "$O\samples.csv" -PassThru -WindowStyle Hidden
Snap 'before-app-start'
# the app
foreach ($k in 'HTTPS_PROXY', 'HTTP_PROXY', 'ALL_PROXY') { [Environment]::SetEnvironmentVariable($k, 'http://127.0.0.1:9', 'Process') }; [Environment]::SetEnvironmentVariable('NO_PROXY', '', 'Process')
$tStart = Get-Date; $ap = Start-Process -FilePath $Installed -ArgumentList @('--api-base', $base) -PassThru
$u = (Get-Date).AddSeconds(60); while ((Get-Date) -lt $u -and -not @(App-Windows).Count) { Pump 300 }
L "app started PID $($ap.Id); window after $([Math]::Round(((Get-Date) - $tStart).TotalSeconds, 1)) s"; Pump 3000
$global:tSign = Get-Date
$ok = Sign-In $email $pw $phrase
$pw = ''; $phrase = ''
if (-not $ok) { L "sign-in did not reach the main window; texts: $((Texts2 | Select-Object -First 25) -join ' ; ') || buttons: $((Btns2) -join ' ; ')"; S 'signin-failed' }
Snap 'main-window'
L "display adapter: $((Get-CimInstance Win32_VideoController | % { $_.Name }) -join ' ; '); render modules in the app: $((@((App-Proc).Modules | ? { $_.ModuleName -match 'warp|d3d1|dxgi|vulkan|opengl|dcomp' } | % { $_.ModuleName }) -join ' '))"
$thr = @{}
# ---- population: entries in the sync folder every 10 s until every placeholder is there
$tFirst = $null; $tDone = $null; $tIdleWord = $null; $last = -1; $lastChange = Get-Date; $word = ''; $i = 0
while ($ok -and ((Get-Date) - $global:tSign).TotalMinutes -lt 60) {
  $c = Count-Entries; $el = [Math]::Round(((Get-Date) - $global:tSign).TotalSeconds, 1)
  if ($c[0] -gt 0 -and -not $tFirst) { $tFirst = $el }
  if ($c[0] -ne $last) { $last = $c[0]; $lastChange = Get-Date }
  if (($i % 3) -eq 0) { $word = Status-Word; if ($word -eq 'Up to date' -and -not $tIdleWord) { $tIdleWord = $el } }
  L "POP t=$el s entries=$($c[0]) dirs=$($c[1]) files=$($c[0] - $c[1]) (listing took $($c[2]) ms) status='$word' | $(App-Line)"
  if ($c[0] -ge $Expected) { $tDone = $el; break }
  if ($c[0] -gt 0 -and ((Get-Date) - $lastChange).TotalMinutes -ge 6) { L "POP no new entry for 6 minutes at $($c[0]) of $Expected; going on"; break }
  if (-not $thr['pull-5min'] -and $c[0] -eq 0 -and $el -gt 300) { $thr['pull-5min'] = 1; Threads 'first pull, 5 min in' }
  if (-not $thr['pull-12min'] -and $c[0] -eq 0 -and $el -gt 720) { $thr['pull-12min'] = 1; Threads 'first pull, 12 min in' }
  if (-not $thr['placing'] -and $c[0] -gt 8000) { $thr['placing'] = 1; Threads 'placeholders being made' }
  $i++; Pump 10000
}
L "POPULATION first entry at $tFirst s; every placeholder ($Expected) at $tDone s; last count $last; 'Up to date' first read at $tIdleWord s (all from the password)"
S 'population-done'; Snap 'population-done'
foreach ($l in (Log-Lines 'engine phase timing' 60)) { L "phase: $l" }
if ($false) { # dc-163: first pull and population only
  # the app's own word for done
  $u = (Get-Date).AddMinutes(6); while ((Get-Date) -lt $u) { $word = Status-Word; if ($word -eq 'Up to date') { break }; Pump 10000 }
  L "locked now: $([bool]@(Texts2 | ? { $_ -match '^Beebeeb is locked$' }).Count)"
  L "status after the population: '$word' at $([Math]::Round(((Get-Date) - $global:tSign).TotalSeconds, 1)) s; $(App-Line)"
  $c = Count-Entries; L "entries now $($c[0]) (dirs $($c[1]); listing took $($c[2]) ms)"
  # ---- Explorer on the 5,000-item folder
  $roll = Join-Path $Root 'Camera Roll'
  if (Test-Path -LiteralPath $roll) {
    $sw = [Diagnostics.Stopwatch]::StartNew(); $n = @([IO.Directory]::GetFileSystemEntries($roll)).Count; L "plain listing of Camera Roll: $n entries in $($sw.ElapsedMilliseconds) ms"
    Explorer-Close2
    Explorer-Time $roll 5000 'cold'; Pump 8000; Explorer-Close2; Pump 3000
    Explorer-Time $roll 5000 'warm'; Pump 5000; Explorer-Close2
    Explorer-Time $Root 1 'root'; Pump 5000; Explorer-Close2
  } else { L 'no Camera Roll folder: Explorer not measured' }
  Snap 'after-explorer'
  # ---- idle
  L "IDLE start ($IDLE_MIN min)"
  # Minutes 1-12: the session gets a 1-pixel mouse move every 45 s, so the app's lock (5 minutes without input) stays
  # away. From minute 12 no input: the app locks itself about minute 17. Minimised from minute 25 to 29.
  $locked = $false; $hw = $null
  for ($m = 1; $m -le $IDLE_MIN; $m++) {
    if ($m -eq 4) { Threads 'idle, unlocked, window shown' 20 }
    if ($IDLE_MIN -ge 30) {
      if ($m -eq 12) { $global:NoJiggle = $true; L 'IDLE no more input from here (the app locks after 5 minutes without input)' }
      if ($m -eq 22) { Threads 'idle, locked, window shown' 20 }
      if ($m -eq 25) { $hw = @(App-Windows)[0]; if ($hw) { [M]::ShowWindow($hw, 6) | Out-Null; L 'IDLE window minimised' } }
      if ($m -eq 27) { Threads 'idle, locked, window minimised' 30 }
      if ($m -eq 29) { if ($hw) { [M]::ShowWindow($hw, 9) | Out-Null; L 'IDLE window restored' } }
    }
    Pump 60000
    if (-not $locked -and @(Texts2 | ? { $_ -match '^Beebeeb is locked$' }).Count) { $locked = $true; L "IDLE the app shows 'Beebeeb is locked' (first seen in minute $m)"; S 'locked' }
    if (($m % 5) -eq 0) { Snap "idle-$m-min" } else { L "IDLE $m min | status '$(Status-Word)' | $(App-Line)" } }
  L "IDLE end; status '$(Status-Word)'"; S 'idle-end'
}
# ---- a backup pass (Documents turned on in the settings file while the app is stopped, then a restart)
if ($ok -and $BACKUP) {
  Set-Content -LiteralPath $Switch -Value 'on'
  L "BACKUP app before the restart: $(Quit-App)"
  $toml = Join-Path $env:APPDATA 'beebeeb-test\desktop.toml'
  $t = Get-Content -LiteralPath $toml -Raw
  if ($t -match '(?m)^known_folder_backup\s*=.*$') { $t = [regex]::Replace($t, '(?m)^known_folder_backup\s*=.*$', 'known_folder_backup = ["documents"]') } else { $t = "known_folder_backup = [`"documents`"]`r`n" + $t }
  [IO.File]::WriteAllText($toml, $t, (New-Object Text.UTF8Encoding($false)))
  L "BACKUP settings file: $(@(Get-Content -LiteralPath $toml | ? { $_ -match '^known_folder' }) -join ' ; ')"
  $tRe = Get-Date; $ap = Start-Process -FilePath $Installed -ArgumentList @('--api-base', $base) -PassThru
  $u = (Get-Date).AddSeconds(90); while ((Get-Date) -lt $u -and -not @(App-Windows).Count) { Pump 300 }
  L "BACKUP app restarted PID $($ap.Id); window after $([Math]::Round(((Get-Date) - $tRe).TotalSeconds, 1)) s"
  $tMain = $null; $tUp = $null; $passes = 0
  for ($k = 0; $k -lt 150; $k++) {
    Pump 10000; $el = [Math]::Round(((Get-Date) - $tRe).TotalSeconds, 1)
    if (-not $tMain) { $b = Btns2; if ($b -contains 'Settings' -and $b -contains 'New') { $tMain = $el; L "BACKUP main window at $el s after the restart" } elseif (@(Prompt-Rows).Count) { L "Windows Hello prompt: $(Close-Credential)" } }
    $word = Status-Word; if ($word -eq 'Up to date' -and -not $tUp) { $tUp = $el; L "BACKUP 'Up to date' first read $el s after the restart" }
    $bl = @(Log-Lines 'backup' 400); $passes = @($bl | ? { $_ -match 'unchanged' }).Count
    if (($k % 6) -eq 5) { L "BACKUP t=$el s status='$word' backup log lines=$($bl.Count) pass lines=$passes | $(App-Line)" }
    if ($passes -ge 2 -or $el -gt 1200) { break }
  }
  foreach ($l in (Log-Lines 'backup|engine phase timing' 60)) { L "backup log: $l" }
  L "BACKUP requests the fake answered for uploads and the root: $(@(Get-Content -LiteralPath $fakeErr -ea 0 | ? { $_ -match 'uploads|backups/root' }).Count)"
  Snap 'backup-end'; S 'backup-end'
}
# ---- out
Copy-Item -LiteralPath $LogDir -Destination "$O\applogs" -Recurse -Force -ea 0
$db = @(Get-ChildItem -LiteralPath "$env:LOCALAPPDATA\io.beebeeb.app.test", (Join-Path $env:APPDATA 'beebeeb-test') -Recurse -File -ea 0 | ? { $_.Name -match '\.db' } | % { "$($_.Name)=$([Math]::Round($_.Length / 1MB, 1))MB" }) -join ' '
L "state files: $db"
L "requests the fake answered: $(@(Get-Content -LiteralPath $fakeErr -ea 0).Count)"
Stop-Process -Id $sampler.Id -Force -ea 0
L (Wipe)
Stop-Process -Id $fake.Id -Force -ea 0; Start-Sleep 2
Remove-Item -LiteralPath $fakeOut -Force -ea 0
Remove-Item $Switch -ea 0
L "done (fake output file removed: $(-not (Test-Path -LiteralPath $fakeOut)))"
exit 0

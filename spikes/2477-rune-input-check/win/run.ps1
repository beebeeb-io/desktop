# The Rune input check in a REAL window on Windows (task 2477, PR 0). Runs inside a logged-in desktop session:
#
#   cargo build --locked
#   powershell -NoProfile -File $env:BB_RUN_IN_DESKTOP -Script win\run.ps1 -TimeoutSec 300
#
# It starts the probe, drives it with synthetic OS input (SendInput: the keyboard and mouse path every program
# gets its input from), and checks the probe's own event log. Written to $env:BB_OUT:
#   events.log   every event the probe heard, in order
#   result.txt   one line per check (PASS / FAIL / SKIP) and the count line, which is also the last line printed
#   *.png        the screen at chosen moments
#
# The input-method leg runs only when a Japanese input method is installed for this account; without one every
# check of that leg is SKIP, never PASS. Exit code: the number of failing checks.
param([string]$Exe = '')
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Win
{
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Explicit)] struct INPUTUNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public INPUTUNION u; }
    [DllImport("user32.dll", SetLastError = true)] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] static extern uint MapVirtualKey(uint code, uint type);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hwnd, ref POINT p);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int w, int h, uint flags);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr LoadKeyboardLayout(string id, uint flags);

    public static void Key(ushort vk, bool down) {
        var i = new INPUT { type = 1 };
        i.u.ki = new KEYBDINPUT { wVk = vk, wScan = (ushort)MapVirtualKey(vk, 0), dwFlags = down ? 0u : 2u };
        // Navigation keys are extended keys; without the flag Shift+Home arrives as the numeric pad's.
        if (vk >= 0x21 && vk <= 0x2E) i.u.ki.dwFlags |= 1u;
        SendInput(1, new[] { i }, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void Mouse(uint flags) {
        var i = new INPUT { type = 0 };
        i.u.mi = new MOUSEINPUT { dwFlags = flags };
        SendInput(1, new[] { i }, Marshal.SizeOf(typeof(INPUT)));
    }
}
'@
[void][Win]::SetProcessDPIAware()

$out = $env:BB_OUT
if (-not $out) { throw 'BB_OUT is not set: run this through the desktop-session runner' }
if (-not $Exe) { $Exe = Join-Path $env:CARGO_TARGET_DIR 'debug\rune-input-check.exe' }
if (-not (Test-Path $Exe)) { throw "the probe is not built: $Exe" }
$log = Join-Path $out 'events.log'
$results = New-Object System.Collections.Generic.List[string]
$script:fail = 0; $script:pass = 0; $script:skip = 0

function Check([string]$Name, [bool]$Ok, [string]$Detail = '') {
  if ($Ok) { $script:pass++ } else { $script:fail++ }
  $results.Add(('{0} {1}{2}' -f $(if ($Ok) { 'PASS' } else { 'FAIL' }), $Name, $(if ($Detail) { " :: $Detail" } else { '' })))
}
function Skip([string]$Name, [string]$Why) { $script:skip++; $results.Add("SKIP $Name :: $Why") }
function Lines { if (Test-Path $log) { @(Get-Content -LiteralPath $log -Encoding UTF8) } else { @() } }
# The probe polls every 50 ms; a quarter of a second is five polls.
function Wait([int]$Ms = 250) { Start-Sleep -Milliseconds $Ms }
function Since([int]$From, [string]$Pattern) { $l = Lines; if ($l.Count -le $From) { @() } else { @($l[$From..($l.Count - 1)] | Where-Object { $_ -match $Pattern }) } }
function Last([string]$Pattern) { Lines | Where-Object { $_ -match $Pattern } | Select-Object -Last 1 }
function Mark { (Lines).Count }
# Polls until the condition holds, at most $Ms: a slow first frame must not read as a missing event.
function Until([scriptblock]$Cond, [int]$Ms = 3000) { $end = (Get-Date).AddMilliseconds($Ms); while ((Get-Date) -lt $end) { if (& $Cond) { return }; Start-Sleep -Milliseconds 100 }; }
function Tap([int]$Vk) { [Win]::Key($Vk, $true); [Win]::Key($Vk, $false); Start-Sleep -Milliseconds 60 }
function Chord([int]$Mod, [int]$Vk) { [Win]::Key($Mod, $true); Tap $Vk; [Win]::Key($Mod, $false); Start-Sleep -Milliseconds 60 }
function TypeKeys([string]$Letters) { foreach ($c in $Letters.ToUpper().ToCharArray()) { Tap ([int]$c) } }
function Shot([string]$Name) {
  $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  $bmp = New-Object System.Drawing.Bitmap($b.Width, $b.Height)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  try { $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size); $bmp.Save((Join-Path $out "$Name.png"), [System.Drawing.Imaging.ImageFormat]::Png) }
  finally { $g.Dispose(); $bmp.Dispose() }
}
function State {
  $line = Last '^state '
  if ($line -notmatch '^state text="(?<text>.*)" caret=(?<caret>\d+) caret_cell=(?<cell>\d+) selection=\((?<a>\d+), (?<b>\d+)\) preedit="(?<pre>.*)" keyboard=(?<kb>true|false) canvas=(?<canvas>None|Some\(\[[^\]]*\]\)) caret_drawn=(?<drawn>None|Some\(\[[^\]]*\]\))$') { return $null }
  $nums = { param($s) if ($s -eq 'None') { $null } else { @([regex]::Matches($s, '-?\d+(\.\d+)?') | ForEach-Object { [double]::Parse($_.Value, [Globalization.CultureInfo]::InvariantCulture) }) } }
  [pscustomobject]@{ Text = $Matches.text; Caret = [int]$Matches.caret; Cell = [int]$Matches.cell; A = [int]$Matches.a; B = [int]$Matches.b; Preedit = $Matches.pre
    Keyboard = ($Matches.kb -eq 'true'); Canvas = (& $nums $Matches.canvas); Drawn = (& $nums $Matches.drawn) }
}

# src/lib.rs. PowerShell names ignore case: these must not collide with a parameter called $Cell.
$CellW = 16; $Pad = 16; $LineH = 24
$env:RUNE_INPUT_CHECK_LOG = $log
$proc = Start-Process -FilePath $Exe -PassThru
$hwnd = [IntPtr]::Zero
try {
  # By title: once an input method is installed the process owns other (hidden) top-level windows too.
  # [NullString]: PowerShell would pass $null to a string parameter as "", a class name nothing has.
  for ($i = 0; $i -lt 100 -and $hwnd -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 200; $hwnd = [Win]::FindWindow([NullString]::Value, 'Rune input check') }
  if ($hwnd -eq [IntPtr]::Zero) { throw 'the probe opened no window within 20 s' }
  # Top-most at a fixed place, so nothing (its own console window included) covers it. 0x1 = keep its size.
  [void][Win]::SetWindowPos($hwnd, [IntPtr](-1), 200, 200, 0, 0, 0x1)
  Wait 1000
  $scale = [Win]::GetDpiForWindow($hwnd) / 96.0
  $origin = New-Object Win+POINT
  [void][Win]::ClientToScreen($hwnd, [ref]$origin)
  $s = State
  Check 'the probe draws and reports its canvas' ($null -ne $s -and $null -ne $s.Canvas) ("state: " + (Last '^state '))
  if ($null -eq $s -or $null -eq $s.Canvas) { throw 'no canvas rectangle in the log: nothing to aim at' }
  $canvas = $s.Canvas
  # The screen pixel in front of `cell` on the canvas's line.
  function CellAt([int]$Cell) {
    @([int]($origin.X + ($canvas[0] + $Pad + $Cell * $CellW) * $scale), [int]($origin.Y + ($canvas[1] + $Pad + $LineH / 2) * $scale))
  }
  function MoveTo([int]$Cell) { $p = CellAt $Cell; [void][Win]::SetCursorPos($p[0], $p[1]); [Win]::Mouse(0x1); Start-Sleep -Milliseconds 60 }
  function Click([int]$Cell) { MoveTo $Cell; [Win]::Mouse(0x2); Start-Sleep -Milliseconds 40; [Win]::Mouse(0x4); Wait }

  # --- 5. focus and pointer press -------------------------------------------------------------------------
  $m = Mark
  Click 0
  $s = State
  Check 'a press focuses the canvas (focus line, caret drawn)' ((Since $m '^focus canvas=true$').Count -eq 1 -and $s.Keyboard -and $null -ne $s.Drawn) ("state: " + (Last '^state '))
  Check 'the press arrives with canvas-local coordinates' ((Since $m "^pointer Down x=$Pad y=$($Pad + $LineH / 2) clicks=1").Count -eq 1) ((Since $m '^pointer ') -join ' | ')
  Check 'the probe window is the foreground window' ([Win]::GetForegroundWindow() -eq $hwnd)

  # --- 1. keys and typed text ------------------------------------------------------------------------------
  $m = Mark
  TypeKeys 'hello'
  Until { (State).Text -eq 'hello' }
  $s = State
  Check 'five typed letters arrive as five keys, once each' ((Since $m '^key Text\(').Count -eq 5 -and $s.Text -eq 'hello') ("text=$($s.Text); " + ((Since $m '^key ') -join ' | '))
  Check 'typing sends no input-method commit' ((Since $m '^ime Commit').Count -eq 0) ((Since $m '^ime ') -join ' | ')
  $m = Mark
  Tap 0x25; Tap 0x08; Wait          # Left, Backspace
  $s = State
  Check 'Left and Backspace arrive as keys' ($s.Text -eq 'helo' -and $s.Caret -eq 3 -and (Since $m '^key (Left|Backspace) ').Count -eq 2) ("text=$($s.Text) caret=$($s.Caret)")
  TypeKeys 'l'; Tap 0x23; Wait           # back to "hello", End
  Shot '1-typed'

  # --- 3. the candidate position the platform holds ---------------------------------------------------------
  # Expected from the probe's MODEL (the cell the caret is in front of), not from what it was told was drawn.
  function CaretPx($st) { @((($canvas[0] + $Pad + $st.Cell * $CellW) * $scale), (($canvas[1] + $Pad) * $scale), ($LineH * $scale)) }
  function ImmAt($st) {
    $imm = Last '^imm '
    if ($imm -notmatch 'context=true .*candidate ok=(?<cok>\d+) style=(?<cs>0x[0-9a-f]+) pos=\((?<cx>-?\d+),(?<cy>-?\d+)\) area=\((?<l>-?\d+),(?<t>-?\d+),(?<r>-?\d+),(?<b>-?\d+)\) composition ok=(?<pok>\d+) style=(?<ps>0x[0-9a-f]+) pos=\((?<px>-?\d+),(?<py>-?\d+)\)') { return $false }
    $e = CaretPx $st; $x = $e[0]; $y = $e[1]; $h = $e[2]
    $near = { param($a, $b) [math]::Abs($a - $b) -le 1 }
    ($Matches.cok -ne '0') -and ($Matches.pok -ne '0') -and ($Matches.cs -eq '0x80') -and ($Matches.ps -eq '0x2') -and
      (& $near ([int]$Matches.cx) $x) -and (& $near ([int]$Matches.cy) $y) -and (& $near ([int]$Matches.l) $x) -and (& $near ([int]$Matches.t) $y) -and
      (& $near ([int]$Matches.b) ($y + $h)) -and (& $near ([int]$Matches.px) $x) -and (& $near ([int]$Matches.py) ($y + $h))
  }
  $s = State
  Check 'the input context holds the caret rectangle (candidate and composition forms)' ($s.Cell -eq 5 -and (ImmAt $s)) ("caret cell $($s.Cell), expected px $((CaretPx $s) -join ','); " + (Last '^imm '))
  $moves = 0; $right = 0
  foreach ($vk in 0x24, 0x27, 0x27, 0x23, 0x25) {   # Home, Right, Right, End, Left
    $was = (State).Cell; Tap $vk; Until { (State).Cell -ne $was } 1500; Wait 150
    $moves++; if (ImmAt (State)) { $right++ }
  }
  Check 'the forms follow the caret on every move' ($right -eq $moves) "$right of $moves moves; last: $(Last '^imm ')"
  Tap 0x23; Wait

  # --- 4. the OS clipboard ---------------------------------------------------------------------------------
  [System.Windows.Forms.Clipboard]::Clear()
  [Win]::Key(0x10, $true); Tap 0x24; [Win]::Key(0x10, $false); Wait   # Shift+Home
  $s = State
  Check 'Shift+Home selects (the modifier arrives with the key)' ($s.A -eq 0 -and $s.B -eq 5) "selection=($($s.A), $($s.B))"
  $m = Mark
  Chord 0x11 0x43; Wait 600           # Ctrl+C
  $text = [System.Windows.Forms.Clipboard]::GetText([System.Windows.Forms.TextDataFormat]::UnicodeText)
  $html = [System.Windows.Forms.Clipboard]::GetText([System.Windows.Forms.TextDataFormat]::Html)
  Check 'Ctrl+C puts the selection on the OS clipboard as text' ($text -eq 'hello') "another process read: '$text'"
  Check 'and as HTML beside it (CF_HTML)' ($html -match '<b>hello</b>') ("formats: " + (([System.Windows.Forms.Clipboard]::GetDataObject().GetFormats()) -join ','))
  Tap 0x23; Wait
  [System.Windows.Forms.Clipboard]::SetText(' world', [System.Windows.Forms.TextDataFormat]::UnicodeText)
  $m = Mark
  Chord 0x11 0x56; Wait 600           # Ctrl+V
  $s = State
  Check 'Ctrl+V reads text another process put on the clipboard' ($s.Text -eq 'hello world' -and (Since $m '^paste text=" world" html=false').Count -eq 1) ("text=$($s.Text); " + ((Since $m '^paste ') -join ' | '))
  # Text with an HTML form, as a browser or a word processor copies it.
  $fragment = '<i>R</i>'
  $pre = '<html><body><!--StartFragment-->'; $post = '<!--EndFragment--></body></html>'
  $head = "Version:0.9`r`nStartHTML:{0:D8}`r`nEndHTML:{1:D8}`r`nStartFragment:{2:D8}`r`nEndFragment:{3:D8}`r`n"
  $len = ($head -f 0, 0, 0, 0).Length
  $cf = ($head -f $len, ($len + $pre.Length + $fragment.Length + $post.Length), ($len + $pre.Length), ($len + $pre.Length + $fragment.Length)) + $pre + $fragment + $post
  $data = New-Object System.Windows.Forms.DataObject
  $data.SetData([System.Windows.Forms.DataFormats]::UnicodeText, 'R')
  $data.SetData([System.Windows.Forms.DataFormats]::Html, $cf)
  [System.Windows.Forms.Clipboard]::SetDataObject($data, $true)
  $m = Mark
  Chord 0x11 0x56; Wait 600
  Check 'a paste of text with HTML carries both forms' ((Since $m '^paste text="R" html=true').Count -eq 1) ((Since $m '^paste ') -join ' | ')
  Chord 0x11 0x41; TypeKeys 'hello'; Wait  # Ctrl+A, retype: back to "hello"

  # --- 5. pointer drag, double click, focus loss -----------------------------------------------------------
  $m = Mark
  MoveTo 1; [Win]::Mouse(0x2); Start-Sleep -Milliseconds 60
  MoveTo 2; MoveTo 3; MoveTo 4
  [Win]::Mouse(0x4); Wait
  $s = State
  Check 'a drag selects from the press to the release' ($s.A -eq 1 -and $s.B -eq 4 -and (Since $m '^pointer Move').Count -ge 2 -and (Since $m '^pointer Up x=80 ').Count -eq 1) ("selection=($($s.A), $($s.B)); " + ((Since $m '^pointer ') -join ' | '))
  $m = Mark
  MoveTo 2
  [Win]::Mouse(0x2); [Win]::Mouse(0x4); Start-Sleep -Milliseconds 80; [Win]::Mouse(0x2); [Win]::Mouse(0x4); Wait
  Check 'a second press in place arrives as clicks=2' ((Since $m '^pointer Down .* clicks=2').Count -eq 1) ((Since $m '^pointer Down') -join ' | ')
  $m = Mark
  Tap 0x09; Wait                      # Tab: focus leaves the canvas
  Check 'Tab takes the keyboard away and the probe notices' ((Since $m '^focus canvas=false$').Count -eq 1 -and -not (State).Keyboard) ((Since $m '^focus ') -join ' | ')
  $m = Mark
  $other = New-Object System.Windows.Forms.Form
  $other.Text = 'another window'; $other.StartPosition = 'Manual'; $other.Location = New-Object System.Drawing.Point(200, 520); $other.TopMost = $true
  $other.Show(); $other.Activate(); [System.Windows.Forms.Application]::DoEvents()
  $p = $other.PointToScreen((New-Object System.Drawing.Point(40, 40)))
  [void][Win]::SetCursorPos($p.X, $p.Y); [Win]::Mouse(0x1); [Win]::Mouse(0x2); [Win]::Mouse(0x4)
  [System.Windows.Forms.Application]::DoEvents(); Wait 500
  Check 'another window taking the foreground is heard as window focus=false' ((Since $m '^window focus=false$').Count -eq 1) ((Since $m '^window ') -join ' | ')
  $m = Mark
  $other.Close(); $other.Dispose(); [System.Windows.Forms.Application]::DoEvents()
  Click 5
  Check 'a press brings the window and the canvas back' ((Since $m '^window focus=true$').Count -eq 1 -and (State).Keyboard) ((Since $m '^(window|focus) ') -join ' | ')
  Shot '2-after-focus'

  # --- 2 and 3 with a real input method ----------------------------------------------------------------------
  $ime = 'the composition leg'
  $japanese = Get-WinUserLanguageList | Where-Object { $_.LanguageTag -like 'ja*' -and $_.InputMethodTips.Count -gt 0 }
  if (-not $japanese) {
    foreach ($n in 'a real input method starts a composition', 'pre-edit updates carry the string and its cursor', 'a commit inserts the converted text once',
      'Backspace inside a composition does not reach on_key', 'Escape cancels without touching the text', 'the candidate window opens at the caret (screenshot)') {
      Skip $n 'no Japanese input method is installed for this account'
    }
  } else {
    Tap 0x23; Wait
    # Ask the probe's window to switch to the Japanese layout, then turn the input method on (hiragana).
    $hkl = [Win]::LoadKeyboardLayout('00000411', 0x1)
    [void][Win]::PostMessage($hwnd, 0x50, [IntPtr]::Zero, $hkl)
    Wait 1500
    Tap 0x16; Wait 500                # VK_IME_ON
    $immLine = Last '^imm '
    $m = Mark
    TypeKeys 'ka'; Wait 500
    $s = State
    $ka = [string][char]0x304B
    Check 'a real input method starts a composition' ((Since $m '^ime Preedit\("').Count -ge 1 -and $s.Preedit -eq $ka -and $s.Text -eq 'hello') ("preedit='$($s.Preedit)' text='$($s.Text)' layout: $immLine; " + ((Since $m '^(ime|key) ') -join ' | '))
    Check 'the letters of a composition do not arrive as keys too' ((Since $m '^key Text').Count -eq 0) ((Since $m '^key ') -join ' | ')
    TypeKeys 'na'; Wait 500
    $s = State
    $kana = $ka + [char]0x306A
    Check 'pre-edit updates carry the string and its cursor' ($s.Preedit -eq $kana -and (Since $m '^ime Preedit\(".*", Some\(\(\d+, \d+\)\)\)').Count -ge 2) ("preedit='$($s.Preedit)'; " + ((Since $m '^ime ') -join ' | '))
    Shot '3-preedit'
    $k = Mark
    TypeKeys 'ki'; Tap 0x08; Wait 500     # one more syllable, then Backspace takes it out of the pre-edit
    $s = State
    Check 'Backspace inside a composition does not reach on_key' ((Since $k '^key ').Count -eq 0 -and $s.Preedit -eq $kana -and $s.Text -eq 'hello') ("preedit='$($s.Preedit)' text='$($s.Text)'; " + ((Since $k '^key ') -join ' | '))
    Tap 0x20; Tap 0x20; Wait 800      # Space twice: convert, then open the candidate list
    Shot '4-candidates'
    $s = State
    Check 'the candidate window opens at the caret (screenshot)' ($s.Preedit.Length -gt 0) "pre-edit while converting: '$($s.Preedit)'; caret_drawn: $($s.Drawn -join ','); look at 4-candidates.png"
    $c = Mark
    Tap 0x0D; Wait 600                # Enter: commit
    $s = State
    $commits = @(Since $c '^ime Commit\("')
    Check 'a commit inserts the converted text once' ($commits.Count -eq 1 -and $s.Preedit -eq '' -and $s.Text.StartsWith('hello') -and $s.Text.Length -gt 5 -and (Since $c '^key Enter').Count -eq 0) ("text='$($s.Text)'; " + ((Since $c '^(ime|key) ') -join ' | '))
    $textBefore = $s.Text
    $e = Mark
    TypeKeys 'ka'; Wait 400
    Tap 0x1B; Tap 0x1B; Wait 600      # Escape (twice: some input methods first revert the conversion)
    $s = State
    Check 'Escape cancels without touching the text' ($s.Preedit -eq '' -and $s.Text -eq $textBefore -and (Since $e '^ime Commit').Count -eq 0) ("text='$($s.Text)' preedit='$($s.Preedit)'; " + ((Since $e '^(ime|key) ') -join ' | '))
    Tap 0x1A; Wait 300                # VK_IME_OFF
    $en = [Win]::LoadKeyboardLayout('00000409', 0x1)
    [void][Win]::PostMessage($hwnd, 0x50, [IntPtr]::Zero, $en)
    Wait 800
    $m = Mark
    TypeKeys 'z'; Wait
    Check 'after the input method is off, a letter is a key again' ((Since $m '^key Text\("z"\)').Count -eq 1) ((Since $m '^(ime|key) ') -join ' | ')
    Shot '5-after-ime'
  }
}
catch {
  Check 'the run completed' $false $_.Exception.Message
}
finally {
  if ($hwnd -ne [IntPtr]::Zero) { [void][Win]::PostMessage($hwnd, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) }   # WM_CLOSE
  if (-not $proc.WaitForExit(5000)) { Stop-Process -Id $proc.Id -Force }
}
$count = "real-window: $($script:pass + $script:fail + $script:skip) checks, $script:pass passed, $script:fail failing, $script:skip skipped"
$results.Add($count)
[System.IO.File]::WriteAllLines((Join-Path $out 'result.txt'), $results)
$results | ForEach-Object { [Console]::Out.WriteLine($_) }
exit $script:fail

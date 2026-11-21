Add-Type -AssemblyName System.Drawing, System.Windows.Forms, UIAutomationClient, UIAutomationTypes
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System; using System.Collections.Generic; using System.Drawing; using System.Drawing.Imaging;
using System.Runtime.InteropServices; using System.Text;
public static class N {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, uint d, IntPtr e);
  public static void Click(int x, int y) { SetCursorPos(x, y); System.Threading.Thread.Sleep(150); mouse_event(2, 0, 0, 0, IntPtr.Zero); System.Threading.Thread.Sleep(60); mouse_event(4, 0, 0, 0, IntPtr.Zero); }
  [DllImport("user32.dll", SetLastError = true)] static extern bool GetWindowDisplayAffinity(IntPtr h, out uint a);
  [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
  public static IntPtr[] Windows(uint pid) {
    var found = new List<IntPtr>();
    EnumWindows((h, l) => { uint o; GetWindowThreadProcessId(h, out o); if (o == pid && IsWindowVisible(h)) found.Add(h); return true; }, IntPtr.Zero);
    found.Sort((a, b) => Area(b).CompareTo(Area(a))); return found.ToArray();
  }
  // Every visible top-level window: "hwnd|pid|class|title" (for the windows a prompt adds).
  public static string[] AllVisible() {
    var found = new List<string>();
    EnumWindows((h, l) => { if (IsWindowVisible(h)) { uint o; GetWindowThreadProcessId(h, out o); found.Add(h.ToInt64() + "|" + o + "|" + ClassOf(h) + "|" + Title(h)); } return true; }, IntPtr.Zero);
    return found.ToArray();
  }
  public static RECT Rect(IntPtr h) { RECT r; if (DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf(typeof(RECT))) == 0) return r; GetWindowRect(h, out r); return r; }
  public static long Area(IntPtr h) { var r = Rect(h); return (long)Math.Max(0, r.R - r.L) * Math.Max(0, r.B - r.T); }
  public static string ClassOf(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
  public static string Title(IntPtr h) { var s = new StringBuilder(256); GetWindowText(h, s, 256); return s.ToString(); }
  public static long Affinity(IntPtr h) { uint a; if (GetWindowDisplayAffinity(h, out a)) return (long)a; return -1L - (long)Marshal.GetLastWin32Error(); }
  public static void Shot(int x, int y, int w, int h, string png) {
    if (w <= 0 || h <= 0) return;
    using (var bmp = new Bitmap(w, h, PixelFormat.Format32bppArgb)) {
      using (var g = Graphics.FromImage(bmp)) g.CopyFromScreen(x, y, 0, 0, new Size(w, h));
      bmp.Save(png, ImageFormat.Png);
    }
  }
  public static void ShotWindow(IntPtr h, string png) { var r = Rect(h); Shot(r.L, r.T, r.R - r.L, r.B - r.T, png); }
  // The fake's stderr as it comes, each line with the time it arrived.
  public static System.Collections.Concurrent.ConcurrentQueue<string> Err = new System.Collections.Concurrent.ConcurrentQueue<string>();
  public static void TapErr(System.Diagnostics.Process p) { p.ErrorDataReceived += (s, e) => { if (e.Data != null) Err.Enqueue(DateTime.Now.ToString("HH:mm:ss.fff") + " " + e.Data); }; p.BeginErrorReadLine(); }
  public static string[] TakeErr() { var all = new List<string>(); string line; while (Err.TryDequeue(out line)) all.Add(line); return all.ToArray(); }

  // Cloud Files: who a folder belongs to, and taking the run's own registration away.
  [DllImport("cldapi.dll", CharSet = CharSet.Unicode)] static extern int CfUnregisterSyncRoot(string path);
  [DllImport("cldapi.dll", CharSet = CharSet.Unicode)] static extern int CfGetSyncRootInfoByPath(string path, int infoClass, IntPtr buffer, uint length, out uint returned);
  public static int Unregister(string path) { return CfUnregisterSyncRoot(path); }
  public static string ProviderOf(string path) {
    IntPtr buffer = Marshal.AllocHGlobal(4096);
    try { uint returned; if (CfGetSyncRootInfoByPath(path, 2, buffer, 4096, out returned) != 0) return null; return Marshal.PtrToStringUni(IntPtr.Add(buffer, 4)); }
    finally { Marshal.FreeHGlobal(buffer); }
  }
  // Credential Manager: target NAMES only, never a value (CREDENTIALW: Type at 4, TargetName at 8).
  [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool CredEnumerateW(string filter, uint flags, out uint count, out IntPtr credentials);
  [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool CredDeleteW(string target, uint type, uint flags);
  [DllImport("advapi32.dll")] static extern void CredFree(IntPtr buffer);
  public static string[] Targets(string filter) {
    uint count; IntPtr list; if (!CredEnumerateW(filter, 0, out count, out list)) return new string[0];
    try { var names = new List<string>(); for (int i = 0; i < count; i++) { IntPtr cred = Marshal.ReadIntPtr(list, i * IntPtr.Size); if (Marshal.ReadInt32(cred, 4) == 1) names.Add(Marshal.PtrToStringUni(Marshal.ReadIntPtr(cred, 8))); } return names.ToArray(); }
    finally { CredFree(list); }
  }
  public static bool DeleteGeneric(string target) { return CredDeleteW(target, 1, 0); }
  [StructLayout(LayoutKind.Sequential)] struct KI { public ushort vk; public ushort scan; public uint flags; public uint time; public IntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] struct MI { public int dx; public int dy; public uint data; public uint flags; public uint time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit)] struct IU { [FieldOffset(0)] public MI mi; [FieldOffset(0)] public KI ki; }
  [StructLayout(LayoutKind.Sequential)] struct IN { public uint type; public IU u; }
  [DllImport("user32.dll", SetLastError = true)] static extern uint SendInput(uint n, IN[] inputs, int size);
  static IN K(ushort vk, uint flags) { var i = new IN(); i.type = 1; i.u.ki.vk = vk; i.u.ki.flags = flags; return i; }
  // Digits '0'..'9' of b (their bytes are their virtual-key codes), one at a time. Returns the events Windows accepted.
  public static int TypeDigits(byte[] b, int off, int len) {
    int sent = 0; int sz = Marshal.SizeOf(typeof(IN));
    for (int k = 0; k < len; k++) { ushort vk = b[off + k]; if (vk < 0x30 || vk > 0x39) return -1; sent += (int)SendInput(2, new IN[] { K(vk, 0), K(vk, 2) }, sz); System.Threading.Thread.Sleep(30); }
    return sent;
  }
  public static int Key(ushort vk) { return (int)SendInput(2, new IN[] { K(vk, 0), K(vk, 2) }, Marshal.SizeOf(typeof(IN))); }
  [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint f);
  public static IntPtr RootOf(IntPtr h) { return GetAncestor(h, 2); }
}
"@
[N]::SetProcessDPIAware() | Out-Null
# rec-lib part 2: helpers for the interactive recording agent (not product code)
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Drawing; using System.Drawing.Imaging; using System.Text;
public static class M {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, int d, IntPtr e);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool r);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  public static void Move(int x, int y, int steps, int delayMs) {
    POINT p; GetCursorPos(out p);
    for (int i = 1; i <= steps; i++) { double t = (double)i / steps; t = t * t * (3 - 2 * t); SetCursorPos((int)(p.X + (x - p.X) * t), (int)(p.Y + (y - p.Y) * t)); System.Threading.Thread.Sleep(delayMs); }
    SetCursorPos(x, y);
  }
  public static void Down() { mouse_event(2, 0, 0, 0, IntPtr.Zero); }
  public static void Up() { mouse_event(4, 0, 0, 0, IntPtr.Zero); }
  public static void RDown() { mouse_event(8, 0, 0, 0, IntPtr.Zero); }
  public static void RUp() { mouse_event(16, 0, 0, 0, IntPtr.Zero); }
  public static void Wheel(int notches) { mouse_event(0x800, 0, 0, notches * 120, IntPtr.Zero); }
  public static void FullShot(string png) {
    int w = GetSystemMetrics(0), h = GetSystemMetrics(1);
    using (var bmp = new Bitmap(w, h, PixelFormat.Format32bppArgb)) { using (var g = Graphics.FromImage(bmp)) g.CopyFromScreen(0, 0, 0, 0, new Size(w, h)); bmp.Save(png, ImageFormat.Png); }
  }
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
}
"@
$ErrorActionPreference = 'Continue'
$R = 'C:\bb\dc086'
$shots = "$R\shots"; New-Item -ItemType Directory -Force $shots | Out-Null
$script:appName = 'beebeeb-drive'
$UIA = [System.Windows.Automation.AutomationElement]
$T = [System.Windows.Automation.ControlType]
$Desc = [System.Windows.Automation.TreeScope]::Descendants
function Say([string]$s) { Write-Host $s }
function Pump([int]$ms) { $u = (Get-Date).AddMilliseconds($ms); while ((Get-Date) -lt $u) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 50 } }
function Shot([string]$name) { [M]::FullShot("$shots\$name.png"); "shot $name" }
function App-Procs { @(Get-Process | Where-Object { $_.ProcessName -like $script:appName -or $_.ProcessName -like 'beebeeb-desktop*' }) }
function App-Windows {
  $res = @(); foreach ($p in App-Procs) { $res += @([N]::Windows([uint32]$p.Id) | Where-Object { [N]::Area($_) -ge 40000 }) }
  $res
}
function All-Elements($h) { try { @(([System.Windows.Automation.AutomationElement]::FromHandle($h)).FindAll($Desc, [System.Windows.Automation.Condition]::TrueCondition)) } catch { @() } }
function Els { $r = @(); foreach ($h in App-Windows) { foreach ($e in All-Elements $h) { try { $c = $e.Current; if ($c.Name -or $c.ControlType.ProgrammaticName -eq 'ControlType.Edit') { $r += ,@($h, $e) } } catch {} } }; $r }
# List: type|name|x y w h (screen) of the app's elements; filter by regex on the name
function Dump([string]$rx = '') {
  foreach ($pair in Els) { try { $c = $pair[1].Current; $b = $c.BoundingRectangle
    if ($b.IsEmpty -or ($rx -and $c.Name -notmatch $rx)) { continue }
    '{0}|{1}|{2},{3} {4}x{5}|{6}' -f ($c.ControlType.ProgrammaticName -replace '^ControlType\.', ''), $c.Name, [int]$b.X, [int]$b.Y, [int]$b.Width, [int]$b.Height, $(if ($c.IsOffscreen) { 'off' } else { '' }) } catch {} }
}
function Find-El([string]$name, [string]$type = '', [switch]$Like) {
  foreach ($pair in Els) { try { $c = $pair[1].Current
    $ok = if ($Like) { $c.Name -like $name } else { $c.Name -ceq $name }
    if ($ok -and (-not $type -or $c.ControlType.ProgrammaticName -eq "ControlType.$type") -and -not $c.IsOffscreen) { return $pair } } catch {} }
  return $null
}
function Wait-El([string]$name, [string]$type = '', [int]$sec = 20, [switch]$Like) { $u = (Get-Date).AddSeconds($sec); while ((Get-Date) -lt $u) { $f = if ($Like) { Find-El $name $type -Like } else { Find-El $name $type }; if ($f) { return $f }; Pump 400 }; return $null }
function Front { $h = @(App-Windows)[0]; if ($h) { [N]::SetForegroundWindow($h) | Out-Null; Pump 200 }; $h }
# Real mouse click at the centre of an element (smooth cursor path so the video shows it)
function Click-El($pair, [int]$dx = 0, [int]$dy = 0) {
  if (-not $pair) { return 'not found' }
  $null = Front
  $b = $pair[1].Current.BoundingRectangle
  [M]::Move([int]($b.X + $b.Width / 2 + $dx), [int]($b.Y + $b.Height / 2 + $dy), 25, 12); Pump 250
  [M]::Down(); Start-Sleep -Milliseconds 70; [M]::Up(); Pump 500; return 'clicked'
}
function Click-Name([string]$name, [string]$type = '', [int]$sec = 15, [switch]$Like) { $f = if ($Like) { Wait-El $name $type $sec -Like } else { Wait-El $name $type $sec }; Click-El $f }
function Click-At([int]$x, [int]$y) { [M]::Move($x, $y, 25, 12); Pump 200; [M]::Down(); Start-Sleep -Milliseconds 70; [M]::Up(); Pump 400 }
function RClick-At([int]$x, [int]$y) { [M]::Move($x, $y, 25, 12); Pump 200; [M]::RDown(); Start-Sleep -Milliseconds 70; [M]::RUp(); Pump 400 }
function DClick-At([int]$x, [int]$y) { [M]::Move($x, $y, 25, 12); Pump 200; [M]::Down(); [M]::Up(); Start-Sleep -Milliseconds 90; [M]::Down(); [M]::Up(); Pump 500 }
function Keys([string]$s) { $s -replace '([+^%~(){}\[\]])', '{$1}' }
function Send([string]$k) { [System.Windows.Forms.SendKeys]::SendWait($k); Pump 300 }
function Type-Slow([string]$text) { foreach ($ch in $text.ToCharArray()) { [System.Windows.Forms.SendKeys]::SendWait((Keys ([string]$ch))); Start-Sleep -Milliseconds 55 }; Pump 200 }
function Type-In([string]$name, [string]$text, [string]$type = 'Edit') {
  $f = Wait-El $name $type 15; if (-not $f) { return "no $name" }
  Click-El $f | Out-Null; Type-Slow $text; return 'typed'
}
function Window-State { $h = @(App-Windows)[0]; if (-not $h) { return 'no window' }; $r = [N]::Rect($h); "$($r.L),$($r.T) $($r.R - $r.L)x$($r.B - $r.T) title '$([N]::Title($h))'" }
function Texts { (@(Dump '.') | Where-Object { $_ -match '^(Text|Button|Edit|Hyperlink|ListItem|RadioButton|CheckBox|TabItem|MenuItem)\|' } | ForEach-Object { ($_ -split '\|')[0..1] -join '|' }) -join ' ; ' }
function Prompt-Rows { @([N]::AllVisible() | Where-Object { $p = [int](($_ -split '\|')[1]); try { (Get-Process -Id $p).ProcessName -eq 'CredentialUIBroker' } catch { $false } }) }
function Start-App([string]$exe, [string]$base = 'http://127.0.0.1:3041') {
  foreach ($k in 'HTTPS_PROXY', 'HTTP_PROXY', 'ALL_PROXY') { [Environment]::SetEnvironmentVariable($k, 'http://127.0.0.1:9', 'Process') }; [Environment]::SetEnvironmentVariable('NO_PROXY', '', 'Process')
  $p = Start-Process -FilePath $exe -ArgumentList @('--api-base', $base) -PassThru
  foreach ($k in 'HTTPS_PROXY', 'HTTP_PROXY', 'ALL_PROXY', 'NO_PROXY') { [Environment]::SetEnvironmentVariable($k, $null, 'Process') }
  "started PID $($p.Id)"
}
"rec-lib loaded"
if (-not ('P' -as [type])) { Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class P {
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public static bool Close(IntPtr h) { return PostMessage(h, 0x0010, IntPtr.Zero, IntPtr.Zero); }
}
"@ }
function Close-Credential { foreach ($r in @([N]::AllVisible() | Where-Object { $_ -match '\|Credential Dialog Xaml Host\|' })) { [P]::Close([IntPtr][long](($r -split '\|')[0])) | Out-Null }; Pump 1500; "credential dialogs left: " + @(Prompt-Rows).Count }

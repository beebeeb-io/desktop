#Requires -Version 5.1
<#
.SYNOPSIS
    Native rung (task 2460): the TEST build's sign-in screen is excluded from screen capture.
.DESCRIPTION
    Microsoft's SetWindowDisplayAffinity documentation: with WDA_EXCLUDEFROMCAPTURE (0x11) "the
    window does not appear at all" in a capture; with WDA_MONITOR (0x01) it "appears with no
    content"; before Windows 10 2004, 0x11 "will behave as if WDA_MONITOR is applied". So an
    excluded window reads as WHATEVER IS BEHIND IT (or black on old builds), never reliably black.

    Method: a full-screen magenta backdrop (not topmost) goes up first; the app's window is put in
    front of it; the app's on-screen rect is captured with GDI CopyFromScreen.
      excluded     -> the capture is the backdrop colour (or black: WDA_MONITOR behaviour)
      not excluded -> the app's own pixels show
    Controls prove the method: a plain window reads as its own colour on the desktop (C1); the bare
    backdrop reads as magenta (C2); a plain window in front of the backdrop does NOT read as the
    backdrop (C3: the method can fail).

    Safety: refuses (exit 2, before starting anything) unless the binary carries the TEST identity
    mark, no process of that binary name runs, the TEST app has no state on this machine
    (-RequireCleanState, default on: %APPDATA%\beebeeb-test, %LOCALAPPDATA%\io.beebeeb.app.test,
    Credential Manager targets containing io.beebeeb.app.test; only counts are printed), and an
    outbound firewall block for the binary can be added (needs an elevated shell; -SkipFirewall
    falls back to the proxy layer only and says so). The app also gets HTTP(S)_PROXY pointed at a
    closed local port, and is started with --api-base http://127.0.0.1:9 (decision 2462): its own
    server is that closed port too, so it never talks to the production server. The TEST build
    itself refuses a custom server when it already has data on this machine (data made for the
    default server, or for another one: one stderr line, exit 2), but that is the app's own check:
    this script's clean-state refusal above stays the first line of defence, before anything is
    started. The app is stopped by its PID only. Captures are kept in the printed temp folder;
    what the app created on this machine is removed afterwards (only paths that did not exist
    before the run, so the record of the server it was started for goes with them).

    Ends with "N checks, M failing"; exit 0 only when M = 0; exit 1 on a failing check; exit 2 on
    a refusal. -ParseOnly compiles the helper and exits (proves the script and its C# parse).
.NOTES
    Run from an elevated Windows PowerShell on an interactive desktop (not over RDP: a
    capture-excluded window looks blank there, the lesson of 2026-11-11):
      powershell -NoProfile -ExecutionPolicy Bypass -File capture-exclusion.ps1 -Binary <path to the TEST build's beebeeb-drive.exe>
#>
[CmdletBinding()]
param(
    [string]$Binary,
    [bool]$RequireCleanState = $true,
    [switch]$SkipFirewall,
    [switch]$ParseOnly
)
$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;

public static class BbRung {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    public delegate bool EnumProc(IntPtr hwnd, IntPtr lParam);

    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT p);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint flags);
    [DllImport("user32.dll", SetLastError = true)] static extern bool GetWindowDisplayAffinity(IntPtr h, out uint a);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);

    public static readonly IntPtr HWND_TOPMOST = new IntPtr(-1);
    public const uint SWP_NOSIZE = 0x0001, SWP_SHOWWINDOW = 0x0040;

    /// Visible top-level windows of one process, largest first.
    public static IntPtr[] VisibleWindowsOf(uint pid) {
        var found = new List<IntPtr>();
        EnumWindows((h, l) => {
            uint owner; GetWindowThreadProcessId(h, out owner);
            if (owner == pid && IsWindowVisible(h)) found.Add(h);
            return true;
        }, IntPtr.Zero);
        found.Sort((a, b) => Area(b).CompareTo(Area(a)));
        return found.ToArray();
    }
    /// The rect the window is drawn in (DWM's extended frame bounds: no invisible resize borders).
    public static RECT VisibleRect(IntPtr h) {
        RECT r;
        if (DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf(typeof(RECT))) == 0) return r;
        GetWindowRect(h, out r);
        return r;
    }
    public static long Area(IntPtr h) { var r = VisibleRect(h); return (long)Math.Max(0, r.Right - r.Left) * Math.Max(0, r.Bottom - r.Top); }
    public static string Title(IntPtr h) { var s = new StringBuilder(256); GetWindowText(h, s, 256); return s.ToString(); }
    public static string ClassOf(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
    /// The top-level window under a screen point (GA_ROOT).
    public static IntPtr RootAt(int x, int y) { var p = new POINT { X = x, Y = y }; return GetAncestor(WindowFromPoint(p), 2); }
    /// The affinity, or -1 - GetLastError when Windows does not say (the docs: it answers for layered windows).
    public static long Affinity(IntPtr h) {
        uint a;
        if (GetWindowDisplayAffinity(h, out a)) return (long)a;
        return -1L - (long)Marshal.GetLastWin32Error();
    }

    /// Captures a screen rect (GDI, as a screenshot tool does), saves it, and returns
    /// { share near the colour (r,g,b) within tol, share near black, pixels sampled }.
    public static double[] Capture(int x, int y, int w, int h, int r, int g, int b, int tol, string png) {
        using (var bmp = new Bitmap(w, h, PixelFormat.Format32bppArgb)) {
            using (var gr = Graphics.FromImage(bmp)) gr.CopyFromScreen(x, y, 0, 0, new Size(w, h));
            bmp.Save(png, ImageFormat.Png);
            var data = bmp.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            var buf = new byte[data.Stride * h];
            Marshal.Copy(data.Scan0, buf, 0, buf.Length);
            int stride = data.Stride;
            bmp.UnlockBits(data);
            long near = 0, black = 0, n = 0;
            for (int yy = 0; yy < h; yy += 2) for (int xx = 0; xx < w; xx += 2) {
                int i = yy * stride + xx * 4; int B = buf[i], G = buf[i + 1], R = buf[i + 2]; n++;
                if (Math.Abs(R - r) <= tol && Math.Abs(G - g) <= tol && Math.Abs(B - b) <= tol) near++;
                if (R <= 15 && G <= 15 && B <= 15) black++;
            }
            return new double[] { (double)near / Math.Max(n, 1), (double)black / Math.Max(n, 1), n };
        }
    }
}
"@

if ($ParseOnly) { Write-Host "parse-only: script parsed, helper compiled"; exit 0 }

# Physical pixels everywhere (window rects and the GDI capture agree at any display scale).
[BbRung]::SetProcessDPIAware() | Out-Null

function Refuse([string]$why) { Write-Host "REFUSED: $why"; exit 2 }

# --- Pre-flight: nothing is started before every refusal has been checked -------------------
if (-not $Binary) { Refuse "-Binary <path to the TEST build's beebeeb-drive.exe> is required" }
if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) { Refuse "binary not found: $Binary" }
$Binary = (Resolve-Path -LiteralPath $Binary).Path

# The identity mark build.rs puts in the binary exactly once: refuse anything but the TEST app.
$text = [System.Text.Encoding]::GetEncoding(28591).GetString([System.IO.File]::ReadAllBytes($Binary))
$testMark = $text.Contains('beebeeb-app-identity=io.beebeeb.app.test;')
$realMark = $text.Contains('beebeeb-app-identity=io.beebeeb.app;')
$text = $null
if (-not $testMark -or $realMark) { Refuse "the binary does not carry exactly the TEST identity mark (test: $testMark, real: $realMark)" }

$name = [System.IO.Path]::GetFileNameWithoutExtension($Binary)
$running = @(Get-Process -Name $name -ErrorAction SilentlyContinue).Count
if ($running -gt 0) { Refuse "$running process(es) named $name already run; close them first" }

$stateDirs = @((Join-Path $env:APPDATA 'beebeeb-test'), (Join-Path $env:LOCALAPPDATA 'io.beebeeb.app.test'))
$existedBefore = @{}
foreach ($d in $stateDirs) { $existedBefore[$d] = Test-Path -LiteralPath $d }
function Count-Credentials { @(cmdkey /list 2>$null | Where-Object { $_ -match 'Target:.*io\.beebeeb\.app\.test' }).Count }
$credsBefore = Count-Credentials
if ($RequireCleanState) {
    $dirsPresent = @($stateDirs | Where-Object { $existedBefore[$_] }).Count
    if ($dirsPresent -gt 0 -or $credsBefore -gt 0) {
        Refuse "the TEST app has state on this machine: $dirsPresent of 2 state folders, $credsBefore Credential Manager entries"
    }
}

$firewallRule = $null
if (-not $SkipFirewall) {
    $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $admin) { Refuse "an elevated shell is needed for the outbound firewall block (or pass -SkipFirewall)" }
    $off = @(Get-NetFirewallProfile | Where-Object { [string]$_.Enabled -ne 'True' }).Count
    if ($off -gt 0) { Refuse "$off firewall profile(s) are off: a block rule would not hold (or pass -SkipFirewall)" }
}

$build = [Environment]::OSVersion.Version.Build
Write-Host "Windows build $build$(if ($build -lt 19041) { ': before 2004, 0x11 behaves as WDA_MONITOR (black expected)' })"

$checks = 0; $failing = 0
function Check([bool]$ok, [string]$what) {
    $script:checks++
    if ($ok) { Write-Host "  PASS: $what" } else { $script:failing++; Write-Host "  FAIL: $what" }
}
function Pump([int]$ms) {
    $until = (Get-Date).AddMilliseconds($ms)
    while ((Get-Date) -lt $until) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 50 }
}
function Ratio([double]$v) { [math]::Round($v, 3) }

$work = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
$virtual = [System.Windows.Forms.SystemInformation]::VirtualScreen
$magenta = @(255, 0, 255); $control = @(0, 128, 255); $tol = 24
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) "bb-capture-rung-$([guid]::NewGuid().ToString('N').Substring(0, 8))"
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
Write-Host "Captures in $tempDir"

function New-Plain([int[]]$rgb, [int]$x, [int]$y, [int]$w, [int]$h, [bool]$top) {
    $f = New-Object System.Windows.Forms.Form
    $f.FormBorderStyle = 'None'; $f.ShowInTaskbar = $false; $f.StartPosition = 'Manual'
    $f.BackColor = [System.Drawing.Color]::FromArgb(255, $rgb[0], $rgb[1], $rgb[2])
    $f.Location = New-Object System.Drawing.Point($x, $y); $f.Size = New-Object System.Drawing.Size($w, $h)
    $f.TopMost = $top
    $f.Show(); Pump 800
    return $f
}

$proc = $null; $backdrop = $null
$savedProxy = @{ HTTPS_PROXY = $env:HTTPS_PROXY; HTTP_PROXY = $env:HTTP_PROXY; ALL_PROXY = $env:ALL_PROXY; NO_PROXY = $env:NO_PROXY }
function Restore-Proxy {
    foreach ($k in $savedProxy.Keys) {
        if ($null -eq $savedProxy[$k]) { Remove-Item -Path "env:$k" -ErrorAction SilentlyContinue }
        else { Set-Item -Path "env:$k" -Value $savedProxy[$k] }
    }
}
$cx = $work.Left + 60; $cy = $work.Top + 60; $cs = 300
try {
    Write-Host "`n=== C1: a plain window on the desktop reads as its own colour ==="
    $f = New-Plain $control $cx $cy $cs $cs $true
    $r = [BbRung]::Capture($cx, $cy, $cs, $cs, $control[0], $control[1], $control[2], $tol, (Join-Path $tempDir 'c1-control.png'))
    $f.Close(); $f.Dispose()
    Write-Host "  own colour $(Ratio $r[0]) of $($r[2]) samples"
    Check ($r[0] -ge 0.90) "C1 the capture method sees a normal window"

    Write-Host "`n=== C2: the bare backdrop reads as magenta ==="
    $backdrop = New-Plain $magenta $virtual.Left $virtual.Top $virtual.Width $virtual.Height $false
    $r = [BbRung]::Capture($cx, $cy, $cs, $cs, $magenta[0], $magenta[1], $magenta[2], $tol, (Join-Path $tempDir 'c2-backdrop.png'))
    Write-Host "  magenta $(Ratio $r[0])"
    Check ($r[0] -ge 0.95) "C2 the backdrop is drawn and captured"

    Write-Host "`n=== C3: a plain window in front of the backdrop does NOT read as the backdrop ==="
    $f = New-Plain $control $cx $cy $cs $cs $true
    $r = [BbRung]::Capture($cx, $cy, $cs, $cs, $control[0], $control[1], $control[2], $tol, (Join-Path $tempDir 'c3-control-on-backdrop.png'))
    $m = [BbRung]::Capture($cx, $cy, $cs, $cs, $magenta[0], $magenta[1], $magenta[2], $tol, (Join-Path $tempDir 'c3-magenta-share.png'))
    $f.Close(); $f.Dispose(); Pump 300
    Write-Host "  own colour $(Ratio $r[0]), magenta $(Ratio $m[0])"
    Check ($r[0] -ge 0.90 -and $m[0] -le 0.05) "C3 a window that is not excluded is told apart from the backdrop"

    Write-Host "`n=== The app: the TEST build on a clean machine opens on sign-in (a secret screen) ==="
    if (-not $SkipFirewall) {
        $ruleName = "bb-rung-capture-$([guid]::NewGuid().ToString('N').Substring(0, 8))"
        $firewallRule = New-NetFirewallRule -DisplayName $ruleName -Direction Outbound -Program $Binary -Action Block -Profile Any
        Write-Host "  outbound firewall block added for the binary ($ruleName)"
    } else {
        Write-Host "  -SkipFirewall: no firewall block; only the proxy layer below keeps it off the network"
    }
    # reqwest honours these: every HTTP(S) request of the app goes to a closed local port.
    $env:HTTPS_PROXY = 'http://127.0.0.1:9'; $env:HTTP_PROXY = 'http://127.0.0.1:9'; $env:ALL_PROXY = 'http://127.0.0.1:9'; $env:NO_PROXY = ''
    # A console-subsystem exe: -NoNewWindow keeps it in this console, so no console window of its own appears.
    # --api-base: the TEST build's own server is the same closed local port (it says so on stdout: "api base: ...").
    $proc = Start-Process -FilePath $Binary -ArgumentList '--api-base', 'http://127.0.0.1:9' -NoNewWindow -PassThru
    Restore-Proxy
    Write-Host "  started PID $($proc.Id)"

    $hwnd = [IntPtr]::Zero
    for ($i = 0; $i -lt 40 -and $hwnd -eq [IntPtr]::Zero; $i++) {
        Pump 500
        if ($proc.HasExited) { break }
        $big = @([BbRung]::VisibleWindowsOf([uint32]$proc.Id) | Where-Object { [BbRung]::Area($_) -ge 40000 })
        if ($big.Count -gt 0) { $hwnd = $big[0] }
    }
    $found = $hwnd -ne [IntPtr]::Zero
    if ($found) {
        $title = [BbRung]::Title($hwnd); $class = [BbRung]::ClassOf($hwnd)
        $rect = [BbRung]::VisibleRect($hwnd)
        Write-Host "  window: title '$title', class '$class', $($rect.Right - $rect.Left)x$($rect.Bottom - $rect.Top) at ($($rect.Left),$($rect.Top))"
        if ($class -eq '#32770') { Write-Host "  that is a message box: the app refused to start"; $found = $false }
    } elseif ($proc.HasExited) {
        Write-Host "  the app exited with code $($proc.ExitCode) before showing a window"
    }
    Check $found "C4 the app's own top-level window is found by its PID (visible, largest)"

    if ($found) {
        # In front of the backdrop and inside the working area (no taskbar behind it).
        [BbRung]::SetWindowPos($hwnd, [BbRung]::HWND_TOPMOST, $work.Left + 40, $work.Top + 40, 0, 0,
            [BbRung]::SWP_NOSIZE -bor [BbRung]::SWP_SHOWWINDOW) | Out-Null
        Pump 3000
        $rect = [BbRung]::VisibleRect($hwnd)
        $x = [math]::Max($rect.Left, $work.Left); $y = [math]::Max($rect.Top, $work.Top)
        $w = [math]::Min($rect.Right, $work.Right) - $x; $h = [math]::Min($rect.Bottom, $work.Bottom) - $y
        $front = ([BbRung]::RootAt([int]($x + $w / 2), [int]($y + $h / 2)) -eq $hwnd)
        Check ($w -ge 100 -and $h -ge 100 -and [BbRung]::IsWindowVisible($hwnd) -and $front) "C5 the app is visible, ${w}x${h} on screen, and in front at its centre"

        $r = [BbRung]::Capture($x, $y, $w, $h, $magenta[0], $magenta[1], $magenta[2], $tol, (Join-Path $tempDir 'app-signin.png'))
        Write-Host "  magenta $(Ratio $r[0]), black $(Ratio $r[1]) of $($r[2]) samples"
        $seen = if ($r[0] -ge 0.95) { 'absent from the capture (WDA_EXCLUDEFROMCAPTURE behaviour)' }
                elseif ($r[1] -ge 0.95) { 'black in the capture (WDA_MONITOR behaviour)' }
                else { 'its own pixels show: NOT excluded' }
        Write-Host "  the sign-in window is $seen"
        Check ($r[0] -ge 0.95 -or $r[1] -ge 0.95) "C6 the secret screen does not show in a screen capture"

        $a = [BbRung]::Affinity($hwnd)
        if ($a -ge 0) {
            Write-Host ("  GetWindowDisplayAffinity: 0x{0:X2}" -f $a)
            Check ($a -eq 0x11 -or $a -eq 0x01) "C7 Windows reports the window's display affinity as excluded"
        } else {
            Write-Host "  GetWindowDisplayAffinity did not answer (Win32 error $(-1 - $a)); judged on pixels only (C6)"
        }
    }
} finally {
    if ($proc -and -not $proc.HasExited) {
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        $proc.WaitForExit(5000) | Out-Null
        Write-Host "  stopped PID $($proc.Id)"
    }
    Restore-Proxy
    if ($firewallRule) { Remove-NetFirewallRule -Name $firewallRule.Name -ErrorAction SilentlyContinue; Write-Host "  firewall block removed" }
    if ($backdrop) { $backdrop.Close(); $backdrop.Dispose() }
    foreach ($d in $stateDirs) {
        if (-not $existedBefore[$d] -and (Test-Path -LiteralPath $d)) {
            Start-Sleep -Milliseconds 500
            Remove-Item -LiteralPath $d -Recurse -Force -ErrorAction SilentlyContinue
            Write-Host "  removed what the run created: $d$(if (Test-Path -LiteralPath $d) { ' (FAILED: still there)' })"
        }
    }
    $credsAfter = Count-Credentials
    if ($credsAfter -ne $credsBefore) { Write-Host "  Credential Manager entries for the TEST app: $credsBefore before, $credsAfter after (not removed)" }
}

Write-Host "`n$checks checks, $failing failing"
if ($failing -gt 0) { exit 1 }
exit 0

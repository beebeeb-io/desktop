#Requires -Version 5.1
<#
.SYNOPSIS
    Native rungs R1.1 to R1.7 of the sync-start contract (S1,
    docs/superpowers/specs/2026-11-12-native-sync-start-and-sign-out.md, section 8): from the recovery
    phrase to a registered, running sync, on the engine's fake API.
.DESCRIPTION
    The TEST build signs in to the fake API (`fake_api`, the engine's example: an invented account, its
    password and 12 words, 2 folders and 5 files; loopback only) and is driven through Windows UI
    Automation (the app's accessibility tree, by control type and name), in this order:

      R1.4  launch 1: a wrong phrase first. DW-A24's words; the TEST app's Credential Manager entries
            before = after the phrase (count); no sync folder; the fake answered 1 recovery check
      R1.6  the sync folder's path registered to another provider ("BbRungOther"), then the right
            phrase: the start is refused; the refusal modal says another app syncs the folder
            (screenshot); no placeholder was made; the other provider's registration is unchanged; the
            log line names the refusal's kind and never the folder's path. "Close", then the status
            line says "Sync is off" (behind an open modal the tree holds only the modal)
      R1.6  launch 2, the folder still another provider's: the start at launch is refused and the modal
            opens again. The outside fix (the rung removes its own registration), then the modal's
            "Try again"
      R1.1  the start line named the loopback server (both launches); the folder has the account's 7
            items; the window lists the root's 2 rows (Documents, Photos) once the first check
            finished, and no "cannot be read" text; the status line says a running engine's word
            (Syncing or Up to date); screenshot
      R1.2  the folder's sync-root registration (CfGetSyncRootInfoByPath) names the test identity's
            provider (BeebeebTest); 1 Explorer registration of it; every other sync root of this
            machine unchanged (the SyncRootManager keys before and after: only the new one differs)
      R1.3  7 placeholders (2 folders, 5 files), every file cloud-only by its attributes (the engine's
            own mask: RECALL_ON_DATA_ACCESS, RECALL_ON_OPEN or OFFLINE; each file's attributes are
            printed) and 0 bytes on disk. Opening a file is NOT run: the fake serves no file content
      R1.5  launch 3, with a second root of the engine's QA provider (BeebeebQA, its name and GUID)
            beside it: both stay registered, each to its own provider, and each keeps its own files.
            Whether a callback of one fires for the other is NOT read: the app keeps no engine log
      R1.7  launch 4, after the app's own quit (`--quit-for-update`): Files without the phrase, the
            root's 2 rows, a running engine's word, still 1 registration and 7 placeholders

    Preconditions P (each refuses with exit 2 before anything starts): the binary carries exactly the
    TEST identity mark; the TEST app has nothing on this machine (its settings and state folders, its
    Credential Manager entries, an installed copy, a running copy, a sync root of its provider, a
    folder named "Beebeeb Test" in the profile; counts only, never an entry's value); the fake's base
    is loopback. The app gets HTTP(S)_PROXY pointed at a closed local port (the engine's client never
    proxies a loopback base) and an outbound firewall block for every address but loopback (needs an
    elevated shell and every firewall profile on; -SkipFirewall keeps the proxy layer only and says so).

    Afterwards everything the run made is removed, and the six P counts are printed again (a check:
    all 0): the app's sync root (its Explorer registration and its Cloud Files registration) and
    folder, the rung's own two roots and folders, the TEST app's settings and state folders and its
    Credential Manager entries (P proved there were none before). Processes are stopped by PID only.

    Ends with "N checks, M failing"; exit 0 only when M = 0; exit 1 on a failing check; exit 2 on a
    refusal. -ParseOnly compiles the helper, resolves the WinRT sync-root manager and exits.
.NOTES
    Run by the lead in the build VM's desktop session (UI Automation and screenshots need it), from an
    elevated Windows PowerShell:
      powershell -NoProfile -ExecutionPolicy Bypass -File sync-start-s1.ps1 -Binary <TEST beebeeb-drive.exe> -FakeApi <fake_api.exe>
    fake_api.exe: `cargo build --example fake_api --features test-support` in crates/beebeeb-engine.
#>
[CmdletBinding()]
param(
    [string]$Binary,
    [string]$FakeApi,
    [switch]$SkipFirewall,
    [switch]$ParseOnly,
    [int]$StepSeconds = 60
)
$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;

public static class BbS1 {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    public delegate bool EnumProc(IntPtr hwnd, IntPtr lParam);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);

    // --- Cloud Files: the rung's own roots, and who a folder belongs to ---------------------------
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct CF_SYNC_REGISTRATION {
        public uint StructSize;
        [MarshalAs(UnmanagedType.LPWStr)] public string ProviderName;
        [MarshalAs(UnmanagedType.LPWStr)] public string ProviderVersion;
        public IntPtr SyncRootIdentity; public uint SyncRootIdentityLength;
        public IntPtr FileIdentity; public uint FileIdentityLength;
        public Guid ProviderId;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct CF_SYNC_POLICIES {
        public uint StructSize;
        public ushort HydrationPrimary, HydrationModifier;
        public ushort PopulationPrimary, PopulationModifier;
        public uint InSync, HardLink, PlaceholderManagement;
    }
    [DllImport("cldapi.dll", CharSet = CharSet.Unicode)]
    static extern int CfRegisterSyncRoot(string path, ref CF_SYNC_REGISTRATION reg, ref CF_SYNC_POLICIES pol, uint flags);
    [DllImport("cldapi.dll", CharSet = CharSet.Unicode)]
    static extern int CfUnregisterSyncRoot(string path);
    [DllImport("cldapi.dll", CharSet = CharSet.Unicode)]
    static extern int CfGetSyncRootInfoByPath(string path, int infoClass, IntPtr buffer, uint length, out uint returned);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern uint GetCompressedFileSizeW(string path, out uint high);

    /// Registers `path` as a sync root of `provider` (no callbacks are ever connected). The HRESULT.
    public static int Register(string path, string provider, Guid id) {
        var reg = new CF_SYNC_REGISTRATION { ProviderName = provider, ProviderVersion = "1.0", ProviderId = id };
        reg.StructSize = (uint)Marshal.SizeOf(typeof(CF_SYNC_REGISTRATION));
        // Hydration FULL (2), population ALWAYS_FULL (3): Windows never asks a provider to fill a folder.
        var pol = new CF_SYNC_POLICIES { HydrationPrimary = 2, PopulationPrimary = 3 };
        pol.StructSize = (uint)Marshal.SizeOf(typeof(CF_SYNC_POLICIES));
        return CfRegisterSyncRoot(path, ref reg, ref pol, 0);
    }
    public static int Unregister(string path) { return CfUnregisterSyncRoot(path); }

    /// The provider name of the sync root `path` is in (CF_SYNC_ROOT_INFO_PROVIDER: the name follows the
    /// 4-byte status); null when it is in none.
    public static string ProviderOf(string path) {
        IntPtr buffer = Marshal.AllocHGlobal(4096);
        try {
            uint returned;
            if (CfGetSyncRootInfoByPath(path, 2, buffer, 4096, out returned) != 0) return null;
            return Marshal.PtrToStringUni(IntPtr.Add(buffer, 4));
        } finally { Marshal.FreeHGlobal(buffer); }
    }

    /// Bytes of the file's data on this disk; -1 when Windows does not say.
    public static long OnDisk(string path) {
        uint high; uint low = GetCompressedFileSizeW(path, out high);
        if (low == 0xFFFFFFFF && Marshal.GetLastWin32Error() != 0) return -1;
        return ((long)high << 32) | low;
    }

    // --- Credential Manager: target names only, never a value ------------------------------------
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CredEnumerateW(string filter, uint flags, out uint count, out IntPtr credentials);
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CredDeleteW(string target, uint type, uint flags);
    [DllImport("advapi32.dll")] static extern void CredFree(IntPtr buffer);

    /// The target names of the generic credentials matching `filter` (a prefix and "*"). The blob is
    /// never read: CREDENTIALW's Type is at offset 4 and its TargetName pointer at offset 8.
    public static string[] Targets(string filter) {
        uint count; IntPtr list;
        if (!CredEnumerateW(filter, 0, out count, out list)) return new string[0];
        try {
            var names = new List<string>();
            for (int i = 0; i < count; i++) {
                IntPtr cred = Marshal.ReadIntPtr(list, i * IntPtr.Size);
                if (Marshal.ReadInt32(cred, 4) == 1) names.Add(Marshal.PtrToStringUni(Marshal.ReadIntPtr(cred, 8)));
            }
            return names.ToArray();
        } finally { CredFree(list); }
    }
    public static bool DeleteGeneric(string target) { return CredDeleteW(target, 1, 0); }

    // --- The app's windows -------------------------------------------------------------------------
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
    public static RECT VisibleRect(IntPtr h) {
        RECT r;
        if (DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf(typeof(RECT))) == 0) return r;
        GetWindowRect(h, out r);
        return r;
    }
    public static long Area(IntPtr h) { var r = VisibleRect(h); return (long)Math.Max(0, r.Right - r.Left) * Math.Max(0, r.Bottom - r.Top); }
    public static string ClassOf(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
    public static void Shot(IntPtr h, string file) {
        var r = VisibleRect(h);
        int w = r.Right - r.Left, ht = r.Bottom - r.Top;
        if (w <= 0 || ht <= 0) return;
        using (var bmp = new Bitmap(w, ht)) {
            using (var g = Graphics.FromImage(bmp)) { g.CopyFromScreen(r.Left, r.Top, 0, 0, new Size(w, ht)); }
            bmp.Save(file, ImageFormat.Png);
        }
    }
}
"@

# The Explorer half of a sync root (StorageProviderSyncRootManager): the run removes what it made.
$winrt = $null
try {
    $winrt = [Windows.Storage.Provider.StorageProviderSyncRootManager, Windows.Storage.Provider, ContentType = WindowsRuntime]
} catch { $winrt = $null }
if ($ParseOnly) {
    Write-Host "parse-only: script parsed, helper compiled; WinRT sync-root manager: $(if ($winrt) { 'resolved' } else { 'NOT resolved (the cleanup would delete the registry key instead)' })"
    if ($winrt) { exit 0 } else { exit 1 }
}
[BbS1]::SetProcessDPIAware() | Out-Null

function Refuse([string]$why) { Write-Host "REFUSED: $why"; exit 2 }
$checks = 0; $failing = 0
function Check([bool]$ok, [string]$what) {
    $script:checks++
    if ($ok) { Write-Host "  PASS: $what" } else { $script:failing++; Write-Host "  FAIL: $what" }
}
function Pump([int]$ms) {
    $until = (Get-Date).AddMilliseconds($ms)
    while ((Get-Date) -lt $until) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 50 }
}

# --- P: nothing is started before every refusal has been checked ---------------------------------
foreach ($pair in @(@('-Binary', $Binary), @('-FakeApi', $FakeApi))) {
    if (-not $pair[1]) { Refuse "$($pair[0]) <path> is required" }
    if (-not (Test-Path -LiteralPath $pair[1] -PathType Leaf)) { Refuse "not found: $($pair[1])" }
}
$Binary = (Resolve-Path -LiteralPath $Binary).Path
$FakeApi = (Resolve-Path -LiteralPath $FakeApi).Path
$text = [System.Text.Encoding]::GetEncoding(28591).GetString([System.IO.File]::ReadAllBytes($Binary))
$testMark = $text.Contains('beebeeb-app-identity=io.beebeeb.app.test;')
$realMark = $text.Contains('beebeeb-app-identity=io.beebeeb.app;')
$text = $null
if (-not $testMark -or $realMark) { Refuse "the binary does not carry exactly the TEST identity mark (test: $testMark, real: $realMark)" }

$appName = [System.IO.Path]::GetFileNameWithoutExtension($Binary)
$folder = Join-Path $env:USERPROFILE 'Beebeeb Test'
$stateDirs = @((Join-Path $env:APPDATA 'beebeeb-test'), (Join-Path $env:LOCALAPPDATA 'io.beebeeb.app.test'))
# The TEST identity's credential targets are "io.beebeeb.app.test/<slot>"; the real app's "io.beebeeb.app/<slot>".
$credentialFilter = 'io.beebeeb.app.test/*'
function Count-Credentials { @([BbS1]::Targets($credentialFilter)).Count }
$rootsKey = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager'
function Sync-Roots { @(Get-ChildItem -LiteralPath $rootsKey -ErrorAction SilentlyContinue | ForEach-Object { $_.PSChildName } | Sort-Object) }
function Test-Roots { @(Sync-Roots | Where-Object { $_ -like 'BeebeebTest!*' }) }
$uninstall = @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall')
function State-Counts {
    [ordered]@{
        'settings and state folders' = @($stateDirs | Where-Object { Test-Path -LiteralPath $_ }).Count
        'Credential Manager entries' = Count-Credentials
        'installed copies' = @($uninstall | ForEach-Object { Get-ChildItem -LiteralPath $_ -ErrorAction SilentlyContinue } |
            Where-Object { (Get-ItemProperty -LiteralPath $_.PSPath -ErrorAction SilentlyContinue).DisplayName -eq 'Beebeeb Test' }).Count
        'running copies' = @(Get-Process -Name $appName -ErrorAction SilentlyContinue).Count
        'sync roots of BeebeebTest' = (Test-Roots).Count
        'a folder named Beebeeb Test' = [int](Test-Path -LiteralPath $folder)
    }
}
$counts = State-Counts
foreach ($k in $counts.Keys) { Write-Host "P: $k = $($counts[$k])" }
$dirty = @($counts.Keys | Where-Object { $counts[$_] -ne 0 })
if ($dirty.Count -gt 0) { Refuse "the TEST app has state on this machine ($($dirty -join ', '))" }
if (-not $SkipFirewall) {
    $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $admin) { Refuse "an elevated shell is needed for the outbound firewall block (or pass -SkipFirewall)" }
    $off = @(Get-NetFirewallProfile | Where-Object { [string]$_.Enabled -ne 'True' }).Count
    if ($off -gt 0) { Refuse "$off firewall profile(s) are off: a block rule would not hold (or pass -SkipFirewall)" }
}

$evidence = Join-Path ([System.IO.Path]::GetTempPath()) "bb-s1-rungs-$([guid]::NewGuid().ToString('N').Substring(0, 8))"
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
Write-Host "Evidence in $evidence"
$rootsBefore = Sync-Roots
$fake = $null; $app = $null; $firewallRule = $null; $own = @(); $launches = @()
$savedProxy = @{ HTTPS_PROXY = $env:HTTPS_PROXY; HTTP_PROXY = $env:HTTP_PROXY; ALL_PROXY = $env:ALL_PROXY; NO_PROXY = $env:NO_PROXY }
function Restore-Proxy {
    foreach ($k in $savedProxy.Keys) {
        if ($null -eq $savedProxy[$k]) { Remove-Item -Path "env:$k" -ErrorAction SilentlyContinue } else { Set-Item -Path "env:$k" -Value $savedProxy[$k] }
    }
}

# --- UI Automation: the app's accessibility tree, by control type and name ---------------------
$UIA = [System.Windows.Automation.AutomationElement]
$T = [System.Windows.Automation.ControlType]
$Scope = [System.Windows.Automation.TreeScope]::Descendants
function Find-In($root, [string]$name, $type) {
    if (-not $root) { return $null }
    $cond = New-Object System.Windows.Automation.PropertyCondition($UIA::NameProperty, $name)
    if ($type) {
        $byType = New-Object System.Windows.Automation.PropertyCondition($UIA::ControlTypeProperty, $type)
        $cond = [System.Windows.Automation.AndCondition]::new([System.Windows.Automation.Condition[]]@($cond, $byType))
    }
    try { return $root.FindFirst($Scope, $cond) } catch { return $null }
}
function Root-Of($hwnd) { try { return $UIA::FromHandle($hwnd) } catch { return $null } }
function Find-Now($hwnd, [string]$name, $type) { Find-In (Root-Of $hwnd) $name $type }
function Wait-For($hwnd, [string]$name, $type) {
    for ($i = 0; $i -lt ($StepSeconds * 2); $i++) { $el = Find-Now $hwnd $name $type; if ($el) { return $el }; Pump 500 }
    return $null
}
function Wait-Gone($hwnd, [string]$name, $type) {
    for ($i = 0; $i -lt 20; $i++) { if (-not (Find-Now $hwnd $name $type)) { return $true }; Pump 500 }
    return $false
}
# Every name in the tree that matches `pattern`, case-sensitive.
function Names-Like($hwnd, [string]$pattern) {
    $root = Root-Of $hwnd
    if (-not $root) { return @() }
    try { $all = $root.FindAll($Scope, [System.Windows.Automation.Condition]::TrueCondition) } catch { return @() }
    $names = @()
    foreach ($el in $all) { try { $n = $el.Current.Name } catch { continue }; if ($n -clike $pattern) { $names += $n } }
    return $names
}
function Wait-Names($hwnd, [string]$pattern, [int]$atLeast) {
    for ($i = 0; $i -lt ($StepSeconds * 2); $i++) { $n = @(Names-Like $hwnd $pattern); if ($n.Count -ge $atLeast) { return $n }; Pump 500 }
    return @(Names-Like $hwnd $pattern)
}
function Press($el) {
    if (-not $el) { return $false }
    try { $el.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke() } catch { return $false }
    Pump 500; return $true
}
function Keys([string]$s) { $s -replace '([+^%~(){}\[\]])', '{$1}' }
function Type-Into($hwnd, $el, [string]$keys) {
    if (-not $el) { return $false }
    [BbS1]::SetForegroundWindow($hwnd) | Out-Null
    try { $el.SetFocus() } catch { return $false }
    Pump 300; [System.Windows.Forms.SendKeys]::SendWait($keys); Pump 300; return $true
}
# The 12 words through the clipboard into the first box: a pasted phrase fills all twelve.
function Paste-Phrase($hwnd, [string]$words) {
    $group = Wait-For $hwnd 'Recovery phrase' $T::Group
    if (-not $group) { return $false }
    $edit = New-Object System.Windows.Automation.PropertyCondition($UIA::ControlTypeProperty, $T::Edit)
    try { $first = $group.FindFirst($Scope, $edit) } catch { return $false }
    if (-not $first) { return $false }
    [System.Windows.Forms.Clipboard]::SetText($words)
    $ok = Type-Into $hwnd $first '^v'
    [System.Windows.Forms.Clipboard]::Clear()
    Pump 500; return $ok
}
function Window-Of($proc) {
    for ($i = 0; $i -lt ($StepSeconds * 2); $i++) {
        Pump 500
        if ($proc.HasExited) { Write-Host "  the app exited with code $($proc.ExitCode)"; return [IntPtr]::Zero }
        $big = @([BbS1]::VisibleWindowsOf([uint32]$proc.Id) | Where-Object { [BbS1]::Area($_) -ge 40000 })
        if ($big.Count -gt 0) {
            if ([BbS1]::ClassOf($big[0]) -eq '#32770') { Write-Host "  a message box: the app refused to start"; return [IntPtr]::Zero }
            return $big[0]
        }
    }
    return [IntPtr]::Zero
}
function Start-App {
    $n = $script:launches.Count + 1
    $out = Join-Path $evidence "launch-$n"
    $env:HTTPS_PROXY = 'http://127.0.0.1:9'; $env:HTTP_PROXY = 'http://127.0.0.1:9'; $env:ALL_PROXY = 'http://127.0.0.1:9'; $env:NO_PROXY = ''
    $p = Start-Process -FilePath $Binary -ArgumentList '--api-base', $script:base -PassThru `
        -RedirectStandardOutput "$out.out.txt" -RedirectStandardError "$out.err.txt"
    Restore-Proxy
    Write-Host "  launch $n, PID $($p.Id)"
    $a = @{ Proc = $p; Out = "$out.out.txt"; Err = "$out.err.txt" }
    $script:launches += $a
    return $a
}
function Quit-App($a) {
    if (-not $a -or $a.Proc.HasExited) { return }
    # The app's own quit (the installer's request over the control channel), then the PID.
    Start-Process -FilePath $Binary -ArgumentList '--quit-for-update' -Wait -WindowStyle Hidden | Out-Null
    if ($a.Proc.WaitForExit(15000)) { Write-Host "  the app quit (PID $($a.Proc.Id))"; return }
    Stop-Process -Id $a.Proc.Id -Force -ErrorAction SilentlyContinue; $a.Proc.WaitForExit(5000) | Out-Null
    Write-Host "  the app did not quit in 15 s; stopped PID $($a.Proc.Id)"
}
function Lines($file) { @(Get-Content -LiteralPath $file -ErrorAction SilentlyContinue) }
function Lines-Like($file, [string]$pattern) { @(Lines $file | Where-Object { $_ -clike $pattern }).Count }
function Requests([string]$route) { @(Lines $fakeOut | Where-Object { $_ -ceq "request: $route" }).Count }
function Items { @(Get-ChildItem -LiteralPath $folder -Recurse -Force -ErrorAction SilentlyContinue) }
function Wait-Items([int]$n) {
    for ($i = 0; $i -lt ($StepSeconds * 2); $i++) { if ((Items).Count -ge $n) { break }; Pump 500 }
    return (Items).Count
}
$verify = 'POST /api/v1/auth/verify-recovery-check'
$unread = 'Your files cannot be read right now.'
function Running-Word($hwnd) {
    for ($i = 0; $i -lt ($StepSeconds * 2); $i++) {
        foreach ($w in @('Syncing', 'Up to date')) { if (Find-Now $hwnd $w $T::Text) { return $w } }
        Pump 500
    }
    return $null
}
# The root of the fake account: Documents and Photos, each a row named "<name>, changed <date>, ...".
function Root-Rows($hwnd, [string]$rung) {
    $rows = @(Wait-Names $hwnd '*, changed *' 2)
    Write-Host "  rows: $($rows.Count) ($(@($rows | ForEach-Object { ($_ -split ',')[0] }) -join ', '))"
    Check ($rows.Count -eq 2 -and @($rows | Where-Object { $_ -clike 'Documents, changed *' }).Count -eq 1 -and
        @($rows | Where-Object { $_ -clike 'Photos, changed *' }).Count -eq 1) "$rung the window lists the root's 2 rows, Documents and Photos"
    $said = @(Names-Like $hwnd $unread).Count
    if ($said -gt 0) { Write-Host "  the window says: $unread" }
    Check ($said -eq 0) "$rung no ""cannot be read"" text in the window"
}

try {
    if (-not $SkipFirewall) {
        $ruleName = "bb-rung-s1-$([guid]::NewGuid().ToString('N').Substring(0, 8))"
        $firewallRule = New-NetFirewallRule -DisplayName $ruleName -Direction Outbound -Program $Binary -Action Block -Profile Any `
            -RemoteAddress @('0.0.0.0-126.255.255.255', '128.0.0.0-255.255.255.255', '2000::/3', 'fc00::/7', 'fe80::/10')
        Write-Host "outbound firewall block for every address but loopback ($ruleName)"
    } else { Write-Host "-SkipFirewall: no firewall block; only the proxy layer keeps the app off the network" }

    $fakeOut = Join-Path $evidence 'fake-api.out.txt'
    $fake = Start-Process -FilePath $FakeApi -PassThru -WindowStyle Hidden -RedirectStandardOutput $fakeOut -RedirectStandardError (Join-Path $evidence 'fake-api.err.txt')
    $lines = @()
    for ($i = 0; $i -lt 40 -and $lines.Count -lt 4; $i++) { Pump 250; $lines = @(Lines $fakeOut) }
    $field = { param($name) ($lines | Where-Object { $_ -like "${name}: *" } | Select-Object -First 1) -replace "^${name}: ", '' }
    $script:base = & $field 'base'; $email = & $field 'email'; $password = & $field 'password'; $phrase = & $field 'phrase'
    if ($script:base -notmatch '^http://127\.0\.0\.1:\d+$') { throw "the fake API did not report a loopback base ($($script:base))" }
    Write-Host "fake API at $($script:base) (PID $($fake.Id)); the invented account and its 12 words come from its output"

    Write-Host "`n=== R1.4: a wrong phrase changes nothing ==="
    $app = Start-App
    $hwnd = Window-Of $app.Proc
    Check ($hwnd -ne [IntPtr]::Zero) "R1.4 the app's window is up"
    Press (Wait-For $hwnd 'Sign in' $T::Button) | Out-Null
    $typed = (Type-Into $hwnd (Wait-For $hwnd 'Email' $T::Edit) (Keys $email)) -and
        (Type-Into $hwnd (Wait-For $hwnd 'Password' $T::Edit) "$(Keys $password){ENTER}")
    Check $typed "R1.4 the email and the password are typed"
    Check ($null -ne (Wait-For $hwnd 'Recovery phrase' $T::Group)) "R1.4 the recovery phrase screen (DW-A23) after the password"
    $credsBefore = Count-Credentials; $checksBefore = Requests $verify
    Check ($credsBefore -gt 0) "R1.4 the count sees the TEST app's entries after the password ($credsBefore)"
    Paste-Phrase $hwnd 'zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong' | Out-Null
    Press (Wait-For $hwnd 'Restore keys' $T::Button) | Out-Null
    Check (@(Wait-Names $hwnd "That phrase doesn*" 1).Count -ge 1) "R1.4 DW-A24: that phrase doesn't open this account"
    for ($i = 0; $i -lt 20 -and (Requests $verify) -le $checksBefore; $i++) { Pump 500 }
    Pump 1000
    $checks1 = (Requests $verify) - $checksBefore
    Check ($checks1 -eq 1) "R1.4 the fake answered 1 recovery check ($checks1)"
    Check ((Count-Credentials) -eq $credsBefore) "R1.4 Credential Manager entries before = after the phrase ($credsBefore, $(Count-Credentials))"
    Check (-not (Test-Path -LiteralPath $folder)) "R1.4 no sync folder"

    Write-Host "`n=== R1.6: another provider's folder is refused, and the modal says so ==="
    New-Item -ItemType Directory -Path $folder -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $folder 'theirs.txt') -Value 'another app' -NoNewline
    $hr = [BbS1]::Register($folder, 'BbRungOther', [guid]::NewGuid()); $own += $folder
    Check ($hr -eq 0) ("R1.6 the folder is registered to BbRungOther (0x{0:X8})" -f $hr)
    Paste-Phrase $hwnd $phrase | Out-Null
    Press (Wait-For $hwnd 'Restore keys' $T::Button) | Out-Null
    $modal = Wait-For $hwnd 'Another app syncs this folder' $T::Window
    Check ($null -ne $modal) "R1.6 the refusal modal: another app syncs this folder"
    [BbS1]::SetForegroundWindow($hwnd) | Out-Null; Pump 800
    [BbS1]::Shot($hwnd, (Join-Path $evidence 'r1-6-refusal-modal.png'))
    $names = @(Items | ForEach-Object { $_.Name })
    Check ($names.Count -eq 1 -and $names[0] -eq 'theirs.txt') "R1.6 no placeholder was made (the folder holds only the other app's file: $($names.Count) item(s))"
    Check ([BbS1]::ProviderOf($folder) -eq 'BbRungOther') "R1.6 the other provider's registration is unchanged"
    Check ((Lines-Like $app.Err '*sync was not started: folder_of_another_provider') -eq 1) "R1.6 1 log line names the refusal's kind"
    Check (@(Lines $app.Err | Where-Object { $_.Contains($folder) }).Count -eq 0) "R1.6 0 log lines name the folder's path"
    Press (Find-In $modal 'Close' $T::Button) | Out-Null
    Check (Wait-Gone $hwnd 'Another app syncs this folder' $T::Window) "R1.6 Close closes the modal"
    Check ($null -ne (Wait-For $hwnd 'Sync is off' $T::Text)) "R1.6 the status line says Sync is off"

    Write-Host "`n=== R1.6 at launch, then the outside fix and Try again ==="
    Quit-App $app; $app = $null
    $app = Start-App
    $hwnd = Window-Of $app.Proc
    $modal = Wait-For $hwnd 'Another app syncs this folder' $T::Window
    Check ($null -ne $modal) "R1.6 the start at launch is refused too, and the modal opens"
    Check ((Lines-Like $app.Err '*sync was not started: folder_of_another_provider') -eq 1) "R1.6 1 refusal line at launch"
    $hr = [BbS1]::Unregister($folder); Remove-Item -LiteralPath (Join-Path $folder 'theirs.txt') -Force
    Check ($hr -eq 0 -and $null -eq [BbS1]::ProviderOf($folder)) "R1.6 the outside fix: the rung removed its own registration"
    Check (Press (Find-In $modal 'Try again' $T::Button)) "R1.6 the modal's Try again"

    Write-Host "`n=== R1.1: a registered, running sync, and the window says so ==="
    $count = Wait-Items 7
    Check ($count -eq 7) "R1.1 the sync folder has the account's 7 items ($count)"
    Check ((Lines-Like $app.Err '*beebeeb engine_host: sync started') -eq 1) "R1.1 Try again asked for 1 start, and it started"
    foreach ($l in $launches) {
        Check ((Lines $l.Out) -ccontains "api base: $($script:base) (--api-base)") "R1.1 the start line of $(Split-Path -Leaf $l.Out) names the loopback server"
    }
    Root-Rows $hwnd 'R1.1'
    $word = Running-Word $hwnd
    Check ($null -ne $word) "R1.1 the status line says a running engine's word ($word)"
    [BbS1]::SetForegroundWindow($hwnd) | Out-Null; Pump 800
    [BbS1]::Shot($hwnd, (Join-Path $evidence 'r1-1-files.png'))

    Write-Host "`n=== R1.2: the test identity's registration, and nothing else changed ==="
    $provider = [BbS1]::ProviderOf($folder)
    Check ($provider -eq 'BeebeebTest') "R1.2 the folder's provider is BeebeebTest ($provider)"
    Check ((Test-Roots).Count -eq 1) "R1.2 1 Explorer registration of BeebeebTest"
    $diff = @(Compare-Object -ReferenceObject @($rootsBefore) -DifferenceObject @(Sync-Roots))
    Check ($diff.Count -eq 1 -and $diff[0].SideIndicator -eq '=>' -and $diff[0].InputObject -like 'BeebeebTest!*') "R1.2 the sync roots before and after: only the new one differs ($($diff.Count) different)"

    Write-Host "`n=== R1.3: 7 placeholders, cloud-only, 0 bytes on disk ==="
    $all = @(Items); $files = @($all | Where-Object { -not $_.PSIsContainer })
    $notHere = 0x400000 -bor 0x40000 -bor 0x1000
    foreach ($f in $files) { Write-Host ("  {0}: attributes 0x{1:X8}, {2} byte(s) on disk" -f $f.Name, [int64]$f.Attributes, [BbS1]::OnDisk($f.FullName)) }
    $cloudOnly = @($files | Where-Object { ([int64]$_.Attributes -band $notHere) -ne 0 }).Count
    $onDisk = [int64]($files | ForEach-Object { [BbS1]::OnDisk($_.FullName) } | Measure-Object -Sum).Sum
    Check ($all.Count -eq 7 -and $files.Count -eq 5) "R1.3 7 placeholders: $($all.Count - $files.Count) folders, $($files.Count) files"
    Check ($cloudOnly -eq 5) "R1.3 every file is cloud-only by its attributes ($cloudOnly of 5)"
    Check ($onDisk -eq 0) "R1.3 0 bytes on disk ($onDisk)"
    Write-Host "  NOT RUN: opening a file (the fake API serves no file content)"

    Write-Host "`n=== R1.5: a root of the engine's QA provider beside it ==="
    Quit-App $app; $app = $null
    $qa = Join-Path ([System.IO.Path]::GetTempPath()) "bb-s1-qa-$([guid]::NewGuid().ToString('N').Substring(0, 6))"
    New-Item -ItemType Directory -Path $qa -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $qa 'qa.txt') -Value 'the QA provider' -NoNewline
    $hr = [BbS1]::Register($qa, 'BeebeebQA', [guid]'2e0cdc60-1a9c-48f3-81c9-3e402af235b0'); $own += $qa
    Check ($hr -eq 0) ("R1.5 a second root registered to BeebeebQA (0x{0:X8})" -f $hr)
    $app = Start-App
    $hwnd = Window-Of $app.Proc
    Root-Rows $hwnd 'R1.5'
    Pump 5000
    Check ([BbS1]::ProviderOf($folder) -eq 'BeebeebTest' -and [BbS1]::ProviderOf($qa) -eq 'BeebeebQA') "R1.5 both stay registered, each to its own provider"
    Check ((Get-Content -LiteralPath (Join-Path $qa 'qa.txt') -Raw) -ceq 'the QA provider' -and @(Get-ChildItem -LiteralPath $qa -Force).Count -eq 1) "R1.5 the QA root keeps its own file, and only it"
    Check ((Items).Count -eq 7) "R1.5 the app's root keeps its 7 items"
    Write-Host "  NOT READ: whether a callback of one provider fires for the other (the app keeps no engine log)"

    Write-Host "`n=== R1.7: quit and start again: Files without the phrase ==="
    Quit-App $app; $app = $null
    $app = Start-App
    $hwnd = Window-Of $app.Proc
    Root-Rows $hwnd 'R1.7'
    Check ($null -eq (Find-Now $hwnd 'Recovery phrase' $T::Group) -and $null -eq (Find-Now $hwnd 'Email' $T::Edit)) "R1.7 no sign-in and no phrase screen"
    $word = Running-Word $hwnd
    Check ($null -ne $word) "R1.7 the status line says a running engine's word ($word)"
    Check ((Test-Roots).Count -eq 1) "R1.7 still 1 Explorer registration of BeebeebTest"
    Check ((Items).Count -eq 7) "R1.7 still 7 placeholders"
    [BbS1]::SetForegroundWindow($hwnd) | Out-Null; Pump 800
    [BbS1]::Shot($hwnd, (Join-Path $evidence 'r1-7-files-after-restart.png'))
} finally {
    Quit-App $app
    if ($fake -and -not $fake.HasExited) { Stop-Process -Id $fake.Id -Force -ErrorAction SilentlyContinue; Write-Host "stopped the fake API (PID $($fake.Id))" }
    Restore-Proxy
    if ($firewallRule) { Remove-NetFirewallRule -Name $firewallRule.Name -ErrorAction SilentlyContinue; Write-Host "firewall block removed" }
    Write-Host "`n=== Cleanup: what the run made ==="
    foreach ($id in @(Test-Roots | Where-Object { $rootsBefore -notcontains $_ })) {
        $how = 'not removed'
        if ($winrt) { try { $winrt::Unregister($id); $how = 'removed' } catch { $how = "not removed ($($_.Exception.Message))" } }
        if (Test-Path -LiteralPath "$rootsKey\$id") { Remove-Item -LiteralPath "$rootsKey\$id" -Recurse -Force -ErrorAction SilentlyContinue; $how += '; its registry key deleted' }
        Write-Host "  the app's Explorer registration: $how"
    }
    foreach ($root in @($folder) + $own) {
        if ([BbS1]::ProviderOf($root)) { Write-Host ("  Cloud Files registration of {0}: removed (0x{1:X8})" -f (Split-Path -Leaf $root), [BbS1]::Unregister($root)) }
        if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }
    foreach ($d in $stateDirs) { if (Test-Path -LiteralPath $d) { Remove-Item -LiteralPath $d -Recurse -Force -ErrorAction SilentlyContinue } }
    $targets = @([BbS1]::Targets($credentialFilter))
    $deleted = @($targets | Where-Object { [BbS1]::DeleteGeneric($_) }).Count
    Write-Host "  Credential Manager entries of the TEST app: $deleted of $($targets.Count) removed"
    $after = State-Counts
    foreach ($k in $after.Keys) { Write-Host "  after: $k = $($after[$k])" }
    Check (@($after.Keys | Where-Object { $after[$_] -ne 0 }).Count -eq 0) "cleanup: the TEST app has nothing left on this machine (the six P counts are 0)"
}

Write-Host "`n$checks checks, $failing failing"
if ($failing -gt 0) { exit 1 }
exit 0

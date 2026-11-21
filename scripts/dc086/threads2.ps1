# dc-086: per-thread CPU of the TEST app over a few seconds, by thread name and by the module its start address is in. Read-only.
param([int]$Seconds = 5)
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class TH2 {
  [DllImport("kernel32.dll")] static extern IntPtr OpenThread(uint access, bool inherit, uint id);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
  [DllImport("kernel32.dll")] static extern int GetThreadDescription(IntPtr h, out IntPtr text);
  [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr p);
  [DllImport("ntdll.dll")] static extern int NtQueryInformationThread(IntPtr h, int cls, out IntPtr v, int len, IntPtr ret);
  public static string Name(uint id) {
    IntPtr h = OpenThread(0x0800, false, id); if (h == IntPtr.Zero) return "?";
    try { IntPtr p; if (GetThreadDescription(h, out p) < 0) return "?"; string s = Marshal.PtrToStringUni(p); LocalFree(p); return string.IsNullOrEmpty(s) ? "(unnamed)" : s; } finally { CloseHandle(h); }
  }
  public static long Start(uint id) {
    IntPtr h = OpenThread(0x0040, false, id); if (h == IntPtr.Zero) return 0;
    try { IntPtr v; if (NtQueryInformationThread(h, 9, out v, IntPtr.Size, IntPtr.Zero) != 0) return 0; return v.ToInt64(); } finally { CloseHandle(h); }
  }
}
"@
$p = @(Get-Process beebeeb-desktop-test -ea 0 | Sort-Object WorkingSet64 -Descending)[0]
if (-not $p) { 'app not running'; exit }
$mods = @($p.Modules | ForEach-Object { [pscustomobject]@{ n = $_.ModuleName; b = $_.BaseAddress.ToInt64(); e = $_.BaseAddress.ToInt64() + $_.ModuleMemorySize } })
function Mod([long]$a) { foreach ($m in $mods) { if ($a -ge $m.b -and $a -lt $m.e) { return ('{0}+0x{1:X}' -f $m.n, ($a - $m.b)) } }; 'unknown' }
$a = @{}; foreach ($t in $p.Threads) { try { $a[$t.Id] = $t.TotalProcessorTime.TotalSeconds } catch {} }
Start-Sleep -Seconds $Seconds
$p.Refresh(); $rows = @()
foreach ($t in $p.Threads) { try { $d = $t.TotalProcessorTime.TotalSeconds - $(if ($a.ContainsKey($t.Id)) { $a[$t.Id] } else { 0 }); $rows += [pscustomobject]@{ key = ([TH2]::Name([uint32]$t.Id) + ' @ ' + (Mod ([TH2]::Start([uint32]$t.Id)))); d = $d; total = $t.TotalProcessorTime.TotalSeconds } } catch {} }
"threads: $($rows.Count); CPU in $Seconds s: $([Math]::Round(($rows | Measure-Object d -Sum).Sum, 1)) s; modules: $($mods.Count)"
$rows | Group-Object key | ForEach-Object { [pscustomobject]@{ name = $_.Name; n = $_.Count; d = ($_.Group | Measure-Object d -Sum).Sum; total = ($_.Group | Measure-Object total -Sum).Sum } } | Sort-Object total -Descending | Select-Object -First 10 | ForEach-Object { '{0,-62} threads={1,-3} cpu/{4}s={2,6:0.0} total={3,7:0}s' -f $_.name, $_.n, $_.d, $_.total, $Seconds }
"root entries: $(@([IO.Directory]::GetFileSystemEntries((Join-Path $env:USERPROFILE 'Beebeeb Test'))).Count)"

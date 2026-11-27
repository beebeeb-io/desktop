<#
.SYNOPSIS
  Makes a deterministic backup test tree (same -Seed = same tree, byte for byte).
.DESCRIPTION
  Windows PowerShell 5.1, no modules. Needs no secret, account or host.
  -Root     folder to fill (created; made on first use)
  -Files    number of files (default 41000), 1-64 KiB of seeded pseudo-random bytes each
  -Folders  number of folders under Root (default 2700), Root itself not counted
  -Depth    deepest level under Root (default 7)
  -Seed     same seed + same numbers = same tree (default 1)
  -AddDeep  adds ONE new file in the deepest folder of an existing tree and prints its path
            relative to Root; give the same -Folders -Depth -Seed the tree was made with
  -Verify   (makes the tree first when Root is missing or empty) counts files and folders under Root, prints them with one SHA-256 over the sorted
            relative paths and sizes; exit 1 when the counts differ from -Files / -Folders
  After -AddDeep the counts are one file higher: run -Verify with -Files <n+1> then.
#>
param(
    [Parameter(Mandatory = $true)][string]$Root,
    [int]$Files = 41000,
    [int]$Folders = 2700,
    [int]$Depth = 7,
    [long]$Seed = 1,
    [switch]$AddDeep,
    [switch]$Verify
)
$ErrorActionPreference = 'Stop'

if ($Files -lt 1 -or $Folders -lt 1 -or $Depth -lt 1) { throw 'Files, Folders and Depth must be at least 1' }
if ($Files -lt $Folders) { throw 'Files must be at least Folders (every folder gets a file)' }

if (-not ('BbTreeMaker' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.IO;
using System.Text;

public sealed class BbRng
{
    private ulong s;
    public BbRng(long seed) { s = unchecked((ulong)seed * 0x9E3779B97F4A7C15UL + 0x1234567UL); }
    public ulong Next()
    {
        unchecked
        {
            s += 0x9E3779B97F4A7C15UL;
            ulong z = s;
            z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9UL;
            z = (z ^ (z >> 27)) * 0x94D049BB133111EBUL;
            return z ^ (z >> 31);
        }
    }
    public int Below(int n) { return (int)(Next() % (ulong)n); }
    public byte[] Bytes(int n)
    {
        byte[] b = new byte[n];
        int i = 0;
        while (i < n)
        {
            ulong v = Next();
            for (int k = 0; k < 8 && i < n; k++) { b[i++] = (byte)(v & 0xFF); v >>= 8; }
        }
        return b;
    }
}

public static class BbTreeMaker
{
    static readonly string[] Syl = {
        "ba","be","bi","bo","ku","ka","ke","ko","da","de","di","do","fa","fe","fi","fo",
        "ga","ge","gi","go","la","le","li","lo","ma","me","mi","mo","na","ne","ni","no",
        "pa","pe","pi","po","ra","re","ri","ro","sa","se","si","so","ta","te","ti","to",
        "va","ve","vi","vo","za","ze","zi","zo" };
    static readonly string[] Ext = { ".jpg", ".png", ".heic", ".webp", ".bin" };

    static string Word(BbRng r)
    {
        int n = 2 + r.Below(2);
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < n; i++) sb.Append(Syl[r.Below(Syl.Length)]);
        return sb.ToString();
    }

    // Folder 0 is the root. A chain of Depth folders first (so the deepest level exists),
    // then every other folder under a random earlier folder that is above the deepest level.
    public static void Layout(long seed, int folders, int depth, out int[] parent, out int[] level, out string[] name)
    {
        BbRng r = new BbRng(seed);
        parent = new int[folders + 1];
        level = new int[folders + 1];
        name = new string[folders + 1];
        for (int i = 1; i <= folders; i++)
        {
            if (i <= depth) parent[i] = i - 1;
            else
            {
                int p;
                do { p = r.Below(i); } while (level[p] >= depth);
                parent[i] = p;
            }
            level[i] = level[parent[i]] + 1;
            name[i] = Word(r) + "-" + i.ToString("x");
        }
    }

    static string[] Paths(string root, int folders, int[] parent, string[] name)
    {
        string[] p = new string[folders + 1];
        p[0] = root;
        for (int i = 1; i <= folders; i++) p[i] = Path.Combine(p[parent[i]], name[i]);
        return p;
    }

    public static void Make(string root, int files, int folders, int depth, long seed)
    {
        int[] parent; int[] level; string[] name;
        Layout(seed, folders, depth, out parent, out level, out name);
        string[] paths = Paths(root, folders, parent, name);
        for (int i = 1; i <= folders; i++) Directory.CreateDirectory(paths[i]);
        BbRng r = new BbRng(seed ^ 0x5DEECE66DL);
        DateTime baseTime = new DateTime(2024, 1, 1, 0, 0, 0, DateTimeKind.Utc);
        for (int k = 0; k < files; k++)
        {
            int f = k < folders ? k + 1 : 1 + r.Below(folders);
            string file = Path.Combine(paths[f], Word(r) + "-" + k.ToString("x") + Ext[r.Below(Ext.Length)]);
            int size = 1024 + r.Below(63 * 1024 + 1);
            File.WriteAllBytes(file, r.Bytes(size));
            File.SetLastWriteTimeUtc(file, baseTime.AddMinutes(k));
        }
    }

    public static string AddDeep(string root, int folders, int depth, long seed)
    {
        int[] parent; int[] level; string[] name;
        Layout(seed, folders, depth, out parent, out level, out name);
        string[] paths = Paths(root, folders, parent, name);
        int deepest = Math.Min(depth, folders);
        int n = 1;
        string file;
        while (File.Exists(file = Path.Combine(paths[deepest], "added-" + n.ToString() + ".jpg"))) n++;
        BbRng r = new BbRng(seed + 1000003L * n);
        File.WriteAllBytes(file, r.Bytes(1024 + r.Below(63 * 1024 + 1)));
        return file.Substring(root.Length).TrimStart('\\');
    }
}
'@
}

$rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\')

if ($AddDeep) {
    if (-not (Test-Path -LiteralPath $rootFull)) { throw 'Root does not exist: make the tree first' }
    [BbTreeMaker]::AddDeep($rootFull, $Folders, $Depth, $Seed)
}
else {
    $empty = (-not (Test-Path -LiteralPath $rootFull)) -or ((@(Get-ChildItem -LiteralPath $rootFull -Force)).Count -eq 0)
    if ($empty) {
        [void](New-Item -ItemType Directory -Force -Path $rootFull)
        [BbTreeMaker]::Make($rootFull, $Files, $Folders, $Depth, $Seed)
    }
    elseif (-not $Verify) {
        throw 'Root is not empty: pick an empty folder (or use -Verify / -AddDeep on an existing tree)'
    }
}

if ($Verify) {
    if (-not (Test-Path -LiteralPath $rootFull)) { Write-Output 'Root does not exist'; exit 1 }
    $lines = New-Object System.Collections.Generic.List[string]
    $nFiles = 0
    foreach ($f in [System.IO.Directory]::EnumerateFiles($rootFull, '*', [System.IO.SearchOption]::AllDirectories)) {
        $rel = $f.Substring($rootFull.Length).TrimStart('\').Replace('\', '/')
        $lines.Add($rel + "`t" + (New-Object System.IO.FileInfo $f).Length)
        $nFiles++
    }
    $nFolders = @([System.IO.Directory]::EnumerateDirectories($rootFull, '*', [System.IO.SearchOption]::AllDirectories)).Count
    $lines.Sort([System.StringComparer]::Ordinal)
    $bytes = [System.Text.Encoding]::UTF8.GetBytes((($lines.ToArray()) -join "`n") + "`n")
    $sha = [System.Security.Cryptography.SHA256]::Create()
    $hash = ([System.BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    Write-Output ('files=' + $nFiles)
    Write-Output ('folders=' + $nFolders)
    Write-Output ('sha256=' + $hash)
    if ($nFiles -ne $Files -or $nFolders -ne $Folders) {
        Write-Output ('MISMATCH: wanted files=' + $Files + ' folders=' + $Folders)
        exit 1
    }
}

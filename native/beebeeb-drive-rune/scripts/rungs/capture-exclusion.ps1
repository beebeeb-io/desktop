#Requires -Version 5.1
<#
.SYNOPSIS
    Native rung: verify screen-capture exclusion on secret screens.
.DESCRIPTION
    1. Positive control: creates a small WinForms window with a known colour, captures
       it, and asserts it is NOT black. This proves the capture method works.
    2. Starts the TEST-identity build (which opens on sign-in, a secret screen), captures
       the window region, asserts it IS black (excluded from capture).
    3. Asserts the window rect is non-empty and visible before judging it black.
    Prints "N checks, 0 failing" at the end.
.NOTES
    Task 2460. Run as: PowerShell -ExecutionPolicy Bypass -File capture-exclusion.ps1 [-Binary <path>]
    This rung has NOT been run natively (stated in the PR).
#>
[CmdletBinding()]
param(
    [string]$Binary
)
$ErrorActionPreference = 'Stop'

if (-not $Binary) {
    $repoRoot = (Resolve-Path "$PSScriptRoot\..\..").Path
    $Binary = Join-Path $repoRoot 'target\release\beebeeb-test.exe'
}
if (-not (Test-Path $Binary)) {
    Write-Error "Binary not found at $Binary"
    exit 1
}

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) "bb-capture-rung-$([guid]::NewGuid().ToString('N').Substring(0,8))"
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECT { public int Left, Top, Right, Bottom; }
public class User32Rung {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, ref RECT rect);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
}
"@

$checks = 0; $failing = 0

function Capture-WindowRegion([IntPtr]$Hwnd, [string]$OutPath) {
    $rect = New-Object RECT
    [User32Rung]::GetWindowRect($Hwnd, [ref]$rect) | Out-Null
    $w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
    if ($w -le 0 -or $h -le 0) { return $false }
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
    $g.Dispose(); $bmp.Save($OutPath, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    return $true
}

function Black-Ratio([string]$ImagePath) {
    $bmp = [System.Drawing.Bitmap]::FromFile($ImagePath)
    $black = 0; $sampled = 0
    for ($y = 0; $y -lt $bmp.Height; $y += 2) {
        for ($x = 0; $x -lt $bmp.Width; $x += 2) {
            $px = $bmp.GetPixel($x, $y); $sampled++
            if ($px.R -lt 15 -and $px.G -lt 15 -and $px.B -lt 15) { $black++ }
        }
    }
    $bmp.Dispose()
    return [double]$black / [math]::Max($sampled, 1)
}

function Assert-Visible([IntPtr]$Hwnd) {
    $rect = New-Object RECT
    [User32Rung]::GetWindowRect($Hwnd, [ref]$rect) | Out-Null
    $w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
    $visible = [User32Rung]::IsWindowVisible($Hwnd)
    Write-Host "  Rect: ${w}x${h} at ($($rect.Left),$($rect.Top)), visible=$visible"
    return ($w -gt 0 -and $h -gt 0 -and $visible)
}

# === Check 1: positive control (proves the capture method sees a non-excluded window) ===
Write-Host "`n=== Check 1: positive control (WinForms window, known colour) ==="
$form = New-Object System.Windows.Forms.Form
$form.Text = "bb-capture-control"; $form.Width = 200; $form.Height = 200
$form.BackColor = [System.Drawing.Color]::FromArgb(255, 0, 128, 255)
$form.StartPosition = "CenterScreen"; $form.TopMost = $true; $form.Show()
Start-Sleep -Milliseconds 500
$ctrlShot = Join-Path $tempDir 'control.png'
$ok = Capture-WindowRegion -Hwnd $form.Handle -OutPath $ctrlShot
$form.Close(); $form.Dispose()
$checks++
if (-not $ok) {
    Write-Host "  FAIL: could not capture the control window"
    $failing++
} else {
    $ratio = Black-Ratio $ctrlShot
    Write-Host "  Control black ratio: $([math]::Round($ratio, 3)) (must be < 0.5)"
    if ($ratio -ge 0.5) {
        Write-Host "  FAIL: positive control is mostly black (capture method broken)"
        $failing++
    } else {
        Write-Host "  PASS"
    }
}

# === Check 2: the app's secret screen (sign-in) is excluded ===
Write-Host "`n=== Check 2: secret screen (sign-in, the test build) ==="
$proc = Start-Process -FilePath $Binary -PassThru -WindowStyle Normal
Start-Sleep -Seconds 5
if ($proc.HasExited) {
    Write-Host "  FAIL: binary exited immediately (exit code $($proc.ExitCode))"
    $checks++; $failing++
} else {
    $hwnd = $proc.MainWindowHandle
    if ($hwnd -eq [IntPtr]::Zero) { Start-Sleep -Seconds 3; $hwnd = $proc.MainWindowHandle }
    if ($hwnd -eq [IntPtr]::Zero) {
        Write-Host "  FAIL: no window handle"
        $checks++; $failing++
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    } else {
        [User32Rung]::SetForegroundWindow($hwnd) | Out-Null
        Start-Sleep -Seconds 2

        # Check 2a: window is visible and non-empty.
        $checks++
        if (-not (Assert-Visible $hwnd)) {
            Write-Host "  FAIL: window not visible or zero-size"
            $failing++
        } else {
            Write-Host "  PASS: window visible"
        }

        # Check 2b: captured region is black.
        $secretShot = Join-Path $tempDir 'secret.png'
        $ok = Capture-WindowRegion -Hwnd $hwnd -OutPath $secretShot
        $checks++
        if (-not $ok) {
            Write-Host "  FAIL: could not capture"
            $failing++
        } else {
            $ratio = Black-Ratio $secretShot
            Write-Host "  Secret black ratio: $([math]::Round($ratio, 3)) (must be >= 0.95)"
            if ($ratio -lt 0.95) {
                Write-Host "  FAIL: secret screen is NOT excluded from capture"
                $failing++
            } else {
                Write-Host "  PASS: secret screen excluded"
            }
        }
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    }
}

# === Result ===
Write-Host "`n$checks checks, $failing failing"
Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue 2>$null
if ($failing -gt 0) { exit 1 }
exit 0

# THROWAWAY (task 2477 spike). Runs ON THE WINDOWS BUILD VM only, through remote-build.
#   -Phase install   download the official LibreOffice MSI, check its SHA-256, extract it with an
#                    ADMINISTRATIVE install (msiexec /a: files only, nothing registered)
#   -Phase run       build the spike, prove the firewall block, render the fixtures, write metrics
#   -Phase remove    delete everything the spike put on the machine and count what is left
# Everything lives under ONE folder ($Root); the only other thing touched is one firewall rule.
param(
    [Parameter(Mandatory = $true)][ValidateSet('install', 'run', 'artcraft', 'remove', 'count')][string]$Phase,
    [string]$Root = 'C:\bb\spike-2477'
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Version = '26.2.7'
$MsiName = "LibreOffice_${Version}_Win_x86-64.msi"
$MsiUrl = "https://download.documentfoundation.org/libreoffice/stable/$Version/win/x86_64/$MsiName"
# From $MsiUrl.sha256 (The Document Foundation's own checksum), read on 2026-11-12.
$MsiSha256 = '480fecf759fc6393bd614fbf43753eb95a8198fc1a065d41b727e3b664cd1e59'
$HeaderBase = 'https://raw.githubusercontent.com/LibreOffice/core/libreoffice-26-2-7/include/LibreOfficeKit'
$Headers = @{
    'LibreOfficeKit.h'      = 'd3828a4aa8fb61bd631e841d646b54939402a4be96addb90421d12ad2711f471'
    'LibreOfficeKitInit.h'  = 'c1eefee03b918b6d0ca79d172a9235b9bf53d453a94713f482c26402d614f793'
    'LibreOfficeKitEnums.h' = 'e9051ce6f4cbe3650afe679d8a757894db798a78afe2c45133b506c35cc78d83'
    'LibreOfficeKitTypes.h' = '28700c222ef85b82a6b061f20516872f89c6bee4d0cbeb21de575793c1540064'
}
$RuleName = 'bb-spike-2477-block'
$Here = Split-Path -Parent $MyInvocation.MyCommand.Path

function Sha256([string]$Path) { (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLower() }

function Count-Leftovers([string]$When) {
    $files = 0
    if (Test-Path -LiteralPath $Root) { $files = @(Get-ChildItem -LiteralPath $Root -Recurse -Force -File).Count }
    $keys = @('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*')
    $uninstall = @(Get-ItemProperty $keys -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -like '*LibreOffice*' }).Count
    $vendor = @('HKLM:\SOFTWARE\LibreOffice', 'HKLM:\SOFTWARE\The Document Foundation', 'HKCU:\SOFTWARE\LibreOffice',
        'HKCU:\SOFTWARE\The Document Foundation' | Where-Object { Test-Path $_ }).Count
    $rules = @(Get-NetFirewallRule -DisplayName "$RuleName*" -ErrorAction SilentlyContinue).Count
    $profiles = @("$env:APPDATA\LibreOffice", "$env:LOCALAPPDATA\LibreOffice", "$env:ProgramFiles\LibreOffice",
        "$env:APPDATA\WordCraft", "$env:APPDATA\GridCraft", "$env:APPDATA\DeckCraft" |
        Where-Object { Test-Path -LiteralPath $_ }).Count
    $procs = @(Get-Process -Name 'lok-spike', 'soffice', 'soffice.bin' -ErrorAction SilentlyContinue).Count
    Write-Output ("leftovers[$When]: files_under_root=$files uninstall_entries=$uninstall vendor_registry_keys=$vendor " +
        "firewall_rules=$rules profile_or_program_dirs=$profiles processes=$procs")
}

function Find-Program {
    $bin = Get-ChildItem -LiteralPath "$Root\lo" -Recurse -Filter 'soffice.bin' -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $bin) { throw "no soffice.bin under $Root\lo: run -Phase install first" }
    $bin.DirectoryName
}

if ($Phase -eq 'count') { Count-Leftovers 'now'; exit 0 }

if ($Phase -eq 'install') {
    Count-Leftovers 'before-install'
    New-Item -ItemType Directory -Force $Root | Out-Null
    $msi = "$Root\$MsiName"
    if (-not (Test-Path -LiteralPath $msi)) {
        $t = Get-Date
        & curl.exe -fsSL -o $msi $MsiUrl
        if ($LASTEXITCODE -ne 0) { throw "download failed ($LASTEXITCODE)" }
        Write-Output ("downloaded in {0:N0} s" -f ((Get-Date) - $t).TotalSeconds)
    }
    $got = Sha256 $msi
    Write-Output "msi: $MsiName bytes=$((Get-Item -LiteralPath $msi).Length) sha256=$got"
    if ($got -ne $MsiSha256) { Remove-Item -LiteralPath $msi -Force; throw "MSI SHA-256 mismatch: expected $MsiSha256" }
    Write-Output 'msi sha256: MATCHES the pinned official checksum'
    $sig = Get-AuthenticodeSignature -LiteralPath $msi
    Write-Output "msi signature: $($sig.Status) / $($sig.SignerCertificate.Subject)"
    if (-not (Test-Path -LiteralPath "$Root\lo")) {
        $t = Get-Date
        $p = Start-Process msiexec.exe -ArgumentList '/a', "`"$msi`"", '/qn', '/norestart', "TARGETDIR=`"$Root\lo`"" -Wait -PassThru
        if ($p.ExitCode -ne 0) { throw "msiexec /a exit code $($p.ExitCode)" }
        Write-Output ("extracted (msiexec /a, administrative install) in {0:N0} s" -f ((Get-Date) - $t).TotalSeconds)
    }
    $prog = Find-Program
    $all = Get-ChildItem -LiteralPath "$Root\lo" -Recurse -Force -File
    Write-Output ("extracted: files={0} bytes={1} ({2:N0} MB)" -f $all.Count, ($all | Measure-Object Length -Sum).Sum, (($all | Measure-Object Length -Sum).Sum / 1MB))
    Get-ChildItem -LiteralPath "$Root\lo" -Force | ForEach-Object {
        $s = (Get-ChildItem -LiteralPath $_.FullName -Recurse -Force -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum)
        Write-Output ("  top-level {0}: files={1} MB={2:N0}" -f $_.Name, $s.Count, ($s.Sum / 1MB))
    }
    Write-Output "program dir: $($prog.Substring($Root.Length))"
    foreach ($n in 'sofficeapp.dll', 'mergedlo.dll', 'soffice.bin') {
        $f = Join-Path $prog $n
        if (Test-Path -LiteralPath $f) { Write-Output "  $n bytes=$((Get-Item -LiteralPath $f).Length)" } else { Write-Output "  $n MISSING" }
    }
    Write-Output '  licence / notice files at the top of the install:'
    Get-ChildItem -LiteralPath (Split-Path -Parent $prog) -File -Force | ForEach-Object { Write-Output "    $($_.Name) bytes=$($_.Length)" }
    Count-Leftovers 'after-install'
    exit 0
}

if ($Phase -eq 'remove') {
    Count-Leftovers 'before-remove'
    Get-NetFirewallRule -DisplayName "$RuleName*" -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    "$env:APPDATA\WordCraft", "$env:APPDATA\GridCraft", "$env:APPDATA\DeckCraft" | Where-Object { Test-Path -LiteralPath $_ } | ForEach-Object { Remove-Item -LiteralPath $_ -Recurse -Force }
    if (Test-Path -LiteralPath $Root) { Remove-Item -LiteralPath $Root -Recurse -Force }
    Count-Leftovers 'after-remove'
    exit 0
}

if ($Phase -eq 'artcraft') {
    # The Rust option: the official Windows release zips of WordCraft, GridCraft and DeckCraft, their
    # command-line tools only, on the same fixtures, with the same per-program firewall block.
    $out = $env:BB_OUT
    if (-not $out) { throw 'BB_OUT is not set (run through remote-build)' }
    $log = "$out\artcraft.log"
    function Say([string]$m) { Write-Output $m; Add-Content -LiteralPath $log -Value $m }
    $Apps = @(
        @{ n = 'wordcraft'; v = '0.18.1'; sha = '257f670514057cb14967b88c85ac82d165da43ee7671e185acead61420af0389' },
        @{ n = 'gridcraft'; v = '0.19.1'; sha = '0b4bf8535f4904fac654326a360ff6d74ea6f4d184a8257c106388fafe74c9ac' },
        @{ n = 'deckcraft'; v = '0.18.1'; sha = '6df6f60b2182065c427fe4c3fe1b5d74b92706b238c6f35f3f91e5f359042e99' })
    $ac = "$Root\artcraft"
    New-Item -ItemType Directory -Force $ac, "$out\artcraft" | Out-Null
    $cli = @{}
    foreach ($a in $Apps) {
        $zip = "$ac\$($a.n).zip"
        if (-not (Test-Path -LiteralPath $zip)) {
            & curl.exe -fsSL -o $zip "https://github.com/storytold/$($a.n)/releases/download/v$($a.v)/$($a.n)-$($a.v)-windows-x64-portable.zip"
            if ($LASTEXITCODE -ne 0) { throw "download failed: $($a.n)" }
        }
        if ((Sha256 $zip) -ne $a.sha) { throw "SHA-256 mismatch: $($a.n) (pinned from the release's SHA256SUMS.txt)" }
        if (-not (Test-Path -LiteralPath "$ac\$($a.n)")) { Expand-Archive -LiteralPath $zip -DestinationPath "$ac\$($a.n)" }
        $exe = Get-ChildItem -LiteralPath "$ac\$($a.n)" -Recurse -Filter "$($a.n)-cli.exe" | Select-Object -First 1
        $cli[$a.n] = $exe.FullName
        $sig = Get-AuthenticodeSignature -LiteralPath $exe.FullName
        Say "$($a.n) $($a.v): zip sha256 MATCHES; cli bytes=$($exe.Length); signature=$($sig.Status) / $($sig.SignerCertificate.Subject)"
    }
    function Run-Cli([string]$App, [string]$Tag, [string[]]$A) {
        $so = "$ac\out-$Tag.txt"; $se = "$ac\err-$Tag.txt"
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $p = Start-Process -FilePath $cli[$App] -ArgumentList $A -PassThru -NoNewWindow -RedirectStandardOutput $so -RedirectStandardError $se
        $null = $p.Handle
        if (-not $p.WaitForExit(90000)) { Stop-Process -Id $p.Id -Force; Say "RED ${Tag}: no exit in 90 s, killed pid $($p.Id)"; return 99 }
        $p.WaitForExit(); $sw.Stop()
        $e = (@(Get-Content -LiteralPath $se -ErrorAction SilentlyContinue) | Select-Object -First 2) -join ' | '
        Say ("{0}: exit={1} ms={2} peak_ws_mb={3:N0} stderr='{4}'" -f $Tag, $p.ExitCode, $sw.ElapsedMilliseconds, ($p.PeakWorkingSet64 / 1MB), $e)
        Copy-Item -LiteralPath $so "$out\artcraft\$Tag.stdout.txt"
        return $p.ExitCode
    }
    try {
        foreach ($a in $Apps) { New-NetFirewallRule -DisplayName "$RuleName-$($a.n)" -Direction Outbound -Program $cli[$a.n] -Action Block -Profile Any | Out-Null }
        Say "firewall: $(@(Get-NetFirewallRule -DisplayName "$RuleName-*").Count) of 3 outbound block rules in place (the mechanism the run phase proves with a probe)"
        $fx = "$Here\fixtures"; $lg = "$Root\legacy"; $o = "$out\artcraft"
        # WordCraft: info, then page by page until render refuses a page
        foreach ($f in @("$fx\spike.docx", "$fx\hard.docx", "$fx\remote.docx", "$lg\spike.doc")) {
            if (-not (Test-Path -LiteralPath $f)) { Say "MISSING $f"; continue }
            $n = (Split-Path -Leaf $f) -replace '\.', '-'
            $null = Run-Cli 'wordcraft' "wc-info-$n" @('info', $f)
            foreach ($round in 1, 2) {
                for ($pg = 1; $pg -le 8; $pg++) {
                    $code = Run-Cli 'wordcraft' "wc-r$round-$n-page$pg" @('render', $f, "$o\wc-$n-page$pg.png", '--page', "$pg", '--scale', '1')
                    if ($code -ne 0) { break }
                }
            }
            $null = Run-Cli 'wordcraft' "wc-strict-$n" @('render', $f, "$o\wc-strict-$n.png", '--page', '1', '--scale', '1', '--strict')
        }
        # DeckCraft: every slide
        foreach ($f in @("$fx\spike.pptx", "$fx\hard.pptx", "$lg\spike.ppt")) {
            if (-not (Test-Path -LiteralPath $f)) { Say "MISSING $f"; continue }
            $n = (Split-Path -Leaf $f) -replace '\.', '-'
            New-Item -ItemType Directory -Force "$o\dc-$n" | Out-Null
            $null = Run-Cli 'deckcraft' "dc-info-$n" @('info', $f)
            foreach ($round in 1, 2) { $null = Run-Cli 'deckcraft' "dc-r$round-$n" @('render', $f, '--all', '--scale', '1', "$o\dc-$n") }
        }
        # GridCraft: no page images in its CLI; the model (info, cat with computed values) and its PDF / HTML export
        foreach ($f in @("$fx\spike.xlsx", "$fx\hard.xlsx", "$lg\spike.xls")) {
            if (-not (Test-Path -LiteralPath $f)) { Say "MISSING $f"; continue }
            $n = (Split-Path -Leaf $f) -replace '\.', '-'
            $null = Run-Cli 'gridcraft' "gc-info-$n" @('info', $f, '--json')
            $null = Run-Cli 'gridcraft' "gc-cat-$n" @('cat', $f)
            $null = Run-Cli 'gridcraft' "gc-pdf-$n" @('convert', $f, "$o\gc-$n.pdf")
            $null = Run-Cli 'gridcraft' "gc-html-$n" @('convert', $f, "$o\gc-$n.html")
        }
        $null = Run-Cli 'gridcraft' 'gc-cat-hard-summary' @('cat', "$fx\hard.xlsx", '--sheet', 'Summary')
    }
    finally {
        Get-NetFirewallRule -DisplayName "$RuleName*" -ErrorAction SilentlyContinue | Remove-NetFirewallRule
        Say "firewall rules left: $(@(Get-NetFirewallRule -DisplayName "$RuleName*" -ErrorAction SilentlyContinue).Count)"
    }
    $made = @(Get-ChildItem -LiteralPath "$out\artcraft" -Recurse -Include '*.png', '*.pdf', '*.html')
    Say "artcraft outputs: png=$(@($made | Where-Object Extension -eq '.png').Count) pdf=$(@($made | Where-Object Extension -eq '.pdf').Count) html=$(@($made | Where-Object Extension -eq '.html').Count)"
    $outside = @("$env:APPDATA\WordCraft", "$env:APPDATA\GridCraft", "$env:APPDATA\DeckCraft", "$env:LOCALAPPDATA\WordCraft", "$env:LOCALAPPDATA\GridCraft", "$env:LOCALAPPDATA\DeckCraft" | Where-Object { Test-Path -LiteralPath $_ })
    Say "ArtCraft folders written OUTSIDE the root: $($outside.Count) $($outside -join ', ')"
    exit 0
}

# ---- run
$prog = Find-Program
$out = $env:BB_OUT
if (-not $out) { throw 'BB_OUT is not set (run through remote-build)' }
New-Item -ItemType Directory -Force "$out\png" | Out-Null
$metrics = "$out\metrics.jsonl"
$log = "$out\run.log"
function Say([string]$m) { Write-Output $m; Add-Content -LiteralPath $log -Value $m }

$inc = "$Root\inc\LibreOfficeKit"
New-Item -ItemType Directory -Force $inc | Out-Null
foreach ($h in $Headers.Keys) {
    & curl.exe -fsSL -o "$inc\$h" "$HeaderBase/$h"
    if ($LASTEXITCODE -ne 0) { throw "header download failed: $h" }
    if ((Sha256 "$inc\$h") -ne $Headers[$h]) { throw "header SHA-256 mismatch: $h" }
}
Say "headers: $($Headers.Count) of $($Headers.Count) match their pinned SHA-256 (tag branch libreoffice-26-2-7)"

$env:LOK_INCLUDE_DIR = "$Root\inc"
Push-Location $Here
$ErrorActionPreference = 'Continue'
& cargo build --release 2>&1 | ForEach-Object { "$_" } | Set-Content -LiteralPath "$out\cargo.log"
Get-Content -LiteralPath "$out\cargo.log" | Where-Object { $_ -match 'error|shim\.c\(|LibreOfficeKit.*\.h\(|Finished' } | Select-Object -First 25 | ForEach-Object { Say "cargo: $_" }
$ErrorActionPreference = 'Stop'
Pop-Location
$exe = "$env:CARGO_TARGET_DIR\release\lok-spike.exe"
if (-not (Test-Path -LiteralPath $exe)) { throw "build produced no $exe" }
Say "spike exe bytes=$((Get-Item -LiteralPath $exe).Length)"

function Run-Spike([string]$Label, [string]$Fixture, [string[]]$Extra, [string]$Dir = "$Here\fixtures") {
    $stdout = "$Root\stdout-$Label-$Fixture.txt"
    $stderr = "$Root\stderr-$Label-$Fixture.txt"
    $a = @('render', '--install', $prog, '--profile', "$Root\profile", '--file', "$Dir\$Fixture", '--out', "$out\png", '--label', $Label) + $Extra
    $p = Start-Process -FilePath $exe -ArgumentList $a -PassThru -NoNewWindow -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    $null = $p.Handle
    if (-not $p.WaitForExit(90000)) {
        Stop-Process -Id $p.Id -Force
        Say "RED $Label ${Fixture}: no exit within 90 s, killed pid $($p.Id); last stderr line: $(@(Get-Content -LiteralPath $stderr -ErrorAction SilentlyContinue) | Select-Object -Last 1)"
        Copy-Item -LiteralPath $stderr "$out\stderr-$Label-$Fixture.txt"
        return
    }
    $p.WaitForExit()
    $line = Get-Content -LiteralPath $stdout | Where-Object { $_ -like '{*' } | Select-Object -First 1
    if ($line) { Add-Content -LiteralPath $metrics -Value $line }
    Say "exit=$($p.ExitCode) $Label $Fixture :: $line"
    if (-not $line) { Copy-Item -LiteralPath $stderr "$out\stderr-$Label-$Fixture.txt" }
}

function Make-Legacy {
    # The older binary formats of DW-1511, written by LibreOffice itself from the OOXML fixtures.
    New-Item -ItemType Directory -Force "$Root\legacy" | Out-Null
    $inst = 'file:///' + ("$Root\profile-convert" -replace '\\', '/')
    foreach ($pair in @(@('doc', 'spike.docx'), @('xls', 'spike.xlsx'), @('ppt', 'spike.pptx'))) {
        $p = Start-Process -FilePath "$prog\soffice.exe" -PassThru -NoNewWindow -ArgumentList @(
            "-env:UserInstallation=$inst", '--headless', '--norestore', '--convert-to', $pair[0], '--outdir', "$Root\legacy", "$Here\fixtures\$($pair[1])")
        $null = $p.Handle
        if (-not $p.WaitForExit(120000)) { Stop-Process -Id $p.Id -Force; Say "RED legacy $($pair[0]): soffice --convert-to did not exit in 120 s, killed pid $($p.Id)" }
    }
    Get-Process -Name 'soffice.bin' -ErrorAction SilentlyContinue | ForEach-Object { Stop-Process -Id $_.Id -Force }
    $made = @(Get-ChildItem -LiteralPath "$Root\legacy" -File)
    Say "legacy fixtures written by soffice --convert-to: $($made.Count) of 3 ($(($made | ForEach-Object { "$($_.Name)=$($_.Length)" }) -join ', '))"
    New-Item -ItemType Directory -Force "$out\legacy" | Out-Null
    $made | Copy-Item -Destination "$out\legacy"
}

$probeHost = 'download.documentfoundation.org:443'
Make-Legacy
try {
    # 1. The check must be able to fail: with no rule, the SAME exe reaches the network.
    & $exe probe $probeHost | ForEach-Object { Say "before the rule: $_" }
    $before = $LASTEXITCODE
    # 2. Does the library fetch linked content when nothing stops it? (loopback listener, counted)
    if (Test-Path -LiteralPath "$Root\profile") { Remove-Item -LiteralPath "$Root\profile" -Recurse -Force }
    Run-Spike 'netopen' 'remote.docx' @('--listen')
    # 3. Block every outbound connection of this one program (not a profile-wide block).
    New-NetFirewallRule -DisplayName $RuleName -Direction Outbound -Program $exe -Action Block -Profile Any | Out-Null
    Start-Sleep -Seconds 2
    & $exe probe $probeHost | ForEach-Object { Say "with the rule:   $_" }
    $after = $LASTEXITCODE
    if ($before -ne 0 -or $after -ne 1) { throw "the network block is NOT proven (probe before=$before, with rule=$after; wanted 0 then 1)" }
    Say 'network block: PROVEN (same exe: connected before the rule, no connection with it)'

    # 4. Renders, all with the block in place. "cold" = first start ever (no profile folder yet).
    Remove-Item -LiteralPath "$Root\profile" -Recurse -Force
    $easy = 'spike.docx', 'spike.pptx', 'spike.xlsx'
    foreach ($f in $easy) { if ($f -eq 'spike.docx') { Run-Spike 'cold' $f @() } else { Run-Spike 'warm0' $f @() } }
    foreach ($round in 'warm1', 'warm2') { foreach ($f in $easy) { Run-Spike $round $f @() } }
    foreach ($f in 'spike.docx', 'spike.pptx') { Run-Spike 'hidpi2x' $f @('--scale', '2') }
    foreach ($f in 'hard.docx', 'hard.pptx', 'hard.xlsx') { Run-Spike 'hard' $f @() }
    foreach ($f in 'spike.doc', 'spike.ppt', 'spike.xls') { if (Test-Path -LiteralPath "$Root\legacy\$f") { Run-Spike 'legacy' $f @() "$Root\legacy" } }
    Run-Spike 'blocked' 'remote.docx' @('--listen')
    Run-Spike 'destroy' 'spike.docx' @('--destroy')
    # The spreadsheet hang, narrowed: the same file with OpenCL probing switched off.
    $env:SAL_DISABLE_OPENCL = '1'
    Run-Spike 'noopencl' 'spike.xlsx' @()
    Remove-Item Env:\SAL_DISABLE_OPENCL
    $prof = @(Get-ChildItem -LiteralPath "$Root\profile" -Recurse -Force -File)
    Say ("profile folder after the runs: files={0} KB={1:N0}" -f $prof.Count, (($prof | Measure-Object Length -Sum).Sum / 1KB))
    $outside = @("$env:APPDATA\LibreOffice", "$env:LOCALAPPDATA\LibreOffice" | Where-Object { Test-Path -LiteralPath $_ }).Count
    Say "profile folders written OUTSIDE the root: $outside"
}
finally {
    Get-NetFirewallRule -DisplayName "$RuleName*" -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    Say "firewall rules left: $(@(Get-NetFirewallRule -DisplayName "$RuleName*" -ErrorAction SilentlyContinue).Count)"
}
$pngs = @(Get-ChildItem -LiteralPath "$out\png" -Filter '*.png')
Say "png files written: $($pngs.Count); metrics lines: $(@(Get-Content -LiteralPath $metrics).Count)"

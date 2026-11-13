# THROWAWAY (task 2477 editor spike). Runs on the WINDOWS build VM through remote-build; never on a developer machine.
#   -Phase produce   extract the official LibreOffice MSI (administrative install: files only, nothing registered),
#                    let it re-save the fixtures ("another producer"), return them, remove LibreOffice again
#   -Phase timing    build the two WordCraft spike binaries from OUR subtree repository (tree hash compared with the
#                    upstream release tree) and time one keystroke on the target OS; timings only, no network claim
#   -Phase remove    delete everything this spike put on the machine and count what is left
param(
    [Parameter(Mandatory = $true)][ValidateSet('produce', 'timing', 'remove', 'count')][string]$Phase,
    [string]$Root = 'C:\bb\spike-2477-ed'
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$Version = '26.2.7'
$MsiName = "LibreOffice_${Version}_Win_x86-64.msi"
$MsiUrl = "https://download.documentfoundation.org/libreoffice/stable/$Version/win/x86_64/$MsiName"
# From $MsiUrl.sha256 (The Document Foundation's own checksum), read on 2026-11-12.
$MsiSha256 = '480fecf759fc6393bd614fbf43753eb95a8198fc1a065d41b727e3b664cd1e59'
$Here = Split-Path -Parent $MyInvocation.MyCommand.Path
$Out = $env:BB_OUT
function Scrub([string]$s) { $s.Replace($Root, '<root>').Replace($Here, '<spike>') }

function Count-Leftovers([string]$When) {
    $files = 0
    if (Test-Path -LiteralPath $Root) { $files = @(Get-ChildItem -LiteralPath $Root -Recurse -Force -File).Count }
    $keys = @('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*')
    $uninstall = @(Get-ItemProperty $keys -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -like '*LibreOffice*' }).Count
    $vendor = @('HKLM:\SOFTWARE\LibreOffice', 'HKLM:\SOFTWARE\The Document Foundation', 'HKCU:\SOFTWARE\LibreOffice',
        'HKCU:\SOFTWARE\The Document Foundation' | Where-Object { Test-Path $_ }).Count
    $rules = @(Get-NetFirewallRule -DisplayName 'bb-spike-2477*' -ErrorAction SilentlyContinue).Count
    $profiles = @("$env:APPDATA\LibreOffice", "$env:LOCALAPPDATA\LibreOffice", "$env:ProgramFiles\LibreOffice",
        "$env:APPDATA\WordCraft", "$env:APPDATA\GridCraft", "$env:APPDATA\DeckCraft" | Where-Object { Test-Path -LiteralPath $_ }).Count
    $procs = @(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($Root, 'OrdinalIgnoreCase') }).Count
    Write-Output ("leftovers[$When]: files_under_root=$files uninstall_entries=$uninstall vendor_registry_keys=$vendor " +
        "firewall_rules=$rules profile_or_program_dirs=$profiles processes_started_from_root=$procs")
}

if ($Phase -eq 'count') { Count-Leftovers 'now'; exit 0 }
if ($Phase -eq 'remove') {
    Count-Leftovers 'before-remove'
    if (Test-Path -LiteralPath $Root) { Remove-Item -LiteralPath $Root -Recurse -Force }
    Count-Leftovers 'after-remove'
    exit 0
}

if ($Phase -eq 'produce') {
    Count-Leftovers 'before'
    New-Item -ItemType Directory -Force $Root | Out-Null
    $msi = "$Root\$MsiName"
    & curl.exe -fsSL -o $msi $MsiUrl
    if ($LASTEXITCODE -ne 0) { throw "download failed ($LASTEXITCODE)" }
    $got = (Get-FileHash -Algorithm SHA256 -LiteralPath $msi).Hash.ToLower()
    if ($got -ne $MsiSha256) { throw "MSI SHA-256 mismatch: got $got" }
    Write-Output "msi: $MsiName sha256 MATCHES the pinned official checksum ($got)"
    $p = Start-Process msiexec.exe -ArgumentList '/a', "`"$msi`"", '/qn', '/norestart', "TARGETDIR=`"$Root\lo`"" -Wait -PassThru
    if ($p.ExitCode -ne 0) { throw "msiexec /a exit code $($p.ExitCode)" }
    $soffice = (Get-ChildItem -LiteralPath "$Root\lo" -Recurse -Filter 'soffice.exe' | Select-Object -First 1).FullName
    $dst = "$Out\fixtures-other-producer"
    New-Item -ItemType Directory -Force $dst, "$Root\conv" | Out-Null
    $inst = 'file:///' + ("$Root\profile" -replace '\\', '/')
    $filters = @{ '.docx' = 'docx'; '.docm' = 'docm:MS Word 2007 XML VBA'; '.xlsx' = 'xlsx'; '.xlsm' = 'xlsm:Calc MS Excel 2007 VBA XML'
        '.pptx' = 'pptx'; '.pptm' = 'pptm:Impress MS PowerPoint 2007 XML VBA' }
    $made = 0; $failed = 0
    foreach ($f in Get-ChildItem -LiteralPath "$Here\fixtures" -File | Where-Object { $_.Name -notlike 'remote.*' -and $_.Name -notlike 'big-*' } | Sort-Object Name) {
        $conv = "$Root\conv\$($f.Name)"
        New-Item -ItemType Directory -Force $conv | Out-Null
        $p = Start-Process -FilePath $soffice -PassThru -NoNewWindow -ArgumentList @(
            "-env:UserInstallation=$inst", '--headless', '--norestore', '--convert-to', "`"$($filters[$f.Extension])`"", '--outdir', $conv, $f.FullName)
        $null = $p.Handle
        if (-not $p.WaitForExit(120000)) {
            # stop only what this run started: the launcher by its recorded id, and its children by parent id
            Get-CimInstance Win32_Process -Filter "ParentProcessId=$($p.Id)" | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
            Stop-Process -Id $p.Id -Force
            Write-Output "RED $($f.Name): soffice --convert-to did not exit in 120 s, stopped pid $($p.Id)"; $failed++; continue
        }
        $o = @(Get-ChildItem -LiteralPath $conv -File)
        if ($o.Count -gt 1) { throw "more than one output for $($f.Name)" }
        $o = $o | Select-Object -First 1
        if ($o) { Copy-Item -LiteralPath $o.FullName -Destination "$dst\lo-$($f.Name)"; $made++; Write-Output "LibreOffice $Version re-saved $($f.Name): $($f.Length) -> $($o.Length) bytes" }
        else { $failed++; Write-Output "RED $($f.Name): no output file" }
    }
    Write-Output "produced by LibreOffice ${Version}: $made files, failed: $failed"
    Start-Sleep -Seconds 3
    Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($Root, 'OrdinalIgnoreCase') } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
    Start-Sleep -Seconds 2
    Remove-Item -LiteralPath $Root -Recurse -Force
    Count-Leftovers 'after'
    exit 0
}

# ---- timing ----
New-Item -ItemType Directory -Force $Root | Out-Null
$office = "$Root\office"
if (-not (Test-Path -LiteralPath "$office\.git")) {
    & git clone --quiet --branch main https://github.com/beebeeb-io/desktop-office $office
    if ($LASTEXITCODE -ne 0) { throw 'clone failed' }
}
$src = "$office\wordcraft"
$tree = (& git -C $office rev-parse 'HEAD:wordcraft')
if ($tree -ne '5ca7ae4aea4c80b3847de5926f87acedc61ae80c') { throw "tree $tree DIFFERS from the upstream v0.18.1 tree" }
Write-Output "desktop-office main at $(& git -C $office rev-parse HEAD): wordcraft subtree tree=$tree EQUALS the upstream v0.18.1 tree"
foreach ($c in 'wc-edit', 'wc-engine') {
    New-Item -ItemType Directory -Force "$src\crates\zz-bb-$c" | Out-Null
    Copy-Item -Path "$Here\$c\*" -Destination "$src\crates\zz-bb-$c" -Recurse -Force
}
& git -C $office apply --directory=wordcraft "$Here\patches\wordcraft-engine-font-store-optional.patch"
if ($LASTEXITCODE -ne 0) { Write-Output 'patch: already applied or does not apply (see the build result)' }
$env:CARGO_TARGET_DIR = "$Root\target"
Push-Location $src
try {
    $ErrorActionPreference = 'Continue'
    foreach ($pkg in 'zz-bb-wc-edit', 'zz-bb-wc-engine') {
        & cargo build --release -p $pkg 2>&1 | Out-File -LiteralPath "$Root\build-$pkg.log"
        Write-Output "build ${pkg}: exit=$LASTEXITCODE"
        if ($LASTEXITCODE -ne 0) { Get-Content -LiteralPath "$Root\build-$pkg.log" -Tail 25 | ForEach-Object { Scrub $_ }; exit 1 }
    }
} finally { Pop-Location }
$big = "$Here\fixtures\big-30-pages.docx"
Write-Output "machine: $((Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()), $((Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors) logical processors"
foreach ($run in @(@('zz-bb-wc-edit', 'keystroke'), @('zz-bb-wc-engine', 'keystroke'), @('zz-bb-wc-edit', 'latency'))) {
    & "$Root\target\release\$($run[0]).exe" $run[1] $big 200 2>&1 | ForEach-Object { Write-Output (Scrub "$_") }
    Write-Output "$($run[0]) $($run[1]): exit=$LASTEXITCODE"
}
exit 0

$ErrorActionPreference = 'Continue'
$root = (Get-Location).Path; $T = $env:CARGO_TARGET_DIR
"HEAD: $(git -C $root rev-parse HEAD)  target: $T"
$src = 'C:\bb\target\desktop\native_p40-integ-cp5'
if (-not (Test-Path "$T\release\deps") -and (Test-Path "$src\release\deps")) {
  $t0 = Get-Date
  robocopy "$src\release" "$T\release" /E /MT:16 /NFL /NDL /NJH /NJS /NP /R:0 /W:0 | Out-Null
  "seeded release deps from the integration target in $([int]((Get-Date) - $t0).TotalSeconds) s (robocopy exit $LASTEXITCODE)"
}
$t0 = Get-Date
Set-Location "$root\native\beebeeb-drive-rune"; cmd /c "cargo build --release --locked 2>&1" | Select-Object -Last 3; "APP_EXIT=$LASTEXITCODE after $([int]((Get-Date) - $t0).TotalSeconds) s"
foreach ($h in 'beebeeb-office-render', 'beebeeb-media-helper') {
  Set-Location "$root\native\$h"; cmd /c "cargo build --release --locked 2>&1" | Select-Object -Last 2; "HELPER_EXIT $h=$LASTEXITCODE"
}
$t0 = Get-Date
Set-Location "$root\crates\beebeeb-engine"; cmd /c "cargo build --locked --example dc086_large_fake --features test-support 2>&1" | Select-Object -Last 2; "FAKE_EXIT=$LASTEXITCODE after $([int]((Get-Date) - $t0).TotalSeconds) s"
Set-Location $root
New-Item -ItemType Directory -Force C:\bb\dc086 | Out-Null
Remove-Item C:\bb\dc086\installer -Recurse -Force -ea 0
New-Item -ItemType Directory -Force C:\bb\dc086\installer | Out-Null
Copy-Item "$T\debug\examples\dc086_large_fake.exe" C:\bb\dc086\dc086_large_fake.exe -Force
& powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1 -Exe $T\release\beebeeb-drive.exe -OfficeRenderExe $T\release\beebeeb-office-render.exe -MediaHelperExe $T\release\beebeeb-media-helper.exe -Version 0.9.0 -Mode test -OutDir C:\bb\dc086\installer 2>&1 | Select-Object -Last 6
"INSTALLER_EXIT=$LASTEXITCODE"
Get-ChildItem $T\release\*.exe | ForEach-Object { "$($_.Name) $($_.Length) $((Get-FileHash $_.FullName).Hash.Substring(0,16))" }
Get-ChildItem C:\bb\dc086\installer, C:\bb\dc086\*.exe | ForEach-Object { "$($_.Name) $($_.Length) $((Get-FileHash $_.FullName).Hash)" }
"disk free C: $([int]((Get-PSDrive C).Free / 1GB)) GB"

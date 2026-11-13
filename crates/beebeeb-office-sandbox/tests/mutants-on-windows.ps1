# Shows that the native tests can fail: takes one part of the confinement out of src\windows.rs at a time, runs the
# native tests, counts the failures, and puts the file back. Run it ONLY in a build machine's own checkout, never
# on a branch that is pushed: powershell -NoProfile -ExecutionPolicy Bypass -File tests\mutants-on-windows.ps1
# It prints counts and test names only.
$ErrorActionPreference = 'Continue'
$file = Join-Path (Get-Location) 'src\windows.rs'
$original = [IO.File]::ReadAllText($file)

# name -> (regular expression, replacement). Each expression must match exactly once.
$mutants = [ordered]@{
    'no-container'        = @('attributes\.set\(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities\)\?;', 'let _ = &capabilities;')
    'no-package-opt-out'  = @('attributes\.set\(PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, &packages_policy\)\?;', 'let _ = &packages_policy;')
    'no-handle-list'      = @('attributes\.set\(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &inherited\)\?;', 'let _ = &inherited;')
    'no-job'              = @('attributes\.set\(PROC_THREAD_ATTRIBUTE_JOB_LIST, &jobs\)\?;', 'let _ = &jobs;')
    'inherited-environment' = @('environment\.as_ptr\(\)\.cast\(\),', 'null(),')
    'profile-kept'        = @('container\.unregister\(\)\?;', 'container.registered = false;')
    'no-process-limit'    = @('\s+\| JOB_OBJECT_LIMIT_ACTIVE_PROCESS(?=\s)', '')
    'no-kill-on-close'    = @('= JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE\s+\| ', '= ')
    'memory-cap-times-64' = @('wanted\.JobMemoryLimit = memory;', 'wanted.JobMemoryLimit = memory * 64;')
    'unrestricted-token'  = @('token\.as_raw_handle\(\),(?=\s+application\.as_ptr\(\),)', 'null_mut(),')
    'switch-put-back'     = @('attributes\.set\(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities\)\?;', 'if std::env::var("BEEBEEB_NO_SANDBOX").is_err() { attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities)?; }')
}

$profiles = 'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings'
function Count-Profiles { if (Test-Path $profiles) { @(Get-ChildItem $profiles -ErrorAction SilentlyContinue | Where-Object { (Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue).Moniker -like 'beebeeb.office*' }).Count } else { 0 } }
$profilesBefore = Count-Profiles
$log = Join-Path $env:TEMP ("beebeeb-sandbox-mutant-{0}.log" -f $PID)
$red = 0
try {
    foreach ($name in $mutants.Keys) {
        $pattern, $replacement = $mutants[$name]
        $matches_ = [regex]::Matches($original, $pattern).Count
        if ($matches_ -ne 1) { "mutant ${name}: the pattern matches $matches_ times, expected 1"; exit 4 }
        [IO.File]::WriteAllText($file, [regex]::Replace($original, $pattern, $replacement))
        & cargo test --locked --no-fail-fast --lib --test sandbox -- --test-threads=1 *> $log
        $lines = Get-Content $log
        $result = (@($lines | Where-Object { $_ -match '^test result:' }) | ForEach-Object { ($_ -replace '; 0 ignored.*$', '') -replace '^test result: ', '' }) -join ' + '
        $failed = @($lines | Where-Object { $_ -match '^test .* \.\.\. FAILED' -or $_ -match '^test windows::\S+ \.\.\. .*FAILED' })
        $names = @($lines | Where-Object { $_ -match '^    (windows|tests|block)::' } | ForEach-Object { $_.Trim() } | Sort-Object -Unique)
        if (-not $result) { $result = 'no result line (the build failed?): ' + (@($lines | Where-Object { $_ -match '^error' }) | Select-Object -First 1) }
        if ($names.Count -gt 0) { $red += 1 }
        "mutant ${name}: $($names.Count) tests red | $result | profiles of ours left by the run: $(Count-Profiles)"
        $names | ForEach-Object { "    red: $_" }
        # What the red tests got instead (answers of the probe: codes only).
        @($lines | Where-Object { $_ -match '^\s+left: ' -or $_ -match 'did not start the probe' } | Sort-Object -Unique) | Select-Object -First 6 | ForEach-Object { "    got: $($_.Trim())" }
    }
}
finally {
    [IO.File]::WriteAllText($file, $original)
    Remove-Item $log -Force -ErrorAction SilentlyContinue
}
"mutants: $red of $($mutants.Count) turned at least one test red"
"restored: $(@(& git status --porcelain -- src).Count) changed source files"
# The 'profile-kept' mutant leaves profiles of ours registered; the crate's own clean-up test removes every one.
& cargo test --locked --lib a_left_over_profile *> $null
"clean-up test exit code: $LASTEXITCODE"
"container profiles of ours: before=$profilesBefore after=$(Count-Profiles)"
"probe processes: $(@(Get-Process -Name 'beebeeb-sandbox-probe' -ErrorAction SilentlyContinue).Count)"

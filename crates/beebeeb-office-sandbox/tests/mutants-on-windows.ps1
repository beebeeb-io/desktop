# Shows that the native tests can fail: takes one part of the confinement out of src\windows.rs at a time, runs the
# native tests, counts the failures, and puts the file back. Run it ONLY in a build machine's own checkout, never
# on a branch that is pushed: powershell -NoProfile -ExecutionPolicy Bypass -File tests\mutants-on-windows.ps1
# It prints counts and test names only.
# First a baseline run on the file as it is: a mutant counts as killed only by a test that is red BEYOND the
# baseline's red list, so a test that is already red never makes a mutant look caught.
$ErrorActionPreference = 'Continue'
$file = Join-Path (Get-Location) 'src\windows.rs'
$original = [IO.File]::ReadAllText($file)

# name -> (regular expression, replacement[, regular expression, replacement]...), applied in order. Each expression
# must match exactly once.
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
    # Shared memory (sections and grants).
    'read-grant-writable' = @('Access::Read => FILE_MAP_READ,', 'Access::Read => FILE_MAP_READ | FILE_MAP_WRITE,')
    'read-write-grant-read' = @('Access::ReadWrite => FILE_MAP_READ \| FILE_MAP_WRITE,', 'Access::ReadWrite => FILE_MAP_READ,')
    'section-inherited'   = @('let mapping = owned\(raw\);', 'unsafe { SetHandleInformation(raw, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) }; let mapping = owned(raw);', 'attributes\.set\(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &inherited\)\?;', 'let _ = &inherited;')
    'section-named'       = @('PAGE_READWRITE, high, low, null\(\)\)', 'PAGE_READWRITE, high, low, [98u16, 98, 0].as_ptr())')
    'section-not-zeroed'  = @('zeroize\(self\.as_mut_slice\(\)\);', 'let _ = self.len;')
    # Network capabilities (internetClient and privateNetworkClientServer, enabled). The first keeps the crate's own
    # read-back of the token, which must refuse the helper ("has a capability"); the second takes that read-back
    # out too; the third also removes the package opt-out, so the system's socket library can start.
    'network-capability-refused'    = @('let capabilities = \[SECURITY_CAPABILITIES \{', 'let mut sid_internet = [0u32; 17]; let mut sid_private = [0u32; 17]; let mut length_internet = 68u32; let mut length_private = 68u32; unsafe { CreateWellKnownSid(windows_sys::Win32::Security::WinCapabilityInternetClientSid, null_mut(), sid_internet.as_mut_ptr().cast(), &mut length_internet); CreateWellKnownSid(windows_sys::Win32::Security::WinCapabilityPrivateNetworkClientServerSid, null_mut(), sid_private.as_mut_ptr().cast(), &mut length_private); } let mut network = [SID_AND_ATTRIBUTES { Sid: sid_internet.as_mut_ptr().cast(), Attributes: 4 }, SID_AND_ATTRIBUTES { Sid: sid_private.as_mut_ptr().cast(), Attributes: 4 }]; let capabilities = [SECURITY_CAPABILITIES {', 'Capabilities: null_mut\(\),\s+CapabilityCount: 0,', 'Capabilities: network.as_mut_ptr(), CapabilityCount: 2,')
    'network-capability'            = @('let capabilities = \[SECURITY_CAPABILITIES \{', 'let mut sid_internet = [0u32; 17]; let mut sid_private = [0u32; 17]; let mut length_internet = 68u32; let mut length_private = 68u32; unsafe { CreateWellKnownSid(windows_sys::Win32::Security::WinCapabilityInternetClientSid, null_mut(), sid_internet.as_mut_ptr().cast(), &mut length_internet); CreateWellKnownSid(windows_sys::Win32::Security::WinCapabilityPrivateNetworkClientServerSid, null_mut(), sid_private.as_mut_ptr().cast(), &mut length_private); } let mut network = [SID_AND_ATTRIBUTES { Sid: sid_internet.as_mut_ptr().cast(), Attributes: 4 }, SID_AND_ATTRIBUTES { Sid: sid_private.as_mut_ptr().cast(), Attributes: 4 }]; let capabilities = [SECURITY_CAPABILITIES {', 'Capabilities: null_mut\(\),\s+CapabilityCount: 0,', 'Capabilities: network.as_mut_ptr(), CapabilityCount: 2,', 'if count != 0 \{', 'if count == u32::MAX {')
    'network-capability-no-opt-out' = @('let capabilities = \[SECURITY_CAPABILITIES \{', 'let mut sid_internet = [0u32; 17]; let mut sid_private = [0u32; 17]; let mut length_internet = 68u32; let mut length_private = 68u32; unsafe { CreateWellKnownSid(windows_sys::Win32::Security::WinCapabilityInternetClientSid, null_mut(), sid_internet.as_mut_ptr().cast(), &mut length_internet); CreateWellKnownSid(windows_sys::Win32::Security::WinCapabilityPrivateNetworkClientServerSid, null_mut(), sid_private.as_mut_ptr().cast(), &mut length_private); } let mut network = [SID_AND_ATTRIBUTES { Sid: sid_internet.as_mut_ptr().cast(), Attributes: 4 }, SID_AND_ATTRIBUTES { Sid: sid_private.as_mut_ptr().cast(), Attributes: 4 }]; let capabilities = [SECURITY_CAPABILITIES {', 'Capabilities: null_mut\(\),\s+CapabilityCount: 0,', 'Capabilities: network.as_mut_ptr(), CapabilityCount: 2,', 'if count != 0 \{', 'if count == u32::MAX {', 'attributes\.set\(PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, &packages_policy\)\?;', 'let _ = &packages_policy;')
}

$profiles = 'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings'
function Count-Profiles { if (Test-Path $profiles) { @(Get-ChildItem $profiles -ErrorAction SilentlyContinue | Where-Object { (Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue).Moniker -like 'beebeeb.office*' }).Count } else { 0 } }
function Get-ResultLine($lines) {
    (@($lines | Where-Object { $_ -match '^test result:' }) | ForEach-Object { ($_ -replace '; 0 ignored.*$', '') -replace '^test result: ', '' }) -join ' + '
}
function Get-RedNames($lines) {
    @($lines | Where-Object { $_ -match '^    (windows|tests|block)::' } | ForEach-Object { $_.Trim() } | Sort-Object -Unique)
}
$profilesBefore = Count-Profiles
$log = Join-Path $env:TEMP ("beebeeb-sandbox-mutant-{0}.log" -f $PID)
$red = 0
$survived = @()
$unbuilt = @()

# The baseline: the same run, on the file as it is.
& cargo test --locked --no-fail-fast --lib --test sandbox -- --test-threads=1 *> $log
$lines = Get-Content $log
$baselineResult = Get-ResultLine $lines
if (-not $baselineResult) {
    "baseline: no result line (the build failed?): " + (@($lines | Where-Object { $_ -match '^error' }) | Select-Object -First 1)
    Remove-Item $log -Force -ErrorAction SilentlyContinue
    exit 5
}
$baseline = @(Get-RedNames $lines)
"baseline: $($baseline.Count) tests red | $baselineResult"
$baseline | ForEach-Object { "    baseline red: $_" }

try {
    foreach ($name in $mutants.Keys) {
        $pairs = $mutants[$name]
        $mutated = $original
        for ($i = 0; $i -lt $pairs.Count; $i += 2) {
            $matches_ = [regex]::Matches($mutated, $pairs[$i]).Count
            if ($matches_ -ne 1) { "mutant ${name}: pattern $($i / 2 + 1) matches $matches_ times, expected 1"; exit 4 }
            $mutated = [regex]::Replace($mutated, $pairs[$i], $pairs[$i + 1])
        }
        [IO.File]::WriteAllText($file, $mutated)
        & cargo test --locked --no-fail-fast --lib --test sandbox -- --test-threads=1 *> $log
        $lines = Get-Content $log
        $result = Get-ResultLine $lines
        $names = @(Get-RedNames $lines)
        # Only a test that the baseline did not already have red counts as catching this mutant.
        $beyond = @($names | Where-Object { $baseline -notcontains $_ })
        # A mutant that does not build caught nothing and survived nothing: it is listed on its own.
        if (-not $result) {
            $result = 'no result line (the build failed?): ' + (@($lines | Where-Object { $_ -match '^error' }) | Select-Object -First 1)
            $unbuilt += $name
        } elseif ($beyond.Count -gt 0) { $red += 1 } else { $survived += $name }
        "mutant ${name}: $($beyond.Count) tests red beyond the baseline ($($names.Count) red in all) | $result | profiles of ours left by the run: $(Count-Profiles)"
        $beyond | ForEach-Object { "    red: $_" }
        # What the red tests got instead (answers of the probe: codes only).
        @($lines | Where-Object { $_ -match '^\s+left: ' -or $_ -match 'did not start the probe' } | Sort-Object -Unique) | Select-Object -First 6 | ForEach-Object { "    got: $($_.Trim())" }
    }
}
finally {
    [IO.File]::WriteAllText($file, $original)
    Remove-Item $log -Force -ErrorAction SilentlyContinue
}
"mutants: $red of $($mutants.Count) turned at least one test red beyond the baseline ($($baseline.Count) red at baseline)"
"survived: $($survived.Count)$(if ($survived.Count -gt 0) { ' (' + ($survived -join ', ') + ')' })"
"did not build: $($unbuilt.Count)$(if ($unbuilt.Count -gt 0) { ' (' + ($unbuilt -join ', ') + ')' })"
"restored: $(@(& git status --porcelain -- src).Count) changed source files"
# The 'profile-kept' mutant leaves profiles of ours registered; the crate's own clean-up test removes every one.
& cargo test --locked --lib a_left_over_profile *> $null
"clean-up test exit code: $LASTEXITCODE"
"container profiles of ours: before=$profilesBefore after=$(Count-Profiles)"
"probe processes: $(@(Get-Process -Name 'beebeeb-sandbox-probe' -ErrorAction SilentlyContinue).Count)"

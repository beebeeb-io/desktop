# Shows that the native tests can fail: takes one part of the confinement out of the sources at a time, runs the
# native tests, counts the failures, and puts the files back. Run it ONLY in a build machine's own checkout, never
# on a branch that is pushed: powershell -NoProfile -ExecutionPolicy Bypass -File tests\mutants-on-windows.ps1
# [-Only name,name]   (only the mutants named; every pattern of every mutant is still checked first)
# It prints counts and test names only.
# First a baseline run on the file as it is: a mutant counts as killed only by a test that is red BEYOND the
# baseline's red list, so a test that is already red never makes a mutant look caught.
param([string[]]$Only = @())
$ErrorActionPreference = 'Continue'
# With -File, "a,b" arrives as one string.
$Only = @($Only | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
# The files a mutant may change. A pattern names its file with a leading '@lib ', '@watch ' or '@attributes ';
# without one it is src\windows.rs.
$files = [ordered]@{
    windows    = Join-Path (Get-Location) 'src\windows.rs'
    lib        = Join-Path (Get-Location) 'src\lib.rs'
    watch      = Join-Path (Get-Location) 'src\watch.rs'
    attributes = Join-Path (Get-Location) 'src\attributes.rs'
}
$originals = @{}
foreach ($key in $files.Keys) { $originals[$key] = [IO.File]::ReadAllText($files[$key]) }

# name -> (regular expression, replacement[, regular expression, replacement]...), applied in order. Each expression
# must match exactly once in its file.
$mutants = [ordered]@{
    'no-container'        = @('attributes\.set\(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities\)\?;', 'let _ = &capabilities;')
    # The less-privileged read-back (F12): the token's attribute WIN://NOALLAPPPKG, read from its security attributes
    # (src\attributes.rs; the class TokenIsLessPrivilegedAppContainer is reserved and refused by Windows 11 23H2).
    # The opt-out removed AND its read-back taken out, so the helper runs and the file and token tests must see it.
    'no-package-opt-out'  = @('attributes\.set\(PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, &packages_policy\)\?;', 'let _ = &packages_policy;', '@lib if !facts\.less_privileged \{', 'if false {')
    # The opt-out removed only (a container token that is not less privileged): the read-back finds no attribute and
    # refuses the helper before it runs, "not a less-privileged AppContainer" (every confined test red).
    'no-package-opt-out-refused' = @('attributes\.set\(PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, &packages_policy\)\?;', 'let _ = &packages_policy;')
    # The opt-out kept, the read-back looks for another name: it finds none, so every confined start is refused.
    'lpac-attribute-misnamed'    = @('@attributes "WIN://NOALLAPPPKG";', '"WIN://NOALLAPPPKX";')
    # The read-back takes any attribute's name, or any value of the attribute (0 too): the parse's own tests.
    'lpac-name-ignored'          = @('@attributes if !same_name\(&answer\[name\.\.name \+ name_length\], NO_ALL_APPLICATION_PACKAGES\) \{', 'if false {')
    'lpac-value-ignored'         = @('@attributes return Ok\(u64_at\(answer, value\)\.ok_or\(DOES_NOT_FIT\)\? != 0\);', 'return Ok(true);')
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
    # Profiles (decision 2541).
    'extra-capability'    = @('for name in profile\.capabilities\(\) \{', 'for name in profile.capabilities().iter().chain(&["internetClient"]) {')
    'every-profile-hevc'  = @('let mut granted = CapabilitySids::of\(profile\)\?;', 'let mut granted = CapabilitySids::of(Profile::SystemDecoder { hevc: true })?;')
    'read-back-expects-none' = @('confirm_confined\(&child\.process, &child\.job, &granted, profile\)\?;', 'confirm_confined(&child.process, &child.job, &CapabilitySids::of(Profile::NoCapabilities)?, profile)?;')
    # Network capabilities (internetClient and privateNetworkClientServer, enabled). The first keeps the crate's own
    # read-back of the token, which must refuse the helper ("has a capability"); the second takes that read-back
    # out too; the third also removes the package opt-out, so the system's socket library can start.
    'network-capability-refused'    = @('let capabilities = \[SECURITY_CAPABILITIES \{', 'let mut sid_internet = [0u32; 17]; let mut sid_private = [0u32; 17]; let mut length_internet = 68u32; let mut length_private = 68u32; unsafe { CreateWellKnownSid(windows_sys::Win32::Security::WinCapabilityInternetClientSid, null_mut(), sid_internet.as_mut_ptr().cast(), &mut length_internet); CreateWellKnownSid(windows_sys::Win32::Security::WinCapabilityPrivateNetworkClientServerSid, null_mut(), sid_private.as_mut_ptr().cast(), &mut length_private); } let mut network = [SID_AND_ATTRIBUTES { Sid: sid_internet.as_mut_ptr().cast(), Attributes: 4 }, SID_AND_ATTRIBUTES { Sid: sid_private.as_mut_ptr().cast(), Attributes: 4 }]; let capabilities = [SECURITY_CAPABILITIES {', 'Capabilities: granted\.as_mut_ptr\(\),\s+CapabilityCount: granted\.count\(\),', 'Capabilities: network.as_mut_ptr(), CapabilityCount: 2,')
    'network-capability'            = @('let capabilities = \[SECURITY_CAPABILITIES \{', 'let mut sid_internet = [0u32; 17]; let mut sid_private = [0u32; 17]; let mut length_internet = 68u32; let mut length_private = 68u32; unsafe { CreateWellKnownSid(windows_sys::Win32::Security::WinCapabilityInternetClientSid, null_mut(), sid_internet.as_mut_ptr().cast(), &mut length_internet); CreateWellKnownSid(windows_sys::Win32::Security::WinCapabilityPrivateNetworkClientServerSid, null_mut(), sid_private.as_mut_ptr().cast(), &mut length_private); } let mut network = [SID_AND_ATTRIBUTES { Sid: sid_internet.as_mut_ptr().cast(), Attributes: 4 }, SID_AND_ATTRIBUTES { Sid: sid_private.as_mut_ptr().cast(), Attributes: 4 }]; let capabilities = [SECURITY_CAPABILITIES {', 'Capabilities: granted\.as_mut_ptr\(\),\s+CapabilityCount: granted\.count\(\),', 'Capabilities: network.as_mut_ptr(), CapabilityCount: 2,', '@lib if !facts\.exact_capabilities \{', 'if false {')
    'network-capability-no-opt-out' = @('let capabilities = \[SECURITY_CAPABILITIES \{', 'let mut sid_internet = [0u32; 17]; let mut sid_private = [0u32; 17]; let mut length_internet = 68u32; let mut length_private = 68u32; unsafe { CreateWellKnownSid(windows_sys::Win32::Security::WinCapabilityInternetClientSid, null_mut(), sid_internet.as_mut_ptr().cast(), &mut length_internet); CreateWellKnownSid(windows_sys::Win32::Security::WinCapabilityPrivateNetworkClientServerSid, null_mut(), sid_private.as_mut_ptr().cast(), &mut length_private); } let mut network = [SID_AND_ATTRIBUTES { Sid: sid_internet.as_mut_ptr().cast(), Attributes: 4 }, SID_AND_ATTRIBUTES { Sid: sid_private.as_mut_ptr().cast(), Attributes: 4 }]; let capabilities = [SECURITY_CAPABILITIES {', 'Capabilities: granted\.as_mut_ptr\(\),\s+CapabilityCount: granted\.count\(\),', 'Capabilities: network.as_mut_ptr(), CapabilityCount: 2,', '@lib if !facts\.exact_capabilities \{', 'if false {', 'attributes\.set\(PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, &packages_policy\)\?;', 'let _ = &packages_policy;', '@lib if !facts\.less_privileged \{', 'if false {')
    # A helper on the parent's own token and outside any container (still in the job, under the mitigations): the
    # share test's mutant. The container is not set, the read-back's container, capability, less-privileged and
    # integrity refusals are taken out, and the restricted token is replaced by the parent's (as in
    # 'unrestricted-token').
    'share-plain-token'             = @('attributes\.set\(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities\)\?;', 'let _ = &capabilities;', '@lib if !facts\.app_container \{', 'if false {', '@lib if !facts\.exact_capabilities \{', 'if false {', '@lib if !facts\.less_privileged \{', 'if false {', '@lib if facts\.integrity_rid > LOW_INTEGRITY_RID \{', 'if false {', 'token\.as_raw_handle\(\),(?=\s+application\.as_ptr\(\),)', 'null_mut(),')
    # The hardening (process mitigations, the token read-back, the memory watch, the handle count).
    # No mitigation attribute and the mitigation read-back off (two defects): window and dynamic code must be seen to
    # work in the confined probe. Handle checks stay on in it: Windows turns them on for the AppContainer itself
    # (measured on the Windows test machine, 22631.6199: the confined probe reads raise=1 permanent=1 without the creation bit).
    'no-mitigations'                = @('attributes\.set\(PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, &mitigation_policy\)\?;', 'let _ = &mitigation_policy;', '@watch mitigations: read_mitigations\(process\)\?,', 'mitigations: crate::Profile::NoCapabilities.mitigations(),')
    # A helper outside the container (as 'share-plain-token') without the strict-handle bit and its read-back: the
    # probe survives using a handle it does not hold, and reads the policy as off. 'share-plain-token' alone is the
    # control: there the creation bit alone ends the probe.
    'plain-token-no-strict-handles' = @('attributes\.set\(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities\)\?;', 'let _ = &capabilities;', '@lib if !facts\.app_container \{', 'if false {', '@lib if !facts\.exact_capabilities \{', 'if false {', '@lib if !facts\.less_privileged \{', 'if false {', '@lib if facts\.integrity_rid > LOW_INTEGRITY_RID \{', 'if false {', 'token\.as_raw_handle\(\),(?=\s+application\.as_ptr\(\),)', 'null_mut(),', '\(wanted\.strict_handle_checks, STRICT_HANDLE_CHECKS_ALWAYS_ON\),', '', '@lib if wanted\.strict_handle_checks && !got\.strict_handle_checks \{', 'if false {')
    'no-memory-watch'               = @('child\.watch = Some\(watch\);', 'drop(watch);')
    'watch-private-only'            = @('@lib private\.saturating_add\(shared\) > limit\.saturating_add\(granted\)', 'private > limit.saturating_add(granted)')
    'handle-count-zero'             = @('Ok\(count\)', 'Ok(0)')
    # The raw copy out of a section: the tail read from the start of the view; every word read one word late.
    'copy-tail-offset'              = @('source\.add\(done \+ index\)', 'source.add(index)')
    'copy-word-offset'              = @('source\.add\(index \* 8\)\.cast::<u64>\(\)', 'source.add(index * 8 + 8).cast::<u64>()')
}

$profiles = 'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings'
function Count-Profiles { if (Test-Path $profiles) { @(Get-ChildItem $profiles -ErrorAction SilentlyContinue | Where-Object { (Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue).Moniker -like 'beebeeb.office*' }).Count } else { 0 } }
function Get-ResultLine($lines) {
    (@($lines | Where-Object { $_ -match '^test result:' }) | ForEach-Object { ($_ -replace '; 0 ignored.*$', '') -replace '^test result: ', '' }) -join ' + '
}
function Get-RedNames($lines) {
    @($lines | Where-Object { $_ -match '^    (windows|tests|block|watch|attributes)::' } | ForEach-Object { $_.Trim() } | Sort-Object -Unique)
}
# Every pattern of every mutant is checked before anything is built or changed: a stale pattern stops the run here.
function Get-Mutated($name) {
    $pairs = $mutants[$name]
    $mutated = @{}
    foreach ($key in $files.Keys) { $mutated[$key] = $originals[$key] }
    for ($i = 0; $i -lt $pairs.Count; $i += 2) {
        $pattern = $pairs[$i]
        $target = 'windows'
        if ($pattern -match '^@(\w+) ') { $target = $Matches[1]; $pattern = $pattern.Substring($Matches[0].Length) }
        if (-not $files.Contains($target)) { return "pattern $($i / 2 + 1) names an unknown file '$target'" }
        $count = [regex]::Matches($mutated[$target], $pattern).Count
        if ($count -ne 1) { return "pattern $($i / 2 + 1) matches $count times in $target, expected 1" }
        $mutated[$target] = [regex]::Replace($mutated[$target], $pattern, $pairs[$i + 1])
    }
    return $mutated
}
foreach ($name in $mutants.Keys) {
    $result = Get-Mutated $name
    if ($result -is [string]) { "mutant ${name}: $result"; exit 4 }
}
if ($Only.Count -gt 0) {
    foreach ($name in $Only) { if (-not $mutants.Contains($name)) { "mutant ${name}: unknown"; exit 4 } }
    foreach ($name in @($mutants.Keys)) { if ($Only -notcontains $name) { $mutants.Remove($name) } }
    "only: $($mutants.Count) of $($Only.Count) named mutants"
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
        $mutated = Get-Mutated $name
        foreach ($key in $files.Keys) { [IO.File]::WriteAllText($files[$key], $mutated[$key]) }
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
        # The record lines a red test printed (codes and counts only): every verb's answer, also those after its first
        # failed assertion, which the "got" lines above cannot show.
        @($lines | Where-Object { $_ -match '^(network|share|measurement socket-driver|mitigations|memory watch|handles): ' } | Sort-Object -Unique) | ForEach-Object { "    said: $($_.Trim())" }
    }
}
finally {
    foreach ($key in $files.Keys) { [IO.File]::WriteAllText($files[$key], $originals[$key]) }
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

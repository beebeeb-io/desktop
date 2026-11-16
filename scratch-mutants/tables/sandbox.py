# O-CP3 mutants of lane ov-hardening-sandbox (Office V3, H1): BRANCH-NOTES "## Mutants", the portable (Linux) table
# only, in table order. Root: the desktop tree. Tests: lib unit tests (`src/lib.rs` `mod tests`), `--lib`.
# The Windows table runs through the crate's own `tests/mutants-on-windows.ps1` (not translated here).

L = "crates/beebeeb-office-sandbox/src/lib.rs"

MUT = [
    dict(id="S1-1", crate="sandbox",
         note="spawn takes its own lock: `let launch = launch_lock();` -> a static OWN mutex's LaunchGuard",
         files=[(L, "        let launch = launch_lock();\n",
                 "        let launch = { static OWN: Mutex<()> = Mutex::new(()); LaunchGuard { _held: OWN.lock().unwrap() } };\n")],
         filt="spawn_and_launch_lock_are_one_lock",
         expect=["spawn_and_launch_lock_are_one_lock"]),
    dict(id="S1-2", crate="sandbox",
         note="NoCapabilities `no_window_system_calls: true,` -> `false,`",
         files=[(L, "            Profile::NoCapabilities => Mitigations {\n                no_window_system_calls: true,\n",
                 "            Profile::NoCapabilities => Mitigations {\n                no_window_system_calls: false,\n")],
         filt="mitigations_are_a_closed_table_per_profile check_facts_refuses_each_missing_property",
         expect=["mitigations_are_a_closed_table_per_profile", "check_facts_refuses_each_missing_property"]),
    dict(id="S1-3", crate="sandbox",
         note="check_facts: the `if !facts.less_privileged { .. }` block (LPAC) deleted",
         files=[(L, "    if !facts.less_privileged {\n"
                    "        return Err(SandboxError::NotConfined(\"not a less-privileged AppContainer\"));\n"
                    "    }\n",
                 "")],
         filt="check_facts_refuses_each_missing_property",
         expect=["check_facts_refuses_each_missing_property"]),
    dict(id="S1-4", crate="sandbox",
         note="check_facts: the `if facts.integrity_rid > LOW_INTEGRITY_RID { .. }` block (integrity) deleted",
         files=[(L, "    if facts.integrity_rid > LOW_INTEGRITY_RID {\n"
                    "        return Err(SandboxError::NotConfined(\"its integrity level is above low\"));\n"
                    "    }\n",
                 "")],
         filt="check_facts_refuses_each_missing_property",
         expect=["check_facts_refuses_each_missing_property"]),
    dict(id="S1-5", crate="sandbox",
         note="check_facts: the `if wanted.no_window_system_calls .. { .. }` block (win32k) deleted",
         files=[(L, "    if wanted.no_window_system_calls && !got.no_window_system_calls {\n"
                    "        return Err(SandboxError::NotConfined(\"window-system calls are allowed\"));\n"
                    "    }\n",
                 "")],
         filt="check_facts_refuses_each_missing_property",
         expect=["check_facts_refuses_each_missing_property"]),
    dict(id="S1-6", crate="sandbox",
         note="check_facts: the `if wanted.no_dynamic_code .. { .. }` block (dynamic code) deleted",
         files=[(L, "    if wanted.no_dynamic_code && !got.no_dynamic_code {\n"
                    "        return Err(SandboxError::NotConfined(\"dynamic code is allowed\"));\n"
                    "    }\n",
                 "")],
         filt="check_facts_refuses_each_missing_property",
         expect=["check_facts_refuses_each_missing_property"]),
    dict(id="S1-7", crate="sandbox",
         note="check_facts: the `if wanted.microsoft_signed_only .. { .. }` block (signature) deleted (5 lines in the "
              "merged tree: rustfmt split its Err over three)",
         files=[(L, "    if wanted.microsoft_signed_only && !got.microsoft_signed_only {\n"
                    "        return Err(SandboxError::NotConfined(\n"
                    "            \"images that are not Microsoft-signed may load\",\n"
                    "        ));\n"
                    "    }\n",
                 "")],
         filt="check_facts_refuses_each_missing_property",
         expect=["check_facts_refuses_each_missing_property"]),
    dict(id="S1-8", crate="sandbox",
         note="over_memory: `private.saturating_add(shared) > limit..` -> `private > limit.saturating_add(granted)` (shared commit not counted)",
         files=[(L, "    private.saturating_add(shared) > limit.saturating_add(granted)\n",
                 "    private > limit.saturating_add(granted)\n")],
         filt="over_memory_counts_shared_commit_and_grants",
         expect=["over_memory_counts_shared_commit_and_grants"]),
    dict(id="S1-9", crate="sandbox",
         note="copy_exactly copies the shorter of the two lengths. CHANGED from the note: the note puts "
              "`let n = len.min(out.len()); copy(&mut out[..n]); return Ok(());` in place of the length check, which "
              "leaves the FnOnce `copy(out)` after it (a second use of a moved value, unreachable); the whole body is "
              "replaced instead, same behaviour, one call of `copy`",
         files=[(L, "    if out.len() != len {\n"
                    "        return Err(SandboxError::Limits(\"a copy whose length is not the section's\"));\n"
                    "    }\n"
                    "    copy(out);\n"
                    "    Ok(())\n",
                 "    let n = len.min(out.len());\n"
                 "    copy(&mut out[..n]);\n"
                 "    Ok(())\n")],
         filt="copy_to_needs_the_exact_length",
         expect=["copy_to_needs_the_exact_length"]),
    dict(id="S1-10", crate="sandbox",
         note="GrantedTo::count: the `if counters.iter().any(..) { return; }` block deleted (a child counted per grant, not once)",
         files=[(L, "        if counters.iter().any(|known| Arc::ptr_eq(known, counter)) {\n"
                    "            return;\n"
                    "        }\n",
                 "")],
         filt="granted_bytes_count_each_child_once_and_fall_on_drop",
         expect=["granted_bytes_count_each_child_once_and_fall_on_drop"]),
]

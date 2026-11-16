# O-CP3 mutants of lane ov-hardening-host (V3-H, H2): BRANCH-NOTES "## Mutants", translated to the merged tree
# (desktop `native/ov-integ-cp3`). Root: the desktop tree. Filters are the expected tests' own names (no module-wide
# filter: E2-11 blocks `request_page` until a page arrives, so any other page test in the filter would spin forever).
# Tests: `engine_host::office_host_tests::{,hardening_tests::,anchor_tests::}` (`#[path]` modules of the bin).

W = "native/beebeeb-drive-rune/src/engine_host/office_host/worker.rs"
H = "native/beebeeb-drive-rune/src/engine_host/office_host.rs"
CR = "native/beebeeb-drive-rune/src/engine_host/office_host/crash_reports.rs"

_ANCHOR_SIZE_OLD = "        let (w, h) = check::asked_anchor_size(&probe, &seen, &check::Limits::CONTRACT).ok()?;\n"
_ANCHOR_SIZE_NEW = ("        let (w, h) = ((f64::from(seen.w_pt) * f64::from(scale)).{f}() as u32, "
                    "(f64::from(seen.h_pt) * f64::from(scale)).{f}() as u32);\n")

MUT = [
    # ---------------------------------------------------------------- New (this lane)
    dict(id="H2-R4name", crate="native",
         note="R4 name: W `.filter(|name| name == HELPER_EXE)` removed (\"other.exe\" starts)",
         files=[(W, "        let Some(name) = self.starter.file_name().filter(|name| name == HELPER_EXE) else {\n",
                 "        let Some(name) = self.starter.file_name() else {\n")],
         filt="the_exclusion_names_the_started_file_and_is_written_every_start",
         expect=["the_exclusion_names_the_started_file_and_is_written_every_start"]),
    dict(id="H2-R4every", crate="native",
         note="R4 every start (worker): the exclusion only while `self.links == 0` (calls [exclude, start, start])",
         files=[(W, "        if self.starter.exclude_from_crash_reports(&name).is_err() {\n",
                 "        if self.links == 0 && self.starter.exclude_from_crash_reports(&name).is_err() {\n")],
         filt="the_exclusion_names_the_started_file_and_is_written_every_start",
         expect=["the_exclusion_names_the_started_file_and_is_written_every_start"]),
    dict(id="H2-R4done", crate="native",
         note="R4 every start (DONE back): crash_reports::exclude writes once per process (static DONE swap). Caught "
              "by the test's Linux-only sub-case (production starter, write delta >= 2); on a Windows pass this row "
              "reads SURVIVED by design (the sub-case is #[cfg(not(windows))])",
         files=[(CR, "pub(crate) fn exclude(exe_name: &str) -> Result<(), u32> {\n    write(exe_name)\n}\n",
                 "pub(crate) fn exclude(exe_name: &str) -> Result<(), u32> {\n"
                 "    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);\n"
                 "    if DONE.swap(true, Ordering::SeqCst) {\n"
                 "        return Ok(());\n"
                 "    }\n"
                 "    write(exe_name)\n"
                 "}\n")],
         filt="the_exclusion_names_the_started_file_and_is_written_every_start",
         expect=["the_exclusion_names_the_started_file_and_is_written_every_start"]),
    dict(id="H2-R5", crate="native",
         note="R5: Shared::close leaves the old state (sheet names) in place: `std::mem::replace(..)` -> `OfficeState::Closed(why)`",
         files=[(H, "            let state = std::mem::replace(&mut inner.state, OfficeState::Closed(why));\n",
                 "            let state = OfficeState::Closed(why);\n")],
         filt="a_lock_wipes_the_sheet_names_at_once",
         expect=["a_lock_wipes_the_sheet_names_at_once"]),
    dict(id="H2-R6stop", crate="native",
         note="R6 after a stop: `None => return self.unhandled(frame),` -> `None => return,`",
         files=[(W, "            None => return self.unhandled(frame),\n",
                 "            None => return,\n")],
         filt="unhandled_frames_are_wiped",
         expect=["unhandled_frames_are_wiped"]),
    dict(id="H2-R6earlier", crate="native",
         note="R6 earlier helper: `return self.unhandled(frame);` -> `return;`",
         files=[(W, "            // An earlier helper's frame. Wiped, not handled.\n            return self.unhandled(frame);\n",
                 "            // An earlier helper's frame. Wiped, not handled.\n            return;\n")],
         filt="unhandled_frames_are_wiped",
         expect=["unhandled_frames_are_wiped"]),
    dict(id="H2-R6queued", crate="native",
         note="R6 queued at close: the `while let Ok(input) = self.inputs.try_recv() { .. }` drain in finish() deleted",
         files=[(W, "        while let Ok(input) = self.inputs.try_recv() {\n            self.shared.wipe_input(input);\n        }\n",
                 "")],
         filt="unhandled_frames_are_wiped",
         expect=["unhandled_frames_are_wiped"]),
    dict(id="H2-R6reader", crate="native",
         note="R6 reader: `if let Some(shared) = shared.upgrade() { shared.wipe_input(input); }` deleted in spawn_reader",
         files=[(W, "                    if let Some(shared) = shared.upgrade() {\n"
                    "                        shared.wipe_input(input);\n"
                    "                    }\n",
                 "")],
         filt="unhandled_frames_are_wiped",
         expect=["unhandled_frames_are_wiped"]),
    dict(id="H2-R7", crate="native",
         note="R7: `let _guard = PanicGuard(self.shared.clone());` deleted",
         files=[(W, "        let _guard = PanicGuard(self.shared.clone());\n", "")],
         filt="a_worker_panic_stops_the_helper_and_says_so",
         expect=["a_worker_panic_stops_the_helper_and_says_so"]),
    dict(id="H2-R1area", crate="native",
         note="R1 area: `&& range.area() <= MAX_ASKED_CELLS` deleted in request_cells",
         files=[(H, "                && range.area() <= MAX_ASKED_CELLS\n", "")],
         filt="a_range_over_the_cap_or_outside_the_sheet_is_not_asked",
         expect=["a_range_over_the_cap_or_outside_the_sheet_is_not_asked"]),
    dict(id="H2-R1bounds", crate="native",
         note="R1 bounds: `&& u64::from(range.row) + u64::from(range.rows) <= u64::from(rows)` deleted",
         files=[(H, "                && u64::from(range.row) + u64::from(range.rows) <= u64::from(rows)\n", "")],
         filt="a_range_over_the_cap_or_outside_the_sheet_is_not_asked",
         expect=["a_range_over_the_cap_or_outside_the_sheet_is_not_asked"]),
    dict(id="H2-R1budget", crate="native",
         note="R1 budget: both split count caps -> `true` (peak > 64). Lane: a mutant on MAX_SPLIT_REQUESTS alone or "
              "the 30 s split deadline alone is left green (no test reaches either first); not listed",
         files=[(W, "                budget.outstanding <= MAX_SPLIT_OUTSTANDING && budget.halves <= MAX_SPLIT_REQUESTS\n",
                 "                true\n")],
         filt="a_helper_that_refuses_every_range_is_stopped_by_the_budget",
         expect=["a_helper_that_refuses_every_range_is_stopped_by_the_budget"]),
    dict(id="H2-F10", crate="native",
         note="F10: `.chain(run.outline_since.map(|since| since + self.timeouts.outline))` deleted from deadline() (the `;` kept)",
         files=[(W, "            .map(|split| split.since + self.timeouts.split)\n"
                    "            .chain(run.outline_since.map(|since| since + self.timeouts.outline));\n",
                 "            .map(|split| split.since + self.timeouts.split);\n")],
         filt="an_outline_past_its_time_budget_is_stopped",
         expect=["an_outline_past_its_time_budget_is_stopped"]),
    dict(id="H2-R2", crate="native",
         note="R2: handles_ok `Some(base) => u64::from(count) > ..` -> `Some(_) => false` (40 pages)",
         files=[(W, "            Some(base) => u64::from(count) > u64::from(base) + MAX_IN_FLIGHT as u64 + u64::from(HANDLE_SLACK),\n",
                 "            Some(_) => false,\n")],
         filt="a_helper_that_keeps_its_handles_is_stopped",
         expect=["a_helper_that_keeps_its_handles_is_stopped"]),
    dict(id="H2-R3path", crate="native",
         note="R3 path: copy_section_out reads through as_slice (min of the two lengths) instead of copy_to",
         files=[(H, "    out.len() == section.len() && section.copy_to(out).is_ok()\n",
                 "    { let n = out.len().min(section.len()); out[..n].copy_from_slice(&section.as_slice()[..n]); true }\n")],
         filt="page_pixels_are_copied_without_a_shared_slice",
         expect=["page_pixels_are_copied_without_a_shared_slice"]),
    dict(id="H2-R3zero", crate="native",
         note="R3 no zero page: rendered()'s `if !self.starter.copy_out(..)` -> `if { let _ = ..copy_out(..); false }` "
              "(the page one only, not rendered_anchor's; a page is delivered)",
         files=[(W, "        if !self.starter.copy_out(&section, &mut rgba) {\n"
                    "            // Never a page of zeros shown as the page.\n",
                 "        if { let _ = self.starter.copy_out(&section, &mut rgba); false } {\n"
                 "            // Never a page of zeros shown as the page.\n")],
         filt="page_pixels_are_copied_without_a_shared_slice",
         expect=["page_pixels_are_copied_without_a_shared_slice"]),
    dict(id="H2-R2wiring", crate="native", where="win",
         note="R2 wiring: `impl Helper for sandbox::Child`'s `sandbox::Child::handle_count(self).ok()` -> `None`; seen "
              "only by the Windows real-helper test (#[cfg(windows)], #[ignore], VM 401): helper_handle_count() is None. "
              "The test needs BB_OFFICE_HELPER (the helper exe) and BB_OFFICE_FIXTURES set, else it is red for that reason",
         files=[(H, "        sandbox::Child::handle_count(self).ok()\n", "        None\n")],
         cmd="cargo test --locked --bin beebeeb-drive -- --ignored a_real_confined_helper_renders_page_one",
         filt="a_real_confined_helper_renders_page_one",
         expect=["a_real_confined_helper_renders_page_one"]),
    dict(id="H2-R3wiring", crate="native", where="none",
         note="R3 wiring: Confined::copy_out `copy_section_out(section, out)` -> a direct as_slice copy. No test can see "
              "it (same pixels either way); review only (the shared copy_section_out it should call is the tested one)",
         files=[(H, "        copy_section_out(section, out)\n",
                 "        out.len() == section.len() && { out.copy_from_slice(section.as_slice()); true }\n")],
         filt="",
         expect=[]),
    dict(id="H2-anchorsize-round", crate="native",
         note="anchor size, f64+round: request_anchor's `check::asked_anchor_size(..).ok()?` -> its own f64 round "
              "(the 301 x 151 pt anchor: 401 x 201, not 402 x 202)",
         files=[(H, _ANCHOR_SIZE_OLD, _ANCHOR_SIZE_NEW.format(f="round"))],
         filt="an_anchor_is_asked_only_once_seen_and_sized_by_the_protocol",
         expect=["an_anchor_is_asked_only_once_seen_and_sized_by_the_protocol"]),
    dict(id="H2-anchorsize-ceil", crate="native",
         note="anchor size, f64+ceil: as -round with .ceil() (the 300 x 200 pt anchor: 401, not 400)",
         files=[(H, _ANCHOR_SIZE_OLD, _ANCHOR_SIZE_NEW.format(f="ceil"))],
         filt="an_anchor_is_asked_only_once_seen_and_sized_by_the_protocol",
         expect=["an_anchor_is_asked_only_once_seen_and_sized_by_the_protocol"]),
    dict(id="H2-anchorkind", crate="native",
         note="anchor kind: `(Asked::Anchor { .. }, _) => {}` added before the catch-all `(_, mut other) => {`",
         files=[(W, "            // Any other kind of answer: not what was asked.\n            (_, mut other) => {\n",
                 "            // Any other kind of answer: not what was asked.\n"
                 "            (Asked::Anchor { .. }, _) => {}\n"
                 "            (_, mut other) => {\n")],
         filt="a_wrong_kind_answer_to_an_anchor_kills",
         expect=["a_wrong_kind_answer_to_an_anchor_kills"]),

    # ---------------------------------------------------------------- O-CP2's 24 (row E2), by the table's #
    dict(id="E2-01", crate="native",
         note="E2 #1: opened() `if check::check_opened(..).is_err()` -> `if false`",
         files=[(W, "        if check::check_opened(&opened, &check::Limits::CONTRACT).is_err() {\n", "        if false {\n")],
         filt="opening_becomes_pages_on_a_checked_answer",
         expect=["opening_becomes_pages_on_a_checked_answer"]),
    dict(id="E2-02", crate="native",
         note="E2 #2: rendered() `if check::check_rendered(..).is_err()` -> `if false`",
         files=[(W, "        if check::check_rendered(&request, &size, len as u64, &got, &check::Limits::CONTRACT).is_err() {\n",
                 "        if false {\n")],
         filt="a_rendered_page_of_another_size_kills",
         expect=["a_rendered_page_of_another_size_kills"]),
    dict(id="E2-03", crate="native",
         note="E2 #3: answer id `|id| Some(id) == oldest` -> any id that is out",
         files=[(W, "        if check::check_answer_id(id, |id| Some(id) == oldest).is_err() {\n",
                 "        if check::check_answer_id(id, |id| self.run.as_ref().is_some_and(|run| run.sent.iter().any(|sent| sent.wire == id))).is_err() {\n")],
         filt="an_answer_that_is_not_the_oldest_kills",
         expect=["an_answer_that_is_not_the_oldest_kills"]),
    dict(id="E2-04", crate="native",
         note="E2 #4: `(_, Message::StatsData(got)) => push Stats` added before the catch-all `(_, mut other) => {`",
         files=[(W, "            // Any other kind of answer: not what was asked.\n            (_, mut other) => {\n",
                 "            // Any other kind of answer: not what was asked.\n"
                 "            (_, Message::StatsData(got)) => self.shared.push(Out::Event(Event::Stats { words: got.words })),\n"
                 "            (_, mut other) => {\n")],
         filt="a_wrong_kind_of_answer_kills",
         expect=["a_wrong_kind_of_answer_kills"]),
    dict(id="E2-05", crate="native",
         note="E2 #5: open_refused Password -> `CannotOpen(CannotOpen::Damaged)`",
         files=[(W, "            ErrorCode::Password => self.end_with(OfficeState::NoPreview(NoPreview::Password)),\n",
                 "            ErrorCode::Password => self.end_with(OfficeState::CannotOpen(CannotOpen::Damaged)),\n")],
         filt="error_codes_become_their_states",
         expect=["error_codes_become_their_states"]),
    dict(id="E2-06", crate="native",
         note="E2 #6: cells_refused `if code != ErrorCode::RangeTooLarge {` -> `if true {`",
         files=[(W, "        if code != ErrorCode::RangeTooLarge {\n", "        if true {\n")],
         filt="a_range_too_large_is_asked_again_in_halves",
         expect=["a_range_too_large_is_asked_again_in_halves"]),
    dict(id="E2-07", crate="native",
         note="E2 #7: outline_page `match next {` -> `match None::<u32> {`",
         files=[(W, "        match next {\n", "        match None::<u32> {\n")],
         filt="the_outline_is_asked_until_next_is_none",
         expect=["the_outline_is_asked_until_next_is_none"]),
    dict(id="E2-08", crate="native",
         note="E2 #8: outline_page `if check::check_outline(..).is_err()` -> `if false` (the test's 2 s bound is the red; "
              "the test outline budget is 5 s, so F10 does not end it first)",
         files=[(W, "        if check::check_outline(&asked, &got, pages, &check::Limits::CONTRACT).is_err() {\n",
                 "        if false {\n")],
         filt="an_outline_that_does_not_advance_kills",
         expect=["an_outline_that_does_not_advance_kills"]),
    dict(id="E2-09", crate="native",
         note="E2 #9 (shape changed): deadline() open timer x1000",
         files=[(W, "            Asked::Open => (run.started_at + self.timeouts.open, Stop::TimedOut),\n",
                 "            Asked::Open => (run.started_at + self.timeouts.open * 1000, Stop::TimedOut),\n")],
         filt="the_open_timer_ends_a_silent_helper",
         expect=["the_open_timer_ends_a_silent_helper"]),
    dict(id="E2-10", crate="native",
         note="E2 #10 (shape changed): `sent_at: Instant` added to Sent, set at the push in ask(), and the request timer "
              "counts from the oldest's sent_at instead of oldest_since",
         files=[(W, "struct Sent<T> {\n    wire: u32,\n", "struct Sent<T> {\n    sent_at: Instant,\n    wire: u32,\n"),
                (W, "        run.sent.push_back(Sent {\n            wire: id,\n",
                 "        run.sent.push_back(Sent {\n            sent_at: Instant::now(),\n            wire: id,\n"),
                (W, "            _ => (run.oldest_since + self.timeouts.request, Stop::TimedOut),\n",
                 "            _ => (oldest.sent_at + self.timeouts.request, Stop::TimedOut),\n")],
         filt="the_request_timer_runs_on_the_oldest_only",
         expect=["the_request_timer_runs_on_the_oldest_only"]),
    dict(id="E2-11", crate="native",
         note="E2 #11: request_page waits for a Page event after sending the Render (5 ms poll loop). Filter is this "
              "test only: in any other page test whose page never comes, this loop never ends (the runner has no timeout)",
         files=[(H, "        self.command(Command::Render(RenderAsk {\n"
                    "            id,\n"
                    "            page,\n"
                    "            scale,\n"
                    "            region,\n"
                    "            w,\n"
                    "            h,\n"
                    "        }));\n",
                 "        self.command(Command::Render(RenderAsk {\n"
                 "            id,\n"
                 "            page,\n"
                 "            scale,\n"
                 "            region,\n"
                 "            w,\n"
                 "            h,\n"
                 "        }));\n"
                 "        while !self.poll().iter().any(|e| matches!(e, Event::Page { .. })) {\n"
                 "            std::thread::sleep(Duration::from_millis(5));\n"
                 "        }\n")],
         filt="the_window_never_waits_on_the_helper",
         expect=["the_window_never_waits_on_the_helper"]),
    dict(id="E2-12", crate="native",
         note="E2 #12 (shape changed): the cancelled arm `(Asked::Render { .. } | Asked::Anchor { .. }, ..) if cancelled` deleted",
         files=[(W, "            (Asked::Render { .. } | Asked::Anchor { .. }, Message::Rendered(_) | Message::Error(_)) if cancelled => {\n"
                    "                let _ = self.handles_ok(false);\n"
                    "            }\n",
                 "")],
         filt="a_cancelled_page_is_discarded",
         expect=["a_cancelled_page_is_discarded"]),
    dict(id="E2-13", crate="native",
         note="E2 #13: Shared::close `if let Some(helper) = helper { helper.end(); }` deleted",
         files=[(H, "        if let Some(helper) = helper {\n"
                    "            helper.end();\n"
                    "        }\n"
                    "        let mut wiped = wipe_state(&mut state);\n",
                 "        let mut wiped = wipe_state(&mut state);\n")],
         filt="close_kills_and_wipes",
         expect=["close_kills_and_wipes"]),
    dict(id="E2-14", crate="native",
         note="E2 #14: Shared::adopt `if inner.closed.is_some() { return false; }` deleted",
         files=[(H, "        let mut inner = lock(&self.inner);\n"
                    "        if inner.closed.is_some() {\n"
                    "            return false;\n"
                    "        }\n"
                    "        inner.helper = Some(helper);\n",
                 "        let mut inner = lock(&self.inner);\n"
                 "        inner.helper = Some(helper);\n")],
         filt="close_while_starting_kills_the_late_helper",
         expect=["close_while_starting_kills_the_late_helper"]),
    dict(id="E2-15", crate="native",
         note="E2 #15: kill_all_in closes only hosts that hold a helper now",
         files=[(H, "    for shared in live {\n",
                 "    for shared in live.into_iter().filter(|shared| lock(&shared.inner).helper.is_some()) {\n")],
         filt="kill_all_ends_every_helper_of_its_registry",
         expect=["kill_all_ends_every_helper_of_its_registry"]),
    dict(id="E2-16", crate="native",
         note="E2 #16: `impl Drop for OfficeHost` deleted",
         files=[(H, "impl Drop for OfficeHost {\n"
                    "    fn drop(&mut self) {\n"
                    "        self.close(CloseWhy::Closed);\n"
                    "    }\n"
                    "}\n",
                 "")],
         filt="drop_kills_the_helper",
         expect=["drop_kills_the_helper"]),
    dict(id="E2-17", crate="native",
         note="E2 #17 (shape changed): retry() `OfficeState::Stopped { can_retry: true, .. }` -> `OfficeState::Stopped { .. }` (inside self.read)",
         files=[(H, "        if !self.read(|state| matches!(state, OfficeState::Stopped { can_retry: true, .. })) {\n",
                 "        if !self.read(|state| matches!(state, OfficeState::Stopped { .. })) {\n")],
         filt="try_again_starts_one_new_helper_once",
         expect=["try_again_starts_one_new_helper_once"]),
    dict(id="E2-18", crate="native",
         note="E2 #18: request_page `check::asked_size(..)` -> V2-E's f64+round expression (ov-office-host BRANCH-NOTES "
              "row 18; A4 at 1.0: 595, not 596). Round only, as the E2 row names it",
         files=[(H, "        let (w, h) = check::asked_size(&probe, &size, &check::Limits::CONTRACT).ok()?;\n",
                 "        let (w, h) = region.map_or(((f64::from(size.w_pt) * f64::from(scale)).round() as u32, "
                 "(f64::from(size.h_pt) * f64::from(scale)).round() as u32), |r| (r.w, r.h));\n")],
         filt="sections_are_sized_by_the_protocol",
         expect=["sections_are_sized_by_the_protocol"]),
    dict(id="E2-19", crate="native",
         note="E2 #19 (shape changed): pump_renders `if out >= MAX_IN_FLIGHT {` -> `if false {` (the count includes anchors)",
         files=[(W, "            if out >= MAX_IN_FLIGHT {\n", "            if false {\n")],
         filt="at_most_six_renders_are_out",
         expect=["at_most_six_renders_are_out"]),
    dict(id="E2-20", crate="native",
         note="E2 #20: request_page's whole-page side check -> `if false {`",
         files=[(H, "        if page_w > MAX_PAGE_SIDE_PX || page_h > MAX_PAGE_SIDE_PX {\n", "        if false {\n")],
         filt="a_page_over_the_engine_side_is_not_asked",
         expect=["a_page_over_the_engine_side_is_not_asked"]),
    dict(id="E2-21", crate="native",
         note="E2 #21: open_with `bytes.len() as u64 > check::MAX_INPUT_BYTES` -> `false`",
         files=[(H, "    let refused = if bytes.len() as u64 > check::MAX_INPUT_BYTES {\n", "    let refused = if false {\n")],
         filt="over_100_mib_or_empty_starts_nothing",
         expect=["over_100_mib_or_empty_starts_nothing"]),
    dict(id="E2-22", crate="native",
         note="E2 #22: start_helper, the installed() one: NotInstalled -> `self.stopped(Stop::Failed)`",
         files=[(W, "        if !self.starter.installed() {\n"
                    "            return self.end_with(OfficeState::NoPreview(NoPreview::NotInstalled));\n",
                 "        if !self.starter.installed() {\n"
                 "            return self.stopped(Stop::Failed);\n")],
         filt="a_missing_helper_says_not_installed",
         expect=["a_missing_helper_says_not_installed"]),
    dict(id="E2-23", crate="native",
         note="E2 #23: start_failed `if error == sandbox::SandboxError::Unsupported {` -> `if false {`. The test is "
              "#[cfg(not(windows))]: on a Windows pass this row reads NOT-IN-CONTROL by design",
         files=[(W, "        if error == sandbox::SandboxError::Unsupported {\n", "        if false {\n")],
         filt="no_sandbox_no_helper",
         expect=["no_sandbox_no_helper"]),
    dict(id="E2-24", crate="native",
         note="E2 #24: the exclusion check moved from before the document's section to after `self.starter.start()` "
              "(one anchor spanning both spots)",
         files=[(W, "        // Before EVERY start: Windows Error Reporting keeps no dump of it.\n"
                    "        if self.starter.exclude_from_crash_reports(&name).is_err() {\n"
                    "            return self.stopped(Stop::Failed);\n"
                    "        }\n"
                    "        // The document's section, once: the bytes are copied in and wiped.\n"
                    "        self.doc = match std::mem::replace(&mut self.doc, Doc::Gone) {\n"
                    "            Doc::Bytes(bytes) => match self.starter.section(bytes.len()) {\n"
                    "                Ok(mut section) => {\n"
                    "                    self.starter.fill(&mut section, &bytes);\n"
                    "                    Doc::Section(section)\n"
                    "                }\n"
                    "                Err(error) => {\n"
                    "                    self.doc = Doc::Bytes(bytes);\n"
                    "                    return self.start_failed(error);\n"
                    "                }\n"
                    "            },\n"
                    "            other => other,\n"
                    "        };\n"
                    "        if matches!(self.doc, Doc::Gone) {\n"
                    "            return self.stopped(Stop::Failed);\n"
                    "        }\n"
                    "        let link = match self.starter.start() {\n"
                    "            Ok(link) => link,\n"
                    "            Err(error) => return self.start_failed(error),\n"
                    "        };\n",
                 "        // The document's section, once: the bytes are copied in and wiped.\n"
                 "        self.doc = match std::mem::replace(&mut self.doc, Doc::Gone) {\n"
                 "            Doc::Bytes(bytes) => match self.starter.section(bytes.len()) {\n"
                 "                Ok(mut section) => {\n"
                 "                    self.starter.fill(&mut section, &bytes);\n"
                 "                    Doc::Section(section)\n"
                 "                }\n"
                 "                Err(error) => {\n"
                 "                    self.doc = Doc::Bytes(bytes);\n"
                 "                    return self.start_failed(error);\n"
                 "                }\n"
                 "            },\n"
                 "            other => other,\n"
                 "        };\n"
                 "        if matches!(self.doc, Doc::Gone) {\n"
                 "            return self.stopped(Stop::Failed);\n"
                 "        }\n"
                 "        let link = match self.starter.start() {\n"
                 "            Ok(link) => link,\n"
                 "            Err(error) => return self.start_failed(error),\n"
                 "        };\n"
                 "        // Before EVERY start: Windows Error Reporting keeps no dump of it.\n"
                 "        if self.starter.exclude_from_crash_reports(&name).is_err() {\n"
                 "            return self.stopped(Stop::Failed);\n"
                 "        }\n")],
         filt="crash_reports_are_off_before_the_first_helper_starts",
         expect=["crash_reports_are_off_before_the_first_helper_starts"]),
]

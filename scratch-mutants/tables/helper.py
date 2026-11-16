# O-CP3 mutants: the Office helper (native/beebeeb-office-render) and its protocol (crates/beebeeb-office-protocol).
# Rows: lanes/ov-helper-wire/BRANCH-NOTES.md, "## Mutants: mine" (HW-*) and "## O-CP2's 22 helper mutants (row H3)" (H3-*);
# M8/M10/M15/M17 text from V2-D's own table (lanes/ov-office-helper/BRANCH-NOTES.md). Root: the desktop tree.
# Fail-fast: cargo stops after the first red test binary, so a row whose expected tests sit in two binaries runs
# with --no-fail-fast (else the second never runs and the verdict reads PARTIAL).

CHECK = "crates/beebeeb-office-protocol/src/check.rs"
OFFICE = "crates/beebeeb-office-protocol/src/office.rs"
WIRE = "crates/beebeeb-office-protocol/src/wire.rs"
ENGINE = "native/beebeeb-office-render/src/engine.rs"
FIT = "native/beebeeb-office-render/src/fit.rs"
SERVE = "native/beebeeb-office-render/src/serve.rs"
START = "native/beebeeb-office-render/src/start.rs"
LIB = "native/beebeeb-office-render/src/lib.rs"
LOCK = "native/beebeeb-office-render/Cargo.lock"

PX_LINE = "    let px = (pt * scale).ceil().max(1.0);\n"

SIZE_GUARD = ("    if want != Some(out.len) {\n"
              "        // The app and the engine disagree on the size: the seam fails loudly.\n"
              "        release(memory, out);\n"
              "        return answers.error(id, ErrorCode::Internal);\n"
              "    }\n")

LOCK_TAIL = ('[[package]]\n'
             'name = "zune-jpeg"\n'
             'version = "0.5.15"\n'
             'source = "registry+https://github.com/rust-lang/crates.io-index"\n'
             'checksum = "27bc9d5b815bc103f142aa054f561d9187d191692ec7c2d1e2b4737f8dbd7296"\n'
             'dependencies = [\n'
             ' "zune-core",\n'
             ']\n')


def nff(names):
    """The helper crate's default command with --no-fail-fast (expected tests in two binaries)."""
    return "cargo test --locked --no-fail-fast -- " + names


MUT = [
    # ---------------------------------------------------------------- ov-helper-wire: "Mutants: mine" (17 rows)
    dict(id="HW-1", crate="protocol",
         note="HW row 1 (F7, check.rs:348/351): the exact-size rule gets 1 px slack: got.w/got.h != want → abs_diff > 1 (one mutant, two edits)",
         files=[(CHECK, "    if got.w != want_w {\n", "    if got.w.abs_diff(want_w) > 1 {\n"),
                (CHECK, "    if got.h != want_h {\n", "    if got.h.abs_diff(want_h) > 1 {\n")],
         filt="a_rendered_page_must_equal_the_ask_exactly",
         expect=["a_rendered_page_must_equal_the_ask_exactly"]),
    dict(id="HW-2", crate="protocol",
         note="HW row 2 (check.rs:351): delete the `if got.h != want_h { … }` block (w compared only); the F7 test shares check_rendered_size (816 x 1057 row)",
         files=[(CHECK, "    if got.h != want_h {\n        return Err(Refused::WrongSize);\n    }\n", "")],
         filt="a_rendered_anchor_must_equal_the_ask_exactly a_rendered_page_must_equal_the_ask_exactly",
         expect=["a_rendered_anchor_must_equal_the_ask_exactly", "a_rendered_page_must_equal_the_ask_exactly"]),
    dict(id="HW-3", crate="protocol",
         note="HW row 3 (office.rs:536-542): delete the `match &self.formula { … }` in CellText::put (decode fails or differs)",
         files=[(OFFICE,
                 "        match &self.formula {\n"
                 "            None => o.u8(0),\n"
                 "            Some(formula) => {\n"
                 "                o.u8(1);\n"
                 "                o.str(formula);\n"
                 "            }\n"
                 "        }\n", "")],
         filt="cells_round_trip_with_values_formulas_and_anchors",
         expect=["cells_round_trip_with_values_formulas_and_anchors"]),
    dict(id="HW-4", crate="protocol",
         note="HW row 4 (wire.rs:114-119): In::f64 body → Ok(f64::from_le_bytes(self.take()?)) (NaN/inf decode)",
         files=[(WIRE,
                 "    pub(crate) fn f64(&mut self) -> Result<f64, DecodeError> {\n"
                 "        let v = f64::from_le_bytes(self.take()?);\n"
                 "        if v.is_finite() {\n"
                 "            Ok(v)\n"
                 "        } else {\n"
                 "            Err(DecodeError::NotFinite)\n"
                 "        }\n"
                 "    }\n",
                 "    pub(crate) fn f64(&mut self) -> Result<f64, DecodeError> {\n"
                 "        Ok(f64::from_le_bytes(self.take()?))\n"
                 "    }\n")],
         filt="a_non_finite_value_is_refused_at_decode",
         expect=["a_non_finite_value_is_refused_at_decode"]),
    dict(id="HW-5", crate="protocol",
         note="HW row 5 (check.rs:414-420): delete the `if cell.formula … { return Err(Refused::FormulaTooLong); }` block",
         files=[(CHECK,
                 "        if cell\n"
                 "            .formula\n"
                 "            .as_ref()\n"
                 "            .is_some_and(|f| f.len() > limits.max_formula_bytes)\n"
                 "        {\n"
                 "            return Err(Refused::FormulaTooLong);\n"
                 "        }\n", "")],
         filt="a_formula_over_the_cap_is_refused",
         expect=["a_formula_over_the_cap_is_refused"]),
    dict(id="HW-6", crate="protocol",
         note="HW row 6 (check.rs:448-455): delete the `for a in anchors { … }` loop in check_anchors. CHANGED to compile: "
              "`let mut ids = BTreeSet::new();` goes too (left alone it is E0282, type annotations needed)",
         files=[(CHECK,
                 "    let mut ids = BTreeSet::new();\n"
                 "    for a in anchors {\n"
                 "        if !anchor_placeable(a, limits) {\n"
                 "            return Err(Refused::BadAnchor);\n"
                 "        }\n"
                 "        if !ids.insert(a.id) {\n"
                 "            return Err(Refused::DuplicateAnchor);\n"
                 "        }\n"
                 "    }\n", "")],
         filt="a_bad_anchor_is_refused",
         expect=["a_bad_anchor_is_refused"]),
    dict(id="HW-7", crate="protocol",
         note="HW row 7 (office.rs:759): i.count(Anchor::BYTES) → i.count(1) (2-declared/50-bytes ends Truncated, not LengthPastEnd)",
         files=[(OFFICE, "        let n = i.count(Anchor::BYTES)?;\n", "        let n = i.count(1)?;\n")],
         filt="anchor_counts_are_bounded_before_allocating",
         expect=["anchor_counts_are_bounded_before_allocating"]),
    dict(id="HW-8", crate="protocol",
         note="HW row 8 (office.rs:1044): Message::RenderAnchor's kind KIND_RENDER_ANCHOR → KIND_RENDER",
         files=[(OFFICE, "            Message::RenderAnchor(_) => KIND_RENDER_ANCHOR,\n",
                 "            Message::RenderAnchor(_) => KIND_RENDER,\n")],
         filt="render_anchor_round_trips_in_one_frame",
         expect=["render_anchor_round_trips_in_one_frame"]),
    dict(id="HW-9", crate="protocol",
         note="HW row 9 (check.rs:254, side_pixels): f32 ceil → f64 product + round (a4 rows: 595 vs 596); the same line as H3-M2 (one rule), "
              "so page_pixels_follow_the_shared_vector likely reds too",
         files=[(CHECK, PX_LINE, "    let px = (f64::from(pt) * f64::from(scale)).round().max(1.0) as f32;\n")],
         filt="anchor_pixels_follow_the_shared_vector",
         expect=["anchor_pixels_follow_the_shared_vector"]),
    dict(id="HW-10", crate="helper",
         note="HW row 10 (engine.rs:353, block_cells): `value: c.value,` → `value: None,`",
         files=[(ENGINE, "                value: c.value,\n", "                value: None,\n")],
         filt="a_cell_block_carries_values_formulas_and_anchors",
         expect=["a_cell_block_carries_values_formulas_and_anchors"]),
    dict(id="HW-11", crate="helper",
         note="HW row 11 (fit.rs:158): cut_to_bytes(formula, MAX_FORMULA_BYTES) → formula.truncate(MAX_FORMULA_BYTES.min(formula.len())) "
              "(panic at a non-boundary: no answer)",
         files=[(FIT, "            cut_to_bytes(formula, MAX_FORMULA_BYTES);\n",
                 "            formula.truncate(MAX_FORMULA_BYTES.min(formula.len()));\n")],
         filt="a_long_formula_is_cut_on_a_char_boundary",
         expect=["a_long_formula_is_cut_on_a_char_boundary"]),
    dict(id="HW-12a", crate="helper",
         note="HW row 12, first mutant (fit.rs:169): delete `data.anchors.truncate(MAX_ANCHORS);` (299 kept)",
         files=[(FIT, "    data.anchors.truncate(MAX_ANCHORS);\n", "")],
         filt="too_many_or_too_large_anchors_are_dropped_and_said",
         expect=["too_many_or_too_large_anchors_are_dropped_and_said"]),
    dict(id="HW-12b", crate="helper",
         note="HW row 12, second mutant (fit.rs:168): .retain(|a| anchor_placeable(…) && ids.insert(a.id)) → .retain(|a| ids.insert(a.id)) "
              "(the 30,000 pt one kept)",
         files=[(FIT, "        .retain(|a| anchor_placeable(a, &Limits::CONTRACT) && ids.insert(a.id));\n",
                 "        .retain(|a| ids.insert(a.id));\n")],
         filt="too_many_or_too_large_anchors_are_dropped_and_said",
         expect=["too_many_or_too_large_anchors_are_dropped_and_said"]),
    dict(id="HW-13", crate="helper",
         note="HW row 13 (serve.rs:295): doc.render_anchor(ask.sheet, ask.anchor, ask.scale, out) → Ok(true) (1 colour, anchor_renders 0)",
         files=[(SERVE, "        doc.render_anchor(ask.sheet, ask.anchor, ask.scale, out)\n", "        Ok(true)\n")],
         filt="an_anchor_renders_through_the_pipe",
         expect=["an_anchor_renders_through_the_pipe"]),
    dict(id="HW-14", crate="helper",
         note="HW row 14 (serve.rs:318-322): delete the `if want != Some(out.len) { … }` block in draw; the same edit as H3-M13 (shared draw)",
         files=[(SERVE, SIZE_GUARD, "")],
         filt="an_anchor_size_mismatch_is_an_error_not_a_guess",
         expect=["an_anchor_size_mismatch_is_an_error_not_a_guess"]),
    dict(id="HW-15a", crate="helper",
         note="HW row 15, first mutant (serve.rs:136): `if next.cancelled {` → `if next.cancelled && !matches!(ask, Ask::RenderAnchor(_)) {` "
              "(Rendered for 4)",
         files=[(SERVE,
                 "                if next.cancelled {\n"
                 "                    skip(next.id, &ask, &mut answers, memory)?;\n",
                 "                if next.cancelled && !matches!(ask, Ask::RenderAnchor(_)) {\n"
                 "                    skip(next.id, &ask, &mut answers, memory)?;\n")],
         filt="a_cancelled_anchor_is_skipped",
         expect=["a_cancelled_anchor_is_skipped"]),
    dict(id="HW-15b", crate="helper",
         note="HW row 15, second mutant (serve.rs:248): `Ask::RenderAnchor(anchor) => release(memory, anchor.out),` → `Ask::RenderAnchor(_) => {}` "
              "(maps(anchor) 0)",
         files=[(SERVE, "        Ask::RenderAnchor(anchor) => release(memory, anchor.out),\n", "        Ask::RenderAnchor(_) => {}\n")],
         filt="a_cancelled_anchor_is_skipped",
         expect=["a_cancelled_anchor_is_skipped"]),
    dict(id="HW-16", crate="helper",
         note="HW row 16 (engine.rs:217): a slide sized by the protocol's page rule from the kept sizes, not by the deck ((960, 540) ≠ (7, 3))",
         files=[(ENGINE, "        self.deck.render_size(page, scale).map_err(render_code)\n",
                 "        { let (w, h) = self.sizes[page as usize]; beebeeb_office_protocol::check::page_pixels(&PageSize { w_pt: w, h_pt: h }, scale)"
                 ".ok_or(ErrorCode::Internal) }\n")],
         filt="slides_are_sized_by_the_engine",
         expect=["slides_are_sized_by_the_engine"]),
    dict(id="HW-17", crate="helper", where="win",
         note="HW row 17 (serve.rs:340): a 20 ms sleep as the first line of answer_cells; red on VM 401, release, ignored test",
         files=[(SERVE, "fn answer_cells(id: u32, cells: Cells, doc: &dyn OpenDoc, answers: &mut Answers<'_>) -> Step {\n",
                 "fn answer_cells(id: u32, cells: Cells, doc: &dyn OpenDoc, answers: &mut Answers<'_>) -> Step {\n"
                 "    std::thread::sleep(std::time::Duration::from_millis(20));\n")],
         cmd="cargo test --release --locked --test latency -- --ignored --nocapture latency_cells_p95_within_16ms",
         expect=["latency_cells_p95_within_16ms"]),

    # ---------------------------------------------------------------- O-CP2's 22 helper mutants (row H3)
    dict(id="H3-M1", crate="helper",
         note="H3 M1 (serve.rs:281): the Render closure answers Ok(false) without rendering (1 colour, not >= 10)",
         files=[(SERVE, "        doc.render(render.page, render.scale, render.region, out)\n", "        Ok(false)\n")],
         filt="a_docx_opens_and_its_page_renders_through_the_pipe",
         expect=["a_docx_opens_and_its_page_renders_through_the_pipe"]),
    dict(id="H3-M2-h", crate="helper",
         note="H3 M2, helper half (protocol check.rs:254, side_pixels): f32 product → f64 product, ceil kept (V2-D's text); "
              "Letter at 4/3: 817 vs the engine's 816. Edits the protocol, runs in the helper (path dependency)",
         files=[(CHECK, PX_LINE, "    let px = (f64::from(pt) * f64::from(scale)).ceil().max(1.0) as f32;\n")],
         filt="the_helper_and_the_protocol_size_a_page_alike",
         expect=["the_helper_and_the_protocol_size_a_page_alike"]),
    dict(id="H3-M2-p", crate="protocol",
         note="H3 M2, protocol half: the same edit as H3-M2-h, the protocol's two vector tests",
         files=[(CHECK, PX_LINE, "    let px = (f64::from(pt) * f64::from(scale)).ceil().max(1.0) as f32;\n")],
         filt="page_pixels_follow_the_shared_vector anchor_pixels_follow_the_shared_vector",
         expect=["page_pixels_follow_the_shared_vector", "anchor_pixels_follow_the_shared_vector"]),
    dict(id="H3-M3", crate="helper",
         note="H3 M3 (fit.rs:78): `if all_fit {` → `if true {` (every entry in one answer)",
         files=[(FIT, "    if all_fit {\n", "    if true {\n")],
         filt="an_outline_over_one_frame_comes_in_pages",
         expect=["an_outline_over_one_frame_comes_in_pages"]),
    dict(id="H3-M4", crate="helper",
         note="H3 M4 (serve.rs:350): the OverFrame arm sends the oversized block instead of RangeTooLarge (Error{Internal})",
         files=[(SERVE, "        Err(office::EncodeError::OverFrame) => answers.error(id, ErrorCode::RangeTooLarge),\n",
                 "        Err(office::EncodeError::OverFrame) => answers.send(id, &Message::CellData(data)),\n")],
         filt="a_cell_block_over_one_frame_answers_range_too_large",
         expect=["a_cell_block_over_one_frame_answers_range_too_large"]),
    dict(id="H3-M5", crate="helper",
         note="H3 M5 (fit.rs:129): `more: (total - named) as u32,` → `more: 0,`",
         files=[(FIT, "        more: (total - named) as u32,\n", "        more: 0,\n")],
         filt="sheet_names_are_cut_and_the_rest_counted",
         expect=["sheet_names_are_cut_and_the_rest_counted"]),
    dict(id="H3-M6", crate="helper",
         note="H3 M6 (fit.rs:154): delete the `data.merges = …clip…` line (merges passed through)",
         files=[(FIT, "    data.merges = data.merges.iter().filter_map(|m| clip(m, asked)).collect();\n", "")],
         filt="merges_are_clipped_to_the_asked_range",
         expect=["merges_are_clipped_to_the_asked_range"]),
    dict(id="H3-M7", crate="helper",
         note="H3 M7 (serve.rs:173): delete `queued.cancelled = true;` (Cancel ignored); expected tests in serve and serve_sheets_slides: --no-fail-fast",
         files=[(SERVE, "                queued.cancelled = true;\n", "")],
         cmd=nff("a_cancel_read_before_its_request_starts_skips_it a_cancelled_anchor_is_skipped"),
         expect=["a_cancel_read_before_its_request_starts_skips_it", "a_cancelled_anchor_is_skipped"]),
    dict(id="H3-M8", crate="helper",
         note="H3 M8 (serve.rs:118, after the drain loop): answer a cancelled request when its Cancel is read (V2-D's loop text); ids [2, 8, 4, 6, 10]",
         files=[(SERVE,
                 "        while let Ok(input) = rx.try_recv() {\n"
                 "            take(&mut queue, input)?;\n"
                 "        }\n",
                 "        while let Ok(input) = rx.try_recv() {\n"
                 "            take(&mut queue, input)?;\n"
                 "        }\n"
                 "        for p in queue.pending.iter().filter(|p| p.cancelled) {\n"
                 "            answers.error(p.id, ErrorCode::Internal)?;\n"
                 "        }\n"
                 "        queue.pending.retain(|p| !p.cancelled);\n")],
         filt="answers_come_in_request_order",
         expect=["answers_come_in_request_order"]),
    dict(id="H3-M9", crate="helper",
         note="H3 M9 (serve.rs:228, in opened): the document's `drop(mapped);` → `std::mem::forget(mapped);` (live_reads 1 after Opened)",
         files=[(SERVE, "    drop(mapped);\n    let doc = doc?;\n", "    std::mem::forget(mapped);\n    let doc = doc?;\n")],
         filt="every_mapping_is_dropped_after_its_request",
         expect=["every_mapping_is_dropped_after_its_request"]),
    dict(id="H3-M9b", crate="helper",
         note="H3 M9b (serve.rs:247, skip): `Ask::Render(render) => release(memory, render.out),` → `Ask::Render(_) => {}` (maps(skipped) 0)",
         files=[(SERVE, "        Ask::Render(render) => release(memory, render.out),\n", "        Ask::Render(_) => {}\n")],
         filt="every_mapping_is_dropped_after_its_request",
         expect=["every_mapping_is_dropped_after_its_request"]),
    dict(id="H3-M10", crate="helper",
         note="H3 M10 (serve.rs:159): an undecodable frame is skipped (let-else return Ok(())) instead of EXIT_PROTOCOL (V2-D's text)",
         files=[(SERVE, "    let message = Message::decode(frame.kind, &frame.payload).map_err(|_| EXIT_PROTOCOL)?;\n",
                 "    let Ok(message) = Message::decode(frame.kind, &frame.payload) else { return Ok(()); };\n")],
         filt="a_bad_frame_ends_the_helper",
         expect=["a_bad_frame_ends_the_helper"]),
    dict(id="H3-M11", crate="helper",
         note="H3 M11 (serve.rs:150): `Input::End(FrameError::Closed) => return Err(EXIT_OK),` → EXIT_PROTOCOL (every close()+EXIT_OK test may red too)",
         files=[(SERVE, "        Input::End(FrameError::Closed) => return Err(EXIT_OK),\n",
                 "        Input::End(FrameError::Closed) => return Err(EXIT_PROTOCOL),\n")],
         filt="end_of_input_ends_the_helper_cleanly",
         expect=["end_of_input_ends_the_helper_cleanly"]),
    dict(id="H3-M12", crate="helper",
         note="H3 M12 (engine.rs:62): `eng::OpenError::Password => ErrorCode::Password,` → Damaged; lib + serve binaries: --no-fail-fast",
         files=[(ENGINE, "        eng::OpenError::Password => ErrorCode::Password,\n", "        eng::OpenError::Password => ErrorCode::Damaged,\n")],
         cmd=nff("open_errors_map_to_their_codes open_errors_map_to_the_fixed_codes"),
         expect=["open_errors_map_to_their_codes", "open_errors_map_to_the_fixed_codes"]),
    dict(id="H3-M13", crate="helper",
         note="H3 M13 (serve.rs:318-322): delete the `if want != Some(out.len) { … }` block in draw (same edit as HW-14); serve + serve_sheets_slides: --no-fail-fast",
         files=[(SERVE, SIZE_GUARD, "")],
         cmd=nff("a_page_size_mismatch_is_an_error_not_a_guess an_anchor_size_mismatch_is_an_error_not_a_guess"),
         expect=["a_page_size_mismatch_is_an_error_not_a_guess", "an_anchor_size_mismatch_is_an_error_not_a_guess"]),
    dict(id="H3-M14", crate="helper",
         note="H3 M14 (serve.rs:333): Rendered { w, h, missing_content } → missing_content: false; serve + serve_sheets_slides: --no-fail-fast. "
              "a_remote_picture_… prints SKIPPED when port 47477 is busy (still asserts missing_content)",
         files=[(SERVE, "        Ok(missing_content) => answers.send(id, &Message::Rendered(Rendered { w, h, missing_content })),\n",
                 "        Ok(missing_content) => answers.send(id, &Message::Rendered(Rendered { w, h, missing_content: false })),\n")],
         cmd=nff("a_remote_picture_says_missing_content_and_fetches_nothing an_anchor_renders_through_the_pipe"),
         expect=["a_remote_picture_says_missing_content_and_fetches_nothing", "an_anchor_renders_through_the_pipe"]),
    dict(id="H3-M15", crate="helper",
         note="H3 M15 (start.rs, top of start()): log::set_max_level(log::LevelFilter::Warn) (V2-D's text). The lane's expected "
              "the_helper_installs_no_logger CANNOT be scored by this runner: start() installs the exit-3 hook, so its red is the no_logger "
              "binary exiting 3 with no `test … FAILED` line (read the log: `error: test failed … --test no_logger`, no `test result:` for it). "
              "expect = the pin V2-D also lists (`set_max_level` is a forbidden word); both binaries run (--no-fail-fast)",
         files=[(START, "pub fn start() {\n", "pub fn start() {\n    log::set_max_level(log::LevelFilter::Warn);\n")],
         cmd="cargo test --locked --no-fail-fast --test no_logger --test pins -- the_helper_installs_no_logger the_helper_writes_nothing_but_frames",
         expect=["the_helper_writes_nothing_but_frames"]),
    dict(id="H3-M16", crate="helper",
         note="H3 M16 (serve.rs, any fn): an eprintln! added; placed as the first line of release()",
         files=[(SERVE, "fn release(memory: &dyn Memory, out: GrantRef) {\n    drop(memory.write(out));\n}\n",
                 "fn release(memory: &dyn Memory, out: GrantRef) {\n    eprintln!(\"x\");\n    drop(memory.write(out));\n}\n")],
         filt="the_helper_writes_nothing_but_frames",
         expect=["the_helper_writes_nothing_but_frames"]),
    dict(id="H3-M17", crate="helper",
         note="H3 M17 (start.rs:12): the hook formats the panic: |_| → |info| { let _ = format!(\"{info}\"); exit } (V2-D's text)",
         files=[(START, "    std::panic::set_hook(Box::new(|_| std::process::exit(i32::from(EXIT_PANIC))));\n",
                 "    std::panic::set_hook(Box::new(|info| { let _ = format!(\"{info}\"); std::process::exit(i32::from(EXIT_PANIC)) }));\n")],
         filt="the_panic_hook_writes_nothing",
         expect=["the_panic_hook_writes_nothing"]),
    dict(id="H3-M18", crate="helper", where="none",
         note="H3 M18: append a `ureq` package to the helper's Cargo.lock. Not run here: with --locked cargo refuses the changed lock, "
              "without it cargo prunes the unused entry. pins.rs reads the lock with include_str! (at COMPILE time), so 'build, then edit the "
              "lock, then run the binary' does NOT work for this one: by hand, edit the lock, re-run the pins rustc line from "
              "`cargo test -v --locked --test pins --no-run` (taken before the edit), run that binary, restore the lock",
         files=[(LOCK, LOCK_TAIL, LOCK_TAIL + '\n[[package]]\nname = "ureq"\nversion = "2.0.0"\n')],
         filt="the_helper_links_no_network_crate",
         expect=["the_helper_links_no_network_crate"]),
    dict(id="H3-M19", crate="helper",
         note="H3 M19 (engine.rs:72): `eng::RenderError::TooLarge => ErrorCode::TooLarge,` → Internal",
         files=[(ENGINE, "        eng::RenderError::TooLarge => ErrorCode::TooLarge,\n", "        eng::RenderError::TooLarge => ErrorCode::Internal,\n")],
         filt="render_errors_map_to_the_fixed_codes",
         expect=["render_errors_map_to_the_fixed_codes"]),
    dict(id="H3-M20", crate="helper",
         note="H3 M20 (engine.rs:61): `eng::OpenError::Unsupported => ErrorCode::Unsupported,` → Damaged (fake row, and the real Xls row); "
              "lib + serve binaries: --no-fail-fast",
         files=[(ENGINE, "        eng::OpenError::Unsupported => ErrorCode::Unsupported,\n", "        eng::OpenError::Unsupported => ErrorCode::Damaged,\n")],
         cmd=nff("open_errors_map_to_the_fixed_codes open_errors_map_to_their_codes"),
         expect=["open_errors_map_to_the_fixed_codes", "open_errors_map_to_their_codes"]),
    dict(id="H3-M21", crate="helper",
         note="H3 M21 (lib.rs:35): `pub const EXIT_PANIC: u8 = 3;` → `= 2;`",
         files=[(LIB, "pub const EXIT_PANIC: u8 = 3;\n", "pub const EXIT_PANIC: u8 = 2;\n")],
         filt="exit_codes_are_distinct",
         expect=["exit_codes_are_distinct"]),
]

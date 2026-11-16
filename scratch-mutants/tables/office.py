# O-CP3 mutants: the Office engine repo (beebeeb-office, and one GridCraft scratch edit).
# Rows: lanes/ov-sheets-api/BRANCH-NOTES.md "## Mutants for O-CP3" (SH-*, list order) and lanes/ov-slides-api/BRANCH-NOTES.md:
# Round 2's three (SL-RS2, SL-RD1-1, SL-RD1-2) and the Round 1 table (SL-1 … SL-15, table order). Root: the Office repo;
# crate="office" (dir beebeeb-office), one test binary per command (`--test <file stem>`).
# Left out on purpose: slides R-D1 "deleting only the signature line" (the lane says it stays GREEN).

SHEET = "beebeeb-office/src/sheet.rs"
DRAW = "beebeeb-office/src/sheet/draw.rs"
SLIDE = "beebeeb-office/src/slide.rs"
INFO = "gridcraft/crates/functions/src/info.rs"
LOCK = "beebeeb-office/Cargo.lock"

LOCK_TAIL = ('[[package]]\n'
             'name = "zune-jpeg"\n'
             'version = "0.5.15"\n'
             'source = "registry+https://github.com/rust-lang/crates.io-index"\n'
             'checksum = "27bc9d5b815bc103f142aa054f561d9187d191692ec7c2d1e2b4737f8dbd7296"\n'
             'dependencies = [\n'
             ' "zune-core",\n'
             ']\n')

PRESENTATION_PACKAGE = "        presentation_package(bytes)?;\n"


def sheets(names):
    return "cargo test --locked --test sheets -- " + names


def slides(names):
    return "cargo test --locked --test slides -- " + names


MUT = [
    # ---------------------------------------------------------------- ov-sheets-api: "Mutants for O-CP3"
    dict(id="SH-1", crate="office",
         note="SH 1 (sheet.rs:292): SheetsInfo built from the names sorted; written as a sort of `sheets` just before the Workbook is built",
         files=[(SHEET,
                 "        Ok(Workbook { info: SheetsInfo { sheets }, extra, limits: *limits, drawings_left_out, book })\n",
                 "        sheets.sort_by(|a, b| a.name.cmp(&b.name));\n"
                 "        Ok(Workbook { info: SheetsInfo { sheets }, extra, limits: *limits, drawings_left_out, book })\n")],
         cmd=sheets("a_workbook_opens_with_its_sheets_in_tab_order"),
         expect=["a_workbook_opens_with_its_sheets_in_tab_order"]),
    dict(id="SH-2", crate="office",
         note="SH 2 (sheet.rs:423): `_ => shown.text` → `_ => format!(\"{value:?}\")` (the raw value, no number format)",
         files=[(SHEET, "            _ => shown.text,\n", "            _ => format!(\"{value:?}\"),\n")],
         cmd=sheets("cells_show_their_number_format"),
         expect=["cells_show_their_number_format"]),
    dict(id="SH-3", crate="office",
         note="SH 3 (sheet.rs:415): `Some(*n)` → `None` (a number carries no value)",
         files=[(SHEET, "            Value::Number(n) if n.is_finite() => Some(*n),\n", "            Value::Number(n) if n.is_finite() => None,\n")],
         cmd=sheets("a_numeric_cell_carries_its_value_and_a_text_cell_none"),
         expect=["a_numeric_cell_carries_its_value_and_a_text_cell_none"]),
    dict(id="SH-4", crate="office",
         note="SH 4 (sheet.rs:406): `let formula = …` → `let formula: Option<String> = None;`",
         files=[(SHEET, "        let formula = cell.formula.as_ref().map(|f| format!(\"={}\", f.text));\n",
                 "        let formula: Option<String> = None;\n")],
         cmd=sheets("a_formula_cell_carries_its_formula"),
         expect=["a_formula_cell_carries_its_formula"]),
    dict(id="SH-5", crate="office",
         note="SH 5 (sheet.rs:282): delete `recalculate(&mut book, &uncached);`",
         files=[(SHEET, "        recalculate(&mut book, &uncached);\n", "")],
         cmd=sheets("a_formula_without_a_cached_value_is_calculated"),
         expect=["a_formula_without_a_cached_value_is_calculated"]),
    dict(id="SH-6", crate="office",
         note="SH 6 (sheet.rs:481): `calc.cells_changed(book, uncached);` → `calc.recalc_all(book);` (every formula recalculated; "
              "Calc::recalc_all(&mut self, &mut Workbook) exists in gridcraft-calc)",
         files=[(SHEET, "    calc.cells_changed(book, uncached);\n", "    calc.recalc_all(book);\n")],
         cmd=sheets("a_cached_value_is_shown_as_the_file_has_it"),
         expect=["a_cached_value_is_shown_as_the_file_has_it"]),
    dict(id="SH-7", crate="office",
         note="SH 7 (sheet.rs:313): `.iter_range(r)` → `.iter()` (same item type, every cell of the sheet)",
         files=[(SHEET, "            .iter_range(r)\n", "            .iter()\n")],
         cmd=sheets("a_range_returns_only_its_cells_and_its_sizes"),
         expect=["a_range_returns_only_its_cells_and_its_sizes"]),
    dict(id="SH-8", crate="office",
         note="SH 8 (sheet.rs:358): `sh.freeze.unwrap_or((0, 0))` → `(0, 0)`",
         files=[(SHEET, "            frozen: sh.freeze.unwrap_or((0, 0)),\n", "            frozen: (0, 0),\n")],
         cmd=sheets("merges_and_frozen_panes_come_from_the_model"),
         expect=["merges_and_frozen_panes_come_from_the_model"]),
    dict(id="SH-9", crate="office",
         note="SH 9 (sheet.rs:344): merges `.filter(|m| m.intersects(&r))` → `.filter(|_| true)`",
         files=[(SHEET, "            .filter(|m| m.intersects(&r))\n", "            .filter(|_| true)\n")],
         cmd=sheets("merges_and_frozen_panes_come_from_the_model"),
         expect=["merges_and_frozen_panes_come_from_the_model"]),
    dict(id="SH-10", crate="office",
         note="SH 10 (sheet.rs:347): anchors `filter(|p| touches(…))` → `filter(|_| true)`",
         files=[(SHEET, "        let anchors: Vec<&Placed> = ext.anchors.iter().filter(|p| touches(p.anchor.from, p.anchor.to, &r)).collect();\n",
                 "        let anchors: Vec<&Placed> = ext.anchors.iter().filter(|_| true).collect();\n")],
         cmd=sheets("anchors_touching_the_range_are_listed"),
         expect=["anchors_touching_the_range_are_listed"]),
    dict(id="SH-11", crate="office",
         note="SH 11 (draw.rs:50): delete `out.copy_from_slice(&raster);` and the text loop; return Ok(false) without drawing",
         files=[(DRAW,
                 "    out.copy_from_slice(&raster);\n"
                 "    let mut missing = false;\n"
                 "    for p in &prims {\n"
                 "        if let Prim::Text { .. } = p {\n"
                 "            missing |= !text(out, (w, h), scale, &faces, p);\n"
                 "        }\n"
                 "    }\n"
                 "    Ok(missing)\n",
                 "    Ok(false)\n")],
         cmd=sheets("a_chart_renders_as_pixels_of_its_size"),
         expect=["a_chart_renders_as_pixels_of_its_size"]),
    dict(id="SH-12", crate="office",
         note="SH 12 (sheet.rs:444, page_pixels): f32 ceil → f64 product and round (Letter at 4/3 catches f64, the 74.25 pt picture catches round)",
         files=[(SHEET, "    let (w, h) = ((w_pt * scale).ceil().max(1.0), (h_pt * scale).ceil().max(1.0));\n",
                 "    let (w, h) = ((f64::from(w_pt) * f64::from(scale)).round().max(1.0) as f32, "
                 "(f64::from(h_pt) * f64::from(scale)).round().max(1.0) as f32);\n")],
         cmd=sheets("an_anchor_size_follows_the_page_rule"),
         expect=["an_anchor_size_follows_the_page_rule"]),
    dict(id="SH-13", crate="office",
         note="SH 13 (sheet.rs:383): delete `check_buffer(w, h, out)?;` in render_anchor (a chart answers Engine, a picture panics on the index)",
         files=[(SHEET, "        check_buffer(w, h, out)?;\n", "")],
         cmd=sheets("a_wrong_buffer_or_scale_is_refused"),
         expect=["a_wrong_buffer_or_scale_is_refused"]),
    dict(id="SH-14", crate="office",
         note="SH 14 (draw.rs:78): delete the `if over { … }` block (decode, then check; the probe counts the bytes handed to the decoder)",
         files=[(DRAW, "    if over {\n        frame(out, w, h);\n        return true;\n    }\n", "")],
         cmd=sheets("a_picture_over_the_image_limits_is_not_decoded"),
         expect=["a_picture_over_the_image_limits_is_not_decoded"]),
    dict(id="SH-15", crate="office",
         note="SH 15 (sheet.rs:260): delete the `content > limits.max_cells` check",
         files=[(SHEET,
                 "                if content > limits.max_cells {\n"
                 "                    return Err(OpenError::TooLarge(crate::limits::CELLS));\n"
                 "                }\n", "")],
         cmd=sheets("over_the_sheet_or_cell_limit_is_too_large"),
         expect=["over_the_sheet_or_cell_limit_is_too_large"]),
    dict(id="SH-16", crate="office",
         note="SH 16 (sheet.rs:236): delete `check_content(kind, bytes)?;` (the .ods reaches the reader, Damaged; the probe counts bytes)",
         files=[(SHEET, "        check_content(kind, bytes)?;\n", "")],
         cmd=sheets("only_a_workbook_of_the_asked_kind_opens"),
         expect=["only_a_workbook_of_the_asked_kind_opens"]),
    dict(id="SH-17", crate="office",
         note="SH 17, R-D1 'the container check removed' (ONE mutant, two edits): sheet.rs:670 no-signature Damaged → Unsupported, "
              "and :672 unreadable zip directory Damaged → Unsupported (garbage/CSV catch 670; truncated .xlsx and zip-signature garbage catch 672)",
         files=[(SHEET, "        return Err(OpenError::Damaged);\n", "        return Err(OpenError::Unsupported);\n"),
                (SHEET, "else { return Err(OpenError::Damaged) };\n", "else { return Err(OpenError::Unsupported) };\n")],
         cmd=sheets("bytes_that_are_no_container_are_damaged"),
         expect=["bytes_that_are_no_container_are_damaged"]),
    dict(id="SH-18", crate="office",
         note="SH 18 (sheet.rs:665): an encrypted compound file `OpenError::Password` → `OpenError::Damaged`",
         files=[(SHEET, "            return Err(OpenError::Password);\n", "            return Err(OpenError::Damaged);\n")],
         cmd=sheets("a_protected_or_damaged_workbook_says_so"),
         expect=["a_protected_or_damaged_workbook_says_so"]),
    dict(id="SH-19", crate="office",
         note="SH 19 (sheet.rs:667): `kind == SheetKind::Xls && …` → `false` (Xls → Unsupported)",
         files=[(SHEET,
                 "        return if kind == SheetKind::Xls && format == gridcraft_xlsx::Format::Xls { Ok(()) } else { Err(OpenError::Unsupported) };\n",
                 "        return if false { Ok(()) } else { Err(OpenError::Unsupported) };\n")],
         cmd=sheets("an_xls_from_the_corpus_opens_with_its_cells"),
         expect=["an_xls_from_the_corpus_opens_with_its_cells"]),
    dict(id="SH-20", crate="office",
         note="SH no-reach 1: GridCraft's WEBSERVICE (value_err, info.rs, shared with FILTERXML) connects to an http:// URL in its first "
              "argument before returning #VALUE! (scratch edit in the GridCraft subtree, compiled as beebeeb-office's path dependency); "
              "the listener then counts the connection",
         files=[(INFO,
                 "fn value_err(_a: &[Arg], _c: &mut dyn Ctx) -> R<Value> {\n"
                 "    Err(CellError::Value)\n"
                 "}\n",
                 "fn value_err(_a: &[Arg], _c: &mut dyn Ctx) -> R<Value> {\n"
                 "    if let Ok(url) = text(_a, 0) {\n"
                 "        if let Some(rest) = url.strip_prefix(\"http://\") {\n"
                 "            let _ = std::net::TcpStream::connect(rest.split('/').next().unwrap_or(rest));\n"
                 "        }\n"
                 "    }\n"
                 "    Err(CellError::Value)\n"
                 "}\n")],
         cmd="cargo test --locked --test sheets_no_reach -- a_webservice_formula_fetches_nothing",
         expect=["a_webservice_formula_fetches_nothing"]),
    dict(id="SH-21", crate="office",
         note="SH no-reach 2: reach_of reads the linked file. Keyed off the formula text (`file://` up to the closing quote), not the "
              "parsed sheet name, so it also fires when the formula does not parse (Formula::new keeps an unparsed text, and re-prints a "
              "parsed one with the quoted name, `file://…` included); the canary's access time moves",
         files=[(SHEET,
                 "fn reach_of(book: &gridcraft_model::Workbook, f: &gridcraft_model::Formula) -> Reach {\n",
                 "fn reach_of(book: &gridcraft_model::Workbook, f: &gridcraft_model::Formula) -> Reach {\n"
                 "    if let Some(at) = f.text.find(\"file://\") {\n"
                 "        let rest = &f.text[at + 7..];\n"
                 "        let _ = std::fs::read(rest.split('\\'').next().unwrap_or(rest));\n"
                 "    }\n")],
         cmd="cargo test --locked --test sheets_no_reach -- an_external_workbook_link_reads_nothing",
         expect=["an_external_workbook_link_reads_nothing"]),

    # ---------------------------------------------------------------- ov-slides-api: Round 2
    dict(id="SL-RS2", crate="office",
         note="slides R-S2: lost_pictures body → vec![false; slides] (the reader leaves a plain shape, nothing else flags)",
         files=[(SLIDE,
                 "fn lost_pictures(bytes: &[u8], slides: usize) -> Vec<bool> {\n"
                 "    let Some(mut pkg) = Package::open(bytes) else { return vec![true; slides] };\n"
                 "    let parts = pkg.slide_parts();\n"
                 "    let mut readable: HashMap<String, bool> = HashMap::new();\n"
                 "    let lost: Vec<bool> = parts.iter().map(|part| pkg.slide_loses_a_picture(part, &mut readable)).collect();\n"
                 "    if lost.len() == slides { lost } else { vec![lost.contains(&true); slides] }\n"
                 "}\n",
                 "fn lost_pictures(bytes: &[u8], slides: usize) -> Vec<bool> {\n"
                 "    vec![false; slides]\n"
                 "}\n")],
         cmd=slides("a_picture_whose_part_is_missing_or_unreadable_is_missing"),
         expect=["a_picture_whose_part_is_missing_or_unreadable_is_missing"]),
    dict(id="SL-RD1-1", crate="office",
         note="slides R-D1 1: no ZIP signature `return Err(OpenError::Damaged)` → Unsupported (the old answer)",
         files=[(SLIDE, "    if !bytes.starts_with(ZIP_MAGIC) {\n        return Err(OpenError::Damaged);\n    }\n",
                 "    if !bytes.starts_with(ZIP_MAGIC) {\n        return Err(OpenError::Unsupported);\n    }\n")],
         cmd=slides("a_protected_or_damaged_deck_says_so"),
         expect=["a_protected_or_damaged_deck_says_so"]),
    dict(id="SL-RD1-2", crate="office",
         note="slides R-D1 2: delete `presentation_package(bytes)?;` (red on the probe count). The SAME edit as SL-7 (Round 1), kept as its own row",
         files=[(SLIDE, PRESENTATION_PACKAGE, "")],
         cmd=slides("a_protected_or_damaged_deck_says_so"),
         expect=["a_protected_or_damaged_deck_says_so"]),

    # ---------------------------------------------------------------- ov-slides-api: Round 1 table (line numbers moved)
    dict(id="SL-1", crate="office",
         note="SL 1 (slide.rs:79, now :85): `titles: vec![]`",
         files=[(SLIDE, "            titles: pres.slides.iter().map(|s| title_text(&s.title())).collect(),\n", "            titles: vec![],\n")],
         cmd=slides("a_deck_opens_with_its_slides_sizes_and_titles a_slide_title_is_folded_and_cut_at_255_characters a_corpus_pptx_renders_every_slide"),
         expect=["a_deck_opens_with_its_slides_sizes_and_titles", "a_slide_title_is_folded_and_cut_at_255_characters",
                 "a_corpus_pptx_renders_every_slide"]),
    dict(id="SL-2", crate="office",
         note="SL 2 (slide.rs:153, now :160): drop `out.copy_from_slice(&pixels); unpremultiply(out);` (Ok without rendering)",
         files=[(SLIDE, "        out.copy_from_slice(&pixels);\n        unpremultiply(out);\n", "")],
         cmd=slides("a_slide_renders_as_pixels_of_its_size a_renderer_image_of_another_size_writes_nothing"),
         expect=["a_slide_renders_as_pixels_of_its_size", "a_renderer_image_of_another_size_writes_nothing"]),
    dict(id="SL-3", crate="office",
         note="SL 3 (slide.rs:108, now :115, render_size): f32 ceil → f64 product and round (A4 595.3 at 1.0: 595, not 596)",
         files=[(SLIDE, "        let (pw, ph) = ((w_pt * scale).ceil(), (h_pt * scale).ceil());\n",
                 "        let (pw, ph) = ((f64::from(w_pt) * f64::from(scale)).round() as f32, (f64::from(h_pt) * f64::from(scale)).round() as f32);\n")],
         cmd=slides("render_size_follows_the_shared_vector"),
         expect=["render_size_follows_the_shared_vector"]),
    dict(id="SL-4", crate="office",
         note="SL 4 (slide.rs:150-153, now :157-159): an image of another size: copy the overlapping rows and answer Ok instead of Engine "
              "(written out: the note gives no code)",
         files=[(SLIDE,
                 "        if rw != w || rh != h || pixels.len() != out.len() {\n"
                 "            return Err(RenderError::Engine);\n"
                 "        }\n",
                 "        if rw != w || rh != h || pixels.len() != out.len() {\n"
                 "            let (cw, ch) = (rw.min(w) as usize * 4, rh.min(h) as usize);\n"
                 "            for y in 0..ch {\n"
                 "                let (src, dst) = (y * rw as usize * 4, y * w as usize * 4);\n"
                 "                if src + cw <= pixels.len() {\n"
                 "                    out[dst..dst + cw].copy_from_slice(&pixels[src..src + cw]);\n"
                 "                }\n"
                 "            }\n"
                 "            unpremultiply(out);\n"
                 "            return Ok(RenderInfo { w, h, missing_content: panicked || self.missing_on(n as usize) });\n"
                 "        }\n")],
         cmd=slides("a_renderer_image_of_another_size_writes_nothing"),
         expect=["a_renderer_image_of_another_size_writes_nothing"]),
    dict(id="SL-5", crate="office",
         note="SL 5 (slide.rs:144, now :151): delete `check_buffer(w, h, out)?;` (gets Engine, not BufferSize)",
         files=[(SLIDE, "        check_buffer(w, h, out)?;\n", "")],
         cmd=slides("a_wrong_buffer_scale_or_slide_is_refused"),
         expect=["a_wrong_buffer_scale_or_slide_is_refused"]),
    dict(id="SL-6", crate="office",
         note="SL 6 (slide.rs:66-68, now :71-73): delete the max_pages check",
         files=[(SLIDE,
                 "        if pres.slides.len() as u64 > u64::from(limits.max_pages) {\n"
                 "            return Err(OpenError::TooLarge(crate::limits::PAGES));\n"
                 "        }\n", "")],
         cmd=slides("over_the_slide_limit_is_too_large"),
         expect=["over_the_slide_limit_is_too_large"]),
    dict(id="SL-7", crate="office",
         note="SL 7 (slide.rs:63, now :68): delete `presentation_package(bytes)?;` (import without the content check); same edit as SL-RD1-2",
         files=[(SLIDE, PRESENTATION_PACKAGE, "")],
         cmd=slides("only_a_presentation_package_opens a_protected_or_damaged_deck_says_so"),
         expect=["only_a_presentation_package_opens", "a_protected_or_damaged_deck_says_so"]),
    dict(id="SL-8", crate="office",
         note="SL 8 (slide.rs:214, now :227): an encrypted compound file `OpenError::Password` → `OpenError::Damaged`",
         files=[(SLIDE,
                 "        return Err(if deckcraft_pptx::is_encrypted(bytes) { OpenError::Password } else { OpenError::Unsupported });\n",
                 "        return Err(if deckcraft_pptx::is_encrypted(bytes) { OpenError::Damaged } else { OpenError::Unsupported });\n")],
         cmd=slides("a_protected_or_damaged_deck_says_so"),
         expect=["a_protected_or_damaged_deck_says_so"]),
    dict(id="SL-9", crate="office",
         note="SL 9 (slide.rs:381, now :606): the Media arm's `self.not_drawn = true;` → `{}`",
         files=[(SLIDE, "                self.not_drawn = true;\n                self.pictures.extend(clip.poster);\n",
                 "                {}\n                self.pictures.extend(clip.poster);\n")],
         cmd=slides("a_slide_with_video_shows_its_poster_and_says_content_is_missing"),
         expect=["a_slide_with_video_shows_its_poster_and_says_content_is_missing"]),
    dict(id="SL-10", crate="office", where="none",
         note="SL 10: a scratch `deckcraft-media` package in beebeeb-office/Cargo.lock. Not run here: cargo prunes it (no --locked) or refuses "
              "(--locked). The test reads the lock at RUN time (fs::read_to_string), so by hand: `cargo test --locked --test slides --no-run`, "
              "apply this edit, run the slides binary with the filter, restore the lock",
         files=[(LOCK, LOCK_TAIL, LOCK_TAIL + '\n[[package]]\nname = "deckcraft-media"\nversion = "0.0.0"\n')],
         cmd=slides("the_viewer_links_no_media_or_engine_crate"),
         expect=["the_viewer_links_no_media_or_engine_crate"]),
    dict(id="SL-11", crate="office",
         note="SL 11: drop_oversized_pictures body → {} (canary drawn)",
         files=[(SLIDE,
                 "fn drop_oversized_pictures(pres: &mut Presentation, limits: &Limits) {\n"
                 "    for m in &mut pres.media {\n"
                 "        let Some((w, h)) = wordcraft_render::image_size(m.data.as_slice()) else { continue };\n"
                 "        if w > limits.max_image_side_px || h > limits.max_image_side_px || u64::from(w) * u64::from(h) > limits.max_image_pixels {\n"
                 "            m.data = Arc::new(Vec::new());\n"
                 "        }\n"
                 "    }\n"
                 "}\n",
                 "fn drop_oversized_pictures(pres: &mut Presentation, limits: &Limits) {}\n")],
         cmd=slides("a_picture_over_the_limits_or_that_does_not_decode_is_missing"),
         expect=["a_picture_over_the_limits_or_that_does_not_decode_is_missing"]),
    dict(id="SL-12", crate="office",
         note="SL 12: picture_shows' decode check → true (the broken blob says nothing missing)",
         files=[(SLIDE, "deckcraft_render::decode_image(&m.data).is_some()", "true")],
         cmd=slides("a_picture_over_the_limits_or_that_does_not_decode_is_missing"),
         expect=["a_picture_over_the_limits_or_that_does_not_decode_is_missing"]),
    dict(id="SL-13", crate="office",
         note="SL 13 (slide.rs:181, now :191, picture_shows): a linked http:// picture is connected to before `return false` "
              "(the 'test-only fetcher', written as a plain TcpStream::connect); the listener counts 1",
         files=[(SLIDE,
                 "        if m.link.is_some() || m.data.is_empty() {\n",
                 "        if let Some(link) = &m.link {\n"
                 "            if let Some(rest) = link.strip_prefix(\"http://\") {\n"
                 "                let _ = std::net::TcpStream::connect(rest.split('/').next().unwrap_or(rest));\n"
                 "            }\n"
                 "        }\n"
                 "        if m.link.is_some() || m.data.is_empty() {\n")],
         cmd="cargo test --locked --test slides_no_reach -- a_linked_picture_is_not_fetched",
         expect=["a_linked_picture_is_not_fetched"]),
    dict(id="SL-14", crate="office",
         note="SL 14: the linked file target (file: URL or bare path) read into m.data. MOVED from picture_shows (slide.rs:181, where m is "
              "immutable and a bare read is invisible to this test: it counts canary pixels and missing_content, not access time) to "
              "Deck::open right after drop_oversized_pictures, before `decodes` is built: data set, link cleared, so the canary draws "
              "and missing_content goes false. Relies on the reader keeping a MediaItem with `link` for an r:link picture",
         files=[(SLIDE,
                 "        drop_oversized_pictures(&mut pres, limits);\n",
                 "        drop_oversized_pictures(&mut pres, limits);\n"
                 "        for m in &mut pres.media {\n"
                 "            let Some(link) = m.link.clone() else { continue };\n"
                 "            if let Ok(data) = std::fs::read(link.strip_prefix(\"file://\").unwrap_or(link.as_str())) {\n"
                 "                m.data = Arc::new(data);\n"
                 "                m.link = None;\n"
                 "            }\n"
                 "        }\n")],
         cmd="cargo test --locked --test slides_no_reach -- a_linked_file_target_is_not_read",
         expect=["a_linked_file_target_is_not_read"]),
    dict(id="SL-15", crate="office",
         note="SL 15 (slide.rs:63): Deck::open → `return Err(OpenError::Unsupported)` as its first line (rest unreachable: a warning); "
              "every other slides test reds too",
         files=[(SLIDE,
                 "        crate::prescan(bytes, limits)?;\n" + PRESENTATION_PACKAGE,
                 "        return Err(OpenError::Unsupported);\n        crate::prescan(bytes, limits)?;\n" + PRESENTATION_PACKAGE)],
         cmd=slides("a_corpus_pptx_renders_every_slide"),
         expect=["a_corpus_pptx_renders_every_slide"]),
]

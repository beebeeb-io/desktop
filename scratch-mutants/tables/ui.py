N = "native/beebeeb-drive-rune/src/"
DL = N + "frames/office/docs_live.rs"
SH = N + "frames/office/sheets.rs"
SL = N + "frames/office/slides.rs"
GL = N + "frames/viewers/office_glue.rs"

MUT = [
    dict(id="V5-1", crate="native", where="linux",
         note="ov-ui-binding: docs_live.rs asked_pages returns (1, pages) (every page asked)",
         files=[(DL, "    (lo, hi)\n}\n", "    (1, pages)\n}\n")],
         filt="office_docs_asks_only_the_pages_near_the_screen",
         expect=["office_docs_asks_only_the_pages_near_the_screen"]),
    dict(id="V5-2", crate="native", where="linux",
         note="ov-ui-binding: docs_live.rs paper_live drops the `!` in `if !uses_tiles` (whole pages at 150 %, tiles below)",
         files=[(DL, "    if !uses_tiles(zoom, live.dpi) {\n", "    if uses_tiles(zoom, live.dpi) {\n")],
         filt="office_docs_asks_tiles_at_150_and_whole_pages_below",
         expect=["office_docs_asks_tiles_at_150_and_whole_pages_below"]),
    dict(id="V5-3", crate="native", where="linux",
         note="ov-ui-binding: docs_live.rs a tile's absolute(..) y uses 0.0 for the row offset",
         files=[(DL, ".absolute((col_ * TILE_PX) as f32 / device, (row_ * TILE_PX) as f32 / device)",
                 ".absolute((col_ * TILE_PX) as f32 / device, 0.0)")],
         filt="office_docs_tiles_sit_at_their_place",
         expect=["office_docs_tiles_sit_at_their_place"]),
    dict(id="V5-4", crate="native", where="linux",
         note="ov-ui-binding: docs_live.rs the whole Page key (below the tile scale) asks zoom_pct: 100 instead of zoom",
         files=[(DL, "            live.asks.get(OfficePicture::Page {\n                page: page0,\n                zoom_pct: zoom,\n",
                 "            live.asks.get(OfficePicture::Page {\n                page: page0,\n                zoom_pct: 100,\n")],
         filt="office_docs_a_zoom_change_asks_again",
         expect=["office_docs_a_zoom_change_asks_again"]),
    dict(id="V5-5", crate="native", where="linux",
         note="ov-ui-binding: docs_live.rs Asks::get skips the `seen` lookup. PREDICTED SURVIVED (read, not run): the map "
              "lives for one window() build, and doc_of(6, 150, 1) asks no key twice in one build (pages 1-2, distinct "
              "tiles, one held page each), so the test cannot see the lookup",
         files=[(DL, "        if let Some(held) = self.seen.borrow().get(&which) {\n            return held.clone();\n        }\n", "")],
         filt="office_docs_asks_each_key_once_per_frame",
         expect=["office_docs_asks_each_key_once_per_frame"]),
    dict(id="V5-6", crate="native", where="linux",
         note="ov-ui-binding: docs_live.rs the held-page block (the page under missing tiles) removed",
         files=[(DL,
                 "    if tiles.iter().any(|(_, _, p)| p.is_none()) {\n"
                 "        // No blank while they come: the page as held at the lowest scale, under them.\n"
                 "        let held = live.asks.get(OfficePicture::Page {\n"
                 "            page: page0,\n"
                 "            zoom_pct: 100,\n"
                 "            dpi: live.dpi,\n"
                 "        });\n"
                 "        if let Some(p) = held {\n"
                 "            page = page.child(\n"
                 "                el().key(format!(\"page-held-{n}\"))\n"
                 "                    .absolute(0.0, 0.0)\n"
                 "                    .size(w, h)\n"
                 "                    .pixels_fit(&p, SurfaceFit::Fill),\n"
                 "            );\n"
                 "        }\n"
                 "    }\n",
                 "")],
         filt="office_docs_a_missing_tile_draws_the_held_page_under_it",
         expect=["office_docs_a_missing_tile_draws_the_held_page_under_it"]),
    dict(id="V5-7", crate="native", where="linux",
         note="ov-ui-binding: sheets.rs window_live asks cells(view.sheet, 0, 0, ..) (not from the top left)",
         files=[(SH, "    cells(view.sheet, view.top_left.row, view.top_left.col, rows * 2, cols * 2);\n",
                 "    cells(view.sheet, 0, 0, rows * 2, cols * 2);\n")],
         filt="office_sheets_asks_the_cells_in_view",
         expect=["office_sheets_asks_the_cells_in_view"]),
    dict(id="V5-8", crate="native", where="linux",
         note="ov-ui-binding: sheets.rs scroll_key without the .clamp (row/col cast as they are)",
         files=[(SH, "        row: nr.clamp(0, (rows - 1).max(0)) as u32,\n        col: nc.clamp(0, (cols - 1).max(0)) as u32,\n",
                 "        row: nr as u32,\n        col: nc as u32,\n")],
         filt="office_sheets_scroll_stops_at_the_extent",
         expect=["office_sheets_scroll_stops_at_the_extent"]),
    dict(id="V5-9", crate="native", where="linux",
         note="ov-ui-binding: sheets.rs formula_text returns the cell's text first (an early return added before the formula)",
         files=[(SH, "        .find(|c| c.row == view.current.row && c.col == view.current.col);\n"
                     "    if let Some(f) = here.and_then(|c| c.formula.as_ref()) {\n",
                 "        .find(|c| c.row == view.current.row && c.col == view.current.col);\n"
                 "    if let Some(c) = here {\n        return c.text.clone();\n    }\n"
                 "    if let Some(f) = here.and_then(|c| c.formula.as_ref()) {\n")],
         filt="office_sheets_formula_bar_shows_the_current_cells_formula",
         expect=["office_sheets_formula_bar_shows_the_current_cells_formula"]),
    dict(id="V5-10", crate="native", where="linux",
         note="ov-ui-binding: sheets.rs stats uses number(&cell.text) only (cell.value ignored)",
         files=[(SH, "        if let Some(v) = cell.value.or_else(|| number(&cell.text)) {\n",
                 "        if let Some(v) = number(&cell.text) {\n")],
         filt="office_sheets_stats_use_the_values",
         expect=["office_sheets_stats_use_the_values"]),
    dict(id="V5-11", crate="native", where="linux",
         note="ov-ui-binding: sheets.rs anchors_of places each anchor at absolute(0.0, 0.0) (the anchor's own box; the "
              "layer's absolute(0.0, 0.0) is already 0)",
         files=[(SH, "            .absolute(ROW_HEAD_W + x, HEADER_H + y)\n", "            .absolute(0.0, 0.0)\n")],
         filt="office_sheets_an_anchor_draws_its_picture_at_its_place",
         expect=["office_sheets_an_anchor_draws_its_picture_at_its_place"]),
    dict(id="V5-12", crate="native", where="linux",
         note="ov-ui-binding: sheets.rs status ignores missing_content (`if view.missing_content` -> `if false`)",
         files=[(SH, "    if view.missing_content {\n", "    if false {\n")],
         filt="office_sheets_quiet_line_when_content_is_missing",
         expect=["office_sheets_quiet_line_when_content_is_missing"]),
    dict(id="V5-13", crate="native", where="linux",
         note="ov-ui-binding: slides.rs thumbs_asked returns u32::MAX (every thumbnail asked; deck(60) is finite: no hang)",
         files=[(SL, "    (2.0 * screen_h / (THUMB_H + 10.0)).ceil() as u32 + 1\n", "    u32::MAX\n")],
         filt="office_slides_thumbnails_ask_their_slides",
         expect=["office_slides_thumbnails_ask_their_slides"]),
    dict(id="V5-14", crate="native", where="linux",
         note="ov-ui-binding: slides.rs present_live draws slide_of_words (no pixels) in place of the asked picture: "
              "present_of(cx, deck, slide_of_words(None)); the picture is still asked, `slide` unused",
         files=[(SL, "    present_of(cx, deck, slide)\n", "    present_of(cx, deck, slide_of_words(None))\n")],
         filt="office_slides_present_draws_the_slide_pixels",
         expect=["office_slides_present_draws_the_slide_pixels"]),
    dict(id="V5-15", crate="native", where="linux",
         note="ov-ui-binding: shell/host.rs poll_capture drops the `|| slides::presenting(s)` term",
         files=[(N + "shell/host.rs",
                 "        && crate::frames::files::share::shows_secret(s))\n"
                 "    // A deck in Present (the viewer contract §5.4): its slides stay out of captures and screen shares.\n"
                 "    || crate::frames::office::slides::presenting(s);\n",
                 "        && crate::frames::files::share::shows_secret(s));\n")],
         filt="shell::host::tests::office_present_is_kept_out_of_captures",
         expect=["office_present_is_kept_out_of_captures"]),
    dict(id="V5-16", crate="native", where="linux",
         note="ov-ui-binding: office_glue.rs office_try_again skips source.retry (the match on it deleted; `id` unused)",
         files=[(GL,
                 "    match ui.source.retry(&id) {\n"
                 "        Ok(file) => ui.replace(file),\n"
                 "        Err(_) => {\n"
                 "            if let Body::OfficeState(body) = &mut ui.file.body {\n"
                 "                body.stopped_again();\n"
                 "            }\n"
                 "        }\n"
                 "    }\n",
                 "")],
         filt="office_states_try_again_asks_the_live_source_once",
         expect=["office_states_try_again_asks_the_live_source_once"]),
    dict(id="V5-17", crate="native", where="linux",
         note="ov-ui-binding: office_glue.rs window()'s `live` forced false (the one place the docs/sheets/slides windows "
              "pick live). Lane: every live test red, fixture tests green; expect = one asking test per live test file. Not literal 'every': "
              "office_try_again and on_key still ask source.office_live(), so the try-again and key-scroll tests may stay green",
         files=[(GL, "    let live = ui.source.office_live();\n", "    let live = false;\n")],
         filt="office_docs_asks_only_the_pages_near_the_screen office_sheets_asks_the_cells_in_view "
              "office_slides_thumbnails_ask_their_slides",
         expect=["office_docs_asks_only_the_pages_near_the_screen", "office_sheets_asks_the_cells_in_view",
                 "office_slides_thumbnails_ask_their_slides"]),
    dict(id="V5-sizept", crate="native", where="linux",
         note="lead (0b004601): docs.rs size_pt back to the old filter (no finite / 20:1 bound). PREDICTED SURVIVED (read, "
              "not run): the box width is PAGE_W x zoom whatever the file says, and paper_live walks only the rows "
              "touching the band (same commit), so the (1, 1e9) page asks 2 cols x 4 rows + 1 held = 9 < 200; NaN was "
              "already refused by `> 0.0`. The pre-commit row loop also skipped rows outside the band, so the ask count "
              "cannot see either half of 0b004601 (a test gap: assert the page box, e.g. page 1's height = A4's)",
         files=[(N + "frames/office/docs.rs",
                 "        .filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0 && h / w <= 20.0 && w / h <= 20.0)\n",
                 "        .filter(|(w, h)| *w > 0.0 && *h > 0.0)\n")],
         filt="office_docs_an_absurd_page_size_is_bounded",
         expect=["office_docs_an_absurd_page_size_is_bounded"]),
]

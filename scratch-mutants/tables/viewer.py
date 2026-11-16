# O-CP3 mutants: ov-viewer-glue (V3-1 + CP2 row 10.3), lane head 920de161, from its BRANCH-NOTES "## Mutants" table.
# Line numbers in the notes are the lane's head; anchors here are found by text in the merged tree (native/ov-integ-cp3).
N = "native/beebeeb-drive-rune/src/"
VO = N + "engine_host/viewer_office.rs"
GLUE = N + "engine_host/viewer_office/glue.rs"
CELLS = N + "engine_host/viewer_office/cells.rs"
KINDS = N + "frames/viewers/kinds.rs"
SOURCE = N + "frames/viewers/source.rs"
MOD = N + "frames/viewers/mod.rs"
STATES = N + "frames/viewers/states.rs"
VIEWER = N + "engine_host/viewer.rs"
T = "viewer::office_tests::"

MUT = [
    dict(id="V1-L1", crate="native",
         note="L1 (lock 1): VO builds Body::Word(..) for a Docx Pages (the board's WordDoc stands in for the editor's body)",
         files=[(VO,
                 "                    model.body = Body::Doc(DocBody::new(view, \"\"));\n",
                 "                    model.body = if kind == OfficeKind::Docx { Body::Word(crate::frames::viewers::office_board::service_agreement()) } else { Body::Doc(DocBody::new(view, \"\")) };\n")],
         filt=T + "office_editor_body_never_from_the_engine_source",
         expect=["office_editor_body_never_from_the_engine_source"]),
    dict(id="V1-L3", crate="native",
         note="L3 (lock 3): EngineViewerSource overrides save_document to call engine.save_text (returns Failed, so the test's writes==0 goes red: writes 1)",
         files=[(VIEWER,
                 "    fn finds(&self) -> bool {\n        self.viewer_find_finds()\n    }\n",
                 "    fn save_document(&mut self, file_id: &str, base: u32) -> SaveOutcome {\n"
                 "        let _ = self.engine.save_text(self.engine.session_turn(), file_id, \"\", i64::from(base), 0);\n"
                 "        SaveOutcome::Failed\n"
                 "    }\n\n"
                 "    fn finds(&self) -> bool {\n        self.viewer_find_finds()\n    }\n")],
         filt=T + "office_engine_source_saves_nothing",
         expect=["office_engine_source_saves_nothing"]),
    dict(id="V1-R1", crate="native",
         note="R1: office_route_with ignores `ships`: `let shipped = kind != OfficeKind::Ppt;` (the per-type match replaced; `ships` unused: a warning)",
         files=[(KINDS,
                 "    let shipped = match kind {\n"
                 "        OfficeKind::Docx => ships.docx,\n"
                 "        OfficeKind::Doc => ships.doc,\n"
                 "        OfficeKind::Xlsx => ships.xlsx,\n"
                 "        OfficeKind::Xls => ships.xls,\n"
                 "        OfficeKind::Pptx => ships.pptx,\n"
                 "        OfficeKind::Ppt => false,\n"
                 "    };\n",
                 "    let shipped = kind != OfficeKind::Ppt;\n")],
         filt=T + "the_route_follows_both_switches",
         expect=["the_route_follows_both_switches"]),
    dict(id="V1-R2", crate="native",
         note="R2: OFFICE_SHIPS `docx: true` (merged tree: docx false, the test asserts OFFICE_SHIPS == NONE)",
         files=[(KINDS,
                 "pub const OFFICE_SHIPS: OfficeShips = OfficeShips {\n    docx: false,\n",
                 "pub const OFFICE_SHIPS: OfficeShips = OfficeShips {\n    docx: true,\n")],
         filt=T + "the_shipped_constants_are_off",
         expect=["the_shipped_constants_are_off"]),
    dict(id="V1-F1", crate="native",
         note="F1: glue.rs office_begin's 100 MB check moved after the read (the worker starts, then TooLarge is returned): two tuples, same file, disjoint anchors. TIMING-DEPENDENT kill: the test sleeps 50 ms, then asserts fetches == 0; the fake counts the fetch before its size check, so red only if the worker reaches fetch within 50 ms (a loaded VM can give a false SURVIVED)",
         files=[(GLUE,
                 "        // The contract's limit is the app's, before decrypting: nothing is read.\n"
                 "        if facts.size > check::MAX_INPUT_BYTES {\n"
                 "            return state(ViewState::NoPreview(Reason::TooLarge));\n"
                 "        }\n",
                 ""),
                (GLUE,
                 "        if !started {\n"
                 "            log(\"viewer: could not start a thread for an open file\");\n"
                 "            return Body::Failed;\n"
                 "        }\n",
                 "        if !started {\n"
                 "            log(\"viewer: could not start a thread for an open file\");\n"
                 "            return Body::Failed;\n"
                 "        }\n"
                 "        if facts.size > check::MAX_INPUT_BYTES {\n"
                 "            return state(ViewState::NoPreview(Reason::TooLarge));\n"
                 "        }\n")],
         filt=T + "over_100_mb_fetches_nothing",
         expect=["over_100_mb_fetches_nothing"]),
    dict(id="V1-F2", crate="native",
         note="F2: glue.rs `OfficeRoute::NoPreview(_)` falls through to the read (`NoPreview(kind) => kind`)",
         files=[(GLUE,
                 "            OfficeRoute::NoPreview(_) => return state(ViewState::NoPreview(Reason::UnsupportedType)),\n",
                 "            OfficeRoute::NoPreview(kind) => kind,\n")],
         filt=T + "ppt_and_unshipped_types_fetch_nothing",
         expect=["ppt_and_unshipped_types_fetch_nothing"]),
    dict(id="V1-F3", crate="native",
         note="F3: glue.rs read call `check::MAX_INPUT_BYTES` -> IN_MEMORY_MAX (written as `super::super::IN_MEMORY_MAX`: glue.rs does not import it)",
         files=[(GLUE,
                 "                        recorded,\n                        check::MAX_INPUT_BYTES,\n",
                 "                        recorded,\n                        super::super::IN_MEMORY_MAX,\n")],
         filt=T + "an_office_file_between_64_and_100_mib_is_read_whole",
         expect=["an_office_file_between_64_and_100_mib_is_read_whole"]),
    dict(id="V1-S1", crate="native",
         note="S1: VO take_state `CannotOpen::Damaged` -> Reason::UnsupportedType",
         files=[(VO,
                 "            HostState::CannotOpen(CannotOpen::Damaged) => failure(ViewState::NoPreview(Reason::DamagedOrOverLimit)),\n",
                 "            HostState::CannotOpen(CannotOpen::Damaged) => failure(ViewState::NoPreview(Reason::UnsupportedType)),\n")],
         filt=T + "host_states_become_their_bodies",
         expect=["host_states_become_their_bodies"]),
    dict(id="V1-A1", crate="native",
         note="A1: VO picture() drops the `asking` branch (an ask on its way is asked again every call)",
         files=[(VO,
                 "        if let Some(id) = pictures.asking.get(&which).copied() {\n"
                 "            if let Some(out) = pictures.out.get_mut(&id) {\n"
                 "                out.wanted = turn;\n"
                 "            }\n"
                 "        } else if !pictures.refused.contains(&which) {\n",
                 "        if !pictures.refused.contains(&which) {\n")],
         filt=T + "a_page_is_asked_once_and_drawn_when_it_comes",
         expect=["a_page_is_asked_once_and_drawn_when_it_comes"]),
    dict(id="V1-A2", crate="native",
         note="A2: VO tile_region `w: TILE_PX` (the last column not clipped)",
         files=[(VO,
                 "        w: TILE_PX.min(w - x),\n",
                 "        w: TILE_PX,\n")],
         filt=T + "tiles_ask_their_region",
         expect=["tiles_ask_their_region"]),
    dict(id="V1-A3", crate="native",
         note="A3: VO picture() returns None instead of pictures.nearest(which) (`nearest` unused: a warning)",
         files=[(VO,
                 "        pictures.nearest(which)\n    }\n",
                 "        None\n    }\n")],
         filt=T + "a_new_scale_keeps_the_old_picture_until_the_new_one_comes",
         expect=["a_new_scale_keeps_the_old_picture_until_the_new_one_comes"]),
    dict(id="V1-A4", crate="native",
         note="A4: VO end_turn deletes `host.cancel(id);` (`host` unused: a warning)",
         files=[(VO,
                 "                host.cancel(id);\n",
                 "")],
         filt=T + "what_scrolled_past_is_cancelled",
         expect=["what_scrolled_past_is_cancelled"]),
    dict(id="V1-A5", crate="native",
         note="A5: VO hold_to `max_by_key` (drops the most recently wanted); the loop still removes one per turn, so it ends",
         files=[(VO,
                 "                .min_by_key(|(_, held)| held.wanted)\n",
                 "                .max_by_key(|(_, held)| held.wanted)\n")],
         filt=T + "the_texture_cap_drops_the_least_recently_wanted",
         expect=["the_texture_cap_drops_the_least_recently_wanted"]),
    dict(id="V1-C1", crate="native",
         note="C1: cells.rs merge's `Some(&at)` arm pushes the new cell (appended, the old one not replaced)",
         files=[(CELLS,
                 "                Some(&at) => {\n"
                 "                    let old = &mut view.cells[at];\n"
                 "                    old.text.zeroize();\n"
                 "                    if let Some(formula) = old.formula.as_mut() {\n"
                 "                        formula.zeroize();\n"
                 "                    }\n"
                 "                    *old = new;\n"
                 "                }\n",
                 "                Some(_) => {\n"
                 "                    view.cells.push(new);\n"
                 "                }\n")],
         filt=T + "cells_merge_into_the_sheet_by_place",
         expect=["cells_merge_into_the_sheet_by_place"]),
    dict(id="V1-P1", crate="native",
         note="P1: VO Pictures::forget deletes `self.held.clear();` (the cache kept on Closed)",
         files=[(VO,
                 "    fn forget(&mut self) {\n        self.held.clear();\n",
                 "    fn forget(&mut self) {\n")],
         filt=T + "a_lock_drops_every_picture_and_wipes_the_body",
         expect=["a_lock_drops_every_picture_and_wipes_the_body"]),
    dict(id="V1-K1", crate="native",
         note="K1: frames/viewers/mod.rs Ui::pump deletes `body.carry_view(&self.file.body);` (`mut body` unused mut: a warning)",
         files=[(MOD,
                 "                body.carry_view(&self.file.body);\n",
                 "")],
         filt=T + "a_document_keeps_its_page_and_zoom_when_a_picture_comes "
              + T + "a_deck_keeps_its_slide_and_zoom_when_a_picture_comes "
              + T + "a_sheet_keeps_its_scroll_and_selection_when_cells_come",
         expect=["a_document_keeps_its_page_and_zoom_when_a_picture_comes",
                 "a_deck_keeps_its_slide_and_zoom_when_a_picture_comes",
                 "a_sheet_keeps_its_scroll_and_selection_when_cells_come"]),
    dict(id="V1-K2", crate="native",
         note="K2: source.rs carry_view deletes the document's `new.view.zoom = old.view.zoom;` (the deck's same line is untouched)",
         files=[(SOURCE,
                 "                new.at = old.at;\n                new.view.zoom = old.view.zoom;\n",
                 "                new.at = old.at;\n")],
         filt=T + "a_document_keeps_its_page_and_zoom_when_a_picture_comes",
         expect=["a_document_keeps_its_page_and_zoom_when_a_picture_comes"]),
    dict(id="V1-K3", crate="native",
         note="K3: source.rs carry_view drops the `if new.sheet == old.sheet` guard (the test's second half: another sheet gets the scroll)",
         files=[(SOURCE,
                 "            (Body::Sheet(new), Body::Sheet(old)) if new.sheet == old.sheet => {\n",
                 "            (Body::Sheet(new), Body::Sheet(old)) => {\n")],
         filt=T + "another_file_carries_nothing",
         expect=["another_file_carries_nothing"]),
    dict(id="V1-T1", crate="native",
         note="T1: VO thumb_scale `/ 72.0` instead of `/ 96.0`",
         files=[(VO,
                 "    THUMB_W_PT * f32::from(dpi) / 96.0 / w_pt\n",
                 "    THUMB_W_PT * f32::from(dpi) / 72.0 / w_pt\n")],
         filt=T + "a_thumbnail_is_asked_at_its_width",
         expect=["a_thumbnail_is_asked_at_its_width"]),
    dict(id="V1-S3", crate="native",
         note="S3 (10.3 step 3): source.rs Stopped::can_retry drops `| Stopped::Changed`",
         files=[(SOURCE,
                 "                | Stopped::CopyNotReadable\n                | Stopped::Changed\n",
                 "                | Stopped::CopyNotReadable\n")],
         filt=T + "a_version_that_kept_changing_says_try_again",
         expect=["a_version_that_kept_changing_says_try_again"]),
    dict(id="V1-M2", crate="engine",
         note="M2 (10.3 step 1, seen red by the lane): engine viewer_api.rs failure() -> `Refused(403) | VersionChanged => Failed` (the two arms of 01c36784 put back as one)",
         files=[("crates/beebeeb-engine/src/app/viewer_api.rs",
                 "            DirectError::Refused(403) => OpenFileError::NoAccess,\n"
                 "            DirectError::VersionChanged => OpenFileError::ChangedWhileOpening,\n",
                 "            DirectError::Refused(403) | DirectError::VersionChanged => OpenFileError::Failed,\n")],
         filt="app::viewer_api::refusal_words_tests",
         expect=["failure_names_a_403_as_no_access", "failure_names_a_version_that_kept_changing"]),
    dict(id="V1-M2n", crate="native",
         note="M2n (10.3 step 2, seen red by the lane): engine_host/viewer.rs body_of's NoAccess and ChangedWhileOpening arms -> Body::Failed (c9ab3817 put back)",
         files=[(VIEWER,
                 "        OpenFileError::NoAccess => Body::Stopped(Stopped::NoAccess),\n"
                 "        OpenFileError::ChangedWhileOpening => Body::Stopped(Stopped::Changed),\n",
                 "        OpenFileError::NoAccess => Body::Failed,\n"
                 "        OpenFileError::ChangedWhileOpening => Body::Failed,\n")],
         filt=T + "pinned_route_403_is_not_said_as_not_decrypted " + T + "a_version_that_kept_changing_says_try_again",
         expect=["pinned_route_403_is_not_said_as_not_decrypted", "a_version_that_kept_changing_says_try_again"]),
    dict(id="V1-W1", crate="native",
         note="W1: frames/viewers/states.rs status(): the NoAccess and Changed lines say the old catch-all words (Body::Failed's \"Not decrypted: nothing changed\"); both arms, one combined arm",
         files=[(STATES,
                 "                Stopped::NoAccess => \"Not opened: you no longer have access to this file\",\n"
                 "                Stopped::Changed => \"Not opened: it changed while it was opening\",\n",
                 "                Stopped::NoAccess | Stopped::Changed => \"Not decrypted: nothing changed\",\n")],
         filt="frames::viewers::states::refusal_words_tests",
         expect=["no_access_and_changed_say_what_happened"]),
]

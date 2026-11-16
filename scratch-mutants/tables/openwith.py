# ov-open-with (V3-6, DW-1562 Open With + #460 F11 launch lock): BRANCH-NOTES "## Mutants" 1-12 and the DELTA's
# "Mutants (new, not run)" 13-17. Anchors copied from the merged tree (native/ov-integ-cp3); the notes' line numbers are
# the lane's heads (fedbdbf5 for 1-12, 4018e17e for 13-17).
OWM = "native/beebeeb-drive-rune/src/frames/files/open_with_menu.rs"
APPS = "native/beebeeb-drive-rune/src/engine_host/open_with_apps.rs"
EH = "native/beebeeb-drive-rune/src/engine_host.rs"
LAUNCH = "native/beebeeb-drive-rune/src/engine_host/launch.rs"
MENU_T = "frames::files::open_with_menu_tests::"

MUT = [
    dict(id="OW-1", crate="native", where="linux",
         note="OW #1 (open_with_menu.rs:112, now offered_for :93): `&& viewer_shows(..)` removed -> offered for every "
              "single unprotected file, switches off too",
         files=[(OWM,
                 "        [only] if !only.is_folder && !only.is_protected && !s.protects_selection() && viewer_shows(s, &only.name) => {\n",
                 "        [only] if !only.is_folder && !only.is_protected && !s.protects_selection() => {\n")],
         filt=MENU_T + "open_with_offered_only_for_a_shown_office_file",
         expect=["open_with_offered_only_for_a_shown_office_file"]),
    dict(id="OW-2", crate="native", where="linux",
         note="OW #2 (open_with_menu.rs:136, now entries() :117): the submenu starts empty (no Beebeeb, no separator). "
              "Changed: `vec![]` written `let mut out: Vec<Entry> = vec![];` (type spelled out; same mutant)",
         files=[(OWM,
                 "    let mut out = vec![Entry::item(BEEBEEB_ID, s.identity.product_name()), Entry::Sep];\n",
                 "    let mut out: Vec<Entry> = vec![];\n")],
         filt=MENU_T + "open_with_lists_beebeeb_the_apps_and_other",
         expect=["open_with_lists_beebeeb_the_apps_and_other"]),
    dict(id="OW-3", crate="native", where="linux",
         note="OW #3 (open_with_menu.rs:229, now pick_open_with :306-310): the `if !self.selection_is_kept() { toast; "
              "return; }` block deleted (with its comment line) -> a file not kept goes to the app",
         files=[(OWM,
                 "        // Only the copy kept on this PC goes to another app: nothing is fetched or decrypted for it.\n"
                 "        if !self.selection_is_kept() {\n"
                 "            self.show_toast(Toast::KeepFirst, tx);\n"
                 "            return;\n"
                 "        }\n",
                 "")],
         filt=MENU_T + "an_app_press_on_a_file_not_kept_says_keep_first_and_opens_nothing",
         expect=["an_app_press_on_a_file_not_kept_says_keep_first_and_opens_nothing"]),
    dict(id="OW-4", crate="native", where="linux",
         note="OW #4 (open_with_menu.rs:240, now :318): `.nth(place)` -> `.nth(0)` (always the first app)",
         files=[(OWM,
                 "            let Some(app) = self.actions.source.apps_for(&item).into_iter().nth(place) else {\n",
                 "            let Some(app) = self.actions.source.apps_for(&item).into_iter().nth(0) else {\n")],
         filt=MENU_T + "an_app_press_opens_the_kept_copy_in_that_app",
         expect=["an_app_press_opens_the_kept_copy_in_that_app"]),
    dict(id="OW-5", crate="native", where="linux",
         note="OW #5 (open_with_menu.rs:220, now :298-302): the `if id == BEEBEEB_ID { open_selection; return; }` block "
              "replaced by `self.actions.source.open_in_app(&item, &AppChoice { id: \"x\", label: \"x\" });` (as the note; "
              "an unused-Result warning only)",
         files=[(OWM,
                 "        if id == BEEBEEB_ID {\n"
                 "            // The viewer: what Open (Beebeeb) does.\n"
                 "            self.open_selection(tx);\n"
                 "            return;\n"
                 "        }\n",
                 "        self.actions.source.open_in_app(&item, &AppChoice { id: \"x\".into(), label: \"x\".into() });\n")],
         filt=MENU_T + "beebeeb_in_the_submenu_opens_the_viewer",
         expect=["beebeeb_in_the_submenu_opens_the_viewer"]),
    dict(id="OW-6", crate="native", where="linux",
         note="OW #6 (open_with_apps.rs:43, choices()): `!is_own(name, own_exe) && ` removed -> Beebeeb's own exe listed",
         files=[(APPS,
                 "        .filter(|(name, label)| !is_own(name, own_exe) && !label.trim().is_empty())\n",
                 "        .filter(|(name, label)| !label.trim().is_empty())\n")],
         filt="engine_host::open_with_apps::tests::apps_for_leaves_out_beebeeb_and_caps_the_list",
         expect=["apps_for_leaves_out_beebeeb_and_caps_the_list"]),
    dict(id="OW-7", crate="native", where="linux",
         note="OW #7 (engine_host.rs:244, now open_path's linux arm :246, confirmed in the merged tree; the reveal arm "
              "`.arg(parent)` :313 is not touched): `launch::spawn(Command::new(\"xdg-open\").arg(path))` -> "
              "`Command::new(\"xdg-open\").arg(path).spawn()` -> a bare start in engine_host.rs (held count 7 -> 6); "
              "the scan reads the source text, so the cfg(linux) arm is seen on every OS",
         files=[(EH,
                 "        launch::spawn(std::process::Command::new(\"xdg-open\").arg(path))\n",
                 "        std::process::Command::new(\"xdg-open\").arg(path).spawn()\n")],
         filt="engine_host::launch_tests::every_launch_takes_the_launch_lock",
         expect=["every_launch_takes_the_launch_lock"]),
    dict(id="OW-8", crate="native", where="linux",
         note="OW #8 (launch.rs:33, spawn_by): `let _held = hold(); start(cmd)` -> `let out = start(cmd); let _held = "
              "hold(); out` (the lock taken after the start)",
         files=[(LAUNCH,
                 "    let _held = hold();\n    start(cmd)\n",
                 "    let out = start(cmd);\n    let _held = hold();\n    out\n")],
         filt="engine_host::launch_tests::launch_spawn_holds_the_lock_while_spawning",
         expect=["launch_spawn_holds_the_lock_while_spawning"]),
    dict(id="OW-9", crate="native", where="linux",
         note="OW #9 (open_with_menu.rs:127, now add_to :108): \"Open (Beebeeb)\" -> \"Open\". The offered test asserts "
              "the literal (labels[..3] == [\"Open (Beebeeb)\", ..]), so it is expected too",
         files=[(OWM,
                 "    *open = Entry::keyed(\"open\", open_label(s.identity.product_name()), \"Enter\");\n",
                 "    *open = Entry::keyed(\"open\", \"Open\", \"Enter\");\n")],
         filt=MENU_T + "dw_1562_scene_draws_the_boards_menu " + MENU_T + "open_with_offered_only_for_a_shown_office_file",
         expect=["dw_1562_scene_draws_the_boards_menu", "open_with_offered_only_for_a_shown_office_file"]),
    dict(id="OW-10", crate="native", where="win",
         note="OW #10 (VM 401; open_with_apps.rs:255, win::invoke): `handler.Invoke(&data)` with an EMPTY data object. "
              "Written: `let empty: IDataObject = windows::Win32::UI::Shell::SHCreateDataObject(None, None, "
              "None::<&IDataObject>)?` (features Win32_System_Com + Win32_UI_Shell_Common are on) and "
              "`Invoke(&empty)`; `data` is left unused (a warning). Inside `mod win` (cfg(windows)): NOT compile-checked "
              "on Linux. The test is cfg(windows) + #[ignore = \"O-CP3: VM 401\"]",
         files=[(APPS,
                 "            launch::locked(|| handler.Invoke(&data)).map_err(|_| ActionError::Failed)\n",
                 "            let empty: IDataObject =\n"
                 "                windows::Win32::UI::Shell::SHCreateDataObject(None, None, None::<&IDataObject>)\n"
                 "                    .map_err(|_| ActionError::Failed)?;\n"
                 "            launch::locked(|| handler.Invoke(&empty)).map_err(|_| ActionError::Failed)\n")],
         filt="engine_host::open_with_apps::tests::invoke_opens_the_kept_path",
         cmd="cargo test --locked --bin beebeeb-drive -- --ignored engine_host::open_with_apps::tests::invoke_opens_the_kept_path",
         expect=["invoke_opens_the_kept_path"]),
    dict(id="OW-11", crate="native", where="linux",
         note="OW #11 (engine_host.rs:727, now kept_here :780-784): kept_here's body -> `true` (turn/id unused: warnings)",
         files=[(EH,
                 "        self.engine.edit_facts_for(turn, id, 0).is_ok_and(|facts| facts.kept)\n"
                 "            && self\n"
                 "                .engine\n"
                 "                .open_file_facts(id)\n"
                 "                .is_ok_and(|facts| facts.session == turn && facts.on_this_pc.is_some())\n",
                 "        true\n")],
         filt="engine_host::files_open_with_tests::open_with_an_app_needs_the_copy_kept_on_this_pc",
         expect=["open_with_an_app_needs_the_copy_kept_on_this_pc"]),
    dict(id="OW-12", crate="native", where="linux",
         note="OW #12 (open_with_menu.rs:112, now offered_for :93): `!only.is_protected && !s.protects_selection() && ` "
              "removed -> offered for a protected (Backups) file",
         files=[(OWM,
                 "        [only] if !only.is_folder && !only.is_protected && !s.protects_selection() && viewer_shows(s, &only.name) => {\n",
                 "        [only] if !only.is_folder && viewer_shows(s, &only.name) => {\n")],
         filt=MENU_T + "open_with_offered_only_for_a_shown_office_file",
         expect=["open_with_offered_only_for_a_shown_office_file"]),
    dict(id="OW-13", crate="native", where="linux",
         note="OW DELTA #13 (launch.rs:21, hold()): a private Mutex of this crate in place of "
              "beebeeb_office_sandbox::launch_lock() (the lane RAN it: red, restored). Written as the lane's own "
              "pre-H1-S body (fedbdbf5): `fn hold() -> impl Sized { static LAUNCH: Mutex<()> ..; LAUNCH.lock()"
              ".unwrap_or_else(PoisonError::into_inner) }` (the return type had to change from LaunchGuard)",
         files=[(LAUNCH,
                 "fn hold() -> beebeeb_office_sandbox::LaunchGuard {\n    beebeeb_office_sandbox::launch_lock()\n}\n",
                 "fn hold() -> impl Sized {\n"
                 "    static LAUNCH: std::sync::Mutex<()> = std::sync::Mutex::new(());\n"
                 "    LAUNCH.lock().unwrap_or_else(std::sync::PoisonError::into_inner)\n"
                 "}\n")],
         filt="engine_host::launch_tests::launch_lock_is_the_sandboxs_own",
         expect=["launch_lock_is_the_sandboxs_own"]),
    dict(id="OW-14", crate="native", where="linux",
         note="OW DELTA #14 (engine_host.rs:756, start_later): the starts run on the calling thread "
              "(`paths.iter().try_for_each(|path| start(path))` + the same failure report) in place of "
              "`self.later.run(..)`; returns Ok(()). Written as the whole `self.later.run(move || {..}).map_err(..)` "
              "expression replaced by its body (so it compiles). No hang: the test's chooser thread lets the lock go "
              "after 3 s, so Open waits ~3 s and `took < 500 ms` goes red",
         files=[(EH,
                 "        self.later\n"
                 "            .run(move || {\n"
                 "                if let Err(why) = paths.iter().try_for_each(|path| start(path))\n"
                 "                    && engine.session_turn() == turn\n"
                 "                {\n"
                 "                    failed\n"
                 "                        .lock()\n"
                 "                        .unwrap_or_else(PoisonError::into_inner)\n"
                 "                        .push(Change::Failed(action, why));\n"
                 "                    engine.wake_host();\n"
                 "                }\n"
                 "            })\n"
                 "            .map_err(|()| ActionError::Failed)\n",
                 "        if let Err(why) = paths.iter().try_for_each(|path| start(path))\n"
                 "            && engine.session_turn() == turn\n"
                 "        {\n"
                 "            failed\n"
                 "                .lock()\n"
                 "                .unwrap_or_else(PoisonError::into_inner)\n"
                 "                .push(Change::Failed(action, why));\n"
                 "            engine.wake_host();\n"
                 "        }\n"
                 "        Ok(())\n")],
         filt="engine_host::files_open_with_tests::files_open_never_waits_on_the_launch_lock",
         expect=["files_open_never_waits_on_the_launch_lock"]),
    dict(id="OW-15", crate="native", where="linux",
         note="OW DELTA #15 (open_with_menu.rs:223, key()): `goes_in` without `Key::Right` (Tab alone goes in)",
         files=[(OWM,
                 "        let goes_in = ev.key == Key::Right || (ev.key == Key::Tab && !ev.mods.shift);\n",
                 "        let goes_in = ev.key == Key::Tab && !ev.mods.shift;\n")],
         filt=MENU_T + "open_with_right_arrow_opens_the_submenu_on_its_first_line",
         expect=["open_with_right_arrow_opens_the_submenu_on_its_first_line"]),
    dict(id="OW-16", crate="native", where="linux",
         note="OW DELTA #16 (open_with_menu.rs:232, key()): the Shift+Tab-on-the-first-line-back arm removed "
              "(`first` then unused: a warning; the match keeps its `_` arm)",
         files=[(OWM,
                 "        Key::Tab if ev.mods.shift && Some(here) == first => return Some(s.leave_open_with(tx)),\n",
                 "")],
         filt=MENU_T + "tab_reaches_the_submenus_lines_inside_the_menu",
         expect=["tab_reaches_the_submenus_lines_inside_the_menu"]),
    dict(id="OW-17", crate="native", where="linux",
         note="OW DELTA #17 (open_with_menu.rs:80, viewer_shows): `kinds::office_route(name)` in place of the "
              "`board_office` match (the scene's switches ignored; shipped constants are off)",
         files=[(OWM,
                 "    let route = match s.actions.open_with.board_office {\n"
                 "        Some(ships) => kinds::office_route_with(name, true, ships),\n"
                 "        None => kinds::office_route(name),\n"
                 "    };\n",
                 "    let route = kinds::office_route(name);\n")],
         filt=MENU_T + "dw_1562_scene_draws_the_boards_menu",
         expect=["dw_1562_scene_draws_the_boards_menu"]),
]

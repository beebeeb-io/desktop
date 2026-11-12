//! The Files window with what the Files frames add: several selected rows, the menus, the toasts,
//! the selected item's own details.

use super::{MenuKind, Toast, details, dialogs, list, menus, rename};
use super::{sidebar, toolbar};
use crate::State;
use crate::theme::tokens::{self, Theme};
use crate::theme::{icons, icons_files};
use crate::ui::{button, caption, empty, file_list, kit, menu, status_line, toast};
use rune::{Cx, El, Tx, col, el, row};

/// "4 on this PC · 3 available offline · 4 online only", counted from the rows.
pub fn pc_summary(s: &State) -> String {
    if s.listing.items.is_empty() {
        // The whole account is empty (DW-0114), or this folder is (DW-1106).
        return if s.listing.folder.id.is_none() {
            "Nothing on this PC yet".to_string()
        } else {
            "Nothing in this folder yet".to_string()
        };
    }
    let count = |status: &str| {
        s.listing
            .items
            .iter()
            .filter(|i| i.pc_status.as_deref() == Some(status))
            .count()
    };
    format!(
        "{} on this PC \u{00B7} {} available offline \u{00B7} {} online only",
        count("on_this_pc"),
        count("available_offline"),
        count("online_only")
    )
}

fn pick(s: &mut State, index: usize, tx: &mut Tx) {
    s.pick_menu(index, tx);
}

fn hover(s: &mut State, index: usize) {
    if let Some(open) = s.actions.menu.as_mut() {
        open.hot = Some(index);
    }
}

fn close(s: &mut State, tx: &mut Tx) {
    s.close_menu_tx(tx);
}

fn undo(s: &mut State, tx: &mut Tx) {
    s.undo(tx);
}

fn retry(s: &mut State, tx: &mut Tx) {
    s.retry(tx);
}

/// The open menu, where the board puts it: the file menu at its point, Sort and New under the toolbar.
fn menu_layer(s: &State, t: &Theme) -> Option<El> {
    let open = s.actions.menu?;
    let layer = |right: f32, surface: El| {
        row()
            .key("menu-layer")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .pt(112.0)
            .pr(right)
            .justify_end()
            .items_start()
            .child(surface)
    };
    let entries = s.menu_entries(open.kind);
    let (name, width) = match open.kind {
        MenuKind::File => ("File", menus::FILE_MENU_W),
        MenuKind::Sort => ("Sort", menus::SORT_MENU_W),
        MenuKind::New => ("New", menus::NEW_MENU_W),
    };
    // The menu is modal, so what is outside it hears no key: its keys are heard on the menu itself.
    let surface =
        menu::menu("menu", name, width, &entries, open.hot, t, pick, hover, close).on_key(super::keys::on_key);
    Some(match open.kind {
        MenuKind::File => surface.absolute(open.at.0, open.at.1),
        // `.wn-menu` of Sort: under its button, 226 pt from the window's right edge on the board,
        // and 36 more with the Open in File Explorer button beside it; of New: 42 pt.
        MenuKind::Sort => layer(226.0 + 36.0, surface),
        MenuKind::New => layer(42.0, surface),
    })
}

fn toast_of(kind: Toast, t: &Theme) -> El {
    match kind {
        Toast::Trashed => toast::toast(
            &icons::TRASH,
            toast::Tone::Neutral,
            "Moved to Trash",
            Some(toast::ToastAction {
                text: "Undo Ctrl+Z",
                name: "Undo",
                shortcut: Some("Ctrl+Z"),
                run: undo,
            }),
            t,
        ),
        Toast::Duplicated => toast::toast(&icons_files::CHECK, toast::Tone::Done, "Duplicated", None, t),
        Toast::Keeping => toast::toast(
            &icons::AVAILABLE_OFFLINE,
            toast::Tone::Neutral,
            "Keeping on this PC",
            None,
            t,
        ),
        Toast::LinkCopied => toast::toast(&icons_files::CHECK, toast::Tone::Done, "Link copied", None, t),
        Toast::FreedUp => toast::toast(
            &icons_files::CHECK,
            toast::Tone::Done,
            "Freed up space on this PC",
            None,
            t,
        ),
        Toast::Failed(action, why) => toast::toast(
            &icons_files::ALERT,
            toast::Tone::Failed,
            &Toast::failure(action, why),
            Some(toast::ToastAction {
                text: "Try again",
                name: "Try again",
                shortcut: None,
                run: retry,
            }),
            t,
        ),
    }
}

/// An empty folder (`.kit-empty`): what to do, and that it is encrypted before it uploads.
fn empty_folder(root: bool, t: &Theme) -> El {
    let body = empty::empty_state(
        "empty",
        &icons::FILES,
        "Nothing here yet",
        // The board's two sentences: a new account's Files (DW-0114), an empty folder (DW-1106).
        if root {
            "Add files or photos. They're encrypted on this PC before they upload."
        } else {
            "Add files or photos, and they're encrypted on this PC before they upload."
        },
        t,
    )
    .child(
        col()
            .key("empty-actions")
            .mt(8.0)
            .gap(8.0)
            .items_center()
            .child(
                button::outline_button("empty-add", "Add files", Some(&icons::PLUS), t)
                    .expanded(false)
                    .on_press(|s: &mut State| s.toggle_menu(MenuKind::New)),
            )
            .child(kit::label(
                "empty-hint",
                "or drop files here from File Explorer",
                empty::CAP,
                t.text_tertiary,
            )),
    );
    el().key("empty-area")
        .role("list")
        .label("Nothing here yet")
        .focusable(true)
        .no_focus_ring()
        .grow(1.0)
        .h_full()
        .center()
        .on_drop_files(|s: &mut State, files: &[std::path::PathBuf], tx: &mut Tx| {
            let folder = s.listing.folder.id.clone();
            s.upload(folder, files, tx);
        })
        .child(body)
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());

    // "Nothing here yet · Add files" is for a folder that takes files. One that takes none
    // (Backups, a service folder) shows its headers over no rows.
    let empty = s.listing.items.is_empty() && s.can_add();
    let mut body = row().key("list-body").grow(1.0).min_h(0.0).w_full();
    if empty {
        // Nothing in the folder: no headers, no panel, one way forward (DW-1106).
        body = body.child(empty_folder(s.listing.folder.id.is_none(), &t));
    } else {
        // The folder as a grid, where the person chose that view (`frames::global::grid`, DW-0119).
        body = body.child(crate::frames::global::grid_body(s, &t).unwrap_or_else(|| {
            col()
                .key("file-list-col")
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .child(list::column_headers(s.actions.sort, &t))
                // A folder that could not be read says so. It never draws as an empty folder.
                .child(match s.list_error {
                    // It holds the keyboard while it shows, so Back still works from the keys.
                    Some(error) => file_list::list_error(error, &t).focusable(true).no_focus_ring(),
                    None => list::rows(s, cx, &t),
                })
        }));
        if s.show_detail_pane {
            let selected = s.selected.and_then(|i| s.listing.items.get(i));
            let facts = selected.map(|item| (item, s.actions.source.details(item, &s.listing.folder.name)));
            body = body.child(details::details_pane(facts, &t));
        }
    }

    let mut main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(toolbar::toolbar(s, &t, 0.0))
        // The one banner of the whole window, or the loading strip (`frames::global`).
        .children(crate::frames::global::under_toolbar(s, &t))
        .child(body)
        .child({
            // The sync state is the source's; a click shows the b's flyout (DW-0401 to DW-1201).
            let state = s.actions.source.sync_state();
            let details = crate::frames::global::status_words(s).unwrap_or_else(|| pc_summary(s));
            let name = match &state {
                Some(state) => format!("{state}. {details}"),
                None => details.clone(),
            };
            // Without a connection the line says so (`frames::global`, DW-0121).
            // While something uploads the line says so, and opens Uploads (DW-0426, DW-0427).
            let uploading = super::uploads::status(s, &details, &t);
            let name = match &uploading {
                Some((_, words)) => format!("{words}. {details}"),
                None => name,
            };
            crate::frames::global::status_line(state.as_deref(), &details, &t)
                .or(uploading.map(|(line, _)| line))
                .unwrap_or_else(|| status_line::status_line(state.as_deref(), &details, &t))
                .role("button")
                .label(name)
                .description("Shows sync status")
                .focusable(true)
                .on_press(|s: &mut State| s.status_pressed())
        });
    if let Some(kind) = s.live_toast() {
        main = main.child(toast::layer(toast_of(kind, &t)));
    }

    let crop = s.actions.board_crop_top;
    let mut body = row()
        .key("body")
        .w_full()
        .child(sidebar::sidebar(s, &t))
        .child(main)
        // What is in front of the whole body: a preview that needs a connection (`frames::global`).
        .children(crate::frames::global::over_body(s, s.actions.window, &t));
    body = if crop > 0.0 {
        // A board frame that draws the lower part of the window only (`.mk-cropfade.at-top`): the
        // body keeps its full height and moves up under the caption, its top fading into it.
        let fade = t.sidebar_bg;
        el().key("body-crop")
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .clip_rounded(0.0)
            .child(body.absolute(0.0, -crop).h(768.0))
            .child(
                el().key("crop-fade")
                    .absolute(0.0, 0.0)
                    .w_full()
                    .h(44.0)
                    .gradient(fade, fade.alpha(0.0)),
            )
    } else {
        crate::frames::global::fit(s, body, &t)
    };
    let window = col()
        .key("window")
        .role("window")
        .label(s.identity.main_window_title())
        .on_key(super::keys::on_key)
        .on_layout(|s: &mut State, [_, _, w, h]| s.actions.window = (w, h))
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption::caption_bar(s.identity.main_window_title(), &t, cx))
        .child(body);
    // What lies over the window: the open menu, the reason a name cannot be given, a dialog.
    let layers: Vec<El> = [
        menu_layer(s, &t),
        rename::tip_layer(s, &t),
        super::uploads::drop_layer(s, &t),
        super::uploads::layer(s, &t),
        dialogs::layer(s, &t),
    ]
    .into_iter()
    .flatten()
    .collect();
    if layers.is_empty() {
        return window;
    }
    let mut root = el().key("files-root").w_full().h_full().child(window);
    for layer in layers {
        root = root.child(layer);
    }
    root
}

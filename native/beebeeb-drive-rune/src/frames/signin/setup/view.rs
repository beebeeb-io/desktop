//! The first run's steps, drawn from [`Ui`] with the setup kit (`ui::setup`) and the account
//! kit (`ui::au`). Every string is the board's; every measurement is its computed CSS.

use rune::tok::Family;
use rune::{Color, Cx, El, Span, TextStyle, col, el, row};

use super::source::{Explorer, LockAfter, PreviewFormat, PreviewSize, Switch};
use super::{Answered, Compare, CompareMode, PHOTO, SHEET, Setting, Step, Ui, act, icons};
use crate::State;
use crate::frames::signin::icons as signin_icons;
use crate::theme::tokens::{self, Theme, type_};
use crate::ui::au::{self, ButtonKind, ButtonSize, ButtonState, Keys, Tone, type_au};
use crate::ui::setup::{self, SegInk, SegSize, Status, StepState};
use crate::ui::{kit, settings_list};

const PRODUCT: &str = "Drive";

/// `.t-sub` at 500 and its mono twin: the sample's size in the preview card.
const SUB_MEDIUM: TextStyle = TextStyle::new(12.0, 500.0).lh(15.0);
const SUB_MONO: TextStyle = TextStyle::new(12.0, 400.0).family(Family::Mono).lh(15.0);
const SUB_MONO_MEDIUM: TextStyle = TextStyle::new(12.0, 500.0).family(Family::Mono).lh(15.0);
/// `.wn-fdlg-ni`: File Explorer's own 13px.
const NAV: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
/// `.t-cap.font-mono`: a weight under a size in the compare view.
const CAP_MONO: TextStyle = TextStyle::new(11.0, 500.0).family(Family::Mono).lh(14.0);

pub fn view(ui: &Ui, name: &str, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());
    // On Windows, Enter presses the focused button or link (and Space does too).
    cx.web_focus();
    let mut k = Keys::new();
    let column = match (ui.step, ui.compare.is_some()) {
        // The board draws step 4 behind the compare view as its heading and its two buttons.
        (Step::Previews, true) => previews_behind(&mut k, &t),
        (Step::Passkey, _) => passkey(&mut k, &t),
        (Step::Lock, _) => lock(ui, &mut k, &t),
        (Step::Backup, _) => backup(ui, &mut k, &t),
        (Step::Previews, _) => previews(ui, &mut k, &t),
        (Step::Explorer, _) => match &ui.explorer {
            Explorer::Added => explorer_added(&mut k, &t),
            Explorer::Adding => explorer_adding(&mut k, &t),
            Explorer::Failed { code } => explorer_failed(WINDOWS_REFUSED, code, &mut k, &t),
            Explorer::NotSyncing => explorer_failed(SYNC_DID_NOT_START, "", &mut k, &t),
        },
        (Step::Notifications, _) => notifications(&mut k, &t),
        (Step::Login, _) => login(ui, &mut k, &t),
        (Step::Done, _) => done(ui, &mut k, &t),
    };
    let steps: Vec<(&str, StepState)> = ui
        .steps()
        .iter()
        .map(|step| {
            let state = match step.cmp(&ui.step) {
                std::cmp::Ordering::Less => StepState::Done,
                std::cmp::Ordering::Equal => StepState::Now,
                std::cmp::Ordering::Greater => StepState::Ahead,
            };
            (step.label(), state)
        })
        .collect();
    let adding_words = format!("Adding {name} to File Explorer\u{2026}");
    let added_words = format!("{name} is in File Explorer");
    let failed_words = format!("Couldn't add {name} to File Explorer");
    let status = match &ui.explorer {
        Explorer::Adding => Status {
            icon: &icons::ADDING,
            words: &adding_words,
            failed: false,
        },
        Explorer::Added => Status {
            icon: &icons::ADDED,
            words: &added_words,
            failed: false,
        },
        Explorer::Failed { .. } | Explorer::NotSyncing => Status {
            icon: &icons::ALERT,
            words: &failed_words,
            failed: true,
        },
    };
    // While the compare view is up, what is behind it takes no press and no focus.
    let behind = ui.compare.is_some();
    let skip = (ui.step != Step::Done).then(|| {
        let link = setup::skip_link(&t);
        if behind {
            link.disabled(true)
        } else {
            link.on_press_tx(act(Ui::skip))
        }
    });
    let over = ui.compare.map(|compare| compare_sheet(ui, compare, &t));
    setup::page(
        name,
        PRODUCT,
        &steps,
        &signin_icons::CHECK,
        &status,
        skip,
        column,
        over,
        &t,
        cx,
    )
    .on_key(super::on_key)
}

/// `.au-actions`: the buttons at the foot of a column, 6 apart, 6 below the column's gap.
fn actions() -> El {
    col().key("au-actions").w_full().gap(6.0).mt(6.0)
}

fn primary(k: &mut Keys, label: &str, state: ButtonState, t: &Theme, to: fn(&mut Ui)) -> El {
    let b = au::button(k, label, None, ButtonKind::Primary, ButtonSize::Lg, state, t);
    if state == ButtonState::Enabled {
        b.on_press_tx(act(to)).default_button()
    } else {
        b
    }
}

fn ghost(k: &mut Keys, label: &str, state: ButtonState, t: &Theme, to: fn(&mut Ui)) -> El {
    let b = au::button(k, label, None, ButtonKind::Ghost, ButtonSize::Lg, state, t);
    if state == ButtonState::Enabled {
        b.on_press_tx(act(to))
    } else {
        b
    }
}

fn switch(row_key: &str, name: &str, on: bool, t: &Theme, to: impl Fn(&mut Ui) + 'static) -> El {
    settings_list::switch(row_key, name, on, t)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(act(to))
}

// ---------------------------------------------------------------------------
// DW-0201: a passkey
// ---------------------------------------------------------------------------

fn passkey(k: &mut Keys, t: &Theme) -> El {
    let rows = vec![
        setup::item(
            k,
            Some(&icons::FACE),
            "Saved by Windows",
            Some("In Passwords or your password manager, behind Windows Hello"),
            |_| Vec::new(),
            None,
            false,
            t,
        ),
        setup::item(
            k,
            Some(&icons::PADLOCK),
            "Private",
            Some("Beebeeb can't use it to open your files by itself."),
            |_| Vec::new(),
            None,
            true,
            t,
        ),
    ];
    setup::column()
        .child(au::heading(
            "Add a passkey",
            Some("Sign in and unlock your files on a new device, with no codes. It doesn't replace your 12 words, so keep those safe."),
            false,
            t,
        ))
        .child(setup::list(k, rows, None, t))
        .child(
            actions()
                .child(primary(k, "Add a passkey", ButtonState::Enabled, t, Ui::add_passkey))
                .child(ghost(k, "Not now", ButtonState::Enabled, t, Ui::not_now)),
        )
}

// ---------------------------------------------------------------------------
// DW-0202: app lock
// ---------------------------------------------------------------------------

fn lock(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let on = ui.lock_on;
    let chosen = LockAfter::ALL.iter().position(|a| *a == ui.lock_after).unwrap_or(1);
    let labels: Vec<&str> = LockAfter::ALL.iter().map(|a| a.label()).collect();
    let rows = vec![
        setup::item(
            k,
            Some(&icons::FACE),
            "App lock",
            Some("Your PC's password works if Windows Hello doesn't"),
            |key| vec![switch(key, "App lock", on, t, Ui::toggle_lock)],
            None,
            false,
            t,
        ),
        setup::item(
            k,
            None,
            "Lock again after",
            None,
            |_| {
                vec![setup::segmented(
                    "seg-0",
                    "Lock again after",
                    &labels,
                    chosen,
                    SegSize::Sm,
                    None,
                    t,
                    |j, tab| tab.on_press_tx(act(move |ui: &mut Ui| ui.set_lock_after(LockAfter::ALL[j]))),
                )]
            },
            None,
            true,
            t,
        ),
    ];
    setup::column()
        .child(au::heading(
            "Lock Beebeeb",
            Some("Ask for Windows Hello when you open the app, so nobody else can look through your files on this PC."),
            false,
            t,
        ))
        .child(setup::list(
            k,
            rows,
            Some("Sync keeps running while the app is locked. File Explorer follows your PC's lock instead."),
            t,
        ))
        .child(
            actions()
                .child(primary(k, "Continue", ButtonState::Enabled, t, Ui::go_on))
                .child(ghost(k, "Not now", ButtonState::Enabled, t, Ui::not_now)),
        )
}

// ---------------------------------------------------------------------------
// DW-0203: back up folders
// ---------------------------------------------------------------------------

fn backup(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let rows: Vec<El> = ui
        .folders
        .iter()
        .enumerate()
        .map(|(i, folder)| {
            setup::item(
                k,
                Some(&icons::FOLDER),
                &folder.name,
                Some(&folder.summary),
                |key| {
                    vec![switch(key, &folder.name, folder.on, t, move |ui: &mut Ui| {
                        ui.toggle_folder(i)
                    })]
                },
                None,
                i > 0,
                t,
            )
        })
        .collect();
    let any = ui.folders.iter().any(|f| f.on);
    setup::column()
        .child(au::heading(
            "Back up your folders",
            Some("Your folders are encrypted on this PC before they upload. They stay where they are."),
            false,
            t,
        ))
        .child(setup::list(
            k,
            rows,
            Some("Change these any time in Settings \u{203a} Backups, and add other folders there."),
            t,
        ))
        .child(
            actions()
                .child(primary(
                    k,
                    "Turn on backup",
                    if any {
                        ButtonState::Enabled
                    } else {
                        ButtonState::Disabled
                    },
                    t,
                    Ui::go_on,
                ))
                .child(ghost(k, "Not now", ButtonState::Enabled, t, Ui::not_now)),
        )
}

// ---------------------------------------------------------------------------
// DW-0204: thumbnails and previews
// ---------------------------------------------------------------------------

const PREVIEWS_SUB: &str = "Photos open instantly from a saved preview, instead of downloading the full original.";

fn preview_card(ui: &Ui, t: &Theme) -> El {
    let (original, _) = ui.pictures();
    let name = ui.preview_name();
    let mut words = row().key("prev-words").grow(1.0).min_w(0.0).h(15.0).items_end();
    words = words.child(kit::label("prev-size", format!("{name} \u{b7}"), SUB_MEDIUM, t.text));
    if let Some(weight) = ui.weight(ui.size, ui.format) {
        // A space of 12px Inter between the dot and the weight, as the board's line has.
        words = words.child(kit::label("prev-size-bytes", weight, SUB_MONO_MEDIUM, t.text).ml(3.33));
    }
    words = words
        .child(kit::label("prev-orig", ", original", type_au::SUB, t.text_tertiary))
        .child(kit::label("prev-orig-bytes", ui.original_weight(), SUB_MONO, t.text_tertiary).ml(3.33));
    row()
        .key("prev-card")
        .role("group")
        .label(format!(
            "Sample: {name}{}, original {}",
            ui.weight(ui.size, ui.format)
                .map(|w| format!(" {w}"))
                .unwrap_or_default(),
            ui.original_weight()
        ))
        .w_full()
        .padding(12.0)
        .gap(12.0)
        .center_y()
        .radius(10.0)
        .fill(t.card)
        .box_shadow(kit::drop(1.0, 2.0, Color::BLACK.alpha(0.04)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            el().key("prev-thumb")
                .role("image")
                .label(if ui.sample().bundled {
                    "Sample photo"
                } else {
                    "Your photo"
                })
                .size(70.0, 52.0)
                .radius(6.0)
                .pixels(original),
        )
        .child(words)
        .child(setup::small_button("btn-0", "Compare", t).on_press_tx(act(Ui::open_compare)))
}

fn previews(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    // "Compare" is the column's first button: `btn-0`.
    let _ = k.next("btn");
    let size = PreviewSize::ALL.iter().position(|s| *s == ui.size).unwrap_or(3);
    let format = PreviewFormat::ALL.iter().position(|f| *f == ui.format).unwrap_or(0);
    let sizes: Vec<&str> = PreviewSize::ALL.iter().map(|s| s.label()).collect();
    let formats: Vec<&str> = PreviewFormat::ALL.iter().map(|f| f.label()).collect();
    let limit = ui.space_limit();
    let label = |key: &str, words: &str| {
        kit::label(
            key.to_string(),
            words.to_uppercase(),
            type_::SECTION_LABEL,
            t.text_tertiary,
        )
        .w_full()
        .mb(-4.0)
    };
    // The limit as the product has it: the board draws the pop-up button and gives it no
    // choices here (they are Settings › Storage's, DW-0712), so it is a value, not a control.
    let space = setup::item(
        k,
        None,
        "Space on this PC",
        None,
        |key| vec![settings_list::pop_button(key, &limit, t).a11y_hidden()],
        None,
        false,
        t,
    )
    .label(format!("Space on this PC, {limit}"));
    setup::column()
        .child(au::heading("Thumbnails and previews", Some(PREVIEWS_SUB), false, t))
        .child(preview_card(ui, t))
        .child(label("label-0", "Preview size"))
        .child(setup::segmented(
            "seg-0",
            "Preview size",
            &sizes,
            size,
            SegSize::Md,
            None,
            t,
            |j, tab| tab.on_press_tx(act(move |ui: &mut Ui| ui.set_size(PreviewSize::ALL[j]))),
        ))
        .child(label("label-1", "Format"))
        .child(setup::segmented(
            "seg-1",
            "Format",
            &formats,
            format,
            SegSize::Md,
            None,
            t,
            |j, tab| tab.on_press_tx(act(move |ui: &mut Ui| ui.set_format(PreviewFormat::ALL[j]))),
        ))
        .child(setup::list(k, vec![space], Some(&ui.fit_line()), t))
        .child(
            actions()
                .child(primary(k, "Continue", ButtonState::Enabled, t, Ui::go_on))
                .child(ghost(k, "Not now", ButtonState::Enabled, t, Ui::not_now)),
        )
}

/// Step 4 as the board draws it behind the compare view: nothing in it takes a press.
fn previews_behind(k: &mut Keys, t: &Theme) -> El {
    setup::column()
        .child(au::heading("Thumbnails and previews", Some(PREVIEWS_SUB), false, t))
        .child(
            actions()
                .child(au::button(
                    k,
                    "Continue",
                    None,
                    ButtonKind::Primary,
                    ButtonSize::Lg,
                    ButtonState::Busy,
                    t,
                ))
                .child(au::button(
                    k,
                    "Not now",
                    None,
                    ButtonKind::Ghost,
                    ButtonSize::Lg,
                    ButtonState::Busy,
                    t,
                )),
        )
}

// ---------------------------------------------------------------------------
// DW-0206, DW-0206b, DW-0280: File Explorer
// ---------------------------------------------------------------------------

/// Windows' own neutral colours (`.kit-os`): the pane is a picture of File Explorer, not Beebeeb.
struct Os {
    card: Color,
    /// The selected row's ground: the ink at 9% over the card.
    selected: Color,
    ink: Color,
    icon: Color,
}

fn os(t: &Theme) -> Os {
    let grey = |v: u8| rune::color::hex(&format!("#{v:02x}{v:02x}{v:02x}")).unwrap_or(Color::BLACK);
    if t.dark {
        Os {
            card: grey(0x16),
            selected: grey(0x29),
            ink: grey(0xe4),
            icon: grey(0xb7),
        }
    } else {
        Os {
            card: grey(0xfd),
            selected: grey(0xe8),
            ink: grey(0x12),
            icon: grey(0x42),
        }
    }
}

fn explorer_added(k: &mut Keys, t: &Theme) -> El {
    let os = os(t);
    let places: [(&str, Option<&'static rune::Icon>); 5] = [
        ("Home", Some(&icons::HOME)),
        ("Gallery", Some(&icons::GALLERY)),
        ("Beebeeb", None),
        ("Desktop", Some(&icons::DESKTOP)),
        ("Documents", Some(&icons::DOCUMENT)),
    ];
    let mut nav = col()
        .key("nav")
        .role("image")
        .label("File Explorer's navigation pane: Home, Gallery, Beebeeb (selected), Desktop, Documents")
        .w(220.0)
        .padding(8.0)
        .gap(2.0)
        .radius(10.0)
        .fill(os.card)
        .box_shadow(kit::drop(1.0, 2.0, Color::BLACK.alpha(0.04)))
        .box_shadow(kit::ring(kit::HAIR, t.separator));
    for (i, (name, icon)) in places.iter().enumerate() {
        let mut ni = row().h(28.0).px(10.0).gap(9.0).center_y().radius(4.0);
        ni = match icon {
            Some(icon) => ni.child(kit::glyph(format!("nav-{i}-icon"), icon, 15.0, os.icon)),
            // The b as Windows shows it in the pane: one colour, the letter cut out of its plate.
            None => ni.key("nav-on").fill(os.ink.alpha(0.09)).child(
                el().key(format!("nav-{i}-b"))
                    .size(15.0, 15.0)
                    // The plate is a glyph, as the board's is an SVG: it keeps its soft edge on a half pixel.
                    // Its top is on half a device pixel (y 295.75 at 200 %): the board's browser draws it from
                    // the pixel above (rows 591 to 620, measured), Rune from the pixel below. A quarter point
                    // up puts both layers on the board's rows; at 100 % and 150 % nothing changes.
                    .mt(-0.25)
                    .mb(0.25)
                    .child(rune::icon(&icons::B_PLATE, 15.0).color(os.icon).absolute(0.0, 0.0))
                    // The mark's b, moved to the middle of the plate as the board's symbol has it
                    // (its mask sits 34.018, 22.026 of 512 further right and down than the mark's).
                    .child(
                        rune::icon(&crate::theme::icons::MARK_B, 15.0)
                            .color(os.selected)
                            .absolute(15.0 * 34.018 / 512.0, 15.0 * 22.026 / 512.0),
                    ),
            ),
        };
        nav = nav.child(ni.child(kit::label(format!("nav-{i}"), *name, NAV, os.ink).a11y_hidden()));
        if i == 2 {
            nav = nav.child(el().key("nav-sep").h(kit::HAIR).mx(8.0).my(5.0).fill(t.separator));
        }
    }
    setup::column()
        .child(au::heading(
            "Beebeeb is in File Explorer",
            Some("Your Beebeeb files show in File Explorer's navigation pane, and in other apps' Open and Save As dialogs."),
            false,
            t,
        ))
        .child(nav)
        .child(setup::note(
            k,
            "Anyone who can use this PC while it's signed in can open these files in File Explorer. File Explorer follows Windows' lock, not Beebeeb's.",
            type_::CAPTION,
            0.0,
            t,
        ))
        .child(
            actions()
                .child(primary(k, "Continue", ButtonState::Enabled, t, Ui::go_on))
                .child(ghost(k, "Show in File Explorer", ButtonState::Enabled, t, Ui::show_in_explorer)),
        )
}

fn explorer_adding(k: &mut Keys, t: &Theme) -> El {
    setup::column()
        .child(au::heading(
            "Adding Beebeeb to File Explorer",
            Some("This takes a few seconds. You can carry on; it finishes by itself."),
            false,
            t,
        ))
        .child(
            // Windows gives no progress: the bar says "busy", not how far.
            el().key("bar")
                .role("progressbar")
                .label("Adding Beebeeb to File Explorer")
                .w_full()
                .h(4.0)
                .radius(2.0)
                .fill(t.text.alpha(0.09))
                .child(
                    el().key("bar-fill")
                        .absolute(0.3 * setup::COLUMN_W, 0.0)
                        .size(0.3 * setup::COLUMN_W, 4.0)
                        .radius(2.0)
                        .fill(t.text_secondary),
                ),
        )
        .child(actions().child(primary(k, "Continue", ButtonState::Enabled, t, Ui::go_on)))
}

/// DW-0280's banner title: Windows was asked and did not finish.
const WINDOWS_REFUSED: &str = "Windows didn't finish adding Beebeeb to File Explorer";
/// The banner's title when sync did not start, so Windows was never asked (`Explorer::NotSyncing`).
const SYNC_DID_NOT_START: &str = "Sync didn't start on this PC, so Beebeeb isn't in File Explorer yet";

/// `code`: Windows' code for support; empty when there is none (the sentence then stands alone).
fn explorer_failed(title: &str, code: &str, k: &mut Keys, t: &Theme) -> El {
    let (mark, _, wash) = au::tone(Tone::Red, t);
    let key = k.next("banner");
    let lead = "Try again in a moment.";
    let words = if code.is_empty() {
        lead.to_string()
    } else {
        format!("{lead} {code}")
    };
    let spans = if code.is_empty() {
        Vec::new()
    } else {
        vec![Span {
            mono: true,
            size: Some(11.0),
            ..Span::new(lead.len() + 1, words.len())
        }]
    };
    let code_style = TextStyle::new(11.0, 400.0).family(Family::Mono).lh(17.0);
    kit::note_text(
        &format!("{key}-code"),
        &format!("{key}-code-box"),
        code,
        code_style,
        t.text_secondary,
    );
    let banner = row()
        .key(key.clone())
        .role("alert")
        .live_region(rune::LiveRegion::Assertive)
        .w_full()
        .px(13.0)
        .py(11.0)
        .gap(10.0)
        .items_start()
        .radius(13.0)
        .fill(wash)
        .child(kit::glyph(format!("{key}-icon"), &icons::EXPLORER, 16.0, mark).mt(1.0))
        .child(
            col()
                .gap(2.0)
                .grow(1.0)
                .min_w(0.0)
                .child(
                    kit::label(format!("{key}-title"), title, type_au::BANNER_TITLE, t.text)
                        .wrap()
                        .w_full(),
                )
                .child(
                    el().w_full()
                        .child(
                            kit::label(
                                format!("{key}-text"),
                                words.clone(),
                                type_au::BANNER_TEXT,
                                t.text_secondary,
                            )
                            .spans(spans)
                            .w_full(),
                        )
                        // The error's own box, for the gate: where the mono run stands in the line.
                        .children((!code.is_empty()).then(|| {
                            el().key(format!("{key}-code-box"))
                                .a11y_hidden()
                                .absolute(rune::text::advance(&format!("{lead} "), type_au::BANNER_TEXT), 1.25)
                                .w(rune::text::advance(code, code_style))
                                .h(14.5)
                        })),
                ),
        );
    setup::column()
        .child(au::heading(
            "Couldn't add Beebeeb to File Explorer",
            Some("Your files are safe, and the Beebeeb window works without File Explorer."),
            false,
            t,
        ))
        .child(banner)
        .child(
            actions()
                .child(primary(k, "Try again", ButtonState::Enabled, t, Ui::go_on))
                .child(ghost(
                    k,
                    "Continue without File Explorer",
                    ButtonState::Enabled,
                    t,
                    Ui::not_now,
                )),
        )
}

// ---------------------------------------------------------------------------
// DW-0207, DW-0281, DW-0208
// ---------------------------------------------------------------------------

fn notifications(k: &mut Keys, t: &Theme) -> El {
    let (mark, _, wash) = au::tone(Tone::Neutral, t);
    setup::column()
        .child(au::heading(
            "Stay informed",
            Some("Get told when sync stops, or when someone signs in to your account. No marketing, ever."),
            false,
            t,
        ))
        .child(
            el().key("tile-at").a11y_hidden().w_full().py(16.0).center().child(
                el().key("tile")
                    .size(64.0, 64.0)
                    .radius(18.0)
                    .fill(wash)
                    .center()
                    .child(kit::glyph("tile-icon", &icons::BELL, 32.0, mark)),
            ),
        )
        .child(
            actions()
                .child(primary(k, "Allow notifications", ButtonState::Enabled, t, Ui::go_on))
                .child(ghost(k, "Not now", ButtonState::Enabled, t, Ui::not_now)),
        )
}

fn login(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let on = ui.login_on;
    let rows = vec![setup::item(
        k,
        Some(&icons::POWER),
        "Open at login",
        None,
        |key| vec![switch(key, "Open at login", on, t, Ui::toggle_login)],
        None,
        false,
        t,
    )];
    setup::column()
        .child(au::heading(
            "Open Beebeeb when you log in",
            Some("So the b is in the notification area and your files sync from the start."),
            false,
            t,
        ))
        .child(setup::list(k, rows, None, t))
        .child(actions().child(primary(k, "Continue", ButtonState::Enabled, t, Ui::go_on)))
}

fn on_off(on: bool) -> &'static str {
    if on { "On" } else { "Off" }
}

fn done(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let r = &ui.results;
    let explorer = match &ui.explorer {
        Explorer::Added => "On",
        Explorer::Adding => "Adding\u{2026}",
        Explorer::Failed { .. } | Explorer::NotSyncing => "Off",
    };
    let backup = if r.backup.is_empty() {
        "Off".to_string()
    } else {
        r.backup.join(", ")
    };
    // A switch whose change is answered after the press draws no value before the answer, and never the value
    // that was asked for when the change did not happen.
    let switch = |switch: Switch, on: bool| match ui.answered(switch) {
        Some(Answered::NotAsked) => "Not set here",
        Some(Answered::Waiting) => "Saving\u{2026}",
        Some(Answered::NotKnown) => "Not changed",
        Some(Answered::NotChanged) | None => on_off(on),
    };
    let rows: [(&str, String, Setting); 7] = [
        (
            "Passkey",
            if r.passkey { "Added" } else { "Not added" }.into(),
            Setting::Passkey,
        ),
        ("App lock", on_off(r.app_lock).into(), Setting::AppLock),
        ("Backup", backup, Setting::Backup),
        ("Previews", ui.preview_name(), Setting::Previews),
        ("File Explorer", explorer.into(), Setting::FileExplorer),
        (
            "Notifications",
            switch(Switch::Notifications, r.notifications).into(),
            Setting::Notifications,
        ),
        (
            "Open at login",
            switch(Switch::OpenAtLogin, r.open_at_login).into(),
            Setting::OpenAtLogin,
        ),
    ];
    let links = ui.opens_settings();
    let rows: Vec<El> = rows
        .into_iter()
        // Only the steps this first run had: no row for a step nothing answered for.
        .filter(|(_, _, setting)| ui.has(*setting))
        .map(|(title, value, setting)| {
            let row = setup::item(
                k,
                None,
                title,
                None,
                |key| vec![setup::value(key, &value, t).a11y_hidden()],
                Some(26.0),
                false,
                t,
            )
            .label(format!("{title}, {value}"));
            if !links {
                // Nothing opens the setting from here: the row is its value, not a link.
                return row.role("group");
            }
            row.role("link")
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(act(move |ui: &mut Ui| ui.open_setting(setting)))
        })
        .collect();
    // What was asked and did not happen, said in words (the rows show what is set).
    let not_changed: Vec<&str> = [
        (Switch::Notifications, "notifications"),
        (Switch::OpenAtLogin, "open at login"),
    ]
    .into_iter()
    .filter(|(switch, _)| matches!(ui.answered(*switch), Some(Answered::NotChanged | Answered::NotKnown)))
    .map(|(_, name)| name)
    .collect();
    setup::column()
        .child(au::heading(
            "You're set up",
            Some("You can change any of this later in Settings."),
            false,
            t,
        ))
        .child(setup::list(k, rows, None, t))
        .children((!not_changed.is_empty()).then(|| {
            el().key("not-changed").role("alert").w_full().child(setup::note(
                k,
                &format!(
                    "Couldn't change {}. You can set it in Settings.",
                    not_changed.join(" and ")
                ),
                type_::CAPTION,
                0.0,
                t,
            ))
        }))
        .child(setup::note(
            k,
            "Your file list and thumbnails keep loading in the background.",
            type_::CAPTION,
            0.0,
            t,
        ))
        .child(actions().child(primary(k, "Open my Beebeeb", ButtonState::Enabled, t, Ui::finish)))
}

// ---------------------------------------------------------------------------
// DW-0209: the compare view
// ---------------------------------------------------------------------------

/// The compare view's ground: "color-mix(in oklch, black 88%, paper)".
fn stage(t: &Theme) -> Color {
    if t.dark {
        Color::BLACK
    } else {
        rune::color::hex("#060505").unwrap_or(Color::BLACK)
    }
}

fn tag(key: &str, words: &str) -> El {
    el().key(format!("{key}-box"))
        .h(18.0)
        .px(8.0)
        .center()
        .radius(4.0)
        .fill(Color::BLACK.alpha(0.5))
        .child(kit::label(key.to_string(), words, type_::CAPTION, Color::WHITE))
}

fn photo(ui: &Ui, compare: Compare, t: &Theme) -> El {
    let (original, preview) = ui.pictures();
    let (w, h) = PHOTO;
    let original_tag = format!("Original \u{b7} {}", ui.original_weight());
    let preview_tag = match ui.weight(ui.size, ui.format) {
        Some(weight) => format!("{} \u{b7} {weight}", ui.preview_name()),
        None => ui.preview_name(),
    };
    let frame = |key: &str| el().key(key.to_string()).size(w, h).radius(4.0);
    match compare.mode {
        CompareMode::Wipe => {
            let x = (compare.wipe * w).round();
            frame("cmp-photo")
                .role("slider")
                .label("Wipe between the original and the preview")
                .range_value(f64::from(compare.wipe * 100.0), 0.0, 100.0)
                .on_step(|s: &mut State, delta: i32, _: &mut rune::Tx| {
                    if let Some(ui) = s.setup.as_mut() {
                        ui.move_wipe(delta as f32 * super::WIPE_STEP);
                    }
                })
                .focusable(true)
                .focus_ring(2.0, 2.0, t.accent_deep)
                .pixels(original)
                // The preview from the line to the right edge, cut from the same picture box.
                .child(
                    el().key("cmp-preview-at")
                        .absolute(x, 0.0)
                        .size(w - x, h)
                        .clip_corners([0.0, 4.0, 4.0, 0.0])
                        .child(el().absolute(-x, 0.0).size(w, h).pixels(preview)),
                )
                .child(
                    el().key("cmp-wipe")
                        .absolute(x - 1.0, 0.0)
                        .size(2.0, h)
                        .fill(Color::WHITE),
                )
                .child(el().absolute(12.0, 12.0).child(tag("cmp-tag-0", &original_tag)))
                .child(
                    el().pin(None, Some(12.0), Some(12.0), None)
                        .child(tag("cmp-tag-1", &preview_tag)),
                )
        }
        CompareMode::Stacked => {
            // Two pictures of half the height, the original above the preview.
            let half = (h - 10.0) / 2.0;
            let one = |key: &str, pixels: &rune::Pixels, words: &str, name: &str| {
                el().key(format!("{key}-pic"))
                    .role("image")
                    .label(name.to_string())
                    .size(half * w / h, half)
                    .radius(4.0)
                    .pixels(pixels)
                    .child(el().absolute(12.0, 12.0).child(tag(key, words)))
            };
            col()
                .key("cmp-photo")
                .size(w, h)
                .gap(10.0)
                .items_center()
                .child(one("cmp-tag-0", original, &original_tag, "The original"))
                .child(one("cmp-tag-1", preview, &preview_tag, "The preview"))
        }
        CompareMode::Flip => {
            let (pixels, key, words) = if compare.original_showing {
                (original, "cmp-tag-0", original_tag.as_str())
            } else {
                (preview, "cmp-tag-1", preview_tag.as_str())
            };
            frame("cmp-photo")
                .role("button")
                .label(if compare.original_showing {
                    "Showing the original. Press to show the preview."
                } else {
                    "Showing the preview. Press to show the original."
                })
                .focusable(true)
                .focus_ring(2.0, 2.0, t.accent_deep)
                .cursor(rune::Cursor::Pointer)
                .pixels(pixels)
                .on_press_tx(act(Ui::flip))
                .child(el().absolute(12.0, 12.0).child(tag(key, words)))
        }
    }
}

/// One choice at the foot of the compare view: a size or a format, with what it weighs.
fn option(j: usize, label: &str, weight: Option<String>, on: bool, t: &Theme) -> El {
    let dim = Color::WHITE.alpha(0.7);
    let mut o = col()
        .key(if on {
            format!("cmp-opt-{j}-on")
        } else {
            format!("cmp-opt-{j}-tab")
        })
        .role("radio")
        .label(match &weight {
            Some(weight) => format!("{label}, {weight}"),
            None => label.to_string(),
        })
        .checked(on)
        // As wide as its wider line, to the fraction of a point: the row is centred.
        .w(24.0
            + rune::text::advance(label, type_::HEAD)
                .max(weight.as_deref().map_or(0.0, |w| rune::text::advance(w, CAP_MONO))))
        .h(43.0)
        .py(6.0)
        .items_center()
        .radius(8.0)
        .focusable(true)
        .focus_ring(2.0, 0.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer)
        .child(kit::label(format!("cmp-opt-{j}"), label, type_::HEAD, Color::WHITE).a11y_hidden());
    if on {
        o = o.fill(Color::WHITE.alpha(0.18));
    }
    if let Some(weight) = weight {
        o = o.child(kit::label(format!("cmp-opt-{j}-bytes"), weight, CAP_MONO, dim).a11y_hidden());
    }
    o
}

fn compare_sheet(ui: &Ui, compare: Compare, t: &Theme) -> El {
    let dim = Color::WHITE.alpha(0.7);
    let modes: Vec<&str> = CompareMode::ALL.iter().map(|m| m.label()).collect();
    let mode = CompareMode::ALL.iter().position(|m| *m == compare.mode).unwrap_or(1);
    let head = row()
        .key("cmp-head")
        .w_full()
        .h(48.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .child(
            setup::small_button("cmp-done", "Done", t)
                .on_press_tx(act(Ui::close_compare))
                .default_button(),
        )
        .child(setup::segmented(
            "cmp-seg",
            "How to compare",
            &modes,
            mode,
            SegSize::Sm,
            Some(SegInk {
                track: Color::WHITE.alpha(0.14),
                rest: Color::WHITE.alpha(0.75),
            }),
            t,
            |j, tab| tab.on_press_tx(act(move |ui: &mut Ui| ui.set_compare_mode(CompareMode::ALL[j]))),
        ))
        .child(el().grow(1.0))
        .child(kit::label(
            "cmp-hint",
            match compare.mode {
                CompareMode::Wipe => "Pinch to zoom \u{b7} double-click for 1:1",
                CompareMode::Stacked => "The original above, the preview below",
                CompareMode::Flip => "Press the photo to flip",
            },
            type_::CAPTION,
            dim,
        ));
    let mut foot = row()
        .key("cmp-foot")
        .w_full()
        .pb(16.0)
        .gap(24.0)
        .center()
        .role("group")
        .label("Preview size and format");
    for (j, size) in PreviewSize::ALL.iter().enumerate() {
        let size = *size;
        foot = foot.child(
            option(j, size.label(), ui.weight(size, ui.format), size == ui.size, t)
                .on_press_tx(act(move |ui: &mut Ui| ui.set_size(size))),
        );
    }
    foot = foot.child(
        el().key("cmp-sep")
            .a11y_hidden()
            .size(kit::HAIR, 18.0)
            .mx(2.0)
            .fill(Color::WHITE.alpha(0.3)),
    );
    for (j, format) in PreviewFormat::ALL.iter().enumerate() {
        let format = *format;
        foot = foot.child(
            option(
                PreviewSize::ALL.len() + j,
                format.label(),
                ui.weight(ui.size, format),
                format == ui.format,
                t,
            )
            .on_press_tx(act(move |ui: &mut Ui| ui.set_format(format))),
        );
    }
    foot = foot.child(setup::small_button("cmp-choose", "Choose photo", t).on_press_tx(act(Ui::choose_photo)));
    setup::sheet(
        "Compare a preview with the original",
        SHEET.0,
        SHEET.1,
        col()
            .key("cmp")
            .w_full()
            .h_full()
            .corners([0.0, 0.0, 14.0, 14.0])
            .fill(stage(t))
            .child(head)
            .child(
                el().key("cmp-stage")
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .center()
                    .child(photo(ui, compare, t)),
            )
            .child(foot),
        t,
    )
}

//! Where the global elements are drawn on the window: the three places the Files window asks.

use rune::{El, el};

use super::{BannerKind, Ui, act_banner, close_preview, dismiss_banner, retry_preview, sentence_of, source};
use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::banner::{self, Banner, Run, Tone};
use crate::ui::{blocked_preview, global_icons, load_strip, status_offline};

fn banner_el(kind: BannerKind, status: &source::GlobalStatus, t: &Theme) -> El {
    let (tone, icon) = match kind {
        BannerKind::SignedOut => (Tone::Warn, &global_icons::KEY),
        BannerKind::StorageFull => (Tone::Red, &global_icons::STORAGE),
        BannerKind::SyncProblem | BannerKind::BackupProblem => (Tone::Red, &global_icons::ALERT),
        BannerKind::StorageAlmostFull => (Tone::Warn, &global_icons::STORAGE),
        BannerKind::Offline => (Tone::Neutral, &global_icons::OFFLINE),
    };
    let runs = sentence_of(kind, status);
    banner::banner(
        &Banner {
            tone,
            icon,
            runs: &runs,
            action: kind.action().map(|word| (word, act_banner as banner::Handler)),
            dismiss: kind.dismissible().then_some(dismiss_banner as banner::Handler),
        },
        t,
    )
}

/// 18240 as "18,240".
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn strip_el(loading: &source::Loading, t: &Theme) -> El {
    let mut count = vec![Run::Figure(format!(
        "{} of {}",
        grouped(loading.listed),
        grouped(loading.total)
    ))];
    if let Some(left) = &loading.left {
        count.push(Run::Words(format!(" \u{00B7} {left} left")));
    }
    // The bar moves in whole percents, as the board draws it (18,240 of 40,112 is 45 %).
    let percent = (loading.listed.saturating_mul(100))
        .checked_div(loading.total)
        .unwrap_or(0)
        .min(100);
    let fraction = percent as f32 / 100.0;
    load_strip::load_strip(
        "Getting your files ready",
        &count,
        fraction,
        &format!(
            "File list: {} \u{00B7} Thumbnails: {}",
            loading.file_list.words(),
            loading.thumbnails.words()
        ),
        t,
    )
}

/// What sits between the toolbar and the list: the one banner, or the loading strip when
/// nothing else is to be said.
pub fn under_toolbar(s: &State, t: &Theme) -> Option<El> {
    let ui: &Ui = &s.global;
    if let Some((kind, status)) = ui.banner() {
        return Some(banner_el(kind, &status, t));
    }
    let status = ui.source.status();
    if status.locked {
        return None;
    }
    status.loading.as_ref().map(|loading| strip_el(loading, t))
}

/// What sits over the window's body (the sidebar and the content area, under the caption): the
/// preview panel that needs a connection, on the scrim. `window` is the window's size.
pub fn over_body(s: &State, window: (f32, f32), t: &Theme) -> Option<El> {
    let (_, name) = s.global.blocked_preview.as_ref()?;
    let card = blocked_preview::blocked_preview(
        name,
        &global_icons::OFFLINE,
        "Needs a connection",
        &format!("{name} isn't on this PC yet. Connect to open it."),
        ("Try again", retry_preview),
        close_preview,
        t,
    );
    // The board puts the panel at 420, 150 of a 1278 x 766 body: left of the middle and above
    // it, by the same shares at any size.
    let (w, h) = (window.0, (window.1 - crate::TITLEBAR_H).max(0.0));
    let left = ((w - blocked_preview::CARD_W) * (420.0 / 798.0)).max(0.0);
    let top = ((h - blocked_preview::CARD_H) * (150.0 / 436.0)).max(0.0);
    Some(
        el().key("preview-layer")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .child(
                el().key("scrim")
                    .a11y_hidden()
                    .absolute(0.0, 0.0)
                    .w_full()
                    .h_full()
                    .fill(crate::shell::registry::scrim(t.dark)),
            )
            .child(
                el().key("preview-place")
                    .absolute(left, top)
                    // The window behind is inert while the panel is open; Esc closes it.
                    .modal(false, |s: &mut State| {
                        s.close_blocked_preview();
                    })
                    .child(card),
            ),
    )
}

/// The status line while the sync state says there is no connection: a grey dot and "Offline".
/// `None`: the Files window's own.
pub fn status_line(state: Option<&str>, details: &str, t: &Theme) -> Option<El> {
    (state == Some(super::samples::OFFLINE)).then(|| status_offline::status_offline(details, t))
}

/// The words of the status line where a board frame gives its own (`Ui::board_status_words`).
/// `None`: the Files window counts the folder shown.
pub fn status_words(s: &State) -> Option<String> {
    s.global.board_status_words.clone()
}

/// Is this row the one that could not open (drawn at 45 %, `.mk-row.is-dim`)?
pub fn dims(s: &State, id: &str) -> bool {
    s.global
        .blocked_preview
        .as_ref()
        .is_some_and(|(blocked, _)| blocked == id)
}

/// The body in its place in the window. In the app it fills what the caption leaves. A board
/// frame that draws only the upper part of the window keeps the body at its full height (768,
/// so every row is laid out as on the board) and fades its lower edge into the ground.
pub fn fit(s: &State, body: El, t: &Theme) -> El {
    if !s.global.board_crop_bottom {
        return body.grow(1.0).min_h(0.0);
    }
    let ground = t.sidebar_bg;
    el().key("body-crop")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .clip_rounded(0.0)
        .child(body.absolute(0.0, 0.0).h(768.0))
        .child(
            el().key("crop-fade")
                .a11y_hidden()
                .pin(Some(0.0), None, Some(0.0), Some(0.0))
                .h(44.0)
                .gradient(ground.alpha(0.0), ground),
        )
}

//! Where the global elements are drawn on the window: the three places the Files window asks.

use rune::{El, el};

use super::{BannerKind, Ui, act_banner, dismiss_banner, sentence_of, source};
use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::banner::{self, Banner, Run, Tone};
use crate::ui::{global_icons, load_strip};

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

/// What sits over the window's body (the sidebar and the content area, under the caption).
pub fn over_body(_s: &State, _t: &Theme) -> Option<El> {
    None
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

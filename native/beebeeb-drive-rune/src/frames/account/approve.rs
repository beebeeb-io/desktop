//! Approving a sign-in from another device (DW-A11 to DW-A17): the code typed here (DW-A12), the
//! review of what the server knows about the request (DW-A13), the wait on Windows Hello
//! (DW-A14), and the three ends: approved (DW-A15), denied (DW-A16), expired or already handled
//! (DW-A17). DW-A11 is Windows' own toast: this file has its words ([`notification`]) and what a
//! click on it opens ([`notification_clicked`]); Windows draws it.
//!
//! The code. It is what lets another device receive the account's keys, so it is handled as a
//! secret that the screen shows as typed: ONE secure, revealed `TextEdit` (`Approve::code`),
//! drawn by Rune's secure text input alone. It is in no element label, no accessibility text and
//! no `String` of this file; it reaches the source as a borrowed `&str` and is never written to
//! a log or to the fixture's `wire`. It is wiped when the flow ends (`Approve::default()` drops it).
//!
//! Windows Hello. `begin_approval` asks the source to ask Windows; the answer comes back through
//! [`poll`], and only while the wait it answers is still open.

use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::account_set::{self as set, Lead, Look, Row, Size, Tail};
use crate::ui::au::{self, Keys, Tone};
use crate::ui::au_entry::{self, FocusedSpec};
use crate::ui::kit;
use crate::ui::settings_dialog::{DIALOG_TITLE, scrim_color};
use crate::ui::settings_list as lists;
use crate::ui::settings_shell::PANE_TITLE;
use rune::{Align, Color, El, TextEdit, Tx, col, el, row};

use super::source::AccountSource;
use super::{AccountUi, Dialog, Pane, icons};

/// The code field's own key (focus, tests).
pub const CODE_KEY: &str = "acct-approve-code";
/// A code is eight characters, shown as XXXX-XXXX.
const CODE_CHARS: usize = 8;
const DIALOG_W: f32 = 440.0;

/// A sign-in that waits for an answer, as the server describes it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SignInRequest {
    /// The server's id of the request. Not the code.
    pub id: String,
    /// The product the request is for ("Drive").
    pub product: String,
    /// Reported by that device, unverified: its name and its app.
    pub device: String,
    pub app: String,
    /// Measured by the server: the requester cannot choose these.
    pub country: String,
    pub address: String,
    pub asked_at: String,
    /// The account has never signed in from this country.
    pub unusual_place: bool,
}

/// What looking up a code found.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Lookup {
    Found(SignInRequest),
    /// Wrong, expired or already used: the server does not say which, and neither does the screen.
    NotFound,
    Offline,
}

/// How an approval ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Approval {
    /// The code was claimed and the keys were relayed.
    Approved,
    /// Windows Hello failed or was cancelled: nothing was claimed, nothing was relayed.
    Cancelled,
    Expired,
    /// Another device answered first.
    AlreadyHandled,
    Offline,
}

/// How a denial ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Denial {
    Denied,
    Expired,
    AlreadyHandled,
}

/// The codes of the fixture's waiting sign-in requests: the board's (DW-A12, DW-A13), one that
/// runs out when it is answered, one another device answered first, one where Windows Hello is
/// cancelled, and one from a country the account never signed in from.
pub const BOARD_REQUEST_CODE: &str = "K7QM-4WTD";
pub const EXPIRED_REQUEST_CODE: &str = "T2XP-8RED";
pub const HANDLED_REQUEST_CODE: &str = "H4ND-L3D7";
pub const CANCELLED_REQUEST_CODE: &str = "C4NC-3LED";
pub const UNUSUAL_REQUEST_CODE: &str = "N3WP-L4C3";

/// The board's waiting request (DW-A13): Drive on Anna's iPhone, asked from Germany.
pub fn board_request(id: &str, country: &str, unusual_place: bool) -> SignInRequest {
    SignInRequest {
        id: id.into(),
        product: "Drive".into(),
        device: "Anna's iPhone".into(),
        app: "Drive 1.0".into(),
        country: country.into(),
        address: "203.0.113.47".into(),
        asked_at: "09:41:12".into(),
        unusual_place,
    }
}

/// What the fixture's server finds for `code`.
pub fn board_lookup(code: &str) -> Lookup {
    let (id, country, unusual) = match code {
        BOARD_REQUEST_CODE => ("req-iphone", "Germany", false),
        EXPIRED_REQUEST_CODE => ("req-expired", "Germany", false),
        HANDLED_REQUEST_CODE => ("req-handled", "Germany", false),
        CANCELLED_REQUEST_CODE => ("req-cancelled", "Germany", false),
        UNUSUAL_REQUEST_CODE => ("req-unusual", "Brazil", true),
        _ => return Lookup::NotFound,
    };
    Lookup::Found(board_request(id, country, unusual))
}

/// What went wrong with the last press. Nothing was approved in any of these.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Problem {
    NotFound,
    Offline,
}

const NOT_FOUND: &str = "We could not find a waiting request with that code. Check it on the other device.";
const OFFLINE: &str = "Beebeeb can't be reached. Nothing was approved.";

/// What the approval flow holds between presses.
pub struct Approve {
    /// The code as typed (secure, shown).
    pub code: TextEdit,
    /// The request under review.
    pub request: Option<SignInRequest>,
    /// The typed code is this request's code: Approve is live.
    pub matches: bool,
    pub problem: Option<Problem>,
    /// Windows Hello is up and its answer has not come.
    pub waiting: bool,
    /// A scene: draw the field as the board draws one that holds the keyboard.
    pub pictured: bool,
    /// How long the field's text was after the last edit (a Backspace on the hyphen).
    len_before: usize,
}

impl Default for Approve {
    fn default() -> Self {
        let mut code = TextEdit::new("").secure().revealed(true);
        // Room for XXXX-XXXX, so tidying the text in place never moves it.
        code.text.reserve(16);
        Self {
            code,
            request: None,
            matches: false,
            problem: None,
            waiting: false,
            pictured: false,
            len_before: 0,
        }
    }
}

/// Whether `pane` is one of this file's.
fn ours(pane: Pane) -> bool {
    matches!(pane, Pane::ApproveCode | Pane::ApproveReview)
}

/// The page that shows now was just opened (`super::go`): the code page starts empty with the
/// keyboard in its field; any page that is not this flow's ends the flow.
pub fn arrive(ui: &mut AccountUi) {
    if ui.pane == Pane::ApproveReview && ui.dialog.is_some() {
        return;
    }
    leave(ui);
    if ui.pane == Pane::ApproveCode {
        ui.approve.code.focused = true;
    }
}

/// End the flow: a prompt that is still up is dismissed, and the code is wiped.
pub fn leave(ui: &mut AccountUi) {
    if ui.approve.waiting
        && let Some(source) = ui.source.as_mut()
    {
        source.cancel_approval();
    }
    ui.approve = Approve::default();
}

/// Put the typed characters in the code's shape, in place: capitals and digits, eight of them, a
/// hyphen after the fourth. A Backspace that removed the hyphen removes the character before it.
fn tidy(a: &mut Approve) {
    let text = &mut a.code.text;
    let lost_hyphen = a.len_before == 5 && text.len() == 4 && !text.contains('-');
    text.retain(|c| c.is_ascii_alphanumeric());
    text.make_ascii_uppercase();
    if lost_hyphen {
        text.pop();
    }
    text.truncate(CODE_CHARS);
    if text.len() >= 4 {
        text.insert(4, '-');
    }
    a.len_before = text.len();
    a.code.anchor = text.len();
    a.code.focus = text.len();
}

/// Whether the field holds a whole code.
fn whole(a: &Approve) -> bool {
    a.code.text.len() == CODE_CHARS + 1
}

/// The field changed: its shape, and whether it is now the code of the request under review.
pub fn edited(s: &mut State, _tx: &mut Tx) {
    let ui = &mut s.account;
    ui.approve.pictured = false;
    ui.approve.problem = None;
    tidy(&mut ui.approve);
    recheck(ui);
}

/// DW-A13's rule: "Approve stays disabled until the typed code matches the request found".
fn recheck(ui: &mut AccountUi) {
    let a = &mut ui.approve;
    a.matches = false;
    let (Some(request), Some(source)) = (a.request.as_ref(), ui.source.as_mut()) else {
        return;
    };
    if whole(a) {
        a.matches = matches!(source.find_sign_in_request(&a.code.text), Lookup::Found(found) if found.id == request.id);
    }
}

/// Continue on DW-A12: look the code up, and open the review.
fn look_up(ui: &mut AccountUi) -> Option<&'static str> {
    ui.approve.pictured = false;
    if !whole(&ui.approve) {
        ui.approve.problem = Some(Problem::NotFound);
        return None;
    }
    let source = ui.source.as_mut()?;
    match source.find_sign_in_request(&ui.approve.code.text) {
        Lookup::Found(request) => {
            ui.approve.request = Some(request);
            ui.approve.matches = true;
            ui.approve.problem = None;
            ui.approve.code.focused = false;
            Some("DW-A13")
        }
        Lookup::NotFound => {
            ui.approve.problem = Some(Problem::NotFound);
            None
        }
        Lookup::Offline => {
            ui.approve.problem = Some(Problem::Offline);
            None
        }
    }
}

/// The words of Windows' toast for a waiting request (DW-A11): `(title, body)`. No code and no
/// key is in it, and no Approve or Deny: approving needs the code and Windows Hello.
// Phase 2 (the engine's push, the shell's toast) calls this; until then only the tests do.
#[allow(dead_code)]
pub fn notification(request: &SignInRequest) -> (String, String) {
    (
        format!("Allow {} on {}?", request.product, request.device),
        format!(
            "Sign-in request from {}. Open to review it. Don't approve it if it wasn't you.",
            request.country
        ),
    )
}

/// The toast was clicked (DW-A11's one action): the review opens, its code field empty.
/// `false`: no request waits any more.
// Phase 2 (the shell's toast activation) calls this; until then only the tests do.
#[allow(dead_code)]
pub fn notification_clicked(s: &mut State) -> bool {
    let Some(request) = s
        .account
        .source
        .as_ref()
        .and_then(|source| source.waiting_sign_in_request())
    else {
        return false;
    };
    super::go(s, "DW-A52");
    s.account.pane = Pane::ApproveReview;
    s.account.dialog = Some(Dialog::ApproveReview);
    s.account.approve = Approve::default();
    s.account.approve.request = Some(request);
    s.account.approve.code.focused = true;
    true
}

/// Approve on DW-A13: Windows Hello is asked (DW-A14). The keys leave only after it succeeds.
fn approve(s: &mut State) {
    let ui = &mut s.account;
    if !ui.approve.matches || ui.approve.waiting {
        return;
    }
    let (Some(request), Some(source)) = (ui.approve.request.as_ref(), ui.source.as_mut()) else {
        return;
    };
    source.begin_approval(&request.id, &ui.approve.code.text);
    ui.approve.waiting = true;
    ui.approve.problem = None;
    ui.approve.code.focused = false;
    ui.dialog = Some(Dialog::ApproveHello);
}

/// Deny (the button, and Esc): the request ends with nothing shared. A prompt that is up goes first.
fn deny(s: &mut State) {
    let ui = &mut s.account;
    let (Some(request), Some(source)) = (ui.approve.request.as_ref(), ui.source.as_mut()) else {
        return;
    };
    if ui.approve.waiting {
        source.cancel_approval();
        ui.approve.waiting = false;
    }
    let end = match source.deny_sign_in(&request.id) {
        Denial::Denied => Dialog::ApproveDenied,
        Denial::Expired => Dialog::ApproveExpired { handled: false },
        Denial::AlreadyHandled => Dialog::ApproveExpired { handled: true },
    };
    finish(ui, end);
}

/// Show how the request ended. The code is wiped; the request's name stays for the words.
fn finish(ui: &mut AccountUi, end: Dialog) {
    let request = ui.approve.request.take();
    ui.approve = Approve::default();
    ui.approve.request = request;
    ui.dialog = Some(end);
}

/// Windows Hello's answer, and a request that ran out under review. An answer is applied only
/// by the wait it answers. `true`: something changed.
pub fn poll(s: &mut State) -> bool {
    let ui = &mut s.account;
    if !ours(ui.pane) {
        return false;
    }
    let Some(source) = ui.source.as_mut() else {
        return false;
    };
    if ui.approve.waiting && ui.dialog == Some(Dialog::ApproveHello) {
        let Some(answer) = source.approval_answer() else {
            return false;
        };
        ui.approve.waiting = false;
        match answer {
            Approval::Approved => finish(ui, Dialog::ApproveApproved),
            Approval::Expired => finish(ui, Dialog::ApproveExpired { handled: false }),
            Approval::AlreadyHandled => finish(ui, Dialog::ApproveExpired { handled: true }),
            // Back to the review; nothing was sent.
            Approval::Cancelled => ui.dialog = Some(Dialog::ApproveReview),
            Approval::Offline => {
                ui.approve.problem = Some(Problem::Offline);
                ui.dialog = Some(Dialog::ApproveReview);
            }
        }
        return true;
    }
    if ui.dialog == Some(Dialog::ApproveReview)
        && let Some(request) = ui.approve.request.as_ref()
        && let Some(gone) = source.sign_in_request_gone(&request.id)
    {
        let handled = gone == Denial::AlreadyHandled;
        finish(ui, Dialog::ApproveExpired { handled });
        return true;
    }
    false
}

/// Esc on this flow's dialogs. On the review it is Deny (the board); on a result it is the
/// result's one button. `false`: not this file's to answer.
pub fn on_escape(s: &mut State) -> bool {
    match s.account.dialog {
        Some(Dialog::ApproveReview | Dialog::ApproveHello) => deny(s),
        Some(Dialog::ApproveApproved | Dialog::ApproveDenied) => super::go(s, "DW-A55"),
        Some(Dialog::ApproveExpired { .. }) => super::go(s, "DW-A51"),
        _ => return false,
    }
    true
}

fn field_key(s: &mut State, ev: &rune::KeyEvent, tx: &mut Tx) -> bool {
    ev.key == rune::Key::Escape && super::on_key(s, ev, tx)
}

/// The code field, at index 0 of its view.
fn code_field(a: &Approve, label: &str, error: Option<&str>, t: &Theme) -> El {
    let mut keys = Keys::new();
    au_entry::focused_field(
        &mut keys,
        FocusedSpec {
            label,
            icon: None,
            error,
            mono: true,
            pictured: a.pictured,
        },
        &a.code,
        CODE_KEY,
        |s: &mut State| &mut s.account.approve.code,
        edited,
        field_key,
        None,
        t,
    )
}

fn problem_words(problem: Option<Problem>) -> Option<&'static str> {
    problem.map(|p| match p {
        Problem::NotFound => NOT_FOUND,
        Problem::Offline => OFFLINE,
    })
}

/// DW-A12: type the code the other device shows.
pub fn code_pane(a: &Approve, t: &Theme) -> El {
    let error = problem_words(a.problem);
    let mut field =
        col()
            .key("approve-field")
            .w_full()
            .gap(6.0)
            .child(code_field(a, "Code on the other device", error, t));
    if error.is_none() {
        field = field.child(
            kit::label(
                "field-0-msg",
                "Eight characters, shown as XXXX-XXXX.",
                au::type_au::FIELD_MSG,
                t.text_tertiary,
            )
            .role("note")
            .mx(4.0),
        );
    }
    let go_on = set::button(
        "btn-0",
        "Continue",
        Some(&icons::CHECK),
        Look::Primary,
        Size::Medium,
        false,
        t,
    )
    .default_button()
    .on_press(super::act(look_up));
    set::pane(
        vec![
            set::heading(
                "Approve a sign-in",
                Some("Type the code the other device is showing. Only approve a request you started yourself."),
                None,
                t,
            ),
            field,
            row()
                .key("approve-actions")
                .w_full()
                .center_y()
                .gap(10.0)
                .child(go_on)
                .child(kit::label(
                    "row-note",
                    "or scan its QR code with your iPhone instead",
                    set::SUB,
                    t.text_tertiary,
                )),
        ],
        t,
    )
}

/// What lies under the review and its results, as the board draws it: Sign-in and security,
/// its Devices list.
fn behind(source: &dyn AccountSource, t: &Theme) -> El {
    let opens = |index: usize, it: Row, frame: &'static str| {
        set::item(0, index, &it.link(true), Vec::new(), t).on_press(move |s: &mut State| super::go(s, frame))
    };
    set::pane(
        vec![
            set::heading("Sign-in and security", None, None, t),
            lists::list(
                0,
                Some("Devices"),
                vec![
                    opens(
                        0,
                        Row::new(Lead::Icon(&icons::DEVICE_PHONE), "Devices and sessions")
                            .tail(Tail::Value(source.security().sessions.to_string())),
                        "DW-A55",
                    ),
                    opens(
                        1,
                        Row::new(Lead::Icon(&icons::SCAN), "Approve a sign-in").sub("Let another device sign in"),
                        "DW-A12",
                    ),
                ],
                None,
                t,
            ),
        ],
        t,
    )
}

/// A dialog of this flow on its scrim (`.au-scrim` + `.wn-dlg`, 440 wide): the body's children
/// 12 apart, and the foot's buttons sharing its width (`set::dialog_with`'s arithmetic).
fn shell(role: &'static str, name: &str, children: Vec<El>, foot: Vec<(El, bool)>, t: &Theme) -> El {
    let mut body = col().key("dlg-body").w_full().pt(22.0).px(24.0).pb(18.0).gap(12.0);
    for child in children {
        body = body.child(child);
    }
    let mut f = row()
        .key("dlg-foot")
        .w_full()
        .center_y()
        .gap(8.0)
        .py(16.0)
        .px(24.0)
        .fill(t.surface)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout();
    let count = foot.len().max(1) as f32;
    let fixed = |bordered: bool| 24.0 + if bordered { 2.0 } else { 0.0 };
    let free = DIALOG_W - 48.0 - 8.0 * (count - 1.0) - foot.iter().map(|(_, bordered)| fixed(*bordered)).sum::<f32>();
    for (child, bordered) in foot {
        f = f.child(child.w(free / count + fixed(bordered)));
    }
    let dlg = col()
        .key("dlg")
        .role(role)
        .label(name)
        .pixel_snap(false)
        .w(DIALOG_W)
        .radius(8.0)
        .fill(t.card)
        .box_shadow(kit::drop(24.0, 60.0, Color::BLACK.alpha(0.28)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(col().key("dlg-clip").w_full().clip_rounded(8.0).child(body).child(f));
    col()
        .key("scrim")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .fill(scrim_color(t))
        .child(dlg)
}

fn foot_button(index: usize, label: &str, look: Look, dimmed: bool, t: &Theme) -> (El, bool) {
    super::forms::button(index, label, look, dimmed, t)
}

/// The question at the head of the review: the product and the device as it named itself.
fn question(request: &SignInRequest) -> String {
    format!("Allow {} on {}?", request.product, request.device)
}

fn review_head(request: &SignInRequest, t: &Theme) -> El {
    col()
        .key("dlg-h")
        .w_full()
        .gap(6.0)
        .child(
            kit::label("dlg-title", question(request), PANE_TITLE, t.text)
                .role("heading")
                .level(2)
                .wrap()
                .w_full(),
        )
        .child(
            kit::label(
                "dlg-text-0",
                "Someone asked to sign in to your Beebeeb account. Only approve it if that was you.",
                set::SUB,
                t.text_secondary,
            )
            .wrap()
            .w_full(),
        )
}

fn review_foot(a: &Approve, waiting: bool, t: &Theme) -> Vec<(El, bool)> {
    // Approve is the only amber; Deny is not styled as destructive (the board).
    let live = a.matches && !waiting;
    let (mut yes, yes_b) = foot_button(0, "Approve", Look::Primary, !a.matches, t);
    if live {
        // Enter is Approve only when the code matches.
        yes = yes.default_button().on_press(approve);
    }
    let (no, no_b) = foot_button(1, "Deny", Look::Plain, false, t);
    vec![(yes, yes_b), (no.on_press(deny), no_b)]
}

/// DW-A13: everything the server knows about the request, the code, Approve and Deny.
fn review(a: &Approve, request: &SignInRequest, t: &Theme) -> El {
    let fact = |list: usize, index: usize, icon: &'static rune::Icon, label: &str, tail: Tail| {
        set::item(
            list,
            index,
            &Row::new(Lead::Icon(icon), label).tail(tail),
            Vec::new(),
            t,
        )
    };
    let mut children = vec![review_head(request, t)];
    let mut keys = Keys::new();
    if request.unusual_place {
        children.push(au::banner(
            &mut keys,
            Tone::Red,
            &crate::theme::icons_settings::INFO,
            Some("Unusual place"),
            Some(&format!(
                "This account has never signed in from {}. Don't approve it unless that is you.",
                request.country
            )),
            true,
            t,
        ));
    }
    if a.problem == Some(Problem::Offline) {
        children.push(au::banner(
            &mut keys,
            Tone::Red,
            &crate::theme::icons_settings::INFO,
            None,
            Some(OFFLINE),
            true,
            t,
        ));
    }
    children.push(lists::list(
        1,
        Some("Measured by Beebeeb"),
        vec![
            fact(1, 0, &icons::GLOBE, "Country", Tail::Value(request.country.clone())),
            fact(
                1,
                1,
                &icons::ACTIVITY,
                "Network address",
                Tail::Mono(request.address.clone()),
            ),
            fact(1, 2, &icons::CLOCK, "Asked at", Tail::Mono(request.asked_at.clone())),
        ],
        None,
        t,
    ));
    children.push(lists::list(
        2,
        Some("Reported by that device (it can be wrong)"),
        vec![
            fact(2, 0, &icons::DEVICE_PHONE, "Name", Tail::Value(request.device.clone())),
            fact(2, 1, &icons::FOLDER, "App", Tail::Mono(request.app.clone())),
        ],
        None,
        t,
    ));
    children.push(code_field(a, "Code shown on that screen", None, t));
    shell("dialog", &question(request), children, review_foot(a, false, t), t)
}

/// DW-A14: Windows Hello is asking (Windows draws its prompt over this). What approving shares
/// is said in amber: it is encryption state.
fn hello(a: &Approve, request: &SignInRequest, t: &Theme) -> El {
    let mut keys = Keys::new();
    let children = vec![
        review_head(request, t),
        au::banner(
            &mut keys,
            Tone::Amber,
            &icons::PADLOCK,
            Some("Approving shares your keys with that device"),
            Some(
                "It receives the keys that unlock your Drive, Pass, VPN, Chat and Mail data, encrypted for it alone. Beebeeb can't read them.",
            ),
            false,
            t,
        ),
        code_field(a, "Code shown on that screen", None, t),
    ];
    shell("dialog", &question(request), children, review_foot(a, true, t), t)
}

/// The head of a result: a toned tile, what happened, one sentence. All centred.
fn result_head(icon: &'static rune::Icon, tone: Tone, title: &str, words: &str, t: &Theme) -> Vec<El> {
    vec![
        row()
            .key("dlg-hero-row")
            .w_full()
            .center()
            // `.au-hero`: 8 above the tile and 4 under it, inside the body's own 12 between children.
            .pt(8.0)
            .pb(4.0)
            .child(
                el().key("dlg-hero")
                    .child(set::tile("dlg-tile", icon, 56.0, 16.0, tone, t)),
            ),
        col()
            .key("dlg-h")
            .w_full()
            .gap(6.0)
            .items_center()
            .child(
                kit::label("dlg-title", title, DIALOG_TITLE, t.text)
                    .role("heading")
                    .level(2),
            )
            .child(
                // As wide as its words when they fit one line (DW-A16), the column when they wrap.
                kit::label("dlg-text-0", words, set::SUB, t.text_secondary)
                    .wrap()
                    .max_w(DIALOG_W - 48.0)
                    .align(Align::Center),
            ),
    ]
}

fn one_button(label: &str, frame: &'static str, t: &Theme) -> Vec<(El, bool)> {
    let (b, bordered) = foot_button(0, label, Look::Plain, false, t);
    vec![(
        b.default_button().on_press(move |s: &mut State| super::go(s, frame)),
        bordered,
    )]
}

/// DW-A15: approved, and where to undo it (a normal sign-out of that session: no Undo here).
fn approved(request: &SignInRequest, t: &Theme) -> El {
    let mut children = result_head(
        &icons::CHECK,
        Tone::Green,
        "Approved",
        &format!(
            "{} is signed in on {}. You can sign that device out any time in Devices and sessions.",
            request.product, request.device
        ),
        t,
    );
    children.push(lists::list(
        1,
        None,
        vec![set::item(
            1,
            0,
            &Row::new(Lead::Icon(&icons::DEVICE_PHONE), request.device.clone())
                .sub("New session, just now")
                .tail(Tail::Pill {
                    label: "Signed in".into(),
                    tone: Tone::Green,
                    dot: true,
                }),
            Vec::new(),
            t,
        )],
        None,
        t,
    ));
    shell(
        "alertdialog",
        "Approved",
        children,
        one_button("See devices and sessions", "DW-A55", t),
        t,
    )
}

/// DW-A16: denied, what was and was not done, and what to check if it was not them.
fn denied(request: &SignInRequest, t: &Theme) -> El {
    let mut children = result_head(
        &crate::ui::global_icons::CLOSE,
        Tone::Neutral,
        "Denied",
        &format!("Nothing was signed in on {} and no keys were shared.", request.device),
        t,
    );
    let mut keys = Keys::new();
    children.push(au::banner(
        &mut keys,
        Tone::Red,
        &crate::theme::icons_settings::INFO,
        Some("Didn't start this?"),
        Some("Someone may have your email. Change your password, and check Devices and sessions."),
        true,
        t,
    ));
    shell(
        "alertdialog",
        "Denied",
        children,
        one_button("Check devices and sessions", "DW-A55", t),
        t,
    )
}

/// DW-A17: the request ran out, or another device answered it first.
fn expired(handled: bool, t: &Theme) -> El {
    let (title, words) = if handled {
        (
            "Already handled",
            "Another of your devices answered this request. Nothing was approved from this PC.",
        )
    } else {
        (
            "This request expired",
            "Requests last 5 minutes. Nothing was approved. If it was you, ask for a new code on the other device.",
        )
    };
    let children = result_head(&icons::CLOCK, Tone::Neutral, title, words, t);
    shell("alertdialog", title, children, one_button("Done", "DW-A51", t), t)
}

/// The pane under this flow's dialogs, and the dialog that is open.
pub fn review_pane(source: &dyn AccountSource, a: &Approve, dialog: Option<&Dialog>, t: &Theme) -> (El, Option<El>) {
    let over = match (dialog, a.request.as_ref()) {
        (Some(Dialog::ApproveReview), Some(request)) => Some(review(a, request, t)),
        (Some(Dialog::ApproveHello), Some(request)) => Some(hello(a, request, t)),
        (Some(Dialog::ApproveApproved), Some(request)) => Some(approved(request, t)),
        (Some(Dialog::ApproveDenied), Some(request)) => Some(denied(request, t)),
        (Some(Dialog::ApproveExpired { handled }), _) => Some(expired(*handled, t)),
        _ => None,
    };
    (behind(source, t), over)
}

/// The board's frames of this flow as scenes: `fill` is what the code field holds.
pub fn scene(s: &mut State, frame: &'static str) {
    use super::source::BoardAccount;
    let mut account = BoardAccount::new();
    if frame == "DW-A14" {
        account = account.holding_hello();
    }
    s.account.source = Some(Box::new(account));
    super::go(s, "DW-A12");
    let typed = if frame == "DW-A12" {
        &BOARD_REQUEST_CODE[..5]
    } else {
        BOARD_REQUEST_CODE
    };
    super::form::type_in(&mut s.account.approve.code, typed);
    s.account.approve.len_before = typed.len();
    if frame == "DW-A12" {
        // A headless render has no keyboard focus: the board's focused field is drawn as shapes.
        s.account.approve.pictured = true;
        s.account.approve.code.focused = false;
        return;
    }
    if let Some(next) = look_up(&mut s.account) {
        super::go(s, next);
    }
    match frame {
        "DW-A14" => approve(s),
        "DW-A15" => {
            approve(s);
            poll(s);
        }
        "DW-A16" => deny(s),
        "DW-A17" => finish(&mut s.account, Dialog::ApproveExpired { handled: false }),
        _ => {}
    }
}

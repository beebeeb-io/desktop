//! Slice A5 (plan40 batch 1): approving a sign-in from another device (DW-A12 to DW-A17), on the real engine. Part of
//! [`EngineAccountSource`] (`account.rs`). The rules and the crypto are core's, run by the engine
//! (`beebeeb_engine::app::bindings::approve_sign_in`); this file binds them to the window.
//!
//! In order:
//! - The code page and the review (DW-A12, DW-A13): `find_sign_in_request` asks the engine for the request a code names
//!   ([`Engine::find_sign_in_request_for`]) and waits for it on the window's thread, as `step_up` does (the trait
//!   answers at once), bounded by the engine client's 30 s. A code this PC already holds is answered from that
//!   request with no network: Beebeeb counts every miss as a guess, and the review asks again on each edit of the
//!   code. Each request found gets an id of this PC's own (`req-<n>`), never derived from the code.
//! - Approve (DW-A14): the press stamps the session and the account. Nothing is asked while a person's call may not
//!   run (a locked screen, a closed vault, another session: [`Engine::sign_in_approval_allowed_for`]) or for an empty
//!   password. Windows Hello is asked FIRST, with its own line ([`APPROVE_REASON`]); the password and the code wait in
//!   wiped buffers while the prompt is up, and nothing leaves this PC before Windows says "verified". Then a worker
//!   runs the engine's approval ([`Engine::approve_sign_in_for`]: core's step-up, the claim, and the relay that core
//!   seals to the other device's key). Windows Hello not set up on this PC: no approval (it says so).
//! - `approval_answer` hands each ending out once, with its request's id; an ending, or a Windows Hello answer, that
//!   comes under another session or account is dropped, and a prompt still up for one is taken away. One exception:
//!   an approval whose keys left (approved, or not known) and that ended under another turn of the SAME account (a
//!   lock while it ran) is kept, and said as a toast once the window is open again on an account page: the review it
//!   was for ended with the lock, and the other device is signed in all the same. A sign-out, a closed vault or
//!   another account wipes it unsaid (as `account_twofactor.rs` does with backup codes).
//! - A retry after an approval that did not get as far as the claim asks Beebeeb for the code's request again, and
//!   goes on only when that request is the one the review shows (Beebeeb's facts, the expiry included, are the same);
//!   another one ends the review as expired, with no prompt and nothing sent.
//!   `cancel_approval` while Windows Hello is up takes the prompt away, wipes the password and claims nothing; once
//!   Windows said "verified" the approval is on its way and cannot be recalled.
//! - Deny (DW-A16): core's denial, waited for on the window's thread as the lookup. Expiry (DW-A17):
//!   `sign_in_request_gone` reads the request's own expiry, with no network; a timer wakes the window when it passes.
//!
//! Must not: log the code, the password, a request's facts or a server's words (only fixed words); keep the password
//! after the prompt answered; send anything before Windows Hello verified.

use std::collections::VecDeque;
use std::sync::Mutex;

use beebeeb_engine::app::bindings::approve_sign_in::{ApproveError, DenyHandle, FoundRequest, RequestFacts};
use beebeeb_engine::signin::gate::GateState;
use zeroize::Zeroizing;

use crate::engine_host::hello::{HelloCancel, HelloPrompter, HelloResult, NoHello, WindowHello};
use crate::frames::account::approve::Reported;

use super::*;

/// The line Windows shows in its prompt for an approval.
pub const APPROVE_REASON: &str = "Approve a sign-in on another device";
/// How many requests found by code are held at most, for one session and account: the newest.
const HELD_AT_MOST: usize = 4;
/// How long a Deny waits for this PC's own approval of the same request to answer, when Beebeeb says the request
/// is gone (the approval's requests are bounded by the engine client's 30 s each).
const APPROVAL_SETTLES_WITHIN: Duration = Duration::from_secs(35);
/// The products a request can name; anything else is drawn as this app's product (`SignInRequest::bounded`).
const PRODUCTS: [&str; 5] = ["Drive", "Pass", "VPN", "Chat", "Mail"];
/// The toast after the unlock, for an approval that went through while the window's review had ended (a lock).
pub const APPROVED_LATE: &str = "The sign-in you approved went through. That device is signed in.";
/// The toast after the unlock, for an approval whose keys left with no answer that said how it ended.
pub const UNKNOWN_LATE: &str =
    "Couldn't confirm the sign-in you approved. Devices and sessions shows whether that device is signed in.";

/// A request found by its code, for the session and account of the lookup.
struct Held {
    id: String,
    made_for: Stamp,
    shown: SignInRequest,
    /// Core's request: taken by the approval when Windows Hello verified.
    found: Option<FoundRequest>,
    /// The code, for a Deny and for answering the same code again without asking Beebeeb.
    deny: DenyHandle,
    /// When the request runs out (Unix seconds), when Beebeeb's time read.
    expires_at: Option<i64>,
    /// What Beebeeb said about the request at the lookup the review shows: a retry's lookup must say the same.
    facts: RequestFacts,
}

/// An approval that was begun. While Windows Hello is up it holds what the approval needs; once Windows said
/// "verified" it holds nothing (the worker has it).
struct Begun {
    id: String,
    made_for: Stamp,
    /// The number of the Windows Hello prompt it asked: an answer of an earlier prompt (one taken away, for the
    /// same request) is never this one's.
    prompt: u64,
    waiting: Option<Waiting>,
}

/// What waits for Windows Hello: the typed code, the password and the prompt's way out. Wiped on drop.
struct Waiting {
    typed: Zeroizing<String>,
    password: Zeroizing<Vec<u8>>,
    cancel: Option<HelloCancel>,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        // A prompt that is still up when what it guards goes: taken away (never from inside its own answer: the
        // answer only sends down the channel).
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

/// What the prompt's thread and the approval's worker send back.
enum Inner {
    Hello {
        id: String,
        made_for: Stamp,
        prompt: u64,
        result: HelloResult,
    },
    Approved {
        id: String,
        made_for: Stamp,
        result: Result<(), ApproveError>,
    },
}

/// What slice A5 keeps between a press and its answer. The adapter holds one (`EngineAccountSource::account_approve`).
pub(super) struct ApproveCalls {
    held: Vec<Held>,
    next_id: u64,
    /// Counts the Windows Hello prompts asked.
    prompts: u64,
    begun: Option<Begun>,
    /// Endings to hand out, oldest first, each with the stamp it was made under.
    ready: VecDeque<(Stamp, String, Approval)>,
    /// Requests this PC's own approval got through: a Deny after it says so.
    approved_here: Vec<(Stamp, String)>,
    /// An approval whose keys left (`Approved` or `Unknown`) that ended under another turn of the same account: said
    /// as a toast once the window is open again (`account_approve_tell_late`). Wiped unsaid for another account, a
    /// sign-out or a closed vault.
    late: Option<(Stamp, Approval)>,
    hello: Box<dyn HelloPrompter>,
    tx: mpsc::Sender<Inner>,
    rx: mpsc::Receiver<Inner>,
}

impl Default for ApproveCalls {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            held: Vec::new(),
            next_id: 0,
            prompts: 0,
            begun: None,
            ready: VecDeque::new(),
            approved_here: Vec::new(),
            late: None,
            // No prompt until the shell hands one over (`with_approval_window`): no approval either.
            hello: Box::new(NoHello),
            tx,
            rx,
        }
    }
}

impl ApproveCalls {
    /// Everything that is not of the session and account `now`: dropped. A prompt still up for another session is
    /// taken away (dropping its `Waiting`), and its password wiped.
    fn forget_other(&mut self, now: &Stamp) {
        self.held.retain(|held| held.made_for == *now);
        if self.begun.as_ref().is_some_and(|begun| begun.made_for != *now) {
            self.begun = None;
        }
        self.ready.retain(|(made_for, _, _)| made_for == now);
        self.approved_here.retain(|(made_for, _)| made_for == now);
    }

    fn held(&self, id: &str, now: &Stamp) -> Option<&Held> {
        self.held.iter().find(|held| held.id == id && held.made_for == *now)
    }

    fn say(&mut self, made_for: Stamp, id: &str, answer: Approval) {
        self.ready.push_back((made_for, id.to_owned(), answer));
    }

    /// Takes an ending of this PC's approval of `id` that is ready: `Approved` (or not known) means a Deny came too
    /// late.
    fn took_approved(&mut self, id: &str, now: &Stamp) -> bool {
        let before = self.ready.len();
        self.ready.retain(|(made_for, of, answer)| {
            !(made_for == now && of == id && matches!(answer, Approval::Approved | Approval::Unknown))
        });
        let here = self
            .approved_here
            .iter()
            .any(|(made_for, of)| made_for == now && of == id);
        self.ready.len() != before || here
    }
}

/// Whether an ending made under `made_for` is still kept to be said under `now`: the same account (named), and its
/// session open (`open`: in memory and not held as ended). A screen lock keeps it; a sign-out, a closed vault or
/// another account does not.
pub(super) fn keeps_late(made_for: &Stamp, now: &Stamp, open: bool) -> bool {
    open && made_for.1.is_some() && made_for.1 == now.1
}

/// How an approval's ending reads on the screen.
fn approval_of(result: Result<(), ApproveError>) -> Approval {
    match result {
        Ok(()) => Approval::Approved,
        Err(ApproveError::Expired | ApproveError::NotFound) => Approval::Expired,
        Err(ApproveError::AlreadyHandled) => Approval::AlreadyHandled,
        Err(ApproveError::WrongPassword) => Approval::WrongPassword,
        Err(ApproveError::Offline) => Approval::Offline,
        Err(ApproveError::Unknown) => Approval::Unknown,
        // This PC refused before the keys left (a lock, a sign-out, another sign-in; before the claim or, after it,
        // before the relay): nothing was approved. Back to the review.
        Err(ApproveError::NotSignedIn | ApproveError::Locked) => Approval::Cancelled,
        Err(ApproveError::SessionEnded | ApproveError::RateLimited(_) | ApproveError::Refused) => Approval::Refused,
    }
}

fn product_of(client: Option<&str>) -> Option<&'static str> {
    let client = client?.trim();
    PRODUCTS
        .into_iter()
        .find(|product| product.eq_ignore_ascii_case(client))
}

/// The request as the screen may show it: every string bounded by `SignInRequest::bounded`. The country is the
/// two-letter code Beebeeb measured (as Devices and sessions shows it), never more; the time is this PC's.
fn shown_of(id: &str, facts: &RequestFacts) -> SignInRequest {
    let product: Option<&str> = product_of(facts.client.as_deref());
    let app_name = product.or(facts.client.as_deref()).unwrap_or_default();
    let app = match facts.client_version.as_deref() {
        Some(version) if !app_name.is_empty() => format!("{app_name} {version}"),
        _ => app_name.to_owned(),
    };
    let device = facts
        .device_name
        .clone()
        .or_else(|| facts.os.clone())
        .unwrap_or_default();
    let country = facts
        .country
        .as_deref()
        .map(|code| code.trim().to_ascii_uppercase())
        .filter(|code| code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic()))
        .unwrap_or_default();
    let asked_at = chrono::DateTime::parse_from_rfc3339(&facts.requested_at)
        .map(|at| at.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
        .unwrap_or_default();
    SignInRequest::bounded(Reported {
        id,
        product: product.unwrap_or_default(),
        device: &device,
        app: &app,
        country: &country,
        address: &facts.address,
        asked_at: &asked_at,
        // Beebeeb does not say whether the account was seen in that country before: never claimed.
        unusual_place: false,
    })
}

impl<C: CredentialStores + 'static> EngineAccountSource<C> {
    /// The source with its own Windows Hello prompter for approvals (tests: a fake one).
    pub fn with_approval_hello(mut self, hello: Box<dyn HelloPrompter>) -> Self {
        self.account_approve.hello = hello;
        self
    }

    /// The source with the main window's Windows Hello prompt for approvals, with its own line
    /// ([`APPROVE_REASON`]).
    pub fn with_approval_window(self, window: Arc<Mutex<Option<rune::WindowHandle>>>) -> Self {
        self.with_approval_hello(Box::new(WindowHello::for_reason(window, APPROVE_REASON)))
    }

    /// The approval's two pages answer while a session is open (`pane_state` asks this first).
    pub(super) fn account_approve_pane_state(&self, pane: Pane) -> Option<PaneState> {
        if !matches!(pane, Pane::ApproveCode | Pane::ApproveReview) {
            return None;
        }
        Some(if !self.engine.has_session() {
            PaneState::SignedOut
        } else if self.refused() {
            PaneState::SessionEnded
        } else {
            PaneState::Ready
        })
    }

    /// Takes in what the prompt's thread and the worker sent: a "verified" starts the approval, every other ending
    /// is made ready to hand out. Anything of another session or account is dropped.
    fn account_approve_take_in(&mut self) {
        let now = stamp(&self.engine);
        let open = self.open();
        self.account_approve.forget_other(&now);
        while let Ok(inner) = self.account_approve.rx.try_recv() {
            match inner {
                Inner::Hello {
                    id,
                    made_for,
                    prompt,
                    result,
                } => self.account_approve_hello_answered(&now, (id, made_for, prompt), result),
                Inner::Approved { id, made_for, result } => {
                    let calls = &mut self.account_approve;
                    if calls
                        .begun
                        .as_ref()
                        .is_some_and(|b| b.id == id && b.made_for == made_for)
                    {
                        calls.begun = None;
                    }
                    let answer = approval_of(result);
                    if made_for != now {
                        // The keys left while the turn moved (a lock while it ran): the other device is signed in
                        // (or may be) whatever the window shows now. Kept for the same account, said after the
                        // unlock; any other ending approved nothing and goes unsaid.
                        if matches!(answer, Approval::Approved | Approval::Unknown) && keeps_late(&made_for, &now, open)
                        {
                            calls.late = Some((made_for, answer));
                        }
                        continue;
                    }
                    if answer == Approval::Approved {
                        calls.approved_here.push((made_for.clone(), id.clone()));
                    }
                    // A request that was answered, ran out or may have been answered is used up: the same code is
                    // asked of Beebeeb again. One that was not claimed stays for another try.
                    if matches!(
                        answer,
                        Approval::Approved | Approval::Expired | Approval::AlreadyHandled | Approval::Unknown
                    ) {
                        calls.held.retain(|held| held.id != id);
                    }
                    calls.say(made_for, &id, answer);
                }
            }
        }
        self.account_approve_tell_late(&now);
    }

    /// A late approval kept through a lock: wiped for another account, a sign-out or a closed vault; said once, as a
    /// toast, when the window is open (the gate is Ready) on an account page, so it is seen.
    fn account_approve_tell_late(&mut self, now: &Stamp) {
        let open = self.open();
        let Some((made_for, _)) = self.account_approve.late.as_ref() else {
            return;
        };
        if !keeps_late(made_for, now, open) {
            self.account_approve.late = None;
            return;
        }
        if self.engine.gate() != GateState::Ready || self.showing.is_none() {
            return;
        }
        let Some((_, answer)) = self.account_approve.late.take() else {
            return;
        };
        self.say(match answer {
            Approval::Approved => Said::done(APPROVED_LATE),
            _ => Said::failed(UNKNOWN_LATE, None, false),
        });
    }

    /// `of`: the request, the stamp and the prompt the answer is for.
    fn account_approve_hello_answered(&mut self, now: &Stamp, of: (String, Stamp, u64), result: HelloResult) {
        let (id, made_for, prompt) = of;
        let calls = &mut self.account_approve;
        let Some(begun) = calls
            .begun
            .as_mut()
            .filter(|begun| begun.id == id && begun.made_for == made_for && begun.prompt == prompt)
        else {
            return;
        };
        // The prompt has answered: nothing is left to take away.
        let Some(mut waiting) = begun.waiting.take() else {
            // Cancelled meanwhile (`cancel_approval` wiped it): nothing was claimed.
            calls.begun = None;
            return;
        };
        waiting.cancel = None;
        if made_for != *now {
            calls.begun = None;
            return;
        }
        match result {
            HelloResult::Verified => {
                let found = calls
                    .held
                    .iter_mut()
                    .find(|held| held.id == id && held.made_for == made_for)
                    .and_then(|held| held.found.take());
                let Some(found) = found else {
                    calls.begun = None;
                    calls.say(made_for, &id, Approval::Expired);
                    return;
                };
                let (engine, tx) = (self.engine.clone(), calls.tx.clone());
                let (session, mine, of) = (made_for.0, made_for.clone(), id.clone());
                let started = std::thread::Builder::new()
                    .name("account-approve-sign-in".into())
                    .spawn(move || {
                        let mut waiting = waiting;
                        let password = std::mem::replace(&mut waiting.password, Zeroizing::new(Vec::new()));
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            engine.approve_sign_in_for(session, found, &waiting.typed, password)
                        }))
                        // A worker that panicked may have sent anything: not known, never "nothing was approved".
                        .unwrap_or(Err(ApproveError::Unknown));
                        let _ = tx.send(Inner::Approved {
                            id: of,
                            made_for: mine,
                            result,
                        });
                        engine.wake_host();
                    })
                    .is_ok();
                if !started {
                    log("account: could not start a thread for an approval");
                    calls.begun = None;
                    calls.say(made_for, &id, Approval::Offline);
                }
            }
            HelloResult::Cancelled | HelloResult::Failed => {
                calls.begun = None;
                calls.say(made_for, &id, Approval::Cancelled);
            }
            HelloResult::Unavailable => {
                calls.begun = None;
                calls.say(made_for, &id, Approval::NoHello);
            }
        }
    }

    pub(super) fn account_approve_find_sign_in_request(&mut self, code: &str) -> Lookup {
        let now = stamp(&self.engine);
        self.account_approve.forget_other(&now);
        // A code this PC already holds: its request, with no network.
        if let Some(held) = self
            .account_approve
            .held
            .iter()
            .find(|held| held.deny.code_matches(code))
        {
            return Lookup::Found(held.shown.clone());
        }
        if !self.open() {
            return Lookup::Refused;
        }
        let found = match self.engine.find_sign_in_request_for(now.0, code) {
            Ok(found) => found,
            Err(ApproveError::NotFound) => return Lookup::NotFound,
            Err(ApproveError::Offline) => return Lookup::Offline,
            Err(_) => return Lookup::Refused,
        };
        // The answer is for the press's session and account only.
        if stamp(&self.engine) != now {
            return Lookup::Refused;
        }
        let calls = &mut self.account_approve;
        calls.next_id += 1;
        let id = format!("req-{}", calls.next_id);
        let facts = found.facts();
        let shown = shown_of(&id, &facts);
        let unix_now = Utc::now().timestamp();
        let left = found.seconds_left(unix_now);
        calls.held.push(Held {
            id: id.clone(),
            made_for: now,
            shown: shown.clone(),
            deny: found.deny_handle(),
            found: Some(found),
            expires_at: left.map(|left| unix_now + left),
            facts,
        });
        // The oldest go first, never the one an approval is on.
        while calls.held.len() > HELD_AT_MOST {
            let busy = calls.begun.as_ref().map(|begun| begun.id.clone());
            let Some(oldest) = calls.held.iter().position(|held| Some(&held.id) != busy.as_ref()) else {
                break;
            };
            calls.held.remove(oldest);
        }
        // The review learns that the request ran out on the window's next poll: wake it then.
        if let Some(left) = left.filter(|left| (0..=600).contains(left)) {
            // A weak handle: a sleeping timer keeps no engine alive.
            let engine = Arc::downgrade(&self.engine);
            let woke = std::thread::Builder::new()
                .name("account-approve-expiry".into())
                .spawn(move || {
                    std::thread::sleep(Duration::from_secs(left as u64 + 1));
                    if let Some(engine) = engine.upgrade() {
                        engine.wake_host();
                    }
                });
            if woke.is_err() {
                log("account: could not start a thread for a request's expiry");
            }
        }
        // DW-A11: a request reached this PC.
        #[cfg(windows)]
        crate::shell::platform::windows_toast::show_sign_in_request();
        Lookup::Found(shown)
    }

    pub(super) fn account_approve_waiting_sign_in_request(&self, request: &str) -> Option<SignInRequest> {
        let now = stamp(&self.engine);
        let held = self.account_approve.held(request, &now)?;
        let gone = held.expires_at.is_some_and(|at| at <= Utc::now().timestamp());
        (self.open() && !gone).then(|| held.shown.clone())
    }

    /// The password is copied once into a wiped buffer and kept only while Windows Hello is up; it is never logged.
    pub(super) fn account_approve_begin_approval(&mut self, request: &str, code: &str, password: Password<'_>) {
        let now = stamp(&self.engine);
        self.account_approve.forget_other(&now);
        // One approval at a time. This request's own approval already on its way (the window left and came back):
        // its ending answers this press. Another request's: this one is not started, and says so.
        if let Some(begun) = self.account_approve.begun.as_ref() {
            if begun.id != request {
                self.account_approve.say(now, request, Approval::Refused);
                self.engine.wake_host();
            }
            return;
        }
        // A locked screen, a closed vault or another session first: no prompt, nothing sent (a lock is also a new
        // turn of the session, so the request it held is gone by now).
        let refused = match self.engine.sign_in_approval_allowed_for(now.0) {
            Err(error) => Some(approval_of(Err(error))),
            Ok(()) => match self.account_approve.held(request, &now) {
                None => Some(Approval::Expired),
                Some(held) if !held.deny.code_matches(code) => Some(Approval::Expired),
                Some(_) if password.is_empty() => Some(Approval::WrongPassword),
                Some(_) => None,
            },
        };
        if let Some(answer) = refused {
            self.account_approve.say(now, request, answer);
            self.engine.wake_host();
            return;
        }
        // A request whose last approval did not get as far as the claim was consumed by it: Beebeeb is asked for it
        // again (a code that is live is no guess). That answer is approved only when it is the request the review
        // shows: what Beebeeb says about it now (the device, the place, the expiry) must be what it said then.
        let consumed = self
            .account_approve
            .held(request, &now)
            .is_some_and(|held| held.found.is_none());
        if consumed {
            let again = match self.engine.find_sign_in_request_for(now.0, code) {
                Ok(found) => Ok(found),
                Err(ApproveError::NotFound) => Err(Approval::Expired),
                Err(ApproveError::Offline) => Err(Approval::Offline),
                Err(error) => Err(approval_of(Err(error))),
            };
            match again {
                Ok(found) => {
                    let held = self
                        .account_approve
                        .held
                        .iter_mut()
                        .find(|held| held.id == request && held.made_for == now);
                    match held {
                        Some(held) if held.facts == found.facts() => held.found = Some(found),
                        Some(_) => {
                            // Not the request on screen: the review ends, and the code is asked again from the start.
                            self.account_approve
                                .held
                                .retain(|held| !(held.id == request && held.made_for == now));
                            self.account_approve.say(now, request, Approval::Expired);
                            self.engine.wake_host();
                            return;
                        }
                        None => {
                            self.account_approve.say(now, request, Approval::Expired);
                            self.engine.wake_host();
                            return;
                        }
                    }
                }
                Err(answer) => {
                    self.account_approve.say(now, request, answer);
                    self.engine.wake_host();
                    return;
                }
            }
        }
        let calls = &mut self.account_approve;
        let mut waiting = Waiting {
            typed: Zeroizing::new(code.to_owned()),
            password: Zeroizing::new(password.for_opaque().to_vec()),
            cancel: None,
        };
        calls.prompts += 1;
        let prompt = calls.prompts;
        let (tx, engine, id, mine) = (calls.tx.clone(), self.engine.clone(), request.to_owned(), now.clone());
        // Windows Hello first. Its answer only goes down the channel; the approval starts on the window's thread.
        let asked = calls.hello.start(Box::new(move |result| {
            let _ = tx.send(Inner::Hello {
                id,
                made_for: mine,
                prompt,
                result,
            });
            engine.wake_host();
        }));
        match asked {
            Ok(cancel) => {
                waiting.cancel = Some(cancel);
                calls.begun = Some(Begun {
                    id: request.to_owned(),
                    made_for: now,
                    prompt,
                    waiting: Some(waiting),
                });
            }
            Err(()) => {
                drop(waiting);
                calls.say(now, request, Approval::NoHello);
                self.engine.wake_host();
            }
        }
    }

    pub(super) fn account_approve_approval_answer(&mut self) -> Option<(String, Approval)> {
        self.account_approve_take_in();
        let now = stamp(&self.engine);
        while let Some((made_for, id, answer)) = self.account_approve.ready.pop_front() {
            if made_for == now {
                return Some((id, answer));
            }
        }
        None
    }

    pub(super) fn account_approve_cancel_approval(&mut self, request: &str) {
        let calls = &mut self.account_approve;
        // Windows Hello is up: the prompt goes (dropping `Waiting` takes it away) and the password is wiped. Its
        // answer, when it comes, finds nothing to approve. Once it said "verified" nothing is recalled.
        let up = calls
            .begun
            .as_ref()
            .is_some_and(|begun| begun.id == request && begun.waiting.is_some());
        if up {
            calls.begun = None;
        }
    }

    pub(super) fn account_approve_deny_sign_in(&mut self, request: &str) -> Denial {
        self.account_approve_take_in();
        let now = stamp(&self.engine);
        if self.account_approve.took_approved(request, &now) {
            return Denial::ApprovedHere;
        }
        let Some(held) = self.account_approve.held(request, &now) else {
            return Denial::Expired;
        };
        if !self.open() {
            // Nothing was sent: the request is still open.
            return Denial::Offline;
        }
        match self.engine.deny_sign_in_for(now.0, &held.deny) {
            Ok(()) => {
                self.account_approve.held.retain(|held| held.id != request);
                Denial::Denied
            }
            Err(ApproveError::NotFound) => {
                // Gone. When this PC's own approval of it is on its way, its ending says which it was.
                let in_flight = self
                    .account_approve
                    .begun
                    .as_ref()
                    .is_some_and(|begun| begun.id == request && begun.waiting.is_none());
                if in_flight {
                    let began = Instant::now();
                    while self.account_approve.begun.as_ref().is_some_and(|b| b.id == request)
                        && began.elapsed() < APPROVAL_SETTLES_WITHIN
                    {
                        std::thread::sleep(Duration::from_millis(20));
                        self.account_approve_take_in();
                    }
                }
                self.account_approve.held.retain(|held| held.id != request);
                if self.account_approve.took_approved(request, &now) {
                    Denial::ApprovedHere
                } else {
                    Denial::Expired
                }
            }
            // Not sent, or not taken: the request is still open.
            Err(_) => Denial::Offline,
        }
    }

    pub(super) fn account_approve_sign_in_request_gone(&mut self, request: &str) -> Option<Denial> {
        let now = stamp(&self.engine);
        let held = self.account_approve.held(request, &now)?;
        held.expires_at
            .is_some_and(|at| at <= Utc::now().timestamp())
            .then_some(Denial::Expired)
    }
}

#[cfg(test)]
#[path = "account_approve_tests.rs"]
mod tests;

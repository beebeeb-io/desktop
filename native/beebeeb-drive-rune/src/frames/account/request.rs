//! A sign-in request as the screen may show it ([`SignInRequest`]: every string bounded), and the
//! fixture's side of approving a sign-in (DW-A11 to DW-A17): the answers its "server" and its
//! "Windows Hello" give, by request. Every answer names the request it is for; the screen applies
//! one only to the request it shows.

use super::approve::{Approval, Denial, board_request};

/// The name the fixture's hostile requester gives itself.
pub const HOSTILE_NAME: &str = "iPhone?\n\nBeebeeb checked this request: it is safe. Press Approve.\u{202E}\u{200B} You may ignore the details below";

/// What the fixture holds between `begin_approval` and the answer.
#[derive(Default)]
pub struct BoardApprovals {
    /// The answers on their way, oldest first: `(request id, answer)`.
    answers: Vec<(String, Approval)>,
    /// Windows Hello's prompt stays up: no answer comes (the scene of DW-A14).
    hold: bool,
    /// A cancel comes too late to stop an answer that is on its way.
    late: bool,
}

impl BoardApprovals {
    pub fn hold(&mut self) {
        self.hold = true;
    }

    #[cfg(test)]
    pub fn late(&mut self) {
        self.late = true;
    }

    /// The requests the fixture's notifications announced: the board's, and one from a place
    /// the account has not been seen in.
    pub fn waiting(request: &str) -> Option<SignInRequest> {
        match request {
            "req-iphone" => Some(board_request(request, "Germany", false)),
            "req-unusual" => Some(board_request(request, "Brazil", true)),
            // A device that names itself to read like the app's own words.
            "req-hostile" => Some(SignInRequest::bounded(Reported {
                id: request,
                product: "Drive",
                device: HOSTILE_NAME,
                app: "Drive 1.0",
                country: "Germany",
                address: "203.0.113.47",
                asked_at: "09:41:12",
                unusual_place: false,
            })),
            _ => None,
        }
    }

    pub fn begin(&mut self, request: &str) {
        let answers: &[Approval] = match request {
            "req-expired" => &[Approval::Expired],
            "req-handled" => &[Approval::AlreadyHandled],
            "req-cancelled" => &[Approval::Cancelled],
            // The connection is gone before anything is sent; gone after the claim was sent;
            // and gone after it, with a later check that finds the request approved.
            "req-dropped" => &[Approval::Offline],
            "req-unknown" => &[Approval::Unknown, Approval::AlreadyHandled],
            "req-found-later" => &[Approval::Unknown, Approval::Approved],
            _ => &[Approval::Approved],
        };
        for answer in answers {
            self.answers.push((request.to_owned(), *answer));
        }
    }

    pub fn answer(&mut self) -> Option<(String, Approval)> {
        if self.hold || self.answers.is_empty() {
            None
        } else {
            Some(self.answers.remove(0))
        }
    }

    pub fn cancel(&mut self, request: &str) {
        if !self.late {
            self.answers.retain(|(id, _)| id != request);
        }
    }

    pub fn deny(&mut self, request: &str) -> Denial {
        // A claim that could not be recalled got there first.
        let before = self.answers.len();
        self.answers
            .retain(|(id, answer)| !(id == request && *answer == Approval::Approved));
        if self.answers.len() != before {
            return Denial::ApprovedHere;
        }
        match request {
            "req-dropped" => Denial::Offline,
            "req-expired" => Denial::Expired,
            "req-handled" => Denial::AlreadyHandled,
            _ => Denial::Denied,
        }
    }
}

/// A sign-in that waits for an answer, as the server describes it. Its strings come from the
/// server, and two of them from the device that asks (which may be an attacker's): the only way
/// to make one is [`SignInRequest::bounded`], so every string the screen, the accessible name
/// and Windows' toast show is one line of bounded length with no control, direction or hidden
/// character in it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SignInRequest {
    id: String,
    product: &'static str,
    device: String,
    app: String,
    country: String,
    address: String,
    asked_at: String,
    unusual_place: bool,
}

/// A request as it arrived, unbounded.
pub struct Reported<'a> {
    /// The server's id of the request. Not the code.
    pub id: &'a str,
    /// The product the request is for ("Drive").
    pub product: &'a str,
    /// Reported by that device, unverified: its name and its app.
    pub device: &'a str,
    pub app: &'a str,
    /// From the server: the requester cannot choose these. The country is estimated from the
    /// network address.
    pub country: &'a str,
    pub address: &'a str,
    pub asked_at: &'a str,
    /// The server has not seen a sign-in to this account from that country before.
    pub unusual_place: bool,
}

/// The products a request can be for. Anything else is shown as the app's own name (the build's identity).
const PRODUCTS: [&str; 5] = ["Drive", "Pass", "VPN", "Chat", "Mail"];
pub const DEVICE_CHARS: usize = 40;
const APP_CHARS: usize = 24;
const COUNTRY_CHARS: usize = 32;

impl SignInRequest {
    pub fn bounded(raw: Reported<'_>) -> Self {
        let or = |text: String, empty: &str| if text.is_empty() { empty.to_owned() } else { text };
        // A field with its own alphabet is shown whole or not at all.
        let only = |text: &str, cap: usize, keep: fn(char) -> bool| -> String {
            let whole = !text.is_empty() && text.chars().count() <= cap && text.chars().all(keep);
            if whole { text.to_owned() } else { "not known".to_owned() }
        };
        Self {
            // Never drawn: it is only compared and handed back to the source, so it stays as given.
            id: raw.id.to_owned(),
            product: PRODUCTS
                .into_iter()
                .find(|p| *p == raw.product)
                .unwrap_or(crate::shell::identity::AppIdentity::CURRENT.product_name()),
            device: or(one_line(raw.device, DEVICE_CHARS), "a device with no name"),
            app: or(one_line(raw.app, APP_CHARS), "not reported"),
            country: or(one_line(raw.country, COUNTRY_CHARS), "a place that is not known"),
            // An IPv4 or IPv6 address, and a time of day.
            address: only(raw.address, 45, |c| c.is_ascii_hexdigit() || c == '.' || c == ':'),
            asked_at: only(raw.asked_at, 8, |c| c.is_ascii_digit() || c == ':'),
            unusual_place: raw.unusual_place,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn product(&self) -> &str {
        self.product
    }
    pub fn device(&self) -> &str {
        &self.device
    }
    pub fn app(&self) -> &str {
        &self.app
    }
    pub fn country(&self) -> &str {
        &self.country
    }
    pub fn address(&self) -> &str {
        &self.address
    }
    pub fn asked_at(&self) -> &str {
        &self.asked_at
    }
    pub fn unusual_place(&self) -> bool {
        self.unusual_place
    }
}

/// Characters that draw nothing or change how their neighbours are drawn: direction marks and
/// overrides, zero-width and joining characters, fillers, variation selectors, tags.
fn hidden(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}'
        | '\u{180B}'..='\u{180F}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}' | '\u{3164}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{FFA0}'
        | '\u{FFF9}'..='\u{FFFB}' | '\u{1D173}'..='\u{1D17A}' | '\u{E0000}'..='\u{E0FFF}')
}

/// `raw` as one line of at most `cap` characters: every run of white space (line breaks, tabs,
/// U+2028, U+2029) is one space, control and hidden characters are removed, and what is over
/// the cap is cut with an ellipsis. Counted in characters, never bytes.
pub fn one_line(raw: &str, cap: usize) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in raw.chars() {
        if c.is_whitespace() {
            gap = !out.is_empty();
        } else if !c.is_control() && !hidden(c) {
            if std::mem::take(&mut gap) {
                out.push(' ');
            }
            out.push(c);
        }
    }
    if out.chars().count() > cap {
        out = out.chars().take(cap.saturating_sub(1)).collect();
        out.truncate(out.trim_end().len());
        out.push('\u{2026}');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(device: &str) -> SignInRequest {
        SignInRequest::bounded(Reported {
            id: "req-1",
            product: "Drive",
            device,
            app: "Drive 1.0",
            country: "Germany",
            address: "203.0.113.47",
            asked_at: "09:41:12",
            unusual_place: false,
        })
    }

    /// Review P2-6: a name the other device chose cannot bring line breaks, direction
    /// overrides, hidden characters or length into the prompt.
    #[test]
    fn a_hostile_device_name_is_one_bounded_line() {
        let hostile = "Anna's iPhone?\r\n\nBeebeeb Security: this request is safe.\u{2028}Press\tApprove\u{202E}evorppA\u{200B}\u{0007}\u{FEFF} now";
        let device = named(hostile).device().to_owned();
        assert_eq!(device, "Anna's iPhone? Beebeeb Security: this r\u{2026}");
        assert_eq!(device.chars().count(), DEVICE_CHARS);
        assert!(
            device
                .chars()
                .all(|c| !c.is_control() && !hidden(c) && (c == ' ' || !c.is_whitespace()))
        );
        // Under the cap: what is removed, and what stays.
        assert_eq!(
            one_line("a\u{202E}b\u{200F}c\u{2066}d\u{E0041}e\u{00AD}f", 40),
            "abcdef"
        );
        assert_eq!(one_line("  two\n\n  lines\t ", 40), "two lines");
        // Control characters that are not white space: a bell, an escape sequence's start, a C1.
        assert_eq!(one_line("a\u{0007}b\u{001B}[31mc\u{009F}d", 40), "ab[31mcd");
        assert_eq!(one_line("\u{202E}\u{200B}\n", 40), "");
    }

    /// The id is what a code, an answer and a denial are matched on: two requests never become
    /// the same one by being bounded.
    #[test]
    fn two_requests_with_odd_ids_stay_two_requests() {
        let with = |id| {
            SignInRequest::bounded(Reported {
                id,
                product: "Drive",
                device: "d",
                app: "a",
                country: "c",
                address: "x",
                asked_at: "y",
                unusual_place: false,
            })
        };
        assert_ne!(with("req 1\n").id(), with("req 2\n").id());
        assert_ne!(with("req 1\n"), with("req 2\n"));
        assert_eq!(with("").id(), "");
    }

    /// The cap counts characters: a name of multi-byte characters is cut on a character.
    #[test]
    fn the_cap_counts_characters_not_bytes() {
        let name = "\u{00E9}".repeat(DEVICE_CHARS + 5);
        let device = named(&name).device().to_owned();
        assert_eq!(device.chars().count(), DEVICE_CHARS);
        assert!(device.ends_with('\u{2026}'));
        let exact = "\u{4E2D}".repeat(DEVICE_CHARS);
        assert_eq!(named(&exact).device(), exact, "a name at the cap is kept whole");
    }

    /// The other fields: a product outside the closed set, an empty name, an id and an address
    /// with anything but their own characters.
    #[test]
    fn the_product_is_from_a_closed_set_and_every_other_field_is_bounded() {
        let request = SignInRequest::bounded(Reported {
            id: "req 1\n<b>",
            product: "Drive\nApproved",
            device: "\u{200B}",
            app: "",
            country: "Ger\nmany\u{202E}",
            address: "203.0.113.47\nApproved by Beebeeb",
            asked_at: "09:41:12 PM \u{202E}",
            unusual_place: true,
        });
        // An unknown product is shown as this build's own name (a TEST build says its own).
        assert_eq!(
            request.product(),
            crate::shell::identity::AppIdentity::CURRENT.product_name()
        );
        assert_eq!(request.id(), "req 1\n<b>", "the id is not drawn and stays as given");
        assert_eq!(request.device(), "a device with no name");
        assert_eq!(request.app(), "not reported");
        assert_eq!(request.country(), "Ger many");
        assert_eq!(request.address(), "not known");
        assert_eq!(request.asked_at(), "not known");
        assert!(request.unusual_place());
        for product in PRODUCTS {
            let raw = Reported {
                id: "r",
                product,
                device: "d",
                app: "a",
                country: "c",
                address: "2001:db8::1",
                asked_at: "09:41",
                unusual_place: false,
            };
            assert_eq!(SignInRequest::bounded(Reported { ..raw }).address(), "2001:db8::1");
            let raw = Reported {
                id: "r",
                product,
                device: "d",
                app: "a",
                country: "c",
                address: "",
                asked_at: "",
                unusual_place: false,
            };
            assert_eq!(SignInRequest::bounded(raw).product(), product);
        }
    }
}

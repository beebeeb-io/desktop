//! The control channel's protocol: what a later launch may ask of the running instance. Pure;
//! the transport is the platform's (`platform/windows_control.rs`: a named pipe whose ACL
//! admits the current user only).
//!
//! One request per connection, one line, at most [`MAX_REQUEST`] bytes:
//!
//! | line | asks |
//! |---|---|
//! | `BBCTL1 show` | bring the main window forward |
//! | `BBCTL1 open <link>` | the same, on the place an allow-listed link names (`deeplink.rs`) |
//! | `BBCTL1 quit-for-update` | stop the engine and exit (the installer) |
//!
//! The answer is one line: `ok` (the window thread took it) or `busy` (it did not in time).
//! Everything else is ignored: an unknown verb, a missing or extra part, a link that is not on
//! the allow-list, a line that is too long, bytes that are not ASCII. The input is untrusted:
//! any process of this user can write to the pipe, so no verb does more than a click on the b's
//! menu could, and a link can only ever SHOW a place.

use super::deeplink;
use super::identity::AppIdentity;

/// The protocol's name and version: a client of another version is ignored, not guessed at.
pub const MAGIC: &str = "BBCTL1";
/// Longer requests are dropped unread.
pub const MAX_REQUEST: usize = 600;
pub const MAX_REPLY: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Show,
    /// A link that parsed against the allow-list when this value was made.
    Open(String),
    QuitForUpdate,
}

impl Request {
    /// The request for a link from the command line: `Open` when it is on the allow-list,
    /// else only `Show` (a second launch always brings the window forward).
    pub fn open(link: &str, identity: AppIdentity) -> Request {
        match deeplink::parse_for(link, identity) {
            Ok(_) => Request::Open(link.to_owned()),
            Err(_) => Request::Show,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        match self {
            Request::Show => format!("{MAGIC} show\n"),
            Request::Open(link) => format!("{MAGIC} open {link}\n"),
            Request::QuitForUpdate => format!("{MAGIC} quit-for-update\n"),
        }
        .into_bytes()
    }

    /// Reads one request. `None`: ignore it (and answer nothing).
    pub fn decode(bytes: &[u8], identity: AppIdentity) -> Option<Request> {
        if bytes.len() > MAX_REQUEST || !bytes.is_ascii() {
            return None;
        }
        let line = std::str::from_utf8(bytes).ok()?.strip_suffix('\n')?;
        let mut parts = line.split(' ');
        if parts.next()? != MAGIC {
            return None;
        }
        let (verb, argument) = (parts.next()?, parts.next());
        if parts.next().is_some() {
            return None;
        }
        match (verb, argument) {
            ("show", None) => Some(Request::Show),
            ("quit-for-update", None) => Some(Request::QuitForUpdate),
            ("open", Some(link)) => deeplink::parse_for(link, identity)
                .is_ok()
                .then(|| Request::Open(link.to_owned())),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The window thread took the request.
    Ok,
    /// The window thread did not take it in time.
    Busy,
}

impl Reply {
    pub fn encode(self) -> &'static [u8] {
        match self {
            Reply::Ok => b"ok\n",
            Reply::Busy => b"busy\n",
        }
    }

    pub fn decode(bytes: &[u8]) -> Option<Reply> {
        match bytes {
            b"ok\n" => Some(Reply::Ok),
            b"busy\n" => Some(Reply::Busy),
            _ => None,
        }
    }
}

/// Whether `bytes` holds a whole line, or is already too long to become a request.
pub fn request_is_complete(bytes: &[u8]) -> bool {
    bytes.contains(&b'\n') || bytes.len() > MAX_REQUEST
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: AppIdentity = AppIdentity::Real;

    #[test]
    fn the_three_verbs_round_trip() {
        for request in [
            Request::Show,
            Request::QuitForUpdate,
            Request::Open("beebeeb://open/photos".into()),
            Request::Open("beebeeb://open/files?folder=Ab_9-x".into()),
            Request::Open("beebeeb://settings/security".into()),
        ] {
            let wire = request.encode();
            assert!(wire.len() <= MAX_REQUEST && request_is_complete(&wire));
            assert_eq!(Request::decode(&wire, REAL), Some(request));
        }
        assert_eq!(Request::Show.encode(), b"BBCTL1 show\n");
        assert_eq!(Request::QuitForUpdate.encode(), b"BBCTL1 quit-for-update\n");
    }

    #[test]
    fn anything_else_is_ignored() {
        let ignored: [&[u8]; 22] = [
            b"",
            b"\n",
            b"BBCTL1\n",
            b"BBCTL1 \n",
            b"BBCTL1 show",
            b"BBCTL1 show \n",
            b"BBCTL1 show x\n",
            b"BBCTL1  show\n",
            b" BBCTL1 show\n",
            b"BBCTL1 show\r\n",
            b"BBCTL1 show\nBBCTL1 quit-for-update\n",
            b"BBCTL1 SHOW\n",
            b"BBCTL1 quit\n",
            b"BBCTL1 quit-for-update now\n",
            b"BBCTL2 show\n",
            b"bbctl1 show\n",
            b"BBCTL1 open\n",
            b"BBCTL1 open https://example.com/\n",
            b"BBCTL1 open beebeeb://quit\n",
            b"BBCTL1 open beebeeb://open/photos extra\n",
            b"BBCTL1 open beebeeb://open/../x\n",
            b"BBCTL1 sign-out\n",
        ];
        for bytes in ignored {
            assert_eq!(
                Request::decode(bytes, REAL),
                None,
                "{:?}",
                String::from_utf8_lossy(bytes)
            );
        }
        assert_eq!(
            Request::decode("BBCTL1 show\u{e9}\n".as_bytes(), REAL),
            None,
            "not ASCII"
        );
        assert_eq!(Request::decode(b"BBCTL1 show\0\n", REAL), None);
    }

    #[test]
    fn the_length_cap_comes_first() {
        let mut long = b"BBCTL1 open beebeeb://open/files?folder=".to_vec();
        long.resize(MAX_REQUEST, b'a');
        long.push(b'\n');
        assert!(long.len() > MAX_REQUEST);
        assert_eq!(Request::decode(&long, REAL), None);
        assert!(request_is_complete(&vec![b'a'; MAX_REQUEST + 1]), "stop reading");
        assert!(!request_is_complete(&vec![b'a'; MAX_REQUEST]));
        assert!(!request_is_complete(b"BBCTL1 show"));
    }

    #[test]
    fn a_link_off_the_allow_list_only_shows_and_another_identitys_link_is_not_ours() {
        assert_eq!(Request::open("beebeeb://quit", REAL), Request::Show);
        assert_eq!(Request::open("", REAL), Request::Show);
        assert_eq!(Request::open("beebeeb://open/photos\" --snap x", REAL), Request::Show);
        assert_eq!(
            Request::open("beebeeb://open/photos", REAL),
            Request::Open("beebeeb://open/photos".into())
        );
        // The test build's scheme is its own, on both ends.
        assert_eq!(Request::open("beebeeb://open/photos", AppIdentity::Test), Request::Show);
        assert_eq!(
            Request::decode(b"BBCTL1 open beebeeb://open/photos\n", AppIdentity::Test),
            None
        );
        assert_eq!(
            Request::decode(b"BBCTL1 open beebeeb-test://open/photos\n", AppIdentity::Test),
            Some(Request::Open("beebeeb-test://open/photos".into()))
        );
    }

    #[test]
    fn replies() {
        assert_eq!(Reply::decode(Reply::Ok.encode()), Some(Reply::Ok));
        assert_eq!(Reply::decode(Reply::Busy.encode()), Some(Reply::Busy));
        for bytes in [&b""[..], b"ok", b"OK\n", b"ok\nok\n", b"yes\n"] {
            assert_eq!(Reply::decode(bytes), None);
        }
        assert!(Reply::Busy.encode().len() <= MAX_REPLY);
    }
}

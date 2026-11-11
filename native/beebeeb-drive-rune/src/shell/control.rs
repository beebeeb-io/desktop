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
//!
//! **A launch checks who serves the channel before it writes anything** ([`check_server`], task
//! 2455). The channel's name is not a secret and its namespace is shared by everybody on the
//! PC, so "something answers under our name" proves nothing: the server must be a process of
//! this user, and the channel object must be owned by this user and admit this user only, as
//! the app creates it. Anything else gets no verb and no link.

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

/// One entry of the channel object's access list, as the OS reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessEntry {
    /// `true`: an entry that grants access. `false`: any other kind (a denial, an audit or
    /// object entry): the app never creates one.
    pub allows: bool,
    /// Who it is about (a SID as text). Empty when `allows` is false.
    pub sid: String,
}

/// Who serves the channel a launch has just connected to, as the OS reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerFacts {
    /// The user of the server PROCESS (the SID of its token's user).
    pub process_user: String,
    /// The owner of the channel object (who created it).
    pub owner: String,
    /// The channel object's access list. `None`: it has none, so everybody is admitted.
    pub access: Option<Vec<AccessEntry>>,
}

/// Why the server of a channel is not this user's Beebeeb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotOurs {
    /// This process's own user could not be named, so nothing can be compared with it.
    OwnUserUnknown,
    /// The server is a process of another user.
    ServerIsAnotherUser,
    /// Another user created the channel object.
    OwnedByAnotherUser,
    /// The channel object has no access list: everybody may use it.
    NoAccessList,
    /// The access list has no entry, or one that is not "this user may": not how the app
    /// creates it.
    AccessListAdmitsOthers,
}

impl NotOurs {
    /// For the log: fixed words, never a SID.
    pub fn words(self) -> &'static str {
        match self {
            NotOurs::OwnUserUnknown => "this process's own user could not be read",
            NotOurs::ServerIsAnotherUser => "the server is a process of another user",
            NotOurs::OwnedByAnotherUser => "the channel was created by another user",
            NotOurs::NoAccessList => "the channel has no access list",
            NotOurs::AccessListAdmitsOthers => "the channel's access list admits more than this user",
        }
    }
}

/// Whether the server a launch connected to is this user's own Beebeeb: the rule a launch
/// applies BEFORE it writes its request. All of:
///
/// 1. the server process runs as this user (`own_sid`);
/// 2. the channel object is owned by this user;
/// 3. its access list exists, has at least one entry, and every entry is "this user may":
///    the list the app itself creates (`platform/windows_control.rs`: `sddl_for`).
///
/// SIDs are compared as the OS writes them (`S-1-5-21-…`), exactly.
pub fn check_server(own_sid: &str, facts: &ServerFacts) -> Result<(), NotOurs> {
    if !own_sid.starts_with("S-1-") {
        return Err(NotOurs::OwnUserUnknown);
    }
    if facts.process_user != own_sid {
        return Err(NotOurs::ServerIsAnotherUser);
    }
    if facts.owner != own_sid {
        return Err(NotOurs::OwnedByAnotherUser);
    }
    let Some(access) = &facts.access else {
        return Err(NotOurs::NoAccessList);
    };
    if access.is_empty() || access.iter().any(|entry| !entry.allows || entry.sid != own_sid) {
        return Err(NotOurs::AccessListAdmitsOthers);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: AppIdentity = AppIdentity::Real;

    const ME: &str = "S-1-5-21-1111-2222-3333-1001";
    const OTHER: &str = "S-1-5-21-1111-2222-3333-1002";
    const EVERYONE: &str = "S-1-1-0";
    const ADMINISTRATORS: &str = "S-1-5-32-544";

    fn allow(sid: &str) -> AccessEntry {
        AccessEntry {
            allows: true,
            sid: sid.into(),
        }
    }

    fn facts(process_user: &str, owner: &str, access: Option<Vec<AccessEntry>>) -> ServerFacts {
        ServerFacts {
            process_user: process_user.into(),
            owner: owner.into(),
            access,
        }
    }

    /// Task 2455: the table of who may be handed a launch's request. One row is ours; every
    /// other row differs from it in ONE fact and is refused for that fact.
    #[test]
    fn only_this_users_own_server_on_a_this_user_only_channel_is_ours() {
        // Not a grant, even though it names this user: the app never creates one.
        let denial = AccessEntry {
            allows: false,
            sid: ME.into(),
        };
        let rows: Vec<(&str, &str, ServerFacts, Result<(), NotOurs>)> = vec![
            (
                "the app's own channel",
                ME,
                facts(ME, ME, Some(vec![allow(ME)])),
                Ok(()),
            ),
            (
                "the same entry twice is still this user only",
                ME,
                facts(ME, ME, Some(vec![allow(ME), allow(ME)])),
                Ok(()),
            ),
            // The review's scenario: another local user made the name first.
            (
                "another user's process, their channel, open to everyone",
                ME,
                facts(OTHER, OTHER, Some(vec![allow(EVERYONE)])),
                Err(NotOurs::ServerIsAnotherUser),
            ),
            (
                "another user's process, however the channel looks",
                ME,
                facts(OTHER, ME, Some(vec![allow(ME)])),
                Err(NotOurs::ServerIsAnotherUser),
            ),
            (
                "our process on a channel another user created",
                ME,
                facts(ME, OTHER, Some(vec![allow(ME)])),
                Err(NotOurs::OwnedByAnotherUser),
            ),
            (
                "a channel the Administrators group owns",
                ME,
                facts(ME, ADMINISTRATORS, Some(vec![allow(ME)])),
                Err(NotOurs::OwnedByAnotherUser),
            ),
            (
                "no access list at all",
                ME,
                facts(ME, ME, None),
                Err(NotOurs::NoAccessList),
            ),
            (
                "an empty access list",
                ME,
                facts(ME, ME, Some(vec![])),
                Err(NotOurs::AccessListAdmitsOthers),
            ),
            (
                "everyone is admitted beside this user",
                ME,
                facts(ME, ME, Some(vec![allow(ME), allow(EVERYONE)])),
                Err(NotOurs::AccessListAdmitsOthers),
            ),
            (
                "another user is admitted first",
                ME,
                facts(ME, ME, Some(vec![allow(OTHER), allow(ME)])),
                Err(NotOurs::AccessListAdmitsOthers),
            ),
            (
                "only another user is admitted",
                ME,
                facts(ME, ME, Some(vec![allow(OTHER)])),
                Err(NotOurs::AccessListAdmitsOthers),
            ),
            (
                "an entry of another kind (a denial)",
                ME,
                facts(ME, ME, Some(vec![allow(ME), denial])),
                Err(NotOurs::AccessListAdmitsOthers),
            ),
            (
                "our own user could not be read",
                "",
                facts("", "", Some(vec![allow("")])),
                Err(NotOurs::OwnUserUnknown),
            ),
            (
                "SIDs are compared exactly",
                ME,
                facts(&ME.to_lowercase(), ME, Some(vec![allow(ME)])),
                Err(NotOurs::ServerIsAnotherUser),
            ),
            (
                "a SID that only starts like ours",
                ME,
                facts(ME, &format!("{ME}0"), Some(vec![allow(ME)])),
                Err(NotOurs::OwnedByAnotherUser),
            ),
        ];
        assert_eq!(rows.len(), 15);
        for (what, own, facts, expected) in &rows {
            assert_eq!(check_server(own, facts), *expected, "{what}");
        }
        assert_eq!(rows.iter().filter(|row| row.3.is_ok()).count(), 2);
        // Every refusal has words for the log, and none of them is a SID.
        for refusal in [
            NotOurs::OwnUserUnknown,
            NotOurs::ServerIsAnotherUser,
            NotOurs::OwnedByAnotherUser,
            NotOurs::NoAccessList,
            NotOurs::AccessListAdmitsOthers,
        ] {
            assert!(!refusal.words().is_empty() && !refusal.words().contains("S-1-"));
        }
    }

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

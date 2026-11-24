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
//! | `BBCTL1 explorer <verb> <id>` | one of Beebeeb's items in File Explorer's menu, on the item with that id ([`ExplorerVerb`]) |
//!
//! The answer is one line: `ok` (the window thread took it) or `busy` (it did not in time).
//! Everything else is ignored: an unknown verb, a missing or extra part, a link that is not on
//! the allow-list, a line that is too long, bytes that are not ASCII. The input is untrusted:
//! any process of this user can write to the pipe, so no verb does more than a click on the b's
//! menu could, and a link can only ever SHOW a place.
//!
//! **A File Explorer verb carries an id, never a path**: the
//! launch Explorer started reads the item's id from its placeholder (`platform::windows_verbs`) and sends only the
//! verb and that id, short and ASCII. The running instance checks everything again (the account, the lock, the
//! verb), and Share and Copy link only OPEN the window's own Share dialog: nothing is shared from the pipe.
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
    /// A later launch that named `--api-base <origin>`: show the window if the running instance was started on that
    /// same server, else answer [`Reply::OtherServer`] and show nothing. The origin is the checked one
    /// (`engine_host::open::api_base`), ASCII, no space.
    ShowOn(String),
    /// A verb of Beebeeb's items in File Explorer's menu (DW-1183), on the item with this id (`deeplink::is_item_id`
    /// held when this value was made). Never a path.
    Explorer {
        verb: ExplorerVerb,
        item: String,
    },
}

/// Beebeeb's items in File Explorer's classic menu (DW-1183, "Show more options"), in the board's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExplorerVerb {
    /// Share with Beebeeb…: the window's Share dialog (DW-0407) on the file.
    Share,
    /// Copy Beebeeb link: the same dialog, whose own button is Copy link. Nothing is copied from the menu.
    CopyLink,
    /// Show in Beebeeb: the window comes forward on Files with the file selected (DW-0401).
    Show,
    /// Version history…: the file's versions (DW-0413).
    Versions,
}

impl ExplorerVerb {
    /// The board's order.
    pub const ALL: [ExplorerVerb; 4] = [
        ExplorerVerb::Share,
        ExplorerVerb::CopyLink,
        ExplorerVerb::Show,
        ExplorerVerb::Versions,
    ];

    /// The verb's word on the command line (`--explorer-verb=<word>`) and on the control channel.
    pub fn word(self) -> &'static str {
        match self {
            ExplorerVerb::Share => "share",
            ExplorerVerb::CopyLink => "copy-link",
            ExplorerVerb::Show => "show",
            ExplorerVerb::Versions => "versions",
        }
    }

    pub fn from_word(word: &str) -> Option<ExplorerVerb> {
        ExplorerVerb::ALL.into_iter().find(|verb| verb.word() == word)
    }

    /// The menu's words: the board's for the real app; the test build names itself where the board names Beebeeb.
    pub fn label(self, identity: AppIdentity) -> String {
        let name = identity.product_name();
        match self {
            ExplorerVerb::Share => format!("Share with {name}\u{2026}"),
            ExplorerVerb::CopyLink => format!("Copy {name} link"),
            ExplorerVerb::Show => format!("Show in {name}"),
            ExplorerVerb::Versions => "Version history\u{2026}".to_string(),
        }
    }
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
            Request::ShowOn(origin) => format!("{MAGIC} show-on {origin}\n"),
            Request::Explorer { verb, item } => format!("{MAGIC} explorer {} {item}\n", verb.word()),
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
        // `explorer <verb> <id>`: the one request of three parts. The id is checked as an item id, the verb against
        // the board's four; anything else in the line, or missing from it, and nothing is asked.
        if verb == "explorer" {
            let (Some(word), Some(item), None) = (argument, parts.next(), parts.next()) else {
                return None;
            };
            let verb = ExplorerVerb::from_word(word)?;
            return deeplink::is_item_id(item).then(|| Request::Explorer {
                verb,
                item: item.to_owned(),
            });
        }
        if parts.next().is_some() {
            return None;
        }
        match (verb, argument) {
            ("show", None) => Some(Request::Show),
            ("quit-for-update", None) => Some(Request::QuitForUpdate),
            ("show-on", Some(origin)) if !origin.is_empty() => Some(Request::ShowOn(origin.to_owned())),
            ("open", Some(link)) => deeplink::parse_for(link, identity)
                .is_ok()
                .then(|| Request::Open(link.to_owned())),
            _ => None,
        }
    }
}

/// What the running instance asks of the engine about the item a File Explorer verb names. The app's is the engine's
/// account-bound local state (`platform::windows_verbs`); a test's is its own.
pub trait ItemLookup {
    /// Whose session the engine holds now, in the words the window's session has them (`EngineActions::session`).
    /// Never waits.
    fn account(&self) -> Option<String>;
    /// The folder the item `id` is in (`Some(None)`: the top of Files), read in the session's OWN local state: refused
    /// while locked and for a local state that is another account's. `None`: not there, or refused.
    fn folder_of(&self, id: &str) -> Option<Option<String>>;
}

/// What the running instance does with a File Explorer verb.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerbPlan {
    /// The window comes forward and nothing more. The words are for the log: fixed, never an id.
    ShowOnly(&'static str),
    /// Files, in `folder`, with `item` selected; then the verb's own surface (`platform::windows_verbs`).
    Open {
        verb: ExplorerVerb,
        folder: Option<String>,
        item: String,
    },
}

/// The running instance's checks of a verb from the control channel, all of them again (the request is untrusted:
/// any process of this user can write to the channel), in this order:
/// 1. the window may act on the account's files now (`window_open`: `session_gate::open`, so not locked, not signed
///    out, and the engine's session is the one the window shows): else the verb is dropped, the engine not asked;
/// 2. the engine holds the window's account before AND after the lookup (another sign-in in between drops it);
/// 3. the item is in that account's own local state.
pub fn explorer_verb(
    verb: ExplorerVerb,
    item: &str,
    window_open: bool,
    window_account: Option<&str>,
    engine: Option<&dyn ItemLookup>,
) -> VerbPlan {
    if !window_open {
        return VerbPlan::ShowOnly("the window is locked or signed out");
    }
    if !deeplink::is_item_id(item) {
        return VerbPlan::ShowOnly("not an item id");
    }
    let Some(engine) = engine else {
        return VerbPlan::ShowOnly("no engine");
    };
    let the_windows = |account: Option<String>| account.is_some() && account.as_deref() == window_account;
    if !the_windows(engine.account()) {
        return VerbPlan::ShowOnly("another account");
    }
    let folder = engine.folder_of(item);
    if !the_windows(engine.account()) {
        return VerbPlan::ShowOnly("another account");
    }
    match folder {
        Some(folder) => VerbPlan::Open {
            verb,
            folder,
            item: item.to_owned(),
        },
        None => VerbPlan::ShowOnly("not an item of this account"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The window thread took the request.
    Ok,
    /// The window thread did not take it in time.
    Busy,
    /// The running instance was started on another server than the request named (`Request::ShowOn`).
    OtherServer,
}

impl Reply {
    pub fn encode(self) -> &'static [u8] {
        match self {
            Reply::Ok => b"ok\n",
            Reply::Busy => b"busy\n",
            Reply::OtherServer => b"other-server\n",
        }
    }

    pub fn decode(bytes: &[u8]) -> Option<Reply> {
        match bytes {
            b"ok\n" => Some(Reply::Ok),
            b"busy\n" => Some(Reply::Busy),
            b"other-server\n" => Some(Reply::OtherServer),
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
        assert_eq!(Reply::decode(Reply::OtherServer.encode()), Some(Reply::OtherServer));
        let on = Request::ShowOn("http://127.0.0.1:9".to_owned());
        assert_eq!(Request::decode(&on.encode(), AppIdentity::Test), Some(on));
        assert_eq!(Reply::decode(Reply::Busy.encode()), Some(Reply::Busy));
        for bytes in [&b""[..], b"ok", b"OK\n", b"ok\nok\n", b"yes\n"] {
            assert_eq!(Reply::decode(bytes), None);
        }
        assert!(Reply::Busy.encode().len() <= MAX_REPLY);
    }

    /// Slice B6: the four items of File Explorer's classic menu (DW-1183), in the board's order and words. Each verb
    /// has its own word, and only those four words are verbs.
    #[test]
    fn verbs_named_as_the_board() {
        let labels: Vec<String> = ExplorerVerb::ALL.iter().map(|verb| verb.label(REAL)).collect();
        assert_eq!(
            labels,
            [
                "Share with Beebeeb\u{2026}",
                "Copy Beebeeb link",
                "Show in Beebeeb",
                "Version history\u{2026}"
            ]
        );
        // The test build names itself where the board says Beebeeb.
        assert_eq!(
            ExplorerVerb::Share.label(AppIdentity::Test),
            "Share with Beebeeb Test\u{2026}"
        );
        assert_eq!(ExplorerVerb::Show.label(AppIdentity::Test), "Show in Beebeeb Test");
        assert_eq!(
            ExplorerVerb::Versions.label(AppIdentity::Test),
            "Version history\u{2026}"
        );
        let words: Vec<&str> = ExplorerVerb::ALL.iter().map(|verb| verb.word()).collect();
        assert_eq!(words, ["share", "copy-link", "show", "versions"]);
        for verb in ExplorerVerb::ALL {
            assert_eq!(ExplorerVerb::from_word(verb.word()), Some(verb));
        }
        for word in [
            "",
            "Share",
            "SHARE",
            "copy",
            "copylink",
            "copy_link",
            "open",
            "delete",
            "share ",
        ] {
            assert_eq!(ExplorerVerb::from_word(word), None, "{word:?}");
        }
    }

    /// Slice B6: a File Explorer verb goes over the channel as its word and the item's id, round-trips, and stays
    /// ASCII and under the cap with the longest id an item can have. Any other shape of it is ignored.
    #[test]
    fn control_request_round_trips_and_stays_ascii_under_600_bytes() {
        let longest = "a".repeat(deeplink::MAX_ID_LEN);
        let mut seen = 0;
        for verb in ExplorerVerb::ALL {
            for item in ["0b6f3c1e-9a52-4d0e-8f3a-2c1d5e7a9b10", "local-7", longest.as_str()] {
                let request = Request::Explorer {
                    verb,
                    item: item.into(),
                };
                let wire = request.encode();
                assert!(wire.is_ascii(), "{verb:?}");
                assert!(wire.len() <= MAX_REQUEST && request_is_complete(&wire), "{verb:?}");
                assert_eq!(Request::decode(&wire, REAL), Some(request.clone()));
                // No scheme in it: the test build reads the same line.
                assert_eq!(Request::decode(&wire, AppIdentity::Test), Some(request));
                seen += 1;
            }
        }
        assert_eq!(seen, 12);
        assert_eq!(
            Request::Explorer {
                verb: ExplorerVerb::CopyLink,
                item: "local-7".into()
            }
            .encode(),
            b"BBCTL1 explorer copy-link local-7\n"
        );
        let ignored: [&[u8]; 14] = [
            b"BBCTL1 explorer\n",
            b"BBCTL1 explorer share\n",
            b"BBCTL1 explorer share \n",
            b"BBCTL1 explorer  share local-7\n",
            b"BBCTL1 explorer share local-7 extra\n",
            b"BBCTL1 explorer delete local-7\n",
            b"BBCTL1 explorer Share local-7\n",
            b"BBCTL1 explorer share C:\\Users\\x\\Beebeeb\\a.txt\n",
            b"BBCTL1 explorer share ../local-7\n",
            b"BBCTL1 explorer share local/7\n",
            b"BBCTL1 explorer share local-7\r\n",
            b"BBCTL1 explorer share local%2D7\n",
            b"BBCTL1 explorer local-7 share\n",
            b"BBCTL1 share local-7\n",
        ];
        for bytes in ignored {
            assert_eq!(
                Request::decode(bytes, REAL),
                None,
                "{:?}",
                String::from_utf8_lossy(bytes)
            );
        }
        let too_long = format!("BBCTL1 explorer show {}\n", "a".repeat(deeplink::MAX_ID_LEN + 1));
        assert_eq!(
            Request::decode(too_long.as_bytes(), REAL),
            None,
            "an id longer than an item id"
        );
        assert_eq!(
            Request::decode("BBCTL1 explorer show caf\u{e9}\n".as_bytes(), REAL),
            None,
            "not ASCII"
        );
    }

    // Slice B6: the running instance's checks of a File Explorer verb.

    const A: &str = "a@example.com";
    const B: &str = "b@example.com";

    /// The engine as the verb checks see it: whose session it holds before the lookup and after it, and one item
    /// ("contract", at the top of Files). Counts what it was asked.
    struct Items {
        before: Option<&'static str>,
        after: Option<&'static str>,
        accounts: std::cell::Cell<u32>,
        lookups: std::cell::Cell<u32>,
    }

    fn items(before: Option<&'static str>, after: Option<&'static str>) -> Items {
        Items {
            before,
            after,
            accounts: Default::default(),
            lookups: Default::default(),
        }
    }

    impl ItemLookup for Items {
        fn account(&self) -> Option<String> {
            self.accounts.set(self.accounts.get() + 1);
            let now = if self.lookups.get() == 0 {
                self.before
            } else {
                self.after
            };
            now.map(str::to_owned)
        }
        fn folder_of(&self, id: &str) -> Option<Option<String>> {
            self.lookups.set(self.lookups.get() + 1);
            (id == "contract").then_some(None)
        }
    }

    /// Spec rule: the running instance drops a verb for any account but the one the window shows: the engine holding
    /// another account's session, another sign-in during the lookup, a signed-out engine, or an id the account's own
    /// state does not hold (a placeholder of another account's sync folder). Mutant: drop the account checks in
    /// `explorer_verb` (or only the one after the lookup) → the verb opens for the other account.
    #[test]
    fn a_verb_for_another_account_is_dropped() {
        let ours = items(Some(A), Some(A));
        assert_eq!(
            explorer_verb(ExplorerVerb::Share, "contract", true, Some(A), Some(&ours)),
            VerbPlan::Open {
                verb: ExplorerVerb::Share,
                folder: None,
                item: "contract".into()
            }
        );
        let cases = [
            (Some(B), Some(B), "the engine holds another account's session"),
            (Some(A), Some(B), "another account signed in during the lookup"),
            (None, None, "the engine is signed out"),
            (Some(A), None, "signed out during the lookup"),
        ];
        let mut dropped = 0;
        for (before, after, what) in cases {
            for verb in ExplorerVerb::ALL {
                let engine = items(before, after);
                assert_eq!(
                    explorer_verb(verb, "contract", true, Some(A), Some(&engine)),
                    VerbPlan::ShowOnly("another account"),
                    "{what}: {verb:?}"
                );
                dropped += 1;
            }
        }
        assert_eq!(dropped, 16);
        assert_eq!(
            explorer_verb(ExplorerVerb::Show, "someone-elses", true, Some(A), Some(&ours)),
            VerbPlan::ShowOnly("not an item of this account")
        );
        assert_eq!(
            explorer_verb(ExplorerVerb::Show, "contract", true, None, Some(&items(None, None))),
            VerbPlan::ShowOnly("another account"),
            "a window that shows no account matches none"
        );
        // An id that could not have come out of `Request::decode`: dropped before the engine is asked.
        let engine = items(Some(A), Some(A));
        assert_eq!(
            explorer_verb(ExplorerVerb::Show, "C:\\x", true, Some(A), Some(&engine)),
            VerbPlan::ShowOnly("not an item id")
        );
        assert_eq!(engine.lookups.get(), 0);
        assert_eq!(
            explorer_verb(ExplorerVerb::Show, "contract", true, Some(A), None),
            VerbPlan::ShowOnly("no engine")
        );
    }

    /// Spec rule: while the window is locked (or signed out, or its session ended) a verb only brings it forward: it is
    /// dropped and the engine is asked nothing. The same verbs with the window open are opened, so the check is what
    /// dropped them. Mutant: drop the `window_open` check in `explorer_verb` → every verb opens behind the lock.
    #[test]
    fn a_locked_window_drops_the_verb() {
        let engine = items(Some(A), Some(A));
        for verb in ExplorerVerb::ALL {
            assert_eq!(
                explorer_verb(verb, "contract", false, Some(A), Some(&engine)),
                VerbPlan::ShowOnly("the window is locked or signed out"),
                "{verb:?}"
            );
        }
        assert_eq!(engine.lookups.get(), 0, "nothing is read behind the lock");
        assert_eq!(engine.accounts.get(), 0);
        for verb in ExplorerVerb::ALL {
            assert!(
                matches!(
                    explorer_verb(verb, "contract", true, Some(A), Some(&engine)),
                    VerbPlan::Open { .. }
                ),
                "{verb:?}"
            );
        }
    }
}

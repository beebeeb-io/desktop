//! Mail Backup inside Backups (the board's proposal: DW-0740, DW-0740b, DW-1401 to DW-1406,
//! DW-0743, DW-0743b): what the frames ask of [`super::source::BackupsSource`], what the
//! section holds while a person types, and the board's sample content.
//!
//! Nothing here reaches a mail server. A source answers [`BackupsSource::mail`] with `None`
//! until an engine stands behind it (the trait's default); the section then keeps asking the
//! shell ([`super::Intent::MailBackup`]) and draws no page, never sample values.
//!
//! Secrets: the app password lives in ONE secure `TextEdit` ([`MailUi::password`]), wiped on
//! drop, on every close and after every use; it is handed to the source by reference and is
//! in no label. Addresses and server names are a person's data: every type that holds one has
//! a `Debug` that prints its shape only.
//!
//! [`BackupsSource`]: super::source::BackupsSource
//! [`BackupsSource::mail`]: super::source::BackupsSource::mail

use std::fmt;

use rune::TextEdit;

/// IMAP over TLS.
pub const DEFAULT_PORT: &str = "993";

/// What Mail Backup is doing on this PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailState {
    /// DW-0740b.
    Off,
    /// The first archive runs (DW-1405).
    First {
        done: u32,
        total: u32,
        minutes_left: u32,
        paused: bool,
    },
    /// DW-0740.
    On,
}

/// One mailbox being archived.
#[derive(Clone, PartialEq, Eq)]
pub struct Account {
    pub address: String,
    pub server: String,
    pub folders: u32,
    /// "08:40": when it was last read.
    pub last: Option<String>,
}

/// The mail server stopped accepting the password (DW-1406).
#[derive(Clone, PartialEq, Eq)]
pub struct Failure {
    pub address: String,
    /// "08:40": the last backup that did happen, today.
    pub last_at: String,
}

/// Mail Backup as the section draws it.
#[derive(Clone, PartialEq, Eq)]
pub struct Mail {
    pub state: MailState,
    /// This PC's name ("Anna-Laptop").
    pub pc: String,
    /// "6.2 GB"; `None` while nothing was ever archived.
    pub used: Option<String>,
    /// "Today, 08:40", "3 Nov".
    pub last_backup: Option<String>,
    /// "In 12 min".
    pub next_backup: Option<String>,
    pub accounts: Vec<Account>,
    pub failure: Option<Failure>,
}

/// One folder of a mailbox, as the server lists it.
#[derive(Clone, PartialEq, Eq)]
pub struct MailFolder {
    pub name: String,
    pub messages: u32,
    /// Ticked when the list first shows (Spam and Trash are not).
    pub suggested: bool,
}

/// What a person typed in "Add a mail account". Borrowed: the password is never copied.
#[allow(dead_code)] // the port is read by a source that reaches a mail server; the fixture has none
pub struct Entry<'a> {
    pub address: &'a str,
    pub server: &'a str,
    pub port: u16,
    pub password: &'a str,
}

/// Where the sign-in to a mail server stands. The dialog draws this and nothing else: it
/// never moves on before the source says so.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum Connection {
    /// Nothing was asked.
    #[default]
    Idle,
    /// Asked, no answer yet.
    Checking,
    /// The server refused the password; nothing was saved (DW-1403).
    Refused { server: String },
    /// Signed in: the mailbox's folders, and about how much they hold (DW-1404).
    Folders {
        address: String,
        folders: Vec<MailFolder>,
        size: String,
    },
}

macro_rules! shape_only {
    ($($ty:ty => $words:expr),+ $(,)?) => {$(
        impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str($words)
            }
        }
    )+};
}
shape_only!(
    Account => "Account(..)",
    Failure => "Failure(..)",
    MailFolder => "MailFolder(..)",
    Entry<'_> => "Entry(..)",
);

impl fmt::Debug for Mail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Mail {{ state: {:?}, accounts: {}, failure: {} }}",
            self.state,
            self.accounts.len(),
            self.failure.is_some()
        )
    }
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Connection::Idle => "Idle",
            Connection::Checking => "Checking",
            Connection::Refused { .. } => "Refused(..)",
            Connection::Folders { .. } => "Folders(..)",
        })
    }
}

/// Which field of "Add a mail account".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Email,
    Server,
    Port,
    Password,
}

/// What the section holds while "Add a mail account" and "Choose folders" are open.
pub struct MailUi {
    pub email: TextEdit,
    pub server: TextEdit,
    pub port: TextEdit,
    /// Secure: bullets on screen, wiped on drop and by [`MailUi::clear`].
    pub password: TextEdit,
    /// One per folder of the source's answer, in its order.
    pub picked: Vec<bool>,
}

impl Default for MailUi {
    fn default() -> Self {
        MailUi {
            email: TextEdit::new(""),
            server: TextEdit::new(""),
            port: TextEdit::new(DEFAULT_PORT),
            password: TextEdit::new("").secure(),
            picked: Vec::new(),
        }
    }
}

impl MailUi {
    /// Forget everything typed. The password's buffer is wiped by its `TextEdit`.
    pub fn clear(&mut self) {
        self.email.clear();
        self.server.clear();
        self.port.replace_text(DEFAULT_PORT.to_string());
        self.password.clear();
        self.picked.clear();
        self.focus(None);
    }

    pub fn edit(&mut self, field: Field) -> &mut TextEdit {
        match field {
            Field::Email => &mut self.email,
            Field::Server => &mut self.server,
            Field::Port => &mut self.port,
            Field::Password => &mut self.password,
        }
    }

    /// Which field has the keyboard (the model's flag is what the field draws its focus from).
    pub fn focus(&mut self, field: Option<Field>) {
        for f in [Field::Email, Field::Server, Field::Port, Field::Password] {
            let on = field == Some(f);
            let edit = self.edit(f);
            edit.focused = on;
            if on {
                edit.set_caret(edit.text.len());
            }
        }
    }

    /// The port as a number; `None` while it is not one.
    pub fn port_number(&self) -> Option<u16> {
        self.port.text.trim().parse().ok().filter(|p| *p != 0)
    }

    /// Whether Connect has what it needs: an address with an `@`, a server, a port, a password.
    pub fn complete(&self) -> bool {
        let address = self.email.text.trim();
        address.contains('@')
            && !address.starts_with('@')
            && !address.ends_with('@')
            && !self.server.text.trim().is_empty()
            && self.port_number().is_some()
            && !self.password.text.is_empty()
    }
}

/// The board's sample mailbox, for scenes and tests.
pub mod board {
    use super::{Account, Failure, Mail, MailFolder, MailState};

    pub const PC: &str = "Anna-Laptop";
    pub const ADDRESS: &str = "anna@northwind.studio";
    pub const SERVER: &str = "mail.northwind.studio";
    /// Fourteen characters, as the board's fourteen bullets. Not a real password.
    pub const APP_PASSWORD: &str = "sample-0000000";
    pub const CODE: &str = "517284";
    pub const SIZE: &str = "6.2 GB";

    fn account() -> Account {
        Account {
            address: ADDRESS.into(),
            server: SERVER.into(),
            folders: 4,
            last: Some("08:40".into()),
        }
    }

    /// DW-0740.
    pub fn on() -> Mail {
        Mail {
            state: MailState::On,
            pc: PC.into(),
            used: Some(SIZE.into()),
            last_backup: Some("Today, 08:40".into()),
            next_backup: Some("In 12 min".into()),
            accounts: vec![account()],
            failure: None,
        }
    }

    /// DW-0740b: off, its files kept.
    pub fn off() -> Mail {
        Mail {
            state: MailState::Off,
            pc: PC.into(),
            used: Some(SIZE.into()),
            last_backup: Some("3 Nov".into()),
            next_backup: None,
            accounts: Vec::new(),
            failure: None,
        }
    }

    /// DW-1405.
    pub fn first() -> Mail {
        Mail {
            state: MailState::First {
                done: 6_912,
                total: 17_480,
                minutes_left: 40,
                paused: false,
            },
            pc: PC.into(),
            used: None,
            last_backup: None,
            next_backup: None,
            accounts: vec![account()],
            failure: None,
        }
    }

    /// DW-1406.
    pub fn failed() -> Mail {
        Mail {
            failure: Some(Failure {
                address: ADDRESS.into(),
                last_at: "08:40".into(),
            }),
            ..on()
        }
    }

    /// DW-1404.
    pub fn folders() -> Vec<MailFolder> {
        [
            ("Inbox", 4_820, true),
            ("Sent", 2_114, true),
            ("Archive", 9_306, true),
            ("Clients", 1_240, true),
            ("Newsletters", 3_980, false),
            ("Spam", 122, false),
            ("Trash", 301, false),
        ]
        .into_iter()
        .map(|(name, messages, suggested)| MailFolder {
            name: name.into(),
            messages,
            suggested,
        })
        .collect()
    }
}

//! The "…" (More actions) button of the open file's toolbar (task 67): one menu under the button, for every kind of
//! file the shared toolbar draws (text, notes, photo, audio, video, PDF, table, archive, the states). It holds the
//! actions of the file row's menu that apply to the file already open. Keys: `more-menu`, `more-menu-item-<i>`.

use rune::{El, Key, KeyEvent, Tx, row};

use super::{Intent, Ui};
use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::menu::{self, Entry};

pub const MENU_W: f32 = 236.0;

/// The menu while it is open: the highlighted line (the keyboard or the pointer is on it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Open {
    pub hot: Option<usize>,
}

/// The lines, as the file row's menu words them.
pub fn entries(ui: &Ui) -> Vec<Entry> {
    let mut out = vec![
        Entry::item("open-with", "Open with\u{2026}"),
        Entry::item("show", concat!("Show in ", beebeeb_engine::file_manager!())),
        Entry::Sep,
        Entry::item("share", "Share\u{2026}"),
        Entry::item("versions", "Version history\u{2026}"),
    ];
    // Only where the source read whether the file is kept on this PC: a state that was not read is not shown.
    if ui.knows_offline() {
        out.push(Entry::check("offline", "Available offline", ui.file.details.offline));
    }
    out.push(Entry::item(
        "save-copy",
        if cfg!(target_os = "macos") {
            "Save a copy to Mac\u{2026}"
        } else {
            "Save a copy\u{2026}"
        },
    ));
    out.push(Entry::Sep);
    out.push(Entry::keyed("rename", "Rename", "F2"));
    out.push(Entry::danger("trash", "Move to Trash", "Delete"));
    out
}

impl Ui {
    /// The "…" button: opens the menu, or closes it.
    pub fn toggle_more(&mut self) {
        self.more = if self.more.is_some() {
            None
        } else {
            Some(Open::default())
        };
    }

    /// The line `index` is chosen: the menu closes and the file's action runs.
    pub fn pick_more(&mut self, index: usize) {
        let entries = entries(self);
        let Some(id) = entries.get(index).filter(|e| !e.is_off()).and_then(|e| e.id()) else {
            return;
        };
        self.more = None;
        match id {
            "open-with" => self.open_with_from_menu(),
            "show" => self.intent(Intent::ShowInFolder),
            "share" => self.intent(Intent::Share),
            "versions" => self.intent(Intent::VersionHistory),
            "offline" => self.toggle_offline(),
            "save-copy" => self.intent(Intent::SaveCopy),
            "rename" => self.intent(Intent::Rename),
            "trash" => self.intent(Intent::Trash),
            _ => {}
        }
    }
}

/// Dm-78: "Open with…" of the "…" menu is the page's "Open with another app…" (`Intent::ChooseApp`: the chooser, then
/// Finder's copy made present and opened, on a Mac after Finder setup is ready). What becomes of the press comes
/// later, as the file's note (`unsupported::note_of`). The page of a file with no viewer says its note itself; every
/// other open file has no place for it, so here it is the window's message ([`Ui::say_open_with`], at every poll).
impl Ui {
    fn open_with_from_menu(&mut self) {
        // The last press's words go: this press says its own.
        super::unsupported::set_note(&self.file.id, None);
        self.intent(Intent::ChooseApp);
        // A refusal is said already (the message). Else what follows is the note's.
        self.more_open_with = self.toast.is_none().then_some(None);
    }

    /// The file is shown as the page that says its note itself.
    fn says_its_note(&self) -> bool {
        matches!(&self.file.body, super::source::Body::NotPreviewable(why)
            if super::unsupported::UnsupportedReason::of(*why).is_some())
    }

    /// What became of the "…" menu's Open with since the last look: the message says the note's words (Finder is
    /// still being set up, the file is coming down, why it did not open), and goes with the note. A message another
    /// action put there since is left alone, and ends the following. `true`: the message changed.
    pub(super) fn say_open_with(&mut self) -> bool {
        let Some(said) = self.more_open_with.clone() else {
            return false;
        };
        if self.toast != said {
            self.more_open_with = None;
            return false;
        }
        let now = super::unsupported::note_of(&self.file.id).map(|note| note.words());
        if now == said || self.says_its_note() {
            return false;
        }
        self.toast = now.clone();
        self.more_open_with = Some(now);
        true
    }
}

/// The menu's keys: the arrows, Home and End move, Enter picks, Escape closes. The menu is modal, so nothing else
/// reaches the file under it.
pub(super) fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(ui) = super::ui(s) else { return false };
    let Some(open) = ui.more else { return false };
    let entries = entries(ui);
    let hot = match ev.key {
        Key::Down => menu::step(&entries, open.hot, 1),
        Key::Up => menu::step(&entries, open.hot, -1),
        Key::Home => menu::step(&entries, None, 1),
        Key::End => menu::step(&entries, None, -1),
        Key::Enter => {
            if let Some(hot) = open.hot {
                ui.pick_more(hot);
            }
            return true;
        }
        Key::Escape => {
            ui.more = None;
            tx.focus("more-btn", true);
            return true;
        }
        _ => return true,
    };
    if let Some(hot) = hot {
        ui.more = Some(Open { hot: Some(hot) });
        if let Some(key) = menu::item_key("more-menu", &entries, hot) {
            tx.focus(key, false);
        }
    }
    true
}

/// The menu, under the toolbar's "…" at the window's right.
pub fn layer(ui: &Ui, open: Open, t: &Theme) -> El {
    let surface = menu::menu(
        "more-menu",
        "More actions",
        MENU_W,
        &entries(ui),
        open.hot,
        t,
        |s: &mut State, i, _| {
            if let Some(ui) = super::ui(s) {
                ui.pick_more(i);
            }
        },
        |s: &mut State, i| {
            if let Some(ui) = super::ui(s) {
                ui.more = Some(Open { hot: Some(i) });
            }
        },
        |s: &mut State, tx: &mut Tx| {
            if let Some(ui) = super::ui(s) {
                ui.more = None;
                tx.focus("more-btn", true);
            }
        },
    )
    .on_key(on_key);
    row()
        .key("more-menu-layer")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .pt(78.0)
        .pr(96.0)
        .justify_end()
        .items_start()
        .child(surface)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::super::source::{Body, Details, Elsewhere, MdDoc, NoPreview, OpenError, OpenFile, ViewerSource};
    use super::super::unsupported::{self, Note};
    use super::super::{NEEDS_SYNC_ELSEWHERE, Ui};
    use super::entries;

    /// A source that remembers which way to another app it was asked for, and answers `answer`.
    struct Asked {
        asked: Rc<RefCell<Vec<&'static str>>>,
        answer: Elsewhere,
        body: fn() -> Body,
    }

    impl ViewerSource for Asked {
        fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            Ok(OpenFile {
                id: id.into(),
                path: Vec::new(),
                name: "testtttt.md".into(),
                facts: String::new(),
                version: 1,
                body: (self.body)(),
                details: Details {
                    kind: String::new(),
                    size: String::new(),
                    facts: Vec::new(),
                    offline: false,
                },
                newer: None,
            })
        }
        fn poll(&mut self, _id: &str) -> Option<Body> {
            None
        }
        fn cancel(&mut self, _id: &str) {}
        fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            self.open(id)
        }
        fn open_elsewhere(&mut self, _id: &str) -> Elsewhere {
            self.asked.borrow_mut().push("default app");
            self.answer
        }
        fn choose_elsewhere(&mut self, _id: &str) -> Elsewhere {
            self.asked.borrow_mut().push("chooser");
            self.answer
        }
    }

    fn note() -> Body {
        Body::Markdown(MdDoc {
            source: "# a note".into(),
        })
    }

    fn open(id: &str, answer: Elsewhere, body: fn() -> Body) -> (Ui, Rc<RefCell<Vec<&'static str>>>) {
        let asked = Rc::new(RefCell::new(Vec::new()));
        let source = Asked {
            asked: asked.clone(),
            answer,
            body,
        };
        (Ui::open(Box::new(source), id).expect("it opens"), asked)
    }

    fn pick_open_with(ui: &mut Ui) {
        ui.toggle_more();
        let at = entries(ui)
            .iter()
            .position(|e| e.id() == Some("open-with"))
            .expect("the line");
        ui.pick_more(at);
        assert!(ui.more.is_none(), "the menu closed");
    }

    /// Dm-78: the "…" menu's "Open with…" asks the source for the chooser, the very call the page's "Open with
    /// another app…" makes (on a Mac: dm-73's picker, the wait for Finder setup, dm-77's materialise). What becomes
    /// of it later is the file's note, and a file that is shown (a note, here) says it as the window's message.
    #[test]
    fn open_with_of_the_more_menu_asks_for_the_chooser_and_says_what_became_of_it() {
        let (mut ui, asked) = open("more-78-md", Elsewhere::Opened, note);
        unsupported::set_note("more-78-md", Some(Note::Failed("an older press".into())));
        pick_open_with(&mut ui);
        assert_eq!(*asked.borrow(), ["chooser"], "the page's own way, not another one");
        assert_eq!((ui.toast.clone(), unsupported::note_of("more-78-md")), (None, None));
        assert!(!ui.say_open_with(), "nothing to say yet");

        // The press's thread says Finder is still being set up, then that the file comes down, then nothing more.
        let waiting = Note::Failed("Finder is still being set up \u{b7} opens when it is ready".into());
        for note in [waiting, Note::Downloading, Note::Opening] {
            unsupported::set_note("more-78-md", Some(note.clone()));
            assert!(ui.say_open_with());
            assert_eq!(ui.toast, Some(note.words()));
            assert!(!ui.say_open_with(), "said once");
        }
        unsupported::set_note("more-78-md", None);
        assert!(ui.say_open_with());
        assert_eq!(ui.toast, None, "it opened: the message goes");

        // Why it did not open stays, until another action says its own; that one is left alone.
        pick_open_with(&mut ui);
        unsupported::set_note("more-78-md", Some(Note::Failed("why".into())));
        assert!(ui.say_open_with());
        assert_eq!(ui.toast.as_deref(), Some("why"));
        ui.toast = Some("Saved".into());
        unsupported::set_note("more-78-md", None);
        assert!(!ui.say_open_with());
        assert_eq!(ui.toast.as_deref(), Some("Saved"));
        assert_eq!(*asked.borrow(), ["chooser", "chooser"]);
    }

    #[test]
    fn a_refusal_at_the_press_is_the_message_at_once_and_the_page_of_a_file_says_its_note_itself() {
        // Sync really is off (Finder is ready): the source's own answer, said at once; nothing is followed.
        let (mut ui, _) = open("more-78-off", Elsewhere::NeedsSync, note);
        pick_open_with(&mut ui);
        assert_eq!(ui.toast.as_deref(), Some(NEEDS_SYNC_ELSEWHERE));
        unsupported::set_note("more-78-off", Some(Note::Downloading));
        assert!(!ui.say_open_with());
        assert_eq!(ui.toast.as_deref(), Some(NEEDS_SYNC_ELSEWHERE));
        unsupported::set_note("more-78-off", None);

        // A file with no viewer: its page says the note; the message does not say it twice.
        let (mut ui, asked) = open("more-78-eps", Elsewhere::Opened, || {
            Body::NotPreviewable(NoPreview::NoViewer)
        });
        pick_open_with(&mut ui);
        assert_eq!(*asked.borrow(), ["chooser"]);
        unsupported::set_note("more-78-eps", Some(Note::Downloading));
        assert!(!ui.say_open_with());
        assert_eq!(ui.toast, None);
        unsupported::set_note("more-78-eps", None);
    }
}

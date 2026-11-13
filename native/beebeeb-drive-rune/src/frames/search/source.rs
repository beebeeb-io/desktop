//! What Search draws from: the recent searches, the results of a query and the facts of one
//! result ([`SearchSource`]).
//!
//! The board's sample is [`BoardSearch`]: only a board frame's scene sets it (the golden path).
//! A window starts on [`Unwired`] (`SearchUi::default`), which answers that search is not
//! available yet and never returns a result, so a live window can never show the sample as the
//! person's files. The wiring lane implements this trait on the engine: names in the copy of the
//! list on this PC (the board: "so it is instant and works offline. It doesn't look inside
//! files"; "Trash and files not yet listed on this PC aren't searched").

use crate::frames::files::source::Details;
use crate::frames::shared::parts::table::Kind;
use crate::ui::file_list::PcState;

/// What the pills narrow a search to (DW-0802: All · Folders · Photos · Documents).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Narrow {
    #[default]
    All,
    Folders,
    Photos,
    Documents,
}

impl Narrow {
    pub const ALL: [Narrow; 4] = [Narrow::All, Narrow::Folders, Narrow::Photos, Narrow::Documents];

    /// The pill's words.
    pub fn words(self) -> &'static str {
        match self {
            Narrow::All => "All",
            Narrow::Folders => "Folders",
            Narrow::Photos => "Photos",
            Narrow::Documents => "Documents",
        }
    }

    /// Whether an item of this kind is kept.
    pub fn keeps(self, kind: Kind) -> bool {
        match self {
            Narrow::All => true,
            Narrow::Folders => kind == Kind::Folder,
            Narrow::Photos => matches!(kind, Kind::Photo(_)),
            Narrow::Documents => matches!(kind, Kind::Document | Kind::Sheet),
        }
    }
}

/// One result: an item whose name holds the query, and where it is.
#[derive(Clone, PartialEq)]
pub struct Hit {
    /// The item's id, as the engine gives it.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// "Yesterday, 16:42"
    pub changed: String,
    /// "1.2 MB"; a folder's "9 items".
    pub size: String,
    /// Where the item is: "Documents › Legal"; "Files" at the top.
    pub place: String,
    /// The PC column; `None`: the source does not know, and the cell stays empty.
    pub pc: Option<PcState>,
}

/// Never the name, the id or the place: a result is a decrypted file name, and a `{:?}` must not print one.
impl std::fmt::Debug for Hit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hit")
            .field("kind", &self.kind)
            .field("pc", &self.pc)
            .finish_non_exhaustive()
    }
}

/// What a source that answers late did since the last poll.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pumped {
    /// Something a draw reads changed.
    pub changed: bool,
    /// An answer is still on its way: poll again soon.
    pub busy: bool,
}

/// How far the list of files has loaded while a first sign-in still gets it (DW-0802b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loading {
    pub listed: u64,
    pub total: u64,
}

/// The answer to one query.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    /// Nothing can search here yet: a window before the engine lane wires Search. The view says
    /// so, and shows no result.
    Unavailable,
    /// The matches, in the order to show (none is an answer too: DW-0803), and while the list is
    /// still loading, how far it is: some matches may be missing (DW-0802b).
    Found { hits: Vec<Hit>, loading: Option<Loading> },
    /// The query is being answered (off the window's thread); the view says so, never "no results".
    Searching,
}

/// Search's data. Every call answers at once from what is on this PC.
pub trait SearchSource {
    /// `false`: nothing answers yet: the view says search is not available yet. The default, so a
    /// source says so itself once it can answer (task 2468: a source is unwired until it says it is).
    fn wired(&self) -> bool {
        false
    }
    /// The searches run on this PC, newest first ("Recent searches stay on this PC only").
    fn recent(&self) -> Vec<String>;
    /// Forget the recent searches (Clear).
    fn clear_recent(&mut self);
    /// Keep `query` as the newest recent search: a search the person ran (Enter, a result opened).
    fn remember(&mut self, query: &str);
    /// The items whose names hold `query` (any case), narrowed to `narrow`, across every folder.
    fn search(&self, query: &str, narrow: Narrow) -> Answer;
    /// The details panel's facts of the result with this id; `None`: not known, and no panel.
    fn details(&self, id: &str) -> Option<Details>;
    /// Search opened: a source that reads late reads now, when what it holds is old. Never from a draw.
    fn opened(&mut self) {}
    /// The words in the field or the pill changed: a source that answers late starts on this query now, and
    /// `search` says `Searching` until it has it. Never from a draw.
    fn ask(&mut self, _query: &str, _narrow: Narrow) {}
    /// What arrived since the last poll (`frames::search::poll`).
    fn pump(&mut self) -> Pumped {
        Pumped::default()
    }
    /// The gate or the account changed: every name this source holds goes now.
    fn forget(&mut self) {}
}

/// How many recent searches are kept (the iPhone's rule, DI-0801: "Up to 10 recent searches").
pub const RECENT_KEPT: usize = 10;

/// The board's sample (DW-0801 to DW-0803): three recent searches and the items the board finds
/// for "Contr". The golden path only: a board frame's scene sets it.
pub struct BoardSearch {
    recent: Vec<String>,
    items: Vec<(Hit, Details)>,
    loading: Option<Loading>,
}

fn hit(id: &str, name: &str, kind: Kind, changed: &str, size: &str, place: &str, pc: PcState) -> Hit {
    Hit {
        id: id.into(),
        name: name.into(),
        kind,
        changed: changed.into(),
        size: size.into(),
        place: place.into(),
        pc: Some(pc),
    }
}

/// The facts of a sample item, from its row (the board draws the panel of the first only).
fn facts(h: &Hit, kind: &str, offline_note: &str, kept: bool) -> Details {
    Details {
        kind: kind.into(),
        size: h.size.clone(),
        changed: h.changed.clone(),
        changed_on: None,
        added: None,
        where_: h.place.clone(),
        versions: None,
        shared: "Only you".into(),
        offline_note: offline_note.into(),
        kept_offline: kept,
        picture: false,
    }
}

impl BoardSearch {
    pub fn new() -> Self {
        let draft = hit(
            "board-search-1",
            "Contract draft.pdf",
            Kind::Document,
            "Yesterday, 16:42",
            "1.2 MB",
            "Files",
            PcState::OnlineOnly,
        );
        // The details panel of DW-0802 (the board's `MacDetails`).
        let draft_facts = Details {
            changed_on: Some("Anna's iPhone".into()),
            added: Some("28 Oct 2026".into()),
            versions: Some("3 versions".into()),
            shared: "Sam Okafor, pending".into(),
            ..facts(&draft, "PDF document", "Online only now \u{00B7} 1.2 MB", false)
        };
        let signed = hit(
            "board-search-2",
            "Contract signed 2025.pdf",
            Kind::Document,
            "3 Feb 2025",
            "840 KB",
            "Documents \u{203A} Legal",
            PcState::AvailableOffline,
        );
        let signed_facts = facts(&signed, "PDF document", "Kept on this PC \u{00B7} 840 KB", true);
        let contractors = hit(
            "board-search-3",
            "Contractors",
            Kind::Folder,
            "14 Oct 2026",
            "9 items",
            "Documents",
            PcState::AvailableOffline,
        );
        let contractors_facts = facts(&contractors, "Folder", "Kept on this PC", true);
        BoardSearch {
            recent: vec!["contract".into(), "lisbon".into(), "invoice 2026".into()],
            items: vec![
                (draft, draft_facts),
                (signed, signed_facts),
                (contractors, contractors_facts),
            ],
            loading: None,
        }
    }

    /// The same, during a first sign-in: 18,240 of 40,112 items listed (DW-0802b).
    pub fn loading() -> Self {
        BoardSearch {
            loading: Some(Loading {
                listed: 18_240,
                total: 40_112,
            }),
            ..BoardSearch::new()
        }
    }
}

impl Default for BoardSearch {
    fn default() -> Self {
        BoardSearch::new()
    }
}

impl SearchSource for BoardSearch {
    fn wired(&self) -> bool {
        true
    }
    fn recent(&self) -> Vec<String> {
        self.recent.clone()
    }

    fn clear_recent(&mut self) {
        self.recent.clear();
    }

    fn remember(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        self.recent.retain(|q| !q.eq_ignore_ascii_case(query));
        self.recent.insert(0, query.to_string());
        self.recent.truncate(RECENT_KEPT);
    }

    fn search(&self, query: &str, narrow: Narrow) -> Answer {
        let needle = query.trim().to_lowercase();
        let hits = if needle.is_empty() {
            Vec::new()
        } else {
            self.items
                .iter()
                .map(|(h, _)| h)
                .filter(|h| narrow.keeps(h.kind) && h.name.to_lowercase().contains(&needle))
                .cloned()
                .collect()
        };
        Answer::Found {
            hits,
            loading: self.loading,
        }
    }

    fn details(&self, id: &str) -> Option<Details> {
        self.items.iter().find(|(h, _)| h.id == id).map(|(_, d)| d.clone())
    }
}

/// The source of a window before the engine lane wires Search: no recent search, no result,
/// no facts. The view says search is not available yet; it never shows the board's sample.
pub struct Unwired;

/// What a window without an engine searches: the board's sample in a development build, nothing
/// in a build that ships (the rule of `settings::source::first`). A window on the engine is
/// `Unwired` whatever the build (`shell::state::init_on_engine`).
pub fn first(dev_build: bool) -> Box<dyn SearchSource> {
    if dev_build {
        Box::new(BoardSearch::new())
    } else {
        Box::new(Unwired)
    }
}

impl SearchSource for Unwired {
    fn recent(&self) -> Vec<String> {
        Vec::new()
    }
    fn clear_recent(&mut self) {}
    fn remember(&mut self, _query: &str) {}
    fn search(&self, _query: &str, _narrow: Narrow) -> Answer {
        Answer::Unavailable
    }
    fn details(&self, _id: &str) -> Option<Details> {
        None
    }
}

/// Where `query` stands in `name` (any case): the byte range to set in bold, as the board marks
/// the match (`mark`, weight 700). `None`: the name does not hold it (or the query is empty).
pub fn matched(name: &str, query: &str) -> Option<(usize, usize)> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    // Lower-casing can change a character's byte length: compare character by character.
    let chars: Vec<(usize, char)> = name.char_indices().collect();
    let wanted: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    for start in 0..chars.len() {
        let mut lowered = Vec::new();
        for (end, &(_, c)) in chars.iter().enumerate().skip(start) {
            lowered.extend(c.to_lowercase());
            if lowered.len() > wanted.len() || !wanted.starts_with(&lowered) {
                break;
            }
            if lowered.len() == wanted.len() {
                let to = chars.get(end + 1).map_or(name.len(), |&(i, _)| i);
                return Some((chars[start].0, to));
            }
        }
    }
    None
}

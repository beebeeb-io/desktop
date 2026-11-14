//! The board's sample documents of the office editor's frame, for the fixture source and the tests.

use super::office::{Kept, Section, Selection, WordDoc};
use super::office_over::{Command, CommandGroup};

/// Which board frame's document the fixture hands over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Case {
    /// DW-1507: saved, a selection with its tools.
    #[default]
    Selected,
    /// DW-1234: saved, nothing selected.
    Plain,
    /// DW-1552: changes that are not saved.
    Unsaved,
    /// DW-1550: a table was just inserted under "2. Delivery".
    Table,
    /// DW-1556: the editor does not start.
    NoEditor,
}

fn section(heading: &str, text: &str) -> Section {
    Section {
        heading: heading.into(),
        text: text.into(),
        table: None,
    }
}

/// DW-1507.
pub fn service_agreement() -> WordDoc {
    let scope = "Northwind Studio photographs the hotel's terrace, rooms and harbour view over two days and delivers 60 edited photos and a 90-second film.";
    let from = scope.find("delivers").unwrap_or(0);
    WordDoc {
        title: "Service agreement".into(),
        byline: "Northwind Studio and Lisbon Harbour Hotel \u{00B7} 28 October 2026".into(),
        sections: vec![
            section("1. Scope of work", scope),
            section(
                "2. Delivery",
                "Edited photos within 14 days of the shoot, as full-size JPEG and web-size copies, shared through a Beebeeb link that expires after 30 days.",
            ),
            section(
                "3. Fees",
                "A fixed fee of \u{20AC}6,400, half on signing and half on delivery. Travel within Lisbon is included.",
            ),
            section(
                "4. Usage",
                "The hotel may use the photos on its own website, social channels and printed materials for five years.",
            ),
        ],
        outline: [
            "Service agreement",
            "1. Scope of work",
            "2. Delivery",
            "3. Fees",
            "4. Usage",
            "5. Cancellation",
            "6. Signatures",
        ]
        .map(String::from)
        .to_vec(),
        at: 1,
        pages: 4,
        words: 1_284,
        language: "English (UK)".into(),
        saved_at: "09:12".into(),
        selection: Some(Selection {
            section: 0,
            range: from..scope.len(),
            tools_at: (239.0, 122.0),
        }),
        kept: Kept::Saved,
        older: false,
    }
}

/// The agreement as a board frame has it.
pub fn service_agreement_as(case: Case) -> WordDoc {
    let mut doc = service_agreement();
    match case {
        Case::Selected | Case::NoEditor => {}
        Case::Plain => doc.selection = None,
        Case::Unsaved => {
            doc.selection = None;
            doc.kept = Kept::Unsaved;
        }
        Case::Table => {
            doc.selection = None;
            doc.kept = Kept::Unsaved;
            doc.sections[1].table = Some(
                [
                    ["Day", "Location", "Hours"],
                    ["1", "Alfama", "8"],
                    ["2", "Bel\u{00E9}m", "6.5"],
                ]
                .iter()
                .map(|cells| cells.iter().map(|cell| cell.to_string()).collect())
                .collect(),
            );
        }
    }
    doc
}

/// The agreement after a save at 09:41: nothing selected, nothing unsaved.
pub fn service_agreement_saved() -> WordDoc {
    WordDoc {
        selection: None,
        saved_at: "09:41".into(),
        ..service_agreement()
    }
}

/// DW-1511: an older Word file.
pub fn proposal() -> WordDoc {
    WordDoc {
        title: "Proposal".into(),
        byline: "Northwind Studio \u{00B7} 14 March 2012".into(),
        sections: vec![
            section(
                "Who we are",
                "Northwind Studio is a two-person photography studio in Amsterdam. Since 2009 we have photographed hotels, restaurants and the people who run them.",
            ),
            section(
                "What we propose",
                "One shoot day for the new lobby and twelve rooms, delivered as 40 edited photos within ten working days.",
            ),
            section(
                "Fee",
                "\u{20AC}1,850 for the day, editing included. Prints are quoted separately.",
            ),
        ],
        outline: ["Proposal", "Who we are", "What we propose", "Fee"]
            .map(String::from)
            .to_vec(),
        at: 1,
        pages: 1,
        words: 96,
        language: "English (UK)".into(),
        saved_at: "14 Mar 2012".into(),
        selection: None,
        kept: Kept::Saved,
        older: true,
    }
}

/// DW-1551: what the command search finds for "head". Anything else finds nothing here.
pub fn commands(query: &str) -> Vec<CommandGroup> {
    if !query.trim().eq_ignore_ascii_case(QUERY) {
        return Vec::new();
    }
    let group = |title: &str, commands: &[(&str, Option<&str>)]| CommandGroup {
        title: title.into(),
        commands: commands
            .iter()
            .map(|(name, shortcut)| Command {
                name: name.to_string(),
                shortcut: shortcut.map(String::from),
            })
            .collect(),
    };
    vec![
        group(
            "Styles",
            &[("Heading 1", None), ("Heading 2", None), ("Heading 3", None)],
        ),
        group("Format", &[("Bold", Some("Ctrl+B"))]),
        group("Files", &[("Northwind brand guide.pdf", None)]),
    ]
}

/// What DW-1551 has typed.
pub const QUERY: &str = "head";

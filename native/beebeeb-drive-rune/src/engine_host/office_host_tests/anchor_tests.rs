//! Charts and pictures of a sheet (`OfficeHost::request_anchor`, `Event::Anchor`): asked only once seen in checked
//! cells, sized by the protocol's rule, answered only by the kind that request takes.

use beebeeb_office_protocol::check;
use beebeeb_office_protocol::office::{self, AnchorKind, Message};

use super::fake_helper_tests::*;
use super::{no_cells, stopped};
use crate::engine_host::office_host::*;

/// 100 % on a 96 DPI screen.
fn scale_100() -> f32 {
    check::render_scale(1.0, 96.0)
}

fn anchor(id: u32, kind: AnchorKind, w_pt: f32, h_pt: f32) -> Anchor {
    Anchor {
        id,
        kind,
        from_row: 1,
        from_col: 1,
        to_row: 8,
        to_col: 4,
        x_pt: 2.0,
        y_pt: 3.0,
        w_pt,
        h_pt,
    }
}

/// The sheet's two anchors: 300 x 200 pt (400 x 267 px at 100 %, by `ceil` of the `f32` product: `f64` gives 401 for
/// the 300) and 301 x 151 pt (402 x 202 px: `round` gives 401 x 201).
fn the_anchors() -> Vec<Anchor> {
    vec![
        anchor(7, AnchorKind::Chart, 300.0, 200.0),
        anchor(8, AnchorKind::Picture, 301.0, 151.0),
    ]
}

fn one_sheet() -> Message {
    Message::OpenedSheets(office::OpenedSheets {
        sheets: vec![SheetInfo {
            name: "Sheet1".to_owned(),
            rows: 100,
            cols: 20,
        }],
        more: 0,
    })
}

fn cells_with_anchors(range: CellRange) -> Message {
    let mut data = no_cells(range);
    data.anchors = the_anchors();
    Message::CellData(data)
}

fn anchors_of(events: &[Event]) -> Vec<(RequestId, u32, u32, u32, u32, usize)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Anchor {
                id,
                sheet,
                anchor,
                w,
                h,
                rgba,
                ..
            } => Some((*id, *sheet, *anchor, *w, *h, rgba.len())),
            _ => None,
        })
        .collect()
}

#[test]
fn an_anchor_is_asked_only_once_seen_and_sized_by_the_protocol() {
    let registry = Registry::new();
    let fake = Fake::new(|link| {
        let known = the_anchors();
        while let Some((id, message)) = link.next() {
            let reply = match message {
                Message::Open(_) => one_sheet(),
                Message::Cells(cells) => cells_with_anchors(cells.range),
                Message::RenderAnchor(asked) => {
                    let Some(seen) = known.iter().find(|anchor| anchor.id == asked.anchor) else {
                        continue;
                    };
                    let Ok((w, h)) = check::asked_anchor_size(&asked, seen, &check::Limits::CONTRACT) else {
                        continue;
                    };
                    link.paint(asked.out, 0x40);
                    Message::Rendered(office::Rendered {
                        w,
                        h,
                        missing_content: false,
                    })
                }
                _ => continue,
            };
            link.answer(id, reply);
        }
    });
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Xlsx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    assert_eq!(host.request_anchor(0, 7, scale_100()), None, "not seen yet");

    let range = CellRange {
        row: 0,
        col: 0,
        rows: 10,
        cols: 10,
    };
    assert!(host.request_cells(0, range));
    let cells_in = |events: &[Event]| events.iter().any(|event| matches!(event, Event::Cells { .. }));
    assert!(
        pump(&mut host, &mut events, |_, events| cells_in(events)),
        "{:?}",
        host.state()
    );

    let chart = host
        .request_anchor(0, 7, scale_100())
        .expect("the chart was in the cells");
    let picture = host
        .request_anchor(0, 8, scale_100())
        .expect("the picture was in the cells");
    assert_eq!(host.request_anchor(0, 9, scale_100()), None, "an id never seen");
    assert_eq!(
        host.request_anchor(1, 7, scale_100()),
        None,
        "seen on another sheet only"
    );
    assert!(
        pump(&mut host, &mut events, |_, events| anchors_of(events).len() == 2),
        "{:?}",
        host.state()
    );
    let sections = lock(&log.section_sizes).clone();
    assert_eq!(
        sections[sections.len() - 2..],
        [400 * 267 * 4, 402 * 202 * 4],
        "sections of exactly the protocol's size"
    );
    assert_eq!(
        anchors_of(&events),
        vec![
            (chart, 0, 7, 400, 267, 400 * 267 * 4),
            (picture, 0, 8, 402, 202, 402 * 202 * 4)
        ]
    );
    let sent = lock(&log.sent)
        .iter()
        .filter(|(kind, _)| *kind == office::KIND_RENDER_ANCHOR)
        .count();
    assert_eq!(sent, 2, "a frame only for the anchors seen");
    assert_eq!(log.kills(), 0);
    // A close forgets them.
    host.close(CloseWhy::Closed);
    assert_eq!(host.request_anchor(0, 7, scale_100()), None);
}

#[test]
fn a_wrong_kind_answer_to_an_anchor_kills() {
    let registry = Registry::new();
    let fake = Fake::new(|link| {
        while let Some((id, message)) = link.next() {
            let reply = match message {
                Message::Open(_) => one_sheet(),
                Message::Cells(cells) => cells_with_anchors(cells.range),
                // A chart answered with cells.
                Message::RenderAnchor(_) => Message::CellData(no_cells(CellRange {
                    row: 0,
                    col: 0,
                    rows: 1,
                    cols: 1,
                })),
                _ => continue,
            };
            link.answer(id, reply);
        }
    });
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Xlsx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    let range = CellRange {
        row: 0,
        col: 0,
        rows: 10,
        cols: 10,
    };
    assert!(host.request_cells(0, range));
    let cells_in = |events: &[Event]| events.iter().any(|event| matches!(event, Event::Cells { .. }));
    assert!(
        pump(&mut host, &mut events, |_, events| cells_in(events)),
        "{:?}",
        host.state()
    );
    assert!(host.request_anchor(0, 7, scale_100()).is_some());
    assert!(
        pump(&mut host, &mut events, |host, _| stopped(host)),
        "{:?}",
        host.state()
    );
    assert!(
        matches!(
            host.state(),
            OfficeState::Stopped {
                why: Stop::Protocol,
                ..
            }
        ),
        "{:?}",
        host.state()
    );
    assert!(anchors_of(&events).is_empty());
    assert_eq!(log.kills(), 1);
}

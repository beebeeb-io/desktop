//! Round trips and refusals of every Office message.

use super::*;
use crate::frame::MAX_PAYLOAD;

const GRANT: GrantRef = GrantRef {
    wire: 0x0000_0000_0000_01A4,
    len: 3_276_800,
};

/// One value of every message, with every optional part present somewhere.
fn every_message() -> Vec<Message> {
    vec![
        Message::Open(Open {
            kind: DocKind::Docx,
            doc: GRANT,
        }),
        Message::Open(Open {
            kind: DocKind::Doc,
            doc: GRANT,
        }),
        Message::Open(Open {
            kind: DocKind::Xlsx,
            doc: GRANT,
        }),
        Message::Open(Open {
            kind: DocKind::Xls,
            doc: GRANT,
        }),
        Message::Open(Open {
            kind: DocKind::Pptx,
            doc: GrantRef { wire: u64::MAX, len: 0 },
        }),
        Message::Opened(Opened {
            pages: 2,
            sizes: vec![
                PageSize {
                    w_pt: 595.3,
                    h_pt: 841.9,
                },
                PageSize {
                    w_pt: 960.0,
                    h_pt: 540.0,
                },
            ],
        }),
        Message::Opened(Opened {
            pages: 0,
            sizes: Vec::new(),
        }),
        Message::OpenedSheets(OpenedSheets {
            sheets: vec![
                SheetInfo {
                    name: "Begroting 2026".to_owned(),
                    rows: 120,
                    cols: 14,
                },
                SheetInfo {
                    name: String::new(),
                    rows: 0,
                    cols: 0,
                },
                SheetInfo {
                    name: "Überblick €".to_owned(),
                    rows: 1_048_576,
                    cols: 16_384,
                },
            ],
            more: 0,
        }),
        Message::OpenedSheets(OpenedSheets {
            sheets: vec![SheetInfo {
                name: "Blad1".to_owned(),
                rows: 3,
                cols: 2,
            }],
            more: 255,
        }),
        Message::Render(Render {
            page: 3,
            scale: 1.5,
            region: None,
            out: GRANT,
        }),
        Message::Render(Render {
            page: 0,
            scale: 4.0,
            region: Some(Rect {
                x: 512,
                y: 1024,
                w: 512,
                h: 300,
            }),
            out: GrantRef {
                wire: 7,
                len: 512 * 300 * 4,
            },
        }),
        Message::RenderAnchor(RenderAnchor {
            sheet: 1,
            anchor: 7,
            scale: 4.0 / 3.0,
            out: GrantRef {
                wire: 11,
                len: 640 * 427 * 4,
            },
        }),
        Message::Rendered(Rendered {
            w: 794,
            h: 1123,
            missing_content: false,
        }),
        Message::Rendered(Rendered {
            w: 1,
            h: 4096,
            missing_content: true,
        }),
        Message::Cells(Cells {
            sheet: 2,
            range: CellRange {
                row: 10,
                col: 3,
                rows: 40,
                cols: 12,
            },
        }),
        Message::CellData(CellData {
            cells: vec![
                CellText {
                    row: 10,
                    col: 3,
                    text: "Totaal".to_owned(),
                    align: Align::Left,
                    bold: true,
                    value: None,
                    formula: None,
                },
                CellText {
                    row: 11,
                    col: 4,
                    text: "€ 1.234,50".to_owned(),
                    align: Align::Right,
                    bold: false,
                    value: Some(1234.5),
                    formula: Some("=SUM(E2:E11)".to_owned()),
                },
                CellText {
                    row: 12,
                    col: 5,
                    text: String::new(),
                    align: Align::Center,
                    bold: false,
                    value: Some(-0.0),
                    formula: Some(String::new()),
                },
            ],
            col_widths: vec![64.0, 0.0, 120.5],
            row_heights: vec![15.0, 30.25],
            merges: vec![CellRange {
                row: 10,
                col: 3,
                rows: 1,
                cols: 3,
            }],
            frozen: Frozen { rows: 1, cols: 2 },
            anchors: vec![chart(0), picture(7)],
            missing_content: true,
        }),
        Message::CellData(CellData {
            cells: Vec::new(),
            col_widths: Vec::new(),
            row_heights: Vec::new(),
            merges: Vec::new(),
            frozen: Frozen { rows: 0, cols: 0 },
            anchors: Vec::new(),
            missing_content: false,
        }),
        Message::Outline(Outline { from: 0 }),
        Message::Outline(Outline { from: 40 }),
        Message::OutlineData(OutlineData {
            entries: vec![
                OutlineEntry {
                    level: 1,
                    text: "Inleiding".to_owned(),
                    page: 0,
                },
                OutlineEntry {
                    level: 2,
                    text: "Achtergrond en doel".to_owned(),
                    page: 1,
                },
                OutlineEntry {
                    level: 9,
                    text: String::new(),
                    page: 41,
                },
            ],
            next: Some(3),
        }),
        Message::OutlineData(OutlineData {
            entries: vec![OutlineEntry {
                level: 1,
                text: "Bijlagen".to_owned(),
                page: 7,
            }],
            next: None,
        }),
        Message::Stats(Stats),
        Message::StatsData(StatsData { words: 12_345 }),
        Message::Cancel(Cancel),
        Message::Error(Error {
            code: ErrorCode::Damaged,
        }),
        Message::Error(Error {
            code: ErrorCode::TooLarge,
        }),
        Message::Error(Error {
            code: ErrorCode::Unsupported,
        }),
        Message::Error(Error {
            code: ErrorCode::Password,
        }),
        Message::Error(Error {
            code: ErrorCode::Internal,
        }),
        Message::Error(Error {
            code: ErrorCode::RangeTooLarge,
        }),
    ]
}

const ALL_KINDS: [u16; 14] = [
    KIND_OPEN,
    KIND_RENDER,
    KIND_RENDER_ANCHOR,
    KIND_CELLS,
    KIND_OUTLINE,
    KIND_STATS,
    KIND_CANCEL,
    KIND_OPENED,
    KIND_OPENED_SHEETS,
    KIND_RENDERED,
    KIND_CELL_DATA,
    KIND_OUTLINE_DATA,
    KIND_STATS_DATA,
    KIND_ERROR,
];

#[test]
fn office_messages_round_trip() {
    let table = every_message();
    for m in &table {
        let payload = m.encode().expect("under one frame");
        assert_eq!(Message::decode(m.kind(), &payload).as_ref(), Ok(m), "{m:?}");
    }

    // The table covers every kind, and the kinds are distinct.
    for kind in ALL_KINDS {
        assert!(
            table.iter().any(|m| m.kind() == kind),
            "no message of kind {kind:#06x} in the table"
        );
        assert_eq!(
            ALL_KINDS.iter().filter(|k| **k == kind).count(),
            1,
            "kind {kind:#06x} used twice"
        );
    }

    // The per-message codecs agree with `Message`.
    let open = Open {
        kind: DocKind::Pptx,
        doc: GRANT,
    };
    assert_eq!(Open::decode(&open.encode().expect("under one frame")), Ok(open));
    let got = Rendered {
        w: 3,
        h: 2,
        missing_content: true,
    };
    assert_eq!(Rendered::decode(&got.encode().expect("under one frame")), Ok(got));
}

#[test]
fn office_messages_travel_in_frames() {
    let mut stream = Vec::new();
    let table = every_message();
    for (n, m) in table.iter().enumerate() {
        let payload = m.encode().expect("under one frame");
        crate::frame::write_frame(&mut stream, m.kind(), n as u32 * 2, &payload).expect("under the cap");
    }
    let mut r = std::io::Cursor::new(stream);
    for (n, m) in table.iter().enumerate() {
        let frame = crate::frame::read_frame(&mut r).expect("a frame");
        assert_eq!(frame.id, n as u32 * 2);
        assert_eq!(Message::decode(frame.kind, &frame.payload).as_ref(), Ok(m));
    }
    assert_eq!(crate::frame::read_frame(&mut r), Err(crate::frame::FrameError::Closed));
}

/// `make(pad)` is a message whose payload grows by one byte per unit of `pad`. At the pad that makes it exactly one
/// frame it encodes, decodes and is written; one byte more is refused by `encode`.
fn at_the_limit_and_one_byte_over(name: &str, make: impl Fn(usize) -> Message) {
    let base = make(0).encode().expect("an empty pad fits").len();
    let pad = MAX_PAYLOAD - base;
    let at = make(pad);
    let payload = at.encode().expect("exactly one frame is encoded");
    assert_eq!(payload.len(), MAX_PAYLOAD, "{name}");
    assert_eq!(Message::decode(at.kind(), &payload).as_ref(), Ok(&at), "{name}");
    let mut stream = Vec::new();
    crate::frame::write_frame(&mut stream, at.kind(), 2, &payload).expect("a payload of one frame is written");
    assert_eq!(make(pad + 1).encode(), Err(EncodeError::OverFrame), "{name}");
}

#[test]
fn office_encode_refuses_a_message_over_one_frame() {
    // Each answer that can grow, padded by the text of one item.
    at_the_limit_and_one_byte_over("sheets", |pad| {
        Message::OpenedSheets(OpenedSheets {
            sheets: vec![SheetInfo {
                name: "s".repeat(pad),
                rows: 1,
                cols: 1,
            }],
            more: 0,
        })
    });
    at_the_limit_and_one_byte_over("cells", |pad| {
        Message::CellData(CellData {
            cells: vec![CellText {
                row: 0,
                col: 0,
                text: "x".repeat(pad),
                align: Align::Left,
                bold: false,
                value: Some(3.5),
                formula: Some("=A1".to_owned()),
            }],
            col_widths: vec![48.0],
            row_heights: vec![15.0],
            merges: Vec::new(),
            frozen: Frozen { rows: 0, cols: 0 },
            anchors: vec![chart(0)],
            missing_content: false,
        })
    });
    at_the_limit_and_one_byte_over("outline", |pad| {
        Message::OutlineData(OutlineData {
            entries: vec![OutlineEntry {
                level: 1,
                text: "h".repeat(pad),
                page: 0,
            }],
            next: Some(1),
        })
    });

    // Page sizes take 8 bytes each: the two counts and 8,191 sizes are exactly one frame, 8,192 sizes are not.
    let sizes = |n: usize| Opened {
        pages: n as u32,
        sizes: vec![
            PageSize {
                w_pt: 595.3,
                h_pt: 841.9,
            };
            n
        ],
    };
    assert_eq!(sizes(8_191).encode().map(|p| p.len()), Ok(MAX_PAYLOAD));
    assert_eq!(sizes(8_192).encode(), Err(EncodeError::OverFrame));
    assert_eq!(Message::Opened(sizes(8_192)).encode(), Err(EncodeError::OverFrame));
}

#[test]
fn office_decode_refuses_truncated_payloads() {
    for m in every_message() {
        let payload = m.encode().expect("under one frame");
        for cut in 0..payload.len() {
            let got = Message::decode(m.kind(), &payload[..cut]);
            assert!(
                matches!(got, Err(DecodeError::Truncated | DecodeError::LengthPastEnd)),
                "{m:?} cut at {cut}: {got:?}"
            );
        }
    }
}

#[test]
fn office_decode_refuses_trailing_bytes() {
    for m in every_message() {
        let mut payload = m.encode().expect("under one frame");
        payload.push(0);
        assert_eq!(
            Message::decode(m.kind(), &payload),
            Err(DecodeError::TrailingBytes),
            "{m:?}"
        );
    }
}

#[test]
fn office_decode_refuses_an_unknown_kind() {
    for kind in [0u16, 0x0100, 0x0108, 0x0200, 0x0207, 0x02FE, 0x0301, u16::MAX] {
        assert_eq!(Message::decode(kind, &[]), Err(DecodeError::UnknownKind), "{kind:#06x}");
    }
}

fn le32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

#[test]
fn office_decode_refuses_a_declared_length_past_the_end() {
    // One item more than the bytes hold, for every vector (first, so a missing bound fails an assertion here
    // before the u32::MAX cases below can abort the test binary on an allocation).
    // Opened: two sizes declared, one present.
    let mut p = le32(2).to_vec();
    p.extend_from_slice(&le32(2));
    p.extend_from_slice(&595.0f32.to_le_bytes());
    p.extend_from_slice(&842.0f32.to_le_bytes());
    assert_eq!(Opened::decode(&p), Err(DecodeError::LengthPastEnd));

    // OpenedSheets: two sheets declared, room for one.
    let mut p = le32(2).to_vec();
    p.extend_from_slice(&le32(0));
    p.extend_from_slice(&le32(1));
    p.extend_from_slice(&le32(1));
    assert_eq!(OpenedSheets::decode(&p), Err(DecodeError::LengthPastEnd));

    // A sheet name one byte longer than what follows (6 bytes of text and 4 of rows: 10).
    let mut p = le32(1).to_vec();
    p.extend_from_slice(&le32(11));
    p.extend_from_slice(b"Sheet1");
    p.extend_from_slice(&le32(1));
    assert_eq!(OpenedSheets::decode(&p), Err(DecodeError::LengthPastEnd));

    // CellData: the cells, widths, heights and merges vectors in turn. The tail behind the merges' count: frozen panes
    // (8), the anchors' count (4), `missing_content` (1).
    let empty_tail = |widths: u32, heights: u32, merges: u32| {
        let mut t = le32(widths).to_vec();
        t.extend_from_slice(&le32(heights));
        t.extend_from_slice(&le32(merges));
        t.extend_from_slice(&le32(0));
        t.extend_from_slice(&le32(0));
        t.extend_from_slice(&le32(0));
        t.push(0);
        t
    };
    let mut p = le32(1).to_vec();
    p.extend_from_slice(&[0; 15]); // only 15 of a cell's at least 16 bytes
    assert_eq!(CellData::decode(&p), Err(DecodeError::LengthPastEnd));

    // widths: 6 declared, the 21 bytes behind the count hold 5
    let mut p = le32(0).to_vec();
    p.extend_from_slice(&empty_tail(6, 0, 0));
    assert_eq!(CellData::decode(&p), Err(DecodeError::LengthPastEnd));
    // heights: 5 declared, the 17 bytes behind the count hold 4
    let mut p = le32(0).to_vec();
    p.extend_from_slice(&empty_tail(0, 5, 0));
    assert_eq!(CellData::decode(&p), Err(DecodeError::LengthPastEnd));
    // merges: 1 declared, 13 bytes behind the count, a merge takes 16
    let mut p = le32(0).to_vec();
    p.extend_from_slice(&empty_tail(0, 0, 1));
    assert_eq!(CellData::decode(&p), Err(DecodeError::LengthPastEnd));
    // the tail as it is decodes: the premise of the three above
    let mut p = le32(0).to_vec();
    p.extend_from_slice(&empty_tail(0, 0, 0));
    assert!(CellData::decode(&p).is_ok());

    // A cell's text declared u32::MAX bytes long, with 16 bytes behind it.
    let mut p = le32(1).to_vec();
    p.extend_from_slice(&le32(0));
    p.extend_from_slice(&le32(0));
    p.extend_from_slice(&[1, 0]);
    p.extend_from_slice(&le32(u32::MAX));
    p.extend_from_slice(&[b'x'; 16]);
    assert_eq!(CellData::decode(&p), Err(DecodeError::LengthPastEnd));

    // A cell's formula declared u32::MAX bytes long, with 16 bytes behind it.
    let mut p = le32(1).to_vec();
    p.extend_from_slice(&le32(0));
    p.extend_from_slice(&le32(0));
    p.extend_from_slice(&[1, 0]);
    p.extend_from_slice(&le32(0)); // no text
    p.push(0); // no value
    p.push(1); // a formula
    p.extend_from_slice(&le32(u32::MAX));
    p.extend_from_slice(&[b'='; 16]);
    assert_eq!(CellData::decode(&p), Err(DecodeError::LengthPastEnd));

    // OutlineData: two entries declared, room for one.
    let mut p = le32(2).to_vec();
    p.push(1);
    p.extend_from_slice(&le32(0));
    p.extend_from_slice(&le32(0));
    assert_eq!(OutlineData::decode(&p), Err(DecodeError::LengthPastEnd));

    // The largest item there is, declared u32::MAX times with nothing behind it. Trusting the count would ask the
    // allocator for hundreds of gigabytes before the first item fails to read.
    assert_eq!(CellData::decode(&le32(u32::MAX)), Err(DecodeError::LengthPastEnd));
    assert_eq!(OpenedSheets::decode(&le32(u32::MAX)), Err(DecodeError::LengthPastEnd));
    assert_eq!(OutlineData::decode(&le32(u32::MAX)), Err(DecodeError::LengthPastEnd));
    let mut opened = le32(1).to_vec();
    opened.extend_from_slice(&le32(u32::MAX));
    assert_eq!(Opened::decode(&opened), Err(DecodeError::LengthPastEnd));
}

#[test]
fn office_error_code_off_the_list_is_refused() {
    for code in [0u16, 7, 8, 0x00FF, 0x0100, u16::MAX] {
        assert_eq!(
            Error::decode(&code.to_le_bytes()),
            Err(DecodeError::BadCode),
            "code {code}"
        );
        assert_eq!(
            Message::decode(KIND_ERROR, &code.to_le_bytes()),
            Err(DecodeError::BadCode),
            "code {code}"
        );
    }
    for (code, want) in [
        (1u16, ErrorCode::Damaged),
        (2, ErrorCode::TooLarge),
        (3, ErrorCode::Unsupported),
        (4, ErrorCode::Password),
        (5, ErrorCode::Internal),
        (6, ErrorCode::RangeTooLarge),
    ] {
        assert_eq!(Error::decode(&code.to_le_bytes()), Ok(Error { code: want }));
    }
}

#[test]
fn office_decode_refuses_invalid_utf8() {
    let bad: [&[u8]; 3] = [&[0xFF], &[0xC3], &[b'a', 0xED, 0xA0, 0x80]];
    for bytes in bad {
        // A sheet name.
        let mut p = le32(1).to_vec();
        p.extend_from_slice(&le32(bytes.len() as u32));
        p.extend_from_slice(bytes);
        p.extend_from_slice(&le32(1));
        p.extend_from_slice(&le32(1));
        assert_eq!(OpenedSheets::decode(&p), Err(DecodeError::BadUtf8), "{bytes:?}");

        // An outline entry.
        let mut p = le32(1).to_vec();
        p.push(1);
        p.extend_from_slice(&le32(bytes.len() as u32));
        p.extend_from_slice(bytes);
        p.extend_from_slice(&le32(0));
        assert_eq!(OutlineData::decode(&p), Err(DecodeError::BadUtf8), "{bytes:?}");
    }
}

#[test]
fn office_decode_refuses_numbers_that_are_not_finite() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let opened = Opened {
            pages: 1,
            sizes: vec![PageSize { w_pt: bad, h_pt: 842.0 }],
        };
        let payload = opened.encode().expect("under one frame");
        assert_eq!(Opened::decode(&payload), Err(DecodeError::NotFinite));

        let render = Render {
            page: 0,
            scale: bad,
            region: None,
            out: GRANT,
        };
        let payload = render.encode().expect("under one frame");
        assert_eq!(Render::decode(&payload), Err(DecodeError::NotFinite));

        let cells = CellData {
            cells: Vec::new(),
            col_widths: vec![10.0, bad],
            row_heights: Vec::new(),
            merges: Vec::new(),
            frozen: Frozen { rows: 0, cols: 0 },
            anchors: Vec::new(),
            missing_content: false,
        };
        let payload = cells.encode().expect("under one frame");
        assert_eq!(CellData::decode(&payload), Err(DecodeError::NotFinite));
    }
}

#[test]
fn office_decode_refuses_tags_off_their_list() {
    // DocKind.
    for tag in [0u8, 6, 0xFF] {
        let mut p = vec![tag];
        p.extend_from_slice(&[0; 16]);
        assert_eq!(Open::decode(&p), Err(DecodeError::BadTag), "doc kind {tag}");
    }

    // The region flag of a Render.
    let mut p = le32(0).to_vec();
    p.extend_from_slice(&1.0f32.to_le_bytes());
    p.push(2);
    p.extend_from_slice(&[0; 16]);
    assert_eq!(Render::decode(&p), Err(DecodeError::BadTag));

    // The `next` flag of an outline page.
    let mut p = le32(0).to_vec();
    p.push(2);
    p.extend_from_slice(&le32(0));
    assert_eq!(OutlineData::decode(&p), Err(DecodeError::BadTag));

    // A bool that is neither 0 nor 1.
    let mut p = le32(10).to_vec();
    p.extend_from_slice(&le32(10));
    p.push(2);
    assert_eq!(Rendered::decode(&p), Err(DecodeError::BadTag));

    // A cell's alignment and its reserved flag bits; then no value, no formula, and an empty tail (widths, heights,
    // merges, frozen panes, anchors, `missing_content`).
    let cell = |align: u8, flags: u8| {
        let mut p = le32(1).to_vec();
        p.extend_from_slice(&le32(0));
        p.extend_from_slice(&le32(0));
        p.push(align);
        p.push(flags);
        p.extend_from_slice(&le32(0));
        p.extend_from_slice(&[0, 0]);
        for _ in 0..6 {
            p.extend_from_slice(&le32(0));
        }
        p.push(0);
        p
    };
    assert!(
        CellData::decode(&cell(1, 1)).is_ok(),
        "a bold left-aligned cell is fine"
    );
    for align in [0u8, 4, 0xFF] {
        assert_eq!(
            CellData::decode(&cell(align, 0)),
            Err(DecodeError::BadTag),
            "align {align}"
        );
    }
    for flags in [2u8, 0x80, 0xFF] {
        assert_eq!(
            CellData::decode(&cell(1, flags)),
            Err(DecodeError::BadTag),
            "flags {flags:#04x}"
        );
    }
}

#[test]
fn office_debug_shows_no_document_text() {
    let sheets = OpenedSheets {
        sheets: vec![SheetInfo {
            name: "Salarissen".to_owned(),
            rows: 1,
            cols: 1,
        }],
        more: 0,
    };
    let cells = CellData {
        cells: vec![CellText {
            row: 0,
            col: 0,
            text: "geheim".to_owned(),
            align: Align::Left,
            bold: false,
            value: Some(271828.0),
            formula: Some("=VERTROUWELIJK()".to_owned()),
        }],
        col_widths: vec![1.0],
        row_heights: vec![1.0],
        merges: Vec::new(),
        frozen: Frozen { rows: 0, cols: 0 },
        anchors: Vec::new(),
        missing_content: false,
    };
    let outline = OutlineData {
        entries: vec![OutlineEntry {
            level: 1,
            text: "Overname".to_owned(),
            page: 0,
        }],
        next: None,
    };
    let shown = format!("{sheets:?} {cells:?} {outline:?}");
    for secret in ["Salarissen", "geheim", "Overname", "271828", "VERTROUWELIJK"] {
        assert!(!shown.contains(secret), "{shown}");
    }
}

// ---- W0: values, formulas, charts and pictures, and the request that draws one

/// A chart over the cells D11:J25, every field its own value.
fn chart(id: u32) -> Anchor {
    Anchor {
        id,
        kind: AnchorKind::Chart,
        from_row: 10,
        from_col: 3,
        to_row: 24,
        to_col: 9,
        x_pt: 4.5,
        y_pt: 0.75,
        w_pt: 480.0,
        h_pt: 288.0,
    }
}

/// A picture inside the cell A1.
fn picture(id: u32) -> Anchor {
    Anchor {
        id,
        kind: AnchorKind::Picture,
        from_row: 0,
        from_col: 0,
        to_row: 0,
        to_col: 0,
        x_pt: 0.0,
        y_pt: 2.25,
        w_pt: 31.5,
        h_pt: 12.0,
    }
}

fn numeric(row: u32, col: u32, text: &str, value: f64, formula: Option<&str>) -> CellText {
    CellText {
        row,
        col,
        text: text.to_owned(),
        align: Align::Right,
        bold: false,
        value: Some(value),
        formula: formula.map(str::to_owned),
    }
}

#[test]
fn cells_round_trip_with_values_formulas_and_anchors() {
    let block = CellData {
        cells: vec![
            numeric(1, 1, "1,234.50 €", 1234.5, None),
            numeric(2, 1, "12.5%", 0.125, Some("=B2/B9")),
            numeric(3, 1, "3 Nov 2026", 46_329.0, None),
            numeric(9, 1, "13,220", 13_220.0, Some("=SUM(B2:B9)")),
            CellText {
                row: 9,
                col: 0,
                text: "Totaal".to_owned(),
                align: Align::Left,
                bold: true,
                value: None,
                formula: Some("=\"Tot\"&\"aal\" ∑".to_owned()),
            },
            numeric(10, 2, "-0", -0.0, Some("")),
            numeric(11, 2, "1E+308", 1e308, None),
            numeric(12, 2, "tiny", f64::MIN_POSITIVE, None),
        ],
        col_widths: vec![64.0, 80.25, 48.0],
        row_heights: vec![15.0; 13],
        merges: vec![CellRange {
            row: 0,
            col: 0,
            rows: 1,
            cols: 2,
        }],
        frozen: Frozen { rows: 1, cols: 0 },
        anchors: vec![chart(0), picture(1), chart(255)],
        missing_content: true,
    };
    let payload = block.encode().expect("under one frame");
    let back = CellData::decode(&payload).expect("it decodes");
    assert!(back == block, "every field back as it went");
    // Bit for bit: -0.0 stays negative, and every value keeps its bits.
    for (a, b) in back.cells.iter().zip(&block.cells) {
        assert_eq!(a.value.map(f64::to_bits), b.value.map(f64::to_bits));
    }
    assert_eq!(
        Message::decode(KIND_CELL_DATA, &payload),
        Ok(Message::CellData(block.clone()))
    );

    // `Debug` shows counts and lengths: no text, no formula, no value.
    let shown = format!("{back:?} {:?} {:?}", back.cells, back.anchors);
    for secret in ["Totaal", "SUM", "B2/B9", "1234.5", "13220", "0.125", "480", "288", "∑"] {
        assert!(!shown.contains(secret), "{secret} in {shown}");
    }
    assert!(shown.contains("formula_len"), "{shown}");
}

#[test]
fn a_non_finite_value_is_refused_at_decode() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let block = CellData {
            cells: vec![numeric(0, 0, "x", bad, None)],
            col_widths: vec![48.0],
            row_heights: vec![15.0],
            merges: Vec::new(),
            frozen: Frozen { rows: 0, cols: 0 },
            anchors: Vec::new(),
            missing_content: false,
        };
        let payload = block.encode().expect("encode writes what it is given");
        assert_eq!(CellData::decode(&payload), Err(DecodeError::NotFinite), "{bad}");
        assert_eq!(
            Message::decode(KIND_CELL_DATA, &payload),
            Err(DecodeError::NotFinite),
            "{bad}"
        );
    }
    // An anchor's size goes through the f32 rule.
    let mut bad_anchor = chart(0);
    bad_anchor.w_pt = f32::NAN;
    let block = CellData {
        cells: Vec::new(),
        col_widths: Vec::new(),
        row_heights: Vec::new(),
        merges: Vec::new(),
        frozen: Frozen { rows: 0, cols: 0 },
        anchors: vec![bad_anchor],
        missing_content: false,
    };
    let payload = block.encode().expect("encode writes what it is given");
    assert_eq!(CellData::decode(&payload), Err(DecodeError::NotFinite));
}

/// A `CellData` with no cells, widths, heights or merges, and no frozen panes, up to the anchors' count.
fn up_to_the_anchors() -> Vec<u8> {
    let mut p = Vec::new();
    for _ in 0..6 {
        p.extend_from_slice(&le32(0));
    }
    p
}

#[test]
fn anchor_counts_are_bounded_before_allocating() {
    // One anchor is 37 bytes: the id, the kind, four cell numbers, four sizes.
    let mut one = Out::new();
    chart(0).put(&mut one);
    let one = one.finish().expect("small");
    assert_eq!(one.len(), Anchor::BYTES);
    assert_eq!(Anchor::BYTES, 37);

    // Two declared with 50 bytes behind the count: one whole anchor and 13 bytes. The count must be refused before any
    // anchor is read: a reader that trusted a smaller minimum would read the first and end Truncated in the second.
    let mut p = up_to_the_anchors();
    p.extend_from_slice(&le32(2));
    p.extend_from_slice(&one);
    p.extend_from_slice(&[0; 13]);
    assert_eq!(p.len() - up_to_the_anchors().len() - 4, 50);
    assert_eq!(CellData::decode(&p), Err(DecodeError::LengthPastEnd));

    // 2^31 declared with 50 bytes behind: refused without asking the allocator for 2^31 anchors.
    let mut p = up_to_the_anchors();
    p.extend_from_slice(&le32(1 << 31));
    p.extend_from_slice(&[0; 50]);
    assert_eq!(CellData::decode(&p), Err(DecodeError::LengthPastEnd));

    // u32::MAX with nothing behind.
    let mut p = up_to_the_anchors();
    p.extend_from_slice(&le32(u32::MAX));
    assert_eq!(CellData::decode(&p), Err(DecodeError::LengthPastEnd));

    // The premise: one declared, one whole anchor and the flag behind it, decodes.
    let mut p = up_to_the_anchors();
    p.extend_from_slice(&le32(1));
    p.extend_from_slice(&one);
    p.push(0);
    assert_eq!(CellData::decode(&p).map(|d| d.anchors), Ok(vec![chart(0)]));
}

#[test]
fn render_anchor_round_trips_in_one_frame() {
    let ask = RenderAnchor {
        sheet: 3,
        anchor: 41,
        scale: 1.5,
        out: GrantRef {
            wire: 0x0000_0000_0000_02B8,
            len: 720 * 432 * 4,
        },
    };
    let m = Message::RenderAnchor(ask);
    assert_eq!(m.kind(), KIND_RENDER_ANCHOR);
    assert_eq!(KIND_RENDER_ANCHOR, 0x0107);
    let payload = m.encode().expect("under one frame");
    // The sheet, the anchor, the scale, the grant: 4 + 4 + 4 + 16.
    assert_eq!(payload.len(), 28);
    assert_eq!(RenderAnchor::decode(&payload), Ok(ask));

    // One frame on the pipe, and back.
    let mut stream = Vec::new();
    crate::frame::write_frame(&mut stream, m.kind(), 6, &payload).expect("under the cap");
    let mut r = std::io::Cursor::new(stream);
    let frame = crate::frame::read_frame(&mut r).expect("a frame");
    assert_eq!((frame.kind, frame.id), (0x0107, 6));
    assert_eq!(Message::decode(frame.kind, &frame.payload), Ok(m));
    assert_eq!(crate::frame::read_frame(&mut r), Err(crate::frame::FrameError::Closed));

    // A scale that is not finite is refused, as a `Render`'s.
    for bad in [f32::NAN, f32::INFINITY] {
        let payload = RenderAnchor { scale: bad, ..ask }.encode().expect("under one frame");
        assert_eq!(RenderAnchor::decode(&payload), Err(DecodeError::NotFinite));
    }
}

#[test]
fn cell_and_anchor_tags_off_their_list_are_refused() {
    // A cell with no text, then the value's tag, the formula's tag, and the empty tail.
    let cell = |value_tag: u8, formula_tag: u8| {
        let mut p = le32(1).to_vec();
        p.extend_from_slice(&le32(0));
        p.extend_from_slice(&le32(0));
        p.extend_from_slice(&[1, 0]);
        p.extend_from_slice(&le32(0));
        p.push(value_tag);
        if value_tag == 1 {
            p.extend_from_slice(&2.5f64.to_le_bytes());
        }
        p.push(formula_tag);
        if formula_tag == 1 {
            p.extend_from_slice(&le32(2));
            p.extend_from_slice(b"=1");
        }
        for _ in 0..6 {
            p.extend_from_slice(&le32(0));
        }
        p.push(0);
        p
    };
    assert!(CellData::decode(&cell(1, 1)).is_ok(), "a value and a formula are fine");
    assert!(CellData::decode(&cell(0, 0)).is_ok(), "neither is fine");
    for tag in [2u8, 0x80, 0xFF] {
        assert_eq!(
            CellData::decode(&cell(tag, 0)),
            Err(DecodeError::BadTag),
            "value tag {tag}"
        );
        assert_eq!(
            CellData::decode(&cell(0, tag)),
            Err(DecodeError::BadTag),
            "formula tag {tag}"
        );
    }

    // An anchor's kind, and `missing_content`.
    let block = |kind: u8, missing: u8| {
        let mut one = Out::new();
        chart(0).put(&mut one);
        let mut one = one.finish().expect("small");
        one[4] = kind;
        let mut p = up_to_the_anchors();
        p.extend_from_slice(&le32(1));
        p.extend_from_slice(&one);
        p.push(missing);
        p
    };
    assert_eq!(
        CellData::decode(&block(1, 1)).map(|d| (d.anchors[0].kind, d.missing_content)),
        Ok((AnchorKind::Picture, true))
    );
    for kind in [2u8, 0xFF] {
        assert_eq!(
            CellData::decode(&block(kind, 0)),
            Err(DecodeError::BadTag),
            "kind {kind}"
        );
    }
    assert_eq!(
        CellData::decode(&block(0, 2)),
        Err(DecodeError::BadTag),
        "missing_content 2"
    );
}

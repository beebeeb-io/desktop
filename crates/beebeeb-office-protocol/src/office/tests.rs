//! Round trips and refusals of every Office message.

use super::*;

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
                },
                CellText {
                    row: 11,
                    col: 4,
                    text: "€ 1.234,50".to_owned(),
                    align: Align::Right,
                    bold: false,
                },
                CellText {
                    row: 12,
                    col: 5,
                    text: String::new(),
                    align: Align::Center,
                    bold: false,
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
        }),
        Message::CellData(CellData {
            cells: Vec::new(),
            col_widths: Vec::new(),
            row_heights: Vec::new(),
            merges: Vec::new(),
            frozen: Frozen { rows: 0, cols: 0 },
        }),
        Message::Outline(Outline),
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
    ]
}

const ALL_KINDS: [u16; 13] = [
    KIND_OPEN,
    KIND_RENDER,
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
        let payload = m.encode();
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
    assert_eq!(Open::decode(&open.encode()), Ok(open));
    let got = Rendered {
        w: 3,
        h: 2,
        missing_content: true,
    };
    assert_eq!(Rendered::decode(&got.encode()), Ok(got));
}

#[test]
fn office_messages_travel_in_frames() {
    let mut stream = Vec::new();
    let table = every_message();
    for (n, m) in table.iter().enumerate() {
        crate::frame::write_frame(&mut stream, m.kind(), n as u32 * 2, &m.encode()).expect("under the cap");
    }
    let mut r = std::io::Cursor::new(stream);
    for (n, m) in table.iter().enumerate() {
        let frame = crate::frame::read_frame(&mut r).expect("a frame");
        assert_eq!(frame.id, n as u32 * 2);
        assert_eq!(Message::decode(frame.kind, &frame.payload).as_ref(), Ok(m));
    }
    assert_eq!(crate::frame::read_frame(&mut r), Err(crate::frame::FrameError::Closed));
}

#[test]
fn office_decode_refuses_truncated_payloads() {
    for m in every_message() {
        let payload = m.encode();
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
        let mut payload = m.encode();
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
    for kind in [0u16, 0x0100, 0x0107, 0x0200, 0x0207, 0x02FE, 0x0301, u16::MAX] {
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

    // CellData: the cells, widths, heights and merges vectors in turn.
    let empty_tail = |widths: u32, heights: u32, merges: u32| {
        let mut t = le32(widths).to_vec();
        t.extend_from_slice(&le32(heights));
        t.extend_from_slice(&le32(merges));
        t.extend_from_slice(&le32(0));
        t.extend_from_slice(&le32(0));
        t
    };
    let mut p = le32(1).to_vec();
    p.extend_from_slice(&[0; 8]); // only 8 of a cell's at least 14 bytes
    assert_eq!(CellData::decode(&p), Err(DecodeError::LengthPastEnd));

    // widths: 5 declared, the 16 bytes behind the count hold 4
    let mut p = le32(0).to_vec();
    p.extend_from_slice(&empty_tail(5, 0, 0));
    assert_eq!(CellData::decode(&p), Err(DecodeError::LengthPastEnd));
    // heights: 4 declared, the 12 bytes behind the count hold 3
    let mut p = le32(0).to_vec();
    p.extend_from_slice(&empty_tail(0, 4, 0));
    assert_eq!(CellData::decode(&p), Err(DecodeError::LengthPastEnd));
    // merges: 1 declared, 8 bytes behind the count, a merge takes 16
    let mut p = le32(0).to_vec();
    p.extend_from_slice(&empty_tail(0, 0, 1));
    assert_eq!(CellData::decode(&p), Err(DecodeError::LengthPastEnd));

    // A cell's text declared u32::MAX bytes long, with 16 bytes behind it.
    let mut p = le32(1).to_vec();
    p.extend_from_slice(&le32(0));
    p.extend_from_slice(&le32(0));
    p.extend_from_slice(&[1, 0]);
    p.extend_from_slice(&le32(u32::MAX));
    p.extend_from_slice(&[b'x'; 16]);
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
    for code in [0u16, 6, 7, 0x00FF, 0x0100, u16::MAX] {
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
        assert_eq!(Opened::decode(&opened.encode()), Err(DecodeError::NotFinite));

        let render = Render {
            page: 0,
            scale: bad,
            region: None,
            out: GRANT,
        };
        assert_eq!(Render::decode(&render.encode()), Err(DecodeError::NotFinite));

        let cells = CellData {
            cells: Vec::new(),
            col_widths: vec![10.0, bad],
            row_heights: Vec::new(),
            merges: Vec::new(),
            frozen: Frozen { rows: 0, cols: 0 },
        };
        assert_eq!(CellData::decode(&cells.encode()), Err(DecodeError::NotFinite));
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

    // A bool that is neither 0 nor 1.
    let mut p = le32(10).to_vec();
    p.extend_from_slice(&le32(10));
    p.push(2);
    assert_eq!(Rendered::decode(&p), Err(DecodeError::BadTag));

    // A cell's alignment and its reserved flag bits.
    let cell = |align: u8, flags: u8| {
        let mut p = le32(1).to_vec();
        p.extend_from_slice(&le32(0));
        p.extend_from_slice(&le32(0));
        p.push(align);
        p.push(flags);
        p.extend_from_slice(&le32(0));
        for _ in 0..5 {
            p.extend_from_slice(&le32(0));
        }
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
    };
    let cells = CellData {
        cells: vec![CellText {
            row: 0,
            col: 0,
            text: "geheim".to_owned(),
            align: Align::Left,
            bold: false,
        }],
        col_widths: vec![1.0],
        row_heights: vec![1.0],
        merges: Vec::new(),
        frozen: Frozen { rows: 0, cols: 0 },
    };
    let outline = OutlineData {
        entries: vec![OutlineEntry {
            level: 1,
            text: "Overname".to_owned(),
            page: 0,
        }],
    };
    let shown = format!("{sheets:?} {cells:?} {outline:?}");
    for secret in ["Salarissen", "geheim", "Overname"] {
        assert!(!shown.contains(secret), "{shown}");
    }
}

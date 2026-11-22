//! Tests: the live JSON viewer.

use super::*;

fn flat(r: &[Row]) -> Vec<(usize, String)> {
    r.iter()
        .map(|r| {
            (
                r.depth,
                match &r.key {
                    Some(k) => format!("{k}: {}", r.text),
                    None => r.text.clone(),
                },
            )
        })
        .collect()
}

#[test]
fn small_object_rows_and_indents() {
    let rows = parse(r#"{"a":1,"b":[true,null]}"#).unwrap();
    assert_eq!(
        flat(&rows),
        vec![
            (0, "{2}".to_string()),
            (1, "a: 1".to_string()),
            (1, "b: [2]".to_string()),
            (2, "true".to_string()),
            (2, "null".to_string()),
        ]
    );
}

#[test]
fn invalid_reports_line_and_column() {
    let e = parse("{\"a\":").unwrap_err();
    assert!(e.starts_with("Not valid JSON: line 1, column "), "{e}");
}

#[test]
fn caps_at_2000_rows_plus_more() {
    let src = format!("[{}]", vec!["1"; 5000].join(","));
    let rows = parse(&src).unwrap();
    assert_eq!(rows.len(), MAX_ROWS + 1);
    assert_eq!(rows.last().unwrap().text, "... 3001 more");
}

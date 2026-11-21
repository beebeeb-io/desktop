use super::parse;

#[test]
fn quoting() {
    let d = parse("a,b\n1,\"x,y\"\n2,\"say \"\"hi\"\"\"");
    assert_eq!(d.columns[0].name, "a");
    assert_eq!(d.columns[1].name, "b");
    assert_eq!(d.rows[0], vec!["1", "x,y"]);
    assert_eq!(d.rows[1], vec!["2", "say \"hi\""]);
    assert!(d.columns[0].numeric && !d.columns[1].numeric);
}

#[test]
fn semicolon_and_tab() {
    let d = parse("a;b\n1;2");
    assert_eq!(d.separator, "Semicolon-separated");
    assert_eq!(d.rows[0], vec!["1", "2"]);
    assert_eq!(parse("a\tb\n1\t2").separator, "Tab-separated");
}

#[test]
fn caps_rows() {
    let mut s = String::from("n\n");
    for i in 0..5000 {
        s.push_str(&format!("{i}\n"));
    }
    let d = parse(&s);
    assert_eq!(d.rows.len(), 1000);
    assert_eq!(d.total_rows, 5000);
}

#[test]
fn open_quote_no_panic() {
    let d = parse("a,b\n1,\"oops\n2,3");
    assert_eq!(d.total_rows, 1);
    let _ = parse("\"");
    let _ = parse("");
}

#[test]
fn crlf_and_1000_rows() {
    let d = parse("a,b\r\n1,\"x,y\"\r\n2,z\r\n");
    assert_eq!(d.rows.len(), 2);
    assert_eq!(d.rows[0], vec!["1", "x,y"]);
    let mut s = String::from("n\r\n");
    for i in 0..1000 {
        s.push_str(&format!("{i}\r\n"));
    }
    assert_eq!(parse(&s).rows.len(), 1000);
}

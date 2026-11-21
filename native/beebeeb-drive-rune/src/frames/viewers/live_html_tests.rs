use super::*;

fn words(p: &[Piece]) -> String {
    p.iter().map(|x| x.text.as_str()).collect()
}

#[test]
fn heading_para_list() {
    let b = to_blocks("<h1>T</h1><p>a <b>b</b></p><ul><li>x</li></ul>");
    assert_eq!(b.len(), 3);
    assert!(matches!(&b[0], Block::Heading(1, w) if words(w) == "T"));
    match &b[1] {
        Block::Para(w) => {
            assert_eq!(words(w), "a b");
            assert!(w[1].bold);
        }
        o => panic!("{o:?}"),
    }
    assert!(matches!(&b[2], Block::List(i) if i.len() == 1 && words(&i[0]) == "x"));
}

#[test]
fn script_dropped() {
    let b = to_blocks("<script>alert(1)</script><p>ok</p>");
    assert_eq!(b.len(), 1);
    assert!(matches!(&b[0], Block::Para(w) if words(w) == "ok"));
}

#[test]
fn img_alt() {
    let b = to_blocks("<img alt=\"cat\" src=\"https://x\">");
    assert!(matches!(&b[0], Block::Para(w) if words(w) == "[cat]"));
}

#[test]
fn unclosed_and_odd_input_do_not_panic() {
    assert_eq!(to_blocks("<p>hi").len(), 1);
    for s in ["<", "<p", "<a href=\"x", "&", "&#xZZ;", "<!--", "<script>", "a < b", "<pre>x", "</p></ul></li>", "é<é>"] {
        let _ = to_blocks(s);
    }
}

#[test]
fn entities_pre_and_links() {
    let b = to_blocks("<p>a &amp; b&nbsp;&#65;&#x42;</p><pre>x\n <b>y</b></pre><p><a href=\"u\">l</a></p>");
    assert!(matches!(&b[0], Block::Para(w) if words(w) == "a & b\u{a0}AB"));
    assert_eq!(b[1], Block::Code("x\n y".into()));
    assert!(matches!(&b[2], Block::Para(w) if w[0].link.as_deref() == Some("u")));
}

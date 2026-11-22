use super::*;

fn q(n: &str, mc: bool, ww: bool) -> FindQuery {
    FindQuery { needle: n.into(), match_case: mc, whole_word: ww }
}

#[test]
fn counts() {
    let t = "foo Foo food";
    assert_eq!(find_all(t, &q("foo", false, false)).len(), 3);
    assert_eq!(find_all(t, &q("foo", true, false)).len(), 2);
    assert_eq!(find_all(t, &q("foo", true, true)).len(), 1);
}

#[test]
fn whole_word_counts_two_without_case() {
    assert_eq!(find_all("foo Foo food", &q("foo", false, true)).len(), 2);
}

#[test]
fn unicode_boundaries() {
    let t = "É é";
    let m = find_all(t, &q("é", false, false));
    assert_eq!(m.len(), 2);
    for r in m {
        assert!(t.is_char_boundary(r.start) && t.is_char_boundary(r.end));
    }
}

#[test]
fn empty_needle() {
    assert!(find_all("abc", &q("", false, false)).is_empty());
}

#[test]
fn wraps() {
    let m = vec![2..3, 5..6, 8..9];
    assert_eq!(next_match(&m, 0, false), Some(0));
    assert_eq!(next_match(&m, 6, false), Some(2));
    assert_eq!(next_match(&m, 9, false), Some(0));
    assert_eq!(next_match(&m, 5, true), Some(0));
    assert_eq!(next_match(&m, 2, true), Some(2));
    assert_eq!(next_match(&[], 0, true), None);
}

#[test]
fn replace() {
    let (s, n) = replace_all("a b a", &q("a", true, false), "xx");
    assert_eq!((s.as_str(), n), ("xx b xx", 2));
    assert_eq!(replace_range("hello", 1..3, "J"), "hJlo");
}

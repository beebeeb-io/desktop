//! The thumbnail store's own rules, without an engine call: how much it holds and what goes first. What it asks
//! the engine, and whose pictures it hands out, is tested with the Photos source against the engine's fake accounts
//! (`photos_tests.rs`).

use super::*;

fn picture(bytes: usize, used: u64) -> Held {
    Held::Picture {
        pixels: Pixels::new(),
        bytes,
        used,
    }
}

fn key(n: usize) -> Key {
    (format!("file-{n}"), Ask::Picture(ThumbCut::Square))
}

fn pictures(inner: &Inner) -> usize {
    inner
        .held
        .values()
        .filter(|held| matches!(held, Held::Picture { .. }))
        .count()
}

#[test]
fn at_most_the_cap_of_pictures_is_held_and_the_ones_not_looked_at_for_longest_go() {
    let mut inner = Inner::default();
    for n in 0..MAX_PICTURES + 5 {
        inner.held.insert(key(n), picture(10, n as u64));
        inner.bytes += 10;
    }
    // What is no picture is never counted and never dropped for room.
    inner
        .held
        .insert(("a-video".into(), Ask::Length), Held::Length(Some(41)));
    inner.make_room();
    assert_eq!(pictures(&inner), MAX_PICTURES);
    assert_eq!(inner.bytes, MAX_PICTURES * 10);
    for n in 0..5 {
        assert!(!inner.held.contains_key(&key(n)), "the oldest five went");
    }
    assert!(inner.held.contains_key(&key(5)) && inner.held.contains_key(&key(MAX_PICTURES + 4)));
    assert!(inner.held.contains_key(&("a-video".to_string(), Ask::Length)));
}

#[test]
fn at_most_the_cap_of_bytes_is_held() {
    let mut inner = Inner::default();
    let each = MAX_BYTES / 4;
    for n in 0..6 {
        inner.held.insert(key(n), picture(each, n as u64));
        inner.bytes += each;
    }
    inner.make_room();
    assert_eq!(pictures(&inner), 4);
    assert!(inner.bytes <= MAX_BYTES);
    assert!(!inner.held.contains_key(&key(0)) && !inner.held.contains_key(&key(1)));
}

#[test]
fn a_look_under_another_stamp_drops_everything() {
    let mut inner = Inner::default();
    let turn = beebeeb_engine::account::SessionGeneration::for_test(7);
    let first: Stamp = (turn, Some("one@example.eu".into()));
    inner.look(&first);
    inner.held.insert(key(1), picture(10, 1));
    inner.bytes = 10;
    inner.wishes.push_back((key(2), first.clone()));
    inner.in_flight.push(key(3));
    inner.look(&first);
    assert_eq!(pictures(&inner), 1, "the same stamp keeps what is held");
    // The same session number, another account.
    let second: Stamp = (turn, Some("two@example.eu".into()));
    inner.look(&second);
    assert_eq!((pictures(&inner), inner.bytes), (0, 0));
    assert!(inner.wishes.is_empty() && inner.in_flight.is_empty());
}

#[test]
fn a_length_is_written_as_a_tile_writes_it() {
    assert_eq!(length_words(41), "0:41");
    assert_eq!(length_words(125), "2:05");
}

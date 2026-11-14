//! The thumbnail store's own rules, without an engine call: how much it holds and what goes first. What it asks
//! the engine, and whose pictures it hands out, is tested with the Photos source against the engine's fake accounts
//! (`photos_tests.rs`).

use super::*;

/// A picture a draw looked at `ago` ago.
fn picture(bytes: usize, ago: Duration) -> Held {
    let now = Instant::now();
    Held::Picture {
        pixels: Pixels::new(),
        bytes,
        used: now.checked_sub(ago).expect("a clock that has run this long"),
        fetched: now,
    }
}

fn key(n: usize) -> Key {
    (format!("file-{n}"), Ask::Picture(ThumbCut::Square))
}

fn put(inner: &mut Inner, n: usize, bytes: usize, ago: Duration) {
    inner.held.insert(key(n), picture(bytes, ago));
    inner.bytes += bytes;
}

const LONG_AGO: Duration = Duration::from_secs(30);

#[test]
fn at_most_the_cap_of_pictures_is_held_and_the_ones_not_looked_at_for_longest_go() {
    let mut inner = Inner::default();
    for n in 0..MAX_PICTURES + 5 {
        // The lower the number, the longer ago it was looked at.
        put(
            &mut inner,
            n,
            10,
            LONG_AGO + Duration::from_millis((MAX_PICTURES + 5 - n) as u64),
        );
    }
    // What is no picture is never counted and never dropped for room.
    inner.held.insert(("a-video".into(), Ask::Length), Held::Length(41));
    assert!(inner.make_room(Instant::now()));
    assert_eq!(inner.pictures(), MAX_PICTURES);
    assert_eq!(inner.bytes, MAX_PICTURES * 10);
    for n in 0..5 {
        assert!(
            !inner.held.contains_key(&key(n)),
            "the five not looked at for longest went"
        );
    }
    assert!(inner.held.contains_key(&key(5)) && inner.held.contains_key(&key(MAX_PICTURES + 4)));
    assert!(inner.held.contains_key(&("a-video".to_string(), Ask::Length)));
}

#[test]
fn at_most_the_cap_of_bytes_is_held() {
    let mut inner = Inner::default();
    let each = MAX_BYTES / 4;
    for n in 0..6 {
        put(&mut inner, n, each, LONG_AGO + Duration::from_millis(6 - n as u64));
    }
    assert!(inner.make_room(Instant::now()));
    assert_eq!(inner.pictures(), 4);
    assert!(inner.bytes <= MAX_BYTES);
    assert!(!inner.held.contains_key(&key(0)) && !inner.held.contains_key(&key(1)));
}

#[test]
fn a_picture_a_draw_still_looks_at_is_never_dropped_for_room() {
    let mut inner = Inner::default();
    // A grid with more pictures than the store holds, every one of them drawn just now.
    for n in 0..MAX_PICTURES {
        put(&mut inner, n, 10, Duration::ZERO);
    }
    let now = Instant::now();
    assert!(
        !inner.has_room(now),
        "full of pictures that are looked at: nothing more is asked for"
    );
    put(&mut inner, MAX_PICTURES, 10, Duration::ZERO);
    assert!(!inner.make_room(now), "and nothing is pushed out");
    assert_eq!(inner.pictures(), MAX_PICTURES + 1);
    // One that left the screen is the only one that goes.
    put(&mut inner, MAX_PICTURES + 1, 10, LONG_AGO);
    assert!(inner.has_room(now), "there is one to drop for a new picture");
    inner.make_room(now);
    assert!(!inner.held.contains_key(&key(MAX_PICTURES + 1)));
    assert_eq!(
        inner.pictures(),
        MAX_PICTURES + 1,
        "every picture that is looked at stayed"
    );
}

#[test]
fn wishes_are_fetched_in_the_order_they_were_drawn_and_one_no_draw_repeats_is_dropped() {
    let mut inner = Inner::default();
    let stamp: Stamp = (beebeeb_engine::account::SessionGeneration::for_test(3), None);
    let now = Instant::now();
    let long_ago = now.checked_sub(LONG_AGO).unwrap();
    // The first tile's wish was not repeated by any draw since: its tile left the screen.
    inner.wishes.push_back((key(0), stamp.clone(), long_ago));
    inner.wishes.push_back((key(1), stamp.clone(), now));
    inner.wishes.push_back((key(2), stamp.clone(), now));
    assert_eq!(
        inner.next_wish(now).map(|wish| wish.0),
        Some(key(1)),
        "the top of what is on screen first"
    );
    assert_eq!(inner.next_wish(now).map(|wish| wish.0), Some(key(2)));
    assert_eq!(inner.next_wish(now), None);
    assert!(
        inner.wishes.is_empty(),
        "the one that left the screen was dropped unfetched"
    );
}

#[test]
fn a_look_under_another_stamp_drops_everything() {
    let mut inner = Inner::default();
    let turn = beebeeb_engine::account::SessionGeneration::for_test(7);
    let first: Stamp = (turn, Some("one@example.eu".into()));
    inner.look(&first);
    put(&mut inner, 1, 10, Duration::ZERO);
    inner.wishes.push_back((key(2), first.clone(), Instant::now()));
    inner.in_flight.push(key(3));
    inner.look(&first);
    assert_eq!(inner.pictures(), 1, "the same stamp keeps what is held");
    // The same session number, another account.
    let second: Stamp = (turn, Some("two@example.eu".into()));
    inner.look(&second);
    assert_eq!((inner.pictures(), inner.bytes), (0, 0));
    assert!(inner.wishes.is_empty() && inner.in_flight.is_empty());
}

#[test]
fn a_length_is_written_as_a_tile_writes_it() {
    assert_eq!(length_words(41), "0:41");
    assert_eq!(length_words(125), "2:05");
}

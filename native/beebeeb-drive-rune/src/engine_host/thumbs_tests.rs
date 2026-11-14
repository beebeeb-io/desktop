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

fn stamp_of_test() -> Stamp {
    (beebeeb_engine::account::SessionGeneration::for_test(3), None)
}

/// A store filled to three quarters of the byte cap with pictures every draw looks at now.
fn three_quarters_full() -> Inner {
    let mut inner = Inner::default();
    for n in 0..3 {
        put(&mut inner, n, MAX_BYTES / 4, Duration::ZERO);
    }
    inner
}

/// A picture of `bytes` taken in as a fetch's answer.
fn arrives(inner: &mut Inner, n: usize, bytes: usize, now: Instant) {
    inner.take_in(key(n), Ok(picture(bytes, Duration::ZERO)), now);
}

#[test]
fn a_grid_over_the_byte_cap_is_fetched_until_it_is_full_and_then_left_alone() {
    const EACH: usize = 320 * 320 * 4;
    let fits = MAX_BYTES / EACH;
    let tiles = fits + 12;
    let stamp = stamp_of_test();
    let mut inner = Inner::default();
    let start = Instant::now();
    let (mut fetches, mut quiet_rounds) = (0, 0);
    for round in 0..30u32 {
        let now = start + Duration::from_millis(16) * round;
        // A draw: every tile looks at its picture, or wishes for one.
        for n in 0..tiles {
            if let Some(Held::Picture { used, .. }) = inner.held.get_mut(&key(n)) {
                *used = now;
            } else {
                inner.wish(key(n), stamp.clone(), now, false);
            }
        }
        let queued = inner.wishes.len();
        let mut fetched = 0;
        while let Some((wished, _)) = inner.next_wish(now) {
            fetched += 1;
            inner.take_in(wished, Ok(picture(EACH, Duration::ZERO)), now);
        }
        fetches += fetched;
        let arrived = std::mem::take(&mut inner.arrived);
        if round >= 10 {
            assert_eq!(queued, 0, "round {round}: the draw asked for pictures that cannot stay");
            assert_eq!(fetched, 0, "round {round}: something was fetched");
            assert!(!arrived, "round {round}: an arrival was reported");
            quiet_rounds += 1;
        }
    }
    assert_eq!(quiet_rounds, 20);
    assert!(fetches <= fits + WORKERS, "{fetches} fetches for room for {fits}");
    assert_eq!(inner.pictures(), fits);
    assert!(inner.bytes <= MAX_BYTES);
}

#[test]
fn a_picture_that_could_not_stay_is_not_reported() {
    let mut inner = three_quarters_full();
    let now = Instant::now();
    let before = inner.bytes;
    assert!(inner.has_room(now));
    arrives(&mut inner, 10, MAX_BYTES / 2, now);
    assert!(!inner.held.contains_key(&key(10)));
    assert_eq!(inner.bytes, before);
    assert!(!inner.arrived, "a picture that did not stay is not reported");
    assert!(!inner.has_room(now), "and nothing more is asked for");
}

#[test]
fn room_made_later_lets_the_grid_ask_again() {
    let stamp = stamp_of_test();
    // (a) a picture nobody looks at any more.
    let mut inner = three_quarters_full();
    let now = Instant::now();
    arrives(&mut inner, 10, MAX_BYTES / 2, now);
    assert!(!inner.has_room(now));
    if let Some(Held::Picture { used, .. }) = inner.held.get_mut(&key(0)) {
        *used = now.checked_sub(WISH_LIVES).expect("a clock that has run this long");
    }
    assert!(inner.has_room(now));
    assert!(inner.wish(key(11), stamp.clone(), now, false));
    let (wished, _) = inner.next_wish(now).expect("the wish is handed out");
    inner.take_in(wished, Ok(picture(MAX_BYTES / 2, Duration::ZERO)), now);
    assert!(inner.held.contains_key(&key(11)) && !inner.held.contains_key(&key(0)));
    assert!(inner.arrived);
    assert!(!inner.full, "a picture that left made room: the mark is gone");
    // (b) a file that was dropped.
    let mut inner = three_quarters_full();
    arrives(&mut inner, 10, MAX_BYTES / 2, now);
    assert!(!inner.has_room(now));
    inner.drop_file("file-1");
    assert!(inner.has_room(now));
    assert!(inner.wish(key(12), stamp, now, false));
    let (wished, _) = inner.next_wish(now).expect("the wish is handed out");
    inner.take_in(wished, Ok(picture(MAX_BYTES / 4, Duration::ZERO)), now);
    assert!(inner.held.contains_key(&key(12)));
    assert!(inner.arrived);
}

#[test]
fn a_refresh_that_cannot_stay_keeps_the_picture_that_was_held() {
    let stamp = stamp_of_test();
    let mut inner = three_quarters_full();
    let now = Instant::now();
    let before = inner.bytes;
    arrives(&mut inner, 0, MAX_BYTES * 3 / 4, now);
    assert!(
        matches!(inner.held.get(&key(0)), Some(Held::Picture { bytes, .. }) if *bytes == MAX_BYTES / 4),
        "the tile keeps the picture it had"
    );
    assert_eq!(inner.bytes, before);
    assert!(!inner.arrived);
    // Full: a held picture is still refreshed, one that is not held is not asked for.
    inner.wishes.push_back((key(50), stamp.clone(), now));
    inner.wishes.push_back((key(0), stamp, now));
    assert_eq!(inner.next_wish(now).map(|wish| wish.0), Some(key(0)));
    assert_eq!(inner.next_wish(now), None);
}

#[test]
fn a_fetch_that_failed_for_a_passing_reason_owes_one_redraw_and_no_more() {
    let mut inner = Inner::default();
    let t = Instant::now();
    inner.take_in(key(1), Err(ThumbRefused::Offline), t);
    assert!(inner.retry_pending());
    assert!(!inner.retry_due(t + TRY_AGAIN_AFTER - Duration::from_millis(1)));
    assert!(inner.retry_pending());
    assert!(inner.retry_due(t + TRY_AGAIN_AFTER));
    assert!(!inner.retry_due(t + TRY_AGAIN_AFTER));
    assert!(!inner.retry_pending());
    // The redraw failed too: nothing more is owed.
    let t2 = t + TRY_AGAIN_AFTER + Duration::from_secs(1);
    inner.take_in(key(1), Err(ThumbRefused::Failed), t2);
    assert!(!inner.retry_pending());
    assert!(!inner.retry_due(t2 + Duration::from_secs(3600)));
    // A picture at last.
    let t3 = t2 + Duration::from_secs(60);
    arrives(&mut inner, 1, 10, t3);
    assert!(matches!(inner.held.get(&key(1)), Some(Held::Picture { .. })));
    assert!(inner.arrived);
}

#[test]
fn what_cannot_get_better_by_waiting_owes_no_redraw() {
    let mut inner = Inner::default();
    let t = Instant::now();
    inner.take_in(key(1), Err(ThumbRefused::NoThumbnail), t);
    inner.take_in(key(2), Err(ThumbRefused::Unreadable), t);
    assert!(matches!(inner.held.get(&key(1)), Some(Held::Nothing { .. })));
    assert!(matches!(inner.held.get(&key(2)), Some(Held::Nothing { .. })));
    assert!(!inner.retry_pending());
    assert!(!inner.retry_due(t + Duration::from_secs(3600)));
    for (n, refused) in [
        (3, ThumbRefused::NotSignedIn),
        (4, ThumbRefused::SessionEnded),
        (5, ThumbRefused::NotYours),
    ] {
        inner.take_in(key(n), Err(refused), t);
        assert!(
            !inner.held.contains_key(&key(n)),
            "a refusal for the session keeps nothing"
        );
    }
    // A failed refresh keeps the picture and owes nothing.
    put(&mut inner, 6, 10, Duration::ZERO);
    inner.take_in(key(6), Err(ThumbRefused::Offline), t);
    assert!(matches!(inner.held.get(&key(6)), Some(Held::Picture { .. })));
    assert!(!inner.retry_pending());
}

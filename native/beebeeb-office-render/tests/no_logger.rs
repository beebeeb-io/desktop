//! The helper installs no logger: the Office engine's readers log part names and link targets taken from the file,
//! and with the level left off they go nowhere. Its own process (its own test binary), because `start()` installs
//! the process's panic hook: a failed assert here exits with code 3 and no message, still a red.

#[test]
fn the_helper_installs_no_logger() {
    beebeeb_office_render::start();
    assert_eq!(log::max_level(), log::LevelFilter::Off);
}

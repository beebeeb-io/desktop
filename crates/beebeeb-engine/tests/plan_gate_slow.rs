//! w33: a slow plan answer never holds up the start. The plan is asked in the background; the first pull and the
//! calls that publish do not wait for it, and the gate follows when the answer lands.
//! (One engine per test process: one test per file.)

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn a_slow_plan_answer_holds_up_neither_the_unlock_nor_the_first_listing() {
    use beebeeb_engine::signin::gate::GateState;
    use std::time::{Duration, Instant};

    let s = common::setup();
    s.account.set_plan_state("none");
    s.account.set_plan_delay_ms(4000);
    s.sign_in();

    let began = Instant::now();
    s.unlock();
    let folder = s.home.join("Beebeeb Test");
    s.engine.start_sync(folder).expect("the start");
    let rows = common::wait_for_rows(&s.state_db(), 7);
    let first_listing = began.elapsed();
    eprintln!("start-to-first-listing with a 4 s plan answer: {first_listing:?}");
    assert_eq!(rows, 7);
    assert!(
        first_listing < Duration::from_millis(2500),
        "waited on the plan: {first_listing:?}"
    );

    // The answer lands later: the gate follows, published and woken.
    assert!(
        common::wait_until(15, || s.engine.view().gate == GateState::NoPlan),
        "the plan answer is applied when it lands"
    );
    s.engine.stop();
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
